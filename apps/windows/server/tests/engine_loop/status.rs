//! 悬浮状态条随模式与开关变化。

use cloudime_platform::protocol::InputMode;

use crate::support::*;

#[test]
fn status_bar_mode_click_changes_the_global_mode() {
    let mut router = router();
    let recorder = RecordingStatus::default();
    router.set_status_sink(Box::new(recorder.clone()));
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::Chinese,
    });

    // 点「中」：状态条翻成英文，之后每个 DLL 来取都拿到英文（全局一份，不是取一次就清）。
    router.handle_status_event(StatusEvent::ToggleLang);
    assert_eq!(recorder.shown().map(|view| view.english), Some(true));
    assert_eq!(synced_mode(&mut router, SESSION), Some(InputMode::English));
    assert_eq!(synced_mode(&mut router, SESSION), Some(InputMode::English));
    assert_eq!(
        synced_mode(&mut router, SessionId(2)),
        Some(InputMode::English)
    );
}

#[test]
fn status_bar_follows_mode_when_enabled() {
    let mut router = router();
    let recorder = RecordingStatus::default();
    router.set_status_sink(Box::new(recorder.clone()));

    // 中文 → 英文：各刷一次；会话关掉（应用退出）不收；切成别的输入法才收起。
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::Chinese,
    });
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::English,
    });
    router.handle(ClientMessage::CloseSession { session: SESSION });
    let modes: Vec<Option<bool>> = recorder
        .calls()
        .iter()
        .map(|call| call.map(|view| view.english))
        .collect();
    assert_eq!(modes, vec![Some(false), Some(true)]);

    router.handle(ClientMessage::ImeSwitched { session: SESSION });
    assert_eq!(recorder.calls().last(), Some(&None));
}

/// 禁用（系统的 Ctrl + Space 关）时收起状态条；再启用就回来。
#[test]
fn status_bar_hides_while_disabled() {
    let mut router = router();
    let recorder = RecordingStatus::default();
    router.set_status_sink(Box::new(recorder.clone()));
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::English,
    });
    assert_eq!(recorder.shown().map(|view| view.english), Some(true));

    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::Disabled,
    });
    assert_eq!(recorder.calls().last(), Some(&None));
    assert_eq!(synced_mode(&mut router, SESSION), Some(InputMode::Disabled));

    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::Chinese,
    });
    assert_eq!(recorder.shown().map(|view| view.english), Some(false));
}

#[test]
fn status_bar_toggles_char_width_and_scripts() {
    let mut router = router();
    let recorder = RecordingStatus::default();
    router.set_status_sink(Box::new(recorder.clone()));
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::Chinese,
    });

    // 「全角 / 半角」按钮：缺省半角，点一下全角，状态条与下发给 DLL 的按键设置都跟着变
    assert_eq!(
        recorder.shown().map(|view| view.full_width_chars),
        Some(false)
    );
    router.handle_status_event(StatusEvent::ToggleCharWidthType);
    assert_eq!(
        recorder.shown().map(|view| view.full_width_chars),
        Some(true)
    );
    assert!(synced_input(&mut router, SESSION).full_width_chars);
    router.handle_status_event(StatusEvent::ToggleCharWidthType);
    assert_eq!(
        recorder.shown().map(|view| view.full_width_chars),
        Some(false)
    );
    assert!(!synced_input(&mut router, SESSION).full_width_chars);

    // 「简 / 繁」按钮：状态条跟着变（输出繁简是 Core 的事，见 Core 的测试）
    router.handle_status_event(StatusEvent::ToggleSimpTrad);
    assert_eq!(recorder.shown().map(|view| view.traditional), Some(true));
    router.handle_status_event(StatusEvent::ToggleSimpTrad);
    assert_eq!(recorder.shown().map(|view| view.traditional), Some(false));
}

/// DLL 侧内置热键（`Shift + Space`、`Ctrl + Alt + .`）走同一条指示器通路，与状态条那两格等效。
#[test]
fn indicator_commands_toggle_the_same_switches() {
    use cloudime_platform::protocol::IndicatorCommand;

    let mut router = router();
    let recorder = RecordingStatus::default();
    router.set_status_sink(Box::new(recorder.clone()));
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::Chinese,
    });

    router.handle(ClientMessage::Indicator {
        session: SESSION,
        command: IndicatorCommand::ToggleCharWidthType,
    });
    assert_eq!(
        recorder.shown().map(|view| view.full_width_chars),
        Some(true)
    );
    assert!(synced_input(&mut router, SESSION).full_width_chars);

    router.handle(ClientMessage::Indicator {
        session: SESSION,
        command: IndicatorCommand::ToggleSimpTrad,
    });
    assert_eq!(recorder.shown().map(|view| view.traditional), Some(true));

    router.handle(ClientMessage::Indicator {
        session: SESSION,
        command: IndicatorCommand::TogglePunctuation,
    });
    assert_eq!(
        recorder.shown().map(|view| view.full_width_punctuation),
        Some(false)
    );
}

#[test]
fn mode_is_shared_by_every_app() {
    let mut router = router();
    let recorder = RecordingStatus::default();
    router.set_status_sink(Box::new(recorder.clone()));
    let other_app = SessionId(2);

    // 一个应用里切到英文：别的应用、之后新开的应用来取都是英文。
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::English,
    });
    assert_eq!(
        synced_mode(&mut router, other_app),
        Some(InputMode::English)
    );
    assert_eq!(
        synced_mode(&mut router, SessionId(3)),
        Some(InputMode::English)
    );

    // 切成别的输入法收起状态条；再有应用来取模式（又切回云朵输入法）就重新显示，模式照旧。
    router.handle(ClientMessage::ImeSwitched { session: SESSION });
    assert_eq!(recorder.calls().last(), Some(&None));
    assert_eq!(
        synced_mode(&mut router, other_app),
        Some(InputMode::English)
    );
    assert_eq!(recorder.shown().map(|view| view.english), Some(true));
}

/// 会话取一次 `SyncMode`，返回它拿到的全局状态。
fn synced_mode(router: &mut Router, session: SessionId) -> Option<InputMode> {
    match router.handle(ClientMessage::SyncMode { session }) {
        Some(ServerMessage::ModeSync { mode, .. }) => mode,
        other => panic!("SyncMode 应回 ModeSync，实际 {other:?}"),
    }
}

/// 会话取一次 `SyncMode`，返回随它下发的按键行为设置。
fn synced_input(
    router: &mut Router,
    session: SessionId,
) -> cloudime_platform::protocol::InputSettings {
    match router.handle(ClientMessage::SyncMode { session }) {
        Some(ServerMessage::ModeSync { input, .. }) => input,
        other => panic!("SyncMode 应回 ModeSync，实际 {other:?}"),
    }
}
