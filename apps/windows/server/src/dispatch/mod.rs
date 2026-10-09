//! 协议分派：把 DLL 发来的 [`ClientMessage`] 交给 Engine，产出回给 DLL 的 [`ServerMessage`]。
//! 消息分派在 [`message`]，会话在 [`session`]，组句展示状态在 [`composed`]，按键在 [`key`]，
//! 候选窗口输出在 [`candidates`]，状态条在 [`status`]，配置热加载在 [`reload`]，本地整句模型在 [`rescore`]。

mod candidates;
mod clipboard;
mod composed;
mod config;
mod document;
mod key;
mod message;
mod reload;
mod rescore;
mod script;
mod session;
mod status;
mod translate;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cloudime_core::Engine;
use cloudime_platform::SwitchKeys;
use cloudime_platform::protocol::{
    ClientMessage, Frame, IndicatorState, InputMode, InputSettings, ScreenRect, ServerMessage,
    SessionId,
};

use crate::ui::SharedMeasurer;
use document::Document;

pub use self::candidates::{CandidateEvent, CandidateSink, NoopSink, RenderSettings};
use self::composed::Composed;
pub use self::config::RouterConfig;
use self::reload::ConfigReload;
pub use self::reload::DataDirs;
pub use self::rescore::find_model;
use self::rescore::{ModelLoader, RescoreState};
use self::session::SessionInfo;
pub use self::status::{NoopStatusSink, StatusEvent, StatusSink, StatusView};
use self::translate::Translate;

/// 学习数据落盘间隔；Server 没有定时器，借消息节拍看时间。
const LEARNING_FLUSH_INTERVAL: Duration = Duration::from_secs(60);

/// 同一时刻只有一个应用有键盘焦点，所以一个 Engine 持当前组句；焦点切到别的会话时先清掉上一个的残留。
pub struct Router {
    /// 输入内核，进程内唯一。
    engine: Engine,

    /// 每页候选数 / 排布 / 外观 / 翻页键等。
    config: RouterConfig,

    /// 活跃会话及各自的宿主应用。
    sessions: HashMap<SessionId, SessionInfo>,

    /// 当前持有组句的会话。
    focused: Option<SessionId>,

    /// 当前组句的展示状态；没在组句时为 `None`。
    composed: Option<Composed>,

    /// 屏幕提示（当前只有脚本会写，见 `script`），随下一帧下发、下一次按键清。
    notice: Option<String>,

    /// 聚焦会话最近送来的光标前文（应用里已经输入、不在候选窗口里的那段文本）；组句结束就清。
    /// 给脚本看：`cloudime.context`。
    surrounding: String,

    /// 候选文本 → Core 的排名权重（`Query::weights`）：给脚本看 Core 的排序依据（`candidates` 载荷里的
    /// `weight`）。按文本存，所以脚本重排 / 改显示都不影响它。
    candidate_weights: HashMap<String, f64>,

    /// 当前高亮候选在布局里的下标（跨页）。
    highlight: usize,

    /// 展开「更多候选项」没有（组句里的 Tab 切）。只活在一次组句里：组句一结束就回 `false`。
    show_more: bool,

    /// 鼠标正指着哪一格候选（帧里的页内下标）。它指着的那一格又正好是高亮时，
    /// 再编辑拼音不把高亮拉回页首（理由见 [`Router::recompose`]）。鼠标移出候选窗就清掉。
    hover_cell: Option<usize>,

    /// 这轮查询里动过高亮：动过就不再拿重排结果换掉候选。
    navigated: bool,

    /// 上次把学习数据落盘的时间。
    last_flush: Instant,

    /// 配置热加载状态；`None` 表示不热加载。
    reload: Option<ConfigReload>,

    /// 候选窗口输出端；Windows 上由 [`crate::ui`] 注入。
    candidates: Box<dyn CandidateSink>,

    /// 悬浮状态条输出端；Windows 上由 [`crate::ui`] 注入。
    status: Box<dyn StatusSink>,

    /// 全局输入法状态（中文 / 英文 / 禁用），所有应用共用。DLL 切了报来，激活 / 获焦 / 轮询时取走。
    mode: InputMode,

    /// 当前输入法是不是云朵输入法：有 DLL 来取模式就是，切成别的输入法时收起。状态条只在这时显示；
    /// 应用退出不影响它，状态条是桌面常驻的。
    ime_active: bool,

    /// 前台会话最近报来的「焦点在可输入文本区域里」（DLL 每拍 `SyncMode` 带上）：状态切换提示据此决定弹不弹。
    in_text_input: bool,

    /// 前台会话最近报来的 Caps Lock 亮灭（Caps 的按键不经过 Server，只能由 DLL 带上来）。
    caps: bool,

    /// 上次观察到的状态切换提示状态：任一状态（中 / 英、Caps、全 / 半角、简 / 繁、中 / 西文标点）变了才弹。
    /// `None` 表示还没观察过（Server 刚起来不该冒一个提示）。
    last_tip: Option<self::status::TipState>,

    /// 最近一次拿到的光标矩形；组句结束后仍留着，状态切换提示贴它附近。
    last_caret: Option<ScreenRect>,

    /// 聚焦会话最近报来的光标矩形；云联想异步到达时按它原地重摆候选窗口。
    last_rect: Option<ScreenRect>,

    /// 上次真正显示的帧与位置：没变就不重画（组字期间的空转 Poll 很多）。
    last_shown: Option<(Frame, Vec<Option<char>>, ScreenRect)>,

    /// 本地整句模型（`.qjm` 或三件套目录）；没有模型文件为 `None`。
    model_path: Option<PathBuf>,

    /// 进行中的模型加载；加载完接到 Engine 上就清掉。
    model_loader: Option<ModelLoader>,

    /// 本地整句模型开没开（`[candidate] use_local_sentence_organization_model`）；变了才重载 / 卸载。
    applied_model: bool,

    /// 重排的防抖 / 轮询进行态。
    rescore: RescoreState,

    /// 本次按键要不要额外挪光标（成对补全把光标停到括号中间 / 跳过右半边），随 `KeyResult` 下发。
    caret_shift: i16,

    /// 本次按键上屏前要先删掉光标前几个字符（符号映射的两键规则），随 `KeyResult` 下发。
    delete_before: u16,

    /// 上一键成对补全补上的右半边（用户敲的那个键）；紧接着又敲它一下就是「跳过」。
    pending_close: Option<char>,

    /// 任务栏菜单点了「重启输入法服务」：`serve_pipe` 回完这条消息就退出，让新起的实例接管。
    restart_pending: bool,

    /// 鼠标点了候选窗、这个会话的 DLL 还没取走的要上屏文本；下一次 `Poll` 用
    /// `ServerMessage::Update::commit` 带回去（见 [`candidates::CandidateEvent`]）。
    pending_commit: Option<String>,

    /// 本地词典与翻译 Tip（候选窗底部那一行左侧）。
    translate: Translate,

    /// 用户脚本运行时：脚本目录（[`RouterConfig::scripts_dir`]）里的脚本在 [`Router::new`] 时加载，
    /// 之后每次按键派发一次事件（见 [`script`]）。
    scripts: cloudime_script::Runtime,

    /// 脚本 `cloudime.get_curr_config()` 读的那份配置快照：建 Router 时与每次热加载后刷新
    /// （[`Router::set_script_config`]）。存的是 `Config` 的一份拷贝，所以脚本拿到的总是当前那份。
    script_config: Rc<RefCell<cloudime_platform::Config>>,

    /// 脚本 `cloudime.foreground_app()` 读的「当前前台应用」快照：焦点一变就 [`Router::sync_focused_app`] 刷新。
    focused_app_snapshot: Rc<RefCell<Option<String>>>,

    /// 在线翻译（`Ctrl+T`）：候选窗底部单独一行的译文 / 等待 / 失败。
    online: translate::online::Online,

    /// 脚本在**候选窗开着时**要换的主题（`apply_theme` / 动作表 `theme`）：先挂起，等候选窗关掉
    /// 再补上（`script::flush_pending_theme`）—— 免得正在看候选的人被整屏重画打断。
    pending_theme: Option<script::ThemeCommand>,

    /// 本进程启动时配置里的主题（`[theme] curr_theme`）：脚本 `apply_theme(false)` 回退到它。
    theme_at_startup: String,

    /// 脚本的量尺（`cloudime.ui.measure`）：`(当前字体设置, 懒加载的量尺)`。
    /// 字体设置由 `apply_config` 刷新，量尺自己看到变了就重建。
    measure: Rc<RefCell<(RenderSettings, SharedMeasurer)>>,

    /// 输入框文本快照（脚本的 `cloudime.text.*`）：DLL 每段组句起始送一份，见 [`document`](self)。
    document: Document,
}

impl Router {
    pub fn new(engine: Engine, config: RouterConfig) -> Self {
        let mut engine = engine;
        // 先把脚本运行时建起来（`config` 下面要被整体挪进 RouterConfig）。
        let scripts = match config.scripts_dir.as_deref() {
            Some(dir) => cloudime_script::Runtime::load(dir, &config.script_disabled),
            None => cloudime_script::Runtime::none(),
        };
        // 脚本的量尺（`cloudime.ui.measure`）：懒加载的渲染器 + 当前字体设置（`apply_config` 会刷新），
        // 装进脚本运行时 —— 量出来的点宽与候选窗画出来的一致。
        let measure = Rc::new(RefCell::new((
            config.render_settings(),
            SharedMeasurer::default(),
        )));
        scripts.set_measure({
            let measure = measure.clone();
            Rc::new(move |text: &str, font: cloudime_script::MeasureFont| {
                let mut slot = measure.borrow_mut();
                let (settings, measurer) = &mut *slot;
                measurer.measure(settings, text, font)
            })
        });
        // 输入框文本（`cloudime.text.*`）：快照由 DLL 送（`Surrounding.document`）、按显示宽度切片
        // 都在 `Document` 里；脚本这边通过一个闭包问 —— 拿不到就是 `nil`，同时会请 DLL 下一次带上。
        let document = Document::default();
        scripts.set_text_hook({
            let document = document.clone();
            Rc::new(move |range, limit| document.slice(range, limit))
        });
        // 剪贴板（`cloudime.clipboard.*`）：Server 就在用户会话里，直接用 Windows 剪贴板 API
        scripts.set_clipboard(Rc::new(clipboard::set_text), Rc::new(clipboard::get_text));
        // 中文模式的符号映射缺省表在配置里（Core 自己缺省是空表），在这里接上，测试与正式跑的是同一条路
        engine.set_punctuation_mapping(config.punctuation_mapping.clone());
        // 本进程启动时配置里的主题：脚本 `apply_theme(false)` 回退到它
        let theme_at_startup = config.curr_theme.clone();
        let mut router = Self {
            engine,
            config: RouterConfig {
                page_size: config.page_size.max(1),
                ..config
            },
            sessions: HashMap::new(),
            focused: None,
            composed: None,
            notice: None,
            surrounding: String::new(),
            candidate_weights: HashMap::new(),
            highlight: 0,
            show_more: false,
            hover_cell: None,
            navigated: false,
            last_flush: Instant::now(),
            reload: None,
            candidates: Box::new(NoopSink),
            status: Box::new(NoopStatusSink),
            mode: InputMode::default(),
            ime_active: false,
            in_text_input: false,
            caps: false,
            last_tip: None,
            last_caret: None,
            last_rect: None,
            last_shown: None,
            model_path: None,
            model_loader: None,
            applied_model: false,
            rescore: RescoreState::default(),
            caret_shift: 0,
            delete_before: 0,
            pending_close: None,
            restart_pending: false,
            pending_commit: None,
            // 测试里不碰用户目录里的 `translate.db`：学习库不开，Tip 一律按「没学会」上色
            translate: if cfg!(test) {
                Translate::without_learning()
            } else {
                Translate::new()
            },
            online: translate::online::Online::default(),
            pending_theme: None,
            theme_at_startup,
            measure,
            document,
            scripts,
            script_config: Rc::new(RefCell::new(cloudime_platform::Config::default())),
            focused_app_snapshot: Rc::new(RefCell::new(None)),
        };
        // 脚本的 `cloudime.get_curr_config()`：读上面那份快照（`set_script_config` / 热加载时刷新）
        router.scripts.set_config_hook({
            let snapshot = router.script_config.clone();
            Rc::new(move |lua| script::config_table(lua, &snapshot.borrow()))
        });
        // 脚本的 `cloudime.foreground_app()`：读「当前前台应用」快照（焦点一变就刷新）
        router.scripts.set_app_hook({
            let snapshot = router.focused_app_snapshot.clone();
            Rc::new(move || snapshot.borrow().clone())
        });
        // 把当前状态先记成基线：之后第一次真的变了才弹提示（否则第一次切换总被当成基线吞掉）。
        router.last_tip = Some(router.tip_state());
        // 启动时也要走一遍本地词典（`apply_config` 那条路只在配置**变了**时才走：
        // 少了这一句，Server 起来后要等到用户动一次设置才会加载词典）
        router.translate.configure(
            &router.config.translate_dictionary,
            router.config.translate_reset_counter,
        );
        // 脚本都加载完了，通知一声：脚本里可以做准备了。
        router.dispatch_startup_to_scripts();
        router
    }

    /// 下发给 DLL 的按键行为设置：`OpenSession` 的回包带一次，之后每拍 `SyncMode` 也跟着走，
    /// 所以 DLL 不用自己读配置文件，配置改了也不用重开会话。
    ///
    /// 切换键与英文模式两项已不再由配置决定（设置页里没有对应选项了），这里恒发缺省值：
    /// 单击 Shift 切中英、内置英文模式开着，英文模式只由切换键与语言栏 / 状态条按钮进。
    /// `raw_input` 看这个会话的程序在不在「不显示候选框」名单里（`[candidate] program_list_of_hiding_candidate`）。
    pub(super) fn input_settings(&self, session: SessionId) -> InputSettings {
        let app = self
            .sessions
            .get(&session)
            .and_then(|info| info.app.as_deref());
        let raw_input = app.is_some_and(|app| self.config.hides_candidate_for(app));
        InputSettings {
            switch_mode: SwitchKeys::default(),
            english_mode: true,
            shift_letter_compose: true,
            full_width_chars: self.config.full_width_chars,
            raw_input,
            auto_disable_without_text_input: self.config.auto_disable_without_text_input,
            // 组合键脚本在清单里声明的那几套修饰键（`combination_modifiers`）的并集：DLL 据此把命中的
            // 组合键（组句与否都算）送来问一趟；一个这类脚本都没有时是 0，按键路径与以前逐字节一致。
            script_key_modifiers: self.scripts.combination_key_masks(),
            // `key` 触发脚本声明的具体按键（`keys`）：位图，DLL 据此在没组句时也送这些键。
            script_keys: config::script_key_bits(&self.scripts.wanted_keys()),
            // 有 `key` 触发脚本时让 DLL 每个按键前现读一份光标前文送上来（`cloudime.text.*`）。
            script_wants_text: self.scripts.wants_surrounding_text(),
        }
    }

    /// 任务栏图标右键菜单打勾用的开关状态，随 `ModeSync` 每一拍下发。
    pub(super) fn indicator_state(&self) -> IndicatorState {
        IndicatorState {
            full_width_punctuation: self.config.full_width_punctuation,
            english_full_width_punctuation: self.config.english_full_width_punctuation,
            update_available: self.update_available(),
        }
    }

    /// 焦点变了：把「当前前台应用」记进脚本那个只读快照（`cloudime.foreground_app()`）。
    fn sync_focused_app(&self) {
        *self.focused_app_snapshot.borrow_mut() = self.focused_app().map(str::to_owned);
    }

    /// 把当前配置给一份给脚本（`cloudime.get_curr_config`）：`main.rs` 建好 Router 时调一次，
    /// 之后每次配置热加载（`apply_config`）再刷新一次。
    pub fn set_script_config(&mut self, config: &cloudime_platform::Config) {
        *self.script_config.borrow_mut() = config.clone();
    }

    pub fn set_candidate_sink(&mut self, sink: Box<dyn CandidateSink>) {
        sink.configure(self.config.render_settings());
        self.candidates = sink;
    }

    /// 直接碰 Engine：测试里改模式键这类启动时才设的开关。
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    pub fn set_status_sink(&mut self, sink: Box<dyn StatusSink>) {
        self.status = sink;
    }

    /// 处理一条消息；`None` 表示不用回话。到点顺带把学习数据落盘。
    pub fn handle(&mut self, message: ClientMessage) -> Option<ServerMessage> {
        let response = self.dispatch(message);
        if self.last_flush.elapsed() >= LEARNING_FLUSH_INTERVAL {
            self.flush_learning();
        }
        response
    }

    pub fn flush_learning(&mut self) {
        self.engine.flush_learning();
        self.last_flush = Instant::now();
    }

    /// 取走「待重启」标志：任务栏菜单点了「重启输入法服务」，`serve_pipe` 回完这条消息就退出，
    /// 进程正常返回（日志刷盘），比在处理器里直接 `process::exit` 干净。
    pub fn take_restart_pending(&mut self) -> bool {
        std::mem::take(&mut self.restart_pending)
    }
}
