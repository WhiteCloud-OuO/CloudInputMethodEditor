//! 英文模式：纯直通（不出候选、不组句）、Caps 只管大小写、中英切换时的缓冲。

use crate::support::*;

#[test]
fn english_mode_is_pure_passthrough_with_no_candidates() {
    let mut router = router();
    // 字母直接交应用：不组句、不出候选，应用自己上屏（大小写也由应用按 Shift / Caps 算）。
    let (outcome, commit, frame) = type_english(&mut router, "hello");
    assert_eq!((outcome, commit), (KeyOutcome::Passthrough, None));
    assert!(frame.is_empty(), "英文模式不出候选也不组句：{frame:?}");
    // Shift 大小写一样直通。
    let shifted = KeyModifiers {
        shift: true,
        ..ENGLISH
    };
    let (outcome, commit, _) = press(&mut router, letter_with('H', shifted));
    assert_eq!((outcome, commit), (KeyOutcome::Passthrough, None));
    // 空格、回车、Tab、标点、数字都交给应用；英文那份缺省半角，逗号不转。
    for event in [
        KeyEvent::new(0x20, Some(' '), ENGLISH),
        function_key(0x0D),
        function_key(0x09),
        KeyEvent::new(0xBC, Some(','), ENGLISH),
        KeyEvent::new(0x31, Some('1'), ENGLISH),
    ] {
        let (outcome, commit, frame) = press(&mut router, event);
        assert_eq!((outcome, commit), (KeyOutcome::Passthrough, None));
        assert!(frame.is_empty());
    }
}

#[test]
fn caps_lock_only_changes_case_and_never_enters_english_mode() {
    let mut router = router();
    // 没在组句：Caps 亮着的字母交给应用，不出候选、也不切模式（中文照旧）。
    let (outcome, commit, frame) = press(&mut router, letter_with('H', CAPS));
    assert_eq!((outcome, commit), (KeyOutcome::Passthrough, None));
    assert!(frame.is_empty(), "Caps 不出候选：{frame:?}");
    // 中文模式照常出候选；标点也按中文模式转全角（Caps 不参与）。
    let (_, _, frame) = type_letters(&mut router, "ni");
    assert!(!frame.candidates.items.is_empty(), "拼音照常出候选");
    router.handle(ClientMessage::Commit { session: SESSION });
    let (outcome, commit, _) = press(&mut router, KeyEvent::new(0xBC, Some(','), CAPS));
    assert_eq!(
        (outcome, commit.as_deref()),
        (KeyOutcome::Consumed, Some("，"))
    );

    // 组句中 Caps 亮着敲字母：拼音先原样上屏，字母跟着一起插入（键被壳吃掉了，只能我们上屏）。
    type_letters(&mut router, "ni");
    let (_, commit, after) = press(&mut router, letter_with('A', CAPS));
    assert_eq!(commit.as_deref(), Some("niA"));
    assert!(after.is_empty());
}

#[test]
fn chinese_composition_is_flushed_when_english_mode_takes_over() {
    let mut router = router();
    type_letters(&mut router, "ni");
    // 切英文模式敲字母：先按敲的样子把拼音原样上屏，再连同这个字母一起插入。
    let (outcome, commit, frame) = press(&mut router, letter_with('a', ENGLISH));
    assert_eq!(
        (outcome, commit.as_deref()),
        (KeyOutcome::Consumed, Some("nia"))
    );
    assert!(frame.is_empty(), "缓冲区已清：{frame:?}");
}

#[test]
fn switching_to_chinese_mid_stream_starts_a_fresh_composition() {
    let mut router = router();
    // 英文模式没往缓冲区里放过字母，切回中文从头当拼音。
    type_english(&mut router, "hel");
    let (outcome, commit, frame) = press(&mut router, letter('l'));
    assert_eq!((outcome, commit), (KeyOutcome::Consumed, None));
    assert_eq!(preedit(&frame), "l");
}
