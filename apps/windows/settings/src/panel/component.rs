//! 根组件的 Reactor 生命周期：建状态、按消息落盘、画左侧导航 + 当前页。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use cloudime_platform::{
    Config, FullHalfPunctuation, ItemNumberStyle, LayoutMode, LogLevel, MAX_ASSOCIATION_COUNTS,
    MAX_CANDIDATE_COUNT, MAX_NEED_TIMES, MIN_ASSOCIATION_COUNTS, MIN_CANDIDATE_COUNT,
    MIN_NEED_TIMES, MO_HU_YIN_BITS, MouseWordSelection, PAIRWISE_COMPLETION_BITS,
    PUNCTUATION_MAPPING_BITS, PreeditMode, SimpTrad, ThemeColor, ThemeFile,
};
use windows_reactor::*;

use crate::color_dialog::ColorDialog;

use super::controls::{export_logs, log_dir, open_document, open_with_explorer};
use super::font_dialog;
use super::notice::Notice;
use super::pages::phrase::PhraseForm;
use super::pages::{dictionaries, phrase, scripts, theme, translate};
use super::window;
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

// 窗口的位置 / 尺寸记忆（位置恢复、居中兜底、失焦 / 关闭时落盘）都在 `super::window`。

/// 随包的使用手册：安装目录根的 `tutorial.md`（开发时就是仓库根那份，`bundled_root` 会退到那儿）。
/// 文件不在（老版本装的包）返回 `None`，调用方只记一条日志。
fn tutorial_path() -> Option<std::path::PathBuf> {
    let path = cloudime_platform::resources::bundled_root()?.join("tutorial.md");
    path.is_file().then_some(path)
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
        // 窗口的位置 / 尺寸记忆：读一次上次的几何 + 装窗口钩子，得赶在框架建窗之前装好（`create` 就够早）。
        window::install();
        let mut settings = Self {
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
            theme_names: Vec::new(),
            theme_selected: None,
            theme_draft: ThemeFile::default(),
            theme_new_name: String::new(),
            theme_status: String::new(),
            color_dialog: ColorDialog::new(Color::rgb(0, 0, 0)),
            color_slot: None,
        };
        // 主题：列一遍两个目录，再把配置里指定的那份读进草稿。
        settings.refresh_theme_names();
        let current = settings.config.theme.curr_theme.clone();
        let stem = current.trim().trim_end_matches(".json").to_owned();
        settings.select_theme(&stem);
        settings.theme_status.clear();
        settings
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

            // 主题页
            Message::ThemeSelect(Some(index)) => {
                if let Some(name) = self.theme_names.get(index).cloned() {
                    self.select_theme(&name);
                }
            }
            Message::ThemeRefresh => {
                self.refresh_theme_names();
                self.theme_status = "已重新扫描主题目录。".to_owned();
            }
            Message::ThemeNew => {
                self.theme_draft = Self::default_theme_draft();
                self.theme_selected = None;
                self.theme_new_name.clear();
                self.theme_status =
                    "已载入默认主题：改个名字、调颜色，点「确认保存」存下来，再点「应用主题」换上。".to_owned();
            }
            Message::ThemeNewName(text) => self.theme_new_name = text,
            Message::ThemeSave => self.save_theme(),
            Message::ThemeApply => self.apply_theme(),
            Message::ThemeImport => theme::import(self),
            Message::ThemeExport => theme::export(self),
            Message::ThemeColorOpen(slot) => {
                let color = slot.get(&self.theme_draft);
                self.color_dialog.open(Color {
                    a: color.a,
                    r: color.r,
                    g: color.g,
                    b: color.b,
                });
                self.color_slot = Some(slot);
            }
            Message::ThemeColorChanged(color) => self.color_dialog.set_color(color),
            Message::ThemeColorClosed(result) => {
                self.color_dialog.close();
                if result == ContentDialogResult::Primary
                    && let Some(slot) = self.color_slot.take()
                {
                    let color = self.color_dialog.color();
                    slot.set(
                        &mut self.theme_draft,
                        ThemeColor {
                            a: color.a,
                            r: color.r,
                            g: color.g,
                            b: color.b,
                        },
                    );
                }
            }

            // 下拉被清空 / 越界：不改
            _ => {}
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        // 窗口一出现就摆到上次的位置；没有记录（新用户）或那个位置已经不在屏幕上就居中。
        window::place_once();
        context.window_title("云朵输入法 设置");
        // 窗口尺寸要在「显示之前」就定好（见 `window::initial_client_size`）：第一次 publication 就给具体值，
        // 框架建窗时就用它，用户看不到「先宽后窄」。
        // WinUI 3 的标题栏图标不会自动取 exe 里嵌的资源，得显式给 `.ico` 文件路径。
        let (width, height) = window::initial_client_size();
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
            item("theme", "主题", Symbol::ViewAll),
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
