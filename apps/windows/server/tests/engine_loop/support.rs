//! 各模块共用的零件：造 Router、造按键、拆回话、假的状态条与打分器。

pub use std::path::PathBuf;
pub use std::sync::{Arc, Mutex};

pub use cloudime_core::sentence::SentenceScorer;
pub use cloudime_platform::PreeditMode;
pub use cloudime_platform::protocol::{
    ClientMessage, Frame, InputMode, KeyEvent, KeyModifiers, KeyOutcome, PROTOCOL_VERSION,
    ScreenRect, ServerMessage, SessionId,
};
pub use cloudime_windows_server::dispatch::{StatusEvent, StatusSink, StatusView};
pub use cloudime_windows_server::{AssemblySpec, Router, RouterConfig, assembly};

pub const SESSION: SessionId = SessionId(1);

/// Caps Lock 亮着。
pub const CAPS: KeyModifiers = KeyModifiers {
    ctrl: false,
    shift: false,
    alt: false,
    win: false,
    caps: true,
    english_mode: false,
};

/// 持久英文模式（Caps 灭）。
pub const ENGLISH: KeyModifiers = KeyModifiers {
    ctrl: false,
    shift: false,
    alt: false,
    win: false,
    caps: false,
    english_mode: true,
};

/// 样例词库装一个 Router，开好一个会话。
pub fn router() -> Router {
    router_with(RouterConfig::default())
}

pub fn router_with(config: RouterConfig) -> Router {
    router_in(config, None)
}

pub fn router_in(config: RouterConfig, app: Option<String>) -> Router {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let dict = root.join("assets/sample/dict.tsv");
    let engine =
        assembly::assemble(&AssemblySpec::new(dict)).expect("assemble engine from sample data");
    let mut router = Router::new(engine, config);
    // 协议版本与 Server 一致：开会话时把按键行为设置回一次（DLL 不读配置文件，靠它拿切换键）。
    open_session(&mut router, SESSION, app);
    router
}

/// 开一个会话并吃掉 Server 回的按键行为设置。
pub fn open_session(router: &mut Router, session: SessionId, app: Option<String>) {
    match router.handle(ClientMessage::OpenSession {
        session,
        app,
        protocol: PROTOCOL_VERSION,
    }) {
        Some(ServerMessage::SessionOpened { .. }) => {}
        other => panic!("expected SessionOpened, got {other:?}"),
    }
}

pub fn letter(c: char) -> KeyEvent {
    letter_with(c, Default::default())
}

/// `c` 的大小写就是 DLL 按 Shift 解析出的字符。
pub fn letter_with(c: char, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(c.to_ascii_uppercase() as u32, Some(c), modifiers)
}

pub fn press(router: &mut Router, event: KeyEvent) -> (KeyOutcome, Option<String>, Frame) {
    key_result(router.handle(ClientMessage::Key {
        session: SESSION,
        event,
    }))
}

/// 英文模式下敲一串字母，返回最后一次的处理结果。
pub fn type_english(router: &mut Router, text: &str) -> (KeyOutcome, Option<String>, Frame) {
    let mut last = None;
    for c in text.chars() {
        last = Some(press(router, letter_with(c, ENGLISH)));
    }
    last.expect("typed at least one letter")
}

pub fn candidate_texts(frame: &Frame) -> Vec<&str> {
    frame
        .candidates
        .items
        .iter()
        .map(|c| c.text.as_str())
        .collect()
}

pub fn digit(n: u32) -> KeyEvent {
    digit_with(n, Default::default())
}

/// 数字键 1–9；`character` 按 DLL 的解析：按着 Shift 是上档字符。
pub fn digit_with(n: u32, modifiers: KeyModifiers) -> KeyEvent {
    let c = if modifiers.shift {
        b")!@#$%^&*("[n as usize] as char
    } else {
        char::from_digit(n, 10).unwrap()
    };
    KeyEvent::new(0x30 + n, Some(c), modifiers)
}

pub const SHIFT: KeyModifiers = KeyModifiers {
    shift: true,
    ..ALT_OFF
};

pub const ALT_OFF: KeyModifiers = KeyModifiers {
    ctrl: false,
    shift: false,
    alt: false,
    win: false,
    caps: false,
    english_mode: false,
};

/// 带字符的按键（标点等），虚拟键码随便给一个 OEM 键。
pub fn punct(c: char) -> KeyEvent {
    KeyEvent::new(0xBE, Some(c), Default::default())
}

pub fn key_result(message: Option<ServerMessage>) -> (KeyOutcome, Option<String>, Frame) {
    match message {
        Some(ServerMessage::KeyResult {
            outcome,
            commit,
            frame,
            ..
        }) => (outcome, commit, frame),
        other => panic!("expected KeyResult, got {other:?}"),
    }
}

/// 一次按键的完整结果：`(吃不吃, 上屏文本, 光标位移, 先删几个字, 帧)`。
pub fn press_full(
    router: &mut Router,
    event: KeyEvent,
) -> (KeyOutcome, Option<String>, i16, u16, Frame) {
    match router.handle(ClientMessage::Key {
        session: SESSION,
        event,
    }) {
        Some(ServerMessage::KeyResult {
            outcome,
            commit,
            caret_shift,
            delete_before,
            frame,
            ..
        }) => (outcome, commit, caret_shift, delete_before, frame),
        other => panic!("expected KeyResult, got {other:?}"),
    }
}

/// 中文模式下敲一串字母，返回最后一次的处理结果。
pub fn type_letters(router: &mut Router, text: &str) -> (KeyOutcome, Option<String>, Frame) {
    let mut last = None;
    for c in text.chars() {
        last = Some(key_result(router.handle(ClientMessage::Key {
            session: SESSION,
            event: letter(c),
        })));
    }
    last.expect("typed at least one letter")
}

pub fn preedit(frame: &Frame) -> String {
    frame.preedit.iter().map(|s| s.text.as_str()).collect()
}

/// 记录状态条调用：`Some(视图)` 是显示（按当时的模式与开关）、`None` 是收起；另记状态切换提示。
#[derive(Clone, Default)]
pub struct RecordingStatus {
    calls: Arc<Mutex<Vec<Option<StatusView>>>>,
    tips: Arc<Mutex<Vec<(StatusView, bool, ScreenRect)>>>,
}

impl RecordingStatus {
    pub fn calls(&self) -> Vec<Option<StatusView>> {
        self.calls.lock().unwrap().clone()
    }

    /// 最后显示的那个视图（收起时是 `None`）。
    pub fn shown(&self) -> Option<StatusView> {
        self.calls().last().copied().flatten()
    }

    /// 收到过的状态切换提示：视图、Caps Lock 亮灭、光标矩形。
    pub fn tips(&self) -> Vec<(StatusView, bool, ScreenRect)> {
        self.tips.lock().unwrap().clone()
    }
}

impl StatusSink for RecordingStatus {
    fn show_status(&self, view: StatusView) {
        self.calls.lock().unwrap().push(Some(view));
    }

    fn hide_status(&self) {
        self.calls.lock().unwrap().push(None);
    }

    fn show_status_tip(&self, view: StatusView, caps: bool, anchor: ScreenRect) {
        self.tips.lock().unwrap().push((view, caps, anchor));
    }
}

pub fn function_key(virtual_key: u32) -> KeyEvent {
    KeyEvent::new(virtual_key, None, Default::default())
}

/// 假打分器：偏爱某个文本，其余都给低分（与 Core 的重打分测试同款）。
pub struct Prefers(pub &'static str);

impl SentenceScorer for Prefers {
    fn score(&self, _context: &str, texts: &[&str]) -> Vec<f64> {
        texts
            .iter()
            .map(|t| if *t == self.0 { -1.0 } else { -20.0 })
            .collect()
    }
}

/// 接了假模型的 Router：本地整句模型在壳里是异步接法，按键先按词级出候选，停顿后 tick 才换。
pub fn router_with_scorer(preferred: &'static str) -> Router {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut engine = assembly::assemble(&AssemblySpec::new(root.join("assets/sample/dict.tsv")))
        .expect("assemble engine from sample data");
    engine.set_async_sentence_scorer(Some(Box::new(Prefers(preferred))));
    let mut router = Router::new(engine, RouterConfig::default());
    router.handle(ClientMessage::OpenSession {
        session: SESSION,
        app: None,
        protocol: PROTOCOL_VERSION,
    });
    router
}

/// 一直 tick 到首选变成 `text` 或等满 `timeout`；返回最后一帧。
pub fn tick_until_first(router: &mut Router, text: &str, timeout: std::time::Duration) -> Frame {
    let started = std::time::Instant::now();
    loop {
        std::thread::sleep(router.next_tick().min(std::time::Duration::from_millis(20)));
        router.tick();
        let frame = match router.handle(ClientMessage::Poll { session: SESSION }) {
            Some(ServerMessage::Update { frame, .. }) => frame,
            other => panic!("expected Update, got {other:?}"),
        };
        if candidate_texts(&frame).first() == Some(&text) || started.elapsed() > timeout {
            return frame;
        }
    }
}

pub fn press_in(router: &mut Router, session: SessionId, event: KeyEvent) {
    let _ = router.handle(ClientMessage::Key { session, event });
}
