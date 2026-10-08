//! 根组件的 Reactor 生命周期：建状态、按消息落盘、画左侧导航 + 当前页。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, EnumWindows, GetWindowRect, GetWindowThreadProcessId, HCBT_ACTIVATE,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, SetWindowsHookExW, WH_CBT,
};
use windows::core::BOOL;

use cloudime_platform::{
    Config, FullHalfPunctuation, ItemNumberStyle, LayoutMode, LogLevel, MAX_ASSOCIATION_COUNTS,
    MAX_CANDIDATE_COUNT, MAX_NEED_TIMES, MIN_ASSOCIATION_COUNTS, MIN_CANDIDATE_COUNT,
    MIN_NEED_TIMES, MO_HU_YIN_BITS, MouseWordSelection, PAIRWISE_COMPLETION_BITS,
    PUNCTUATION_MAPPING_BITS, PreeditMode, SimpTrad,
};
use windows_reactor::*;

use super::controls::{export_logs, log_dir, open_document, open_with_explorer};
use super::font_dialog;
use super::notice::Notice;
use super::pages::phrase::PhraseForm;
use super::pages::{dictionaries, phrase, scripts, translate};
use super::{Message, REPOSITORY_URL, Settings};

/// 标题栏图标：exe 旁的 `cloudime.ico`（装机包装到 `{app}`，`build.rs` 也给开发时的 exe 旁拷一份）。
/// `WindowVisuals::icon` 只收 `&'static str`，所以算一次绝对路径后 `Box::leak` 成静态串；
/// 文件不在就没有图标（不报错）。
fn window_icon() -> Option<&'static str> {
    static ICON: OnceLock<Option<&'static str>> = OnceLock::new();
    *ICON.get_or_init(|| {
        let path = std::env::current_exe().ok()?.parent()?.join("cloudime.ico");
        if !path.is_file() {
            return None;
        }
        let leaked: &'static str = Box::leak(path.to_string_lossy().into_owned().into_boxed_str());
        Some(leaked)
    })
}

/// 居中只做一次（钩子与 `view` 里那条兜底都会调，不能各挪一次）。
static CENTERED: AtomicBool = AtomicBool::new(false);

/// 把窗口挪到它所在显示器的中央；已经挪过或挪不动返回 `false`。
fn center_window(hwnd: HWND) -> bool {
    if CENTERED.load(Ordering::Relaxed) {
        return false;
    }
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() || rect.right - rect.left <= 320 {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return false;
    }
    let work = info.rcWork;
    let x = work.left + ((work.right - work.left) - (rect.right - rect.left)) / 2;
    let y = work.top + ((work.bottom - work.top) - (rect.bottom - rect.top)) / 2;
    let moved = unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
    if moved.is_err() {
        return false;
    }
    CENTERED.store(true, Ordering::Relaxed);
    true
}

/// 框架建窗后组件只拿得到 `WindowVisuals`，那里面没有位置；等它再到组件里跑一趟（下一次 `view`）
/// 窗口**已经显示**了，挪过去会看到「先左后中」闪一下。所以挂一个本线程的 CBT 钩子：
/// `HCBT_ACTIVATE` 在窗口真正显示之前同步回调，在这里挪就看不到闪动。
fn install_center_hook() {
    unsafe extern "system" fn cbt(code: i32, wparam: WPARAM, _lparam: LPARAM) -> LRESULT {
        // HCBT_ACTIVATE：窗口即将被激活（还没显示），wparam 就是它的 hwnd。
        if code == HCBT_ACTIVATE as i32 {
            center_window(HWND(wparam.0 as *mut core::ffi::c_void));
        }
        unsafe { CallNextHookEx(None, code, wparam, _lparam) }
    }

    // 只钩本线程（也就是 UI 线程）的 CBT 事件：本进程自己的窗口，钩子过程不必放进 DLL。
    let hook = unsafe { SetWindowsHookExW(WH_CBT, Some(cbt), None, GetCurrentThreadId()) };
    if hook.is_err() {
        crate::log::warn("装居中钩子失败，窗口位置会晚一步才对上");
    }
    // 一直挂着不摘：除激活以外的 CBT 事件只做一次比较就返回，代价可以忽略。
}

/// 兜底：万一下一次 `view` 时窗口已经出现而钩子没生效，这里再挪一次。
fn center_window_once() {
    if CENTERED.load(Ordering::Relaxed) {
        return;
    }
    struct Probe {
        pid: u32,
        hwnd: HWND,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` 是下面传进来的 `&mut Probe`，回调期间一直有效。
        let probe = unsafe { &mut *(lparam.0 as *mut Probe) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid != probe.pid {
            return true.into();
        }
        let mut rect = RECT::default();
        // 框架可能还有别的本进程小窗口（消息窗之类）：只认够大的那个。
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() && rect.right - rect.left > 320 {
            probe.hwnd = hwnd;
            return false.into();
        }
        true.into()
    }

    let mut probe = Probe {
        pid: std::process::id(),
        hwnd: HWND(std::ptr::null_mut()),
    };
    unsafe {
        let _ = EnumWindows(
            Some(visit),
            LPARAM(std::ptr::from_mut(&mut probe).cast::<core::ffi::c_void>() as isize),
        );
    }
    if !probe.hwnd.0.is_null() {
        center_window(probe.hwnd);
    }
}

/// 设置窗口打开时的客户区尺寸（DIP）。
///
/// **尺寸必须赶在窗口建出来之前就声明**：框架是「建窗 → 应用 `WindowVisuals` → `Activate`（显示）」
/// 三步，只有第一次 publication 里就给具体值，窗口才会一出现就是最终大小；晚一步（等布局把尺寸
/// 回报上来再缩，那条路删掉了）就会看到「先按系统默认宽度闪一下、再缩」。
///
/// 那一刻窗口还不存在、量不到系统默认值（试过：第一次 `view` 时枚举本进程窗口，一个都没有），
/// 所以直接写死：本机（2560×1440、100% 缩放）系统给的默认客户区是 1912×1028，「宽取 2/3、
/// 高不变」即 1275×1028。小屏由 [`clamp_to_work_area`] 兜住。
const WINDOW_CLIENT_SIZE: (f64, f64) = (1275.0, 1028.0);

/// 随包的使用手册：安装目录根的 `tutorial.md`（开发时就是仓库根那份，`bundled_root` 会退到那儿）。
/// 文件不在（老版本装的包）返回 `None`，调用方只记一条日志。
fn tutorial_path() -> Option<std::path::PathBuf> {
    let path = cloudime_platform::resources::bundled_root()?.join("tutorial.md");
    path.is_file().then_some(path)
}

/// 把想要的客户区尺寸夹进主显示器工作区（DIP），别在小屏上顶出屏幕。
fn clamp_to_work_area((width, height): (f64, f64)) -> (f64, f64) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::HiDpi::GetDpiForSystem;
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };

    let mut rect = RECT::default();
    let params = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(std::ptr::from_mut(&mut rect).cast::<core::ffi::c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if params.is_err() {
        return (width, height);
    }
    let scale = f64::from(unsafe { GetDpiForSystem() }.max(96)) / 96.0;
    (
        width.min(f64::from(rect.right - rect.left) / scale),
        height.min(f64::from(rect.bottom - rect.top) / scale),
    )
}

impl Component for Settings {
    type Input = ();
    type Message = Message;

    fn create(_input: &(), _context: &ComponentContext<Self>) -> Self {
        let path = Self::config_path();
        Self::ensure_config_file(&path);
        // 旧版配置先迁一遍（幂等）；设置程序也可能在 Server 之前启动
        let migration = cloudime_platform::migrate::migrate(&path);
        if !migration.is_empty() {
            crate::log::info(format!(
                "旧版配置与短语已迁移：配置 {}，配置短语 {}，老短语库 {}",
                migration.config, migration.phrases, migration.legacy_phrases
            ));
        }
        let config = Config::load(&path).unwrap_or_default();
        let data_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let root = cloudime_platform::resources::bundled_root().unwrap_or_else(|| data_dir.clone());
        let (phrases, phrase_status) = phrase::load(&cloudime_platform::PhraseStore::locate(&root));
        // 窗口一出现就居中的钩子：得赶在框架建窗之前装好（`create` 就够早，那时窗口还没建）。
        install_center_hook();
        Self {
            config,
            path,
            page: "input".to_string(),
            notice: Notice::default(),
            dictionary_status: String::new(),
            program_query: None,
            phrases,
            phrase_form: PhraseForm::default(),
            phrase_edit: None,
            phrase_status,
            script_status: String::new(),
        }
    }

    fn update(&mut self, message: Message, _context: &ComponentContext<Self>) {
        match message {
            Message::Navigate(Some(tag)) => {
                self.page = tag;
                // 上一页的导入提示不跟着过来
                self.notice.clear();
            }
            Message::Navigate(None) => {}

            // 输入页
            Message::UseJianPin(on) => self.save("input", "use_jian_pin", on),
            Message::MoHuYin(index, on) => {
                if let Some((bit, _)) = MO_HU_YIN_BITS.get(index) {
                    let mut list = self.config.input.mo_hu_yin_list;
                    if on {
                        list |= bit;
                    } else {
                        list &= !bit;
                    }
                    self.save("input", "mo_hu_yin_list", i64::from(list));
                }
            }
            Message::SimpTrad(Some(i)) if i < SimpTrad::ALL.len() => {
                self.save(
                    "input",
                    "simp_trad_chinese_chars_toggle",
                    SimpTrad::ALL[i].key(),
                );
            }
            Message::MixtureInput(on) => self.save("input", "mixture_input", on),
            Message::FullHalfPunctuation(Some(i)) if i < FullHalfPunctuation::ALL.len() => {
                self.save(
                    "input",
                    "full_half_punctuation_marks_toggle",
                    FullHalfPunctuation::ALL[i].key(),
                );
            }
            Message::PairwiseCompletion(index, on) => {
                if let Some((bit, ..)) = PAIRWISE_COMPLETION_BITS.get(index) {
                    let mut mask = self.config.input.punctuation_marks_pairwise_completion;
                    if on {
                        mask |= bit;
                    } else {
                        mask &= !bit;
                    }
                    self.save(
                        "input",
                        "punctuation_marks_pairwise_completion",
                        i64::from(mask),
                    );
                }
            }
            Message::PunctuationMapping(index, on) => {
                if let Some((bit, ..)) = PUNCTUATION_MAPPING_BITS.get(index) {
                    let mut mask = self.config.input.punctuation_marks_mapping;
                    if on {
                        mask |= bit;
                    } else {
                        mask &= !bit;
                    }
                    self.save("input", "punctuation_marks_mapping", i64::from(mask));
                }
            }
            Message::HalfWideAfterDigit(on) => {
                self.save("input", "use_half_wide_punctuation_marks_after_digital", on)
            }
            Message::ShowStatusChangeTip(on) => self.save("input", "show_status_change_tip", on),

            // 候选页
            Message::LocalModel(on) => {
                self.save("candidate", "use_local_sentence_organization_model", on);
            }
            Message::Arrangement(Some(i)) if i < LayoutMode::ALL.len() => {
                self.save(
                    "candidate",
                    "candidate_arrangement_direction",
                    LayoutMode::ALL[i].key(),
                );
            }
            Message::CandidateCount(value) => {
                let count = (value.round() as i64)
                    .clamp(MIN_CANDIDATE_COUNT as i64, MAX_CANDIDATE_COUNT as i64);
                self.save("candidate", "candidate_count", count);
            }
            Message::AssociationCounts(value) => {
                let count = (value.round() as i64)
                    .clamp(MIN_ASSOCIATION_COUNTS as i64, MAX_ASSOCIATION_COUNTS as i64);
                self.save("candidate", "candidate_association_counts", count);
            }
            Message::PickFont(role) => {
                let current = role.current(&self.config.candidate).clone();
                if let Some(choice) = font_dialog::pick_font(&current) {
                    self.save_font(role, &choice);
                }
            }
            Message::ItemNumberStyle(Some(i)) if i < ItemNumberStyle::ALL.len() => {
                self.save(
                    "candidate",
                    "item_number_style",
                    ItemNumberStyle::ALL[i].key(),
                );
            }
            Message::CandidateBoxMinimumWidth(Some(value)) => {
                let width = (value.round() as i64).clamp(0, 2000);
                self.save("candidate", "candidate_box_minimum_width", width);
            }
            Message::ShowMoreCandidates(on) => {
                self.save("candidate", "show_more_candidate_items", on);
            }
            Message::MouseWordSelection(Some(i)) if i < MouseWordSelection::ALL.len() => {
                self.save(
                    "candidate",
                    "mouse_word_selection",
                    MouseWordSelection::ALL[i].key(),
                );
            }
            Message::CandidateItemMaximumWidth(Some(value)) => {
                let width = (value.round() as i64).clamp(0, 2000);
                self.save("candidate", "candidate_item_maximum_width", width);
            }
            Message::ProgramQuery(text) => self.program_query = Some(text),
            Message::ProgramAdd => {
                let name = self
                    .program_query
                    .take()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if !name.is_empty() {
                    let mut programs = self
                        .config
                        .candidate
                        .program_list_of_hiding_candidate
                        .clone();
                    if !self.config.candidate.hides_candidate_for(&name) {
                        programs.push(name);
                        self.save_array("candidate", "program_list_of_hiding_candidate", &programs);
                    }
                }
            }
            Message::ProgramRemove(name) => {
                let mut programs = self
                    .config
                    .candidate
                    .program_list_of_hiding_candidate
                    .clone();
                programs.retain(|program| !program.eq_ignore_ascii_case(&name));
                self.save_array("candidate", "program_list_of_hiding_candidate", &programs);
            }
            Message::Preedit(Some(i)) if i < PreeditMode::ALL.len() => {
                self.save("candidate", "preedit", PreeditMode::ALL[i].key());
            }

            // 词库页
            Message::RemoveWordBank(file) => dictionaries::remove(self, &file),
            Message::RareItems(on) => self.save("word_bank", "rare_items", on),
            Message::ImportDictionary => {
                dictionaries::import(self);
                self.reload();
            }

            // 短语页
            Message::UseDefaultPhrases(on) => {
                self.save("phrase", "use_default_phrases", on);
            }
            Message::PhraseCode(text) => self.phrase_form.code = text,
            Message::PhraseText(text) => self.phrase_form.text = text,
            Message::PhraseTitle(text) => self.phrase_form.title = text,
            Message::PhrasePosition(Some(value)) => self.phrase_form.position = value,
            Message::PhraseSave => phrase::save(self),
            Message::PhraseCancel => {
                self.phrase_form = PhraseForm::default();
                self.phrase_edit = None;
            }
            Message::PhraseEdit(index) => {
                if let Some(selected) = self.phrases.get(index) {
                    self.phrase_form = PhraseForm::from_phrase(selected);
                    self.phrase_edit = Some(index);
                    self.phrase_status.clear();
                }
            }
            Message::PhraseRemove(index) => phrase::remove(self, index),

            // 翻译页
            Message::TranslateEnabled(on) => self.save("translate", "enabled", on),
            Message::TranslateDictionary(Some(index)) => {
                let manifest = translate::manifest(self);
                if let Some(item) = manifest.items().get(index) {
                    self.save("translate", "dictionary", item.file.clone());
                }
            }
            Message::TranslateNeedTimes(value) => {
                let times = (value.round() as i64)
                    .clamp(i64::from(MIN_NEED_TIMES), i64::from(MAX_NEED_TIMES));
                self.save("translate", "need_times", times);
            }
            Message::ResetTranslateLearning => {
                // 设置程序只写配置文件：把这个计数加 1，Server 见到值变了就把学习记录清空
                let next = self.config.translate.reset_counter.wrapping_add(1);
                self.save("translate", "reset_counter", next as i64);
            }

            // 脚本页
            Message::ScriptToggle(file, on) => {
                // 名单里一律按文件名（大小写不敏感）：先摘掉再按需加回去，
                // 免得同一只脚本攒出两条不同大小写的记录。
                let mut disabled = self.config.script.disabled.clone();
                disabled.retain(|name| !name.eq_ignore_ascii_case(&file));
                if !on {
                    disabled.push(file);
                }
                self.save_array("script", "disabled", &disabled);
                self.script_status = "已在配置里记下，重启输入法服务后生效。".to_owned();
            }
            Message::ScriptRemove(file) => scripts::remove(self, &file),
            Message::ScriptEdit(file) => scripts::edit(self, &file),
            Message::ScriptNew => scripts::create(self),
            Message::RestartServer => {
                self.script_status = match crate::server::restart() {
                    Ok(()) => "已请输入法服务重启：几秒后新实例接管，正在打字的应用会重新连上。"
                        .to_owned(),
                    Err(message) => message,
                };
            }

            // 调试页（文件 / 日志 / 学习那几项，原「高级」页）
            Message::VerboseLog(on) => {
                let level = if on { LogLevel::Debug } else { LogLevel::Info };
                self.save("general", "log_level", level.key());
            }
            Message::InputLog(on) => self.save("general", "input_log", on),
            Message::Learning(on) => self.save("general", "learning", on),
            Message::OpenDataDir => {
                Self::ensure_config_file(&self.path);
                open_with_explorer(&self.data_dir().to_string_lossy());
            }
            Message::OpenLogDir => {
                if let Some(logs) = log_dir() {
                    open_with_explorer(&logs.to_string_lossy());
                }
            }
            Message::ExportLogs => export_logs(),
            Message::ClearInputLog => {
                let log = self.data_dir().join("input-log.jsonl");
                if let Err(error) = std::fs::remove_file(&log)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    crate::log::warn(format!("清空输入日志失败: {error}"));
                }
            }

            // 调试页：打开项目 GitHub 页面
            Message::OpenRepository => open_with_explorer(REPOSITORY_URL),

            // 调试页：用系统默认程序打开随包的使用手册；没有默认打开方式时退回记事本
            Message::OpenTutorial => match tutorial_path() {
                Some(path) => open_document(&path),
                None => crate::log::warn("找不到使用手册 tutorial.md（老版本装的包可能没有）"),
            },

            // 调试页
            Message::ShowStatusBar(on) => self.save("status_bar", "show_status_bar", on),
            Message::AutoHideFloatToolBar(on) => {
                self.save("debugging", "auto_hide_float_tool_bar", on);
            }
            Message::AutoDisableWithoutTextInput(on) => {
                self.save("debugging", "auto_disable_without_text_input", on);
            }
            Message::OpenComponents => {
                crate::log::info("「组件」页还没有做，点了只记一条日志");
            }

            // 下拉被清空 / 越界：不改
            _ => {}
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        // 窗口一出现就挪到屏幕中央（框架没有位置接口，只能自己来）。
        center_window_once();
        context.window_title("云朵输入法 设置");
        // 窗口尺寸要在「显示之前」就定好（见 WINDOW_CLIENT_SIZE）：第一次 publication 就给具体值，
        // 框架建窗时就用它，用户看不到「先宽后窄」。
        // WinUI 3 的标题栏图标不会自动取 exe 里嵌的资源，得显式给 `.ico` 文件路径。
        let (width, height) = clamp_to_work_area(WINDOW_CLIENT_SIZE);
        let mut visuals = WindowVisuals::new().client_size(width, height);
        if let Some(icon) = window_icon() {
            visuals = visuals.icon(icon);
        }
        context.window_visuals(visuals);
        let item = |tag: &str, label: &str, symbol| {
            KeyedView::new(
                tag,
                NavigationViewItem::new()
                    .tag(tag)
                    .is_selected(self.page == tag)
                    .slots([
                        SlotView::new(
                            NavigationViewItemSlot::Icon,
                            SymbolIcon::new().symbol(symbol),
                        ),
                        SlotView::new(NavigationViewItemSlot::Content, label),
                    ]),
            )
        };
        let items = [
            item("input", "输入", Symbol::Keyboard),
            item("candidates", "候选", Symbol::DockBottom),
            item("dictionaries", "词库", Symbol::Library),
            item("phrase", "短语", Symbol::Comment),
            item("translate", "翻译", Symbol::Character),
            item("scripts", "脚本", Symbol::Document),
            item("debugging", "调试", Symbol::Repair),
        ];
        NavigationView::new()
            .pane_display_mode(NavigationViewPaneDisplayMode::Left)
            .open_pane_length(220.0)
            .is_pane_open(true)
            .is_pane_toggle_button_visible(false)
            .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
            .is_settings_visible(false)
            .on_selected_tag_changed(context.callback(Message::Navigate))
            .slots([
                SlotView::collection(NavigationViewSlot::MenuItems, items),
                SlotView::new(NavigationViewSlot::Content, self.page_content(context)),
            ])
    }
}
