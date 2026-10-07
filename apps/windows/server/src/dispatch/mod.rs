//! 协议分派：把 DLL 发来的 [`ClientMessage`] 交给 Engine，产出回给 DLL 的 [`ServerMessage`]。
//! 消息分派在 [`message`]，会话在 [`session`]，组句展示状态在 [`composed`]，按键在 [`key`]，
//! 候选窗口输出在 [`candidates`]，状态条在 [`status`]，配置热加载在 [`reload`]，本地整句模型在 [`rescore`]。

mod candidates;
mod composed;
mod config;
mod key;
mod message;
mod reload;
mod rescore;
mod session;
mod status;
mod translate;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cloudime_core::Engine;
use cloudime_platform::SwitchKeys;
use cloudime_platform::protocol::{
    ClientMessage, Frame, IndicatorState, InputMode, InputSettings, ScreenRect, ServerMessage,
    SessionId,
};

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

    /// 屏幕提示（当前没有来源写入），随下一帧下发、下一次按键清。
    notice: Option<String>,

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
}

impl Router {
    pub fn new(engine: Engine, config: RouterConfig) -> Self {
        let mut engine = engine;
        // 中文模式的符号映射缺省表在配置里（Core 自己缺省是空表），在这里接上，测试与正式跑的是同一条路
        engine.set_punctuation_mapping(config.punctuation_mapping.clone());
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
        };
        // 把当前状态先记成基线：之后第一次真的变了才弹提示（否则第一次切换总被当成基线吞掉）。
        router.last_tip = Some(router.tip_state());
        // 启动时也要走一遍本地词典（`apply_config` 那条路只在配置**变了**时才走：
        // 少了这一句，Server 起来后要等到用户动一次设置才会加载词典）
        router.translate.configure(
            &router.config.translate_dictionary,
            router.config.translate_reset_counter,
        );
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
