//! Tab 与分页的三端约定。
use crate::dispatch::{Router, RouterConfig};
use cloudime_core::{CandidateKind, CustomPhrase, Engine};
use cloudime_dictionary::{Dictionary, WordList};
use cloudime_platform::protocol::{
    ClientMessage, Frame, KeyEvent, KeyModifiers, KeyOutcome, PROTOCOL_VERSION, ServerMessage,
    SessionId,
};

fn router(size: usize) -> Router {
    let mut engine = Engine::new(Dictionary::parse("你\tni\t100\n").unwrap()).with_english(
        WordList::parse("hello\thello\t100\nhelp\thelp\t90\nheld\theld\t80\n").unwrap(),
    );
    engine
        .set_custom_phrases(
            (1..=9)
                .map(|position| CustomPhrase {
                    code: "qq".into(),
                    text: format!("第{position}项"),
                    position: (position - 1) as u32,
                })
                .collect(),
        )
        .unwrap();
    let mut router = Router::new(
        engine,
        RouterConfig {
            page_size: size,
            ..Default::default()
        },
    );
    router.handle(ClientMessage::OpenSession {
        session: SessionId(1),
        app: None,
        protocol: PROTOCOL_VERSION,
    });
    router
}
fn key(
    router: &mut Router,
    code: u32,
    character: Option<char>,
    modifiers: KeyModifiers,
) -> (KeyOutcome, Option<String>, Frame) {
    match router
        .handle(ClientMessage::Key {
            session: SessionId(1),
            event: KeyEvent::new(code, character, modifiers),
        })
        .unwrap()
    {
        ServerMessage::KeyResult {
            outcome,
            commit,
            frame,
            ..
        } => (outcome, commit, frame),
        _ => panic!("key result"),
    }
}
fn compose(router: &mut Router, text: &str, modifiers: KeyModifiers) {
    for c in text.chars() {
        key(router, c as u32, Some(c), modifiers);
    }
}
#[test]
fn tab_and_backtab_page_boundaries_and_current_page_selection() {
    for size in [1, 4, 5, 9] {
        let mut router = router(size);
        let normal = KeyModifiers::default();
        let shift = KeyModifiers {
            shift: true,
            ..normal
        };
        assert_eq!(key(&mut router, 9, None, normal).0, KeyOutcome::Passthrough);
        assert_eq!(key(&mut router, 9, None, shift).0, KeyOutcome::Passthrough);
        compose(&mut router, "qq", normal);
        assert_eq!(key(&mut router, 9, None, shift).2.page, 0);
        let last = 9_usize.div_ceil(size) - 1;
        for page in 1..=last {
            let result = key(&mut router, 9, None, normal);
            assert_eq!(result.0, KeyOutcome::Consumed);
            assert_eq!((result.2.page, result.2.highlight), (page, 0));
        }
        let frame = key(&mut router, 9, None, normal).2;
        assert_eq!(frame.page, last);
        let expected = frame.candidates.items[0].text.clone();
        assert_eq!(
            key(&mut router, b'1' as u32, Some('1'), normal).1,
            Some(expected)
        );
        compose(&mut router, "qq", normal);
        key(&mut router, 0x22, None, normal);
        assert_eq!(key(&mut router, 9, None, shift).2.page, 0);
        assert_eq!(key(&mut router, 0x21, None, normal).2.page, 0);
    }
}
#[test]
fn tab_with_raw_input_and_no_candidates_is_consumed_without_commit() {
    let mut router = router(5);
    compose(&mut router, "zzzz", KeyModifiers::default());
    let result = key(&mut router, 9, None, KeyModifiers::default());
    assert_eq!(result.0, KeyOutcome::Consumed);
    assert_eq!(result.1, None);
    assert_eq!(result.2.page, 0);
}

#[test]
fn punctuation_commits_the_highlighted_candidate_first() {
    let normal = KeyModifiers::default();
    // 逗号：先上屏高亮候选 你，再上屏逗号（中文模式转全角）
    let mut r = router(5);
    compose(&mut r, "ni", normal);
    let result = key(&mut r, 0xBC, Some(','), normal);
    assert_eq!(result.0, KeyOutcome::Consumed);
    assert_eq!(result.1.as_deref(), Some("你，"));

    // 拼音分隔符 `'` 仍进缓冲区，不上屏候选
    let mut r = router(5);
    compose(&mut r, "ni", normal);
    assert_eq!(key(&mut r, 0xDE, Some('\''), normal).1, None);

    // 翻页键 `-` / `=`：只翻页，不上屏
    let mut r = router(5);
    compose(&mut r, "ni", normal);
    assert_eq!(key(&mut r, 0xBD, Some('-'), normal).1, None);
    assert_eq!(key(&mut r, 0xBB, Some('='), normal).1, None);

    // 上档的 `_` / `+`：沿用英文直输段，仍进缓冲区
    let mut r = router(5);
    compose(&mut r, "ni", normal);
    let shift = KeyModifiers {
        shift: true,
        ..normal
    };
    assert_eq!(key(&mut r, 0xBD, Some('_'), shift).1, None);
    assert_eq!(key(&mut r, 0xBB, Some('+'), shift).1, None);
}

#[test]
fn ctrl_digit_kills_the_slot_and_command_keys_stay_passthrough() {
    let normal = KeyModifiers::default();
    let ctrl = KeyModifiers {
        ctrl: true,
        ..normal
    };
    // 组句中 Ctrl+1：吃掉这一键、杀掉第一位候选（你），并重排（拼音还在）
    let mut r = router(5);
    compose(&mut r, "ni", normal);
    let result = key(&mut r, 0x31, Some('\u{1}'), ctrl);
    assert_eq!(result.0, KeyOutcome::Consumed);
    assert_eq!(result.1, None);
    assert_eq!(result.2.candidates.items[0].text, "你");
    assert!(result.2.notice.is_some());

    // 没有这一格（只有 1 个候选）时交还应用
    let mut r = router(5);
    compose(&mut r, "ni", normal);
    assert_eq!(
        key(&mut r, 0x32, Some('\u{2}'), ctrl).0,
        KeyOutcome::Passthrough
    );

    // 没在组句、或带 Alt 的组合：归应用
    let mut r = router(5);
    assert_eq!(
        key(&mut r, 0x31, Some('\u{1}'), ctrl).0,
        KeyOutcome::Passthrough
    );
    let mut r = router(5);
    compose(&mut r, "ni", normal);
    let alt = KeyModifiers { alt: true, ..ctrl };
    assert_eq!(
        key(&mut r, 0x31, Some('\u{1}'), alt).0,
        KeyOutcome::Passthrough
    );
}

#[test]
fn ctrl_digit_commits_a_custom_phrase_instead_of_killing() {
    let normal = KeyModifiers::default();
    let ctrl = KeyModifiers {
        ctrl: true,
        ..normal
    };
    let mut r = router(5);
    compose(&mut r, "qq", normal);
    // 第一位是短语「第1项」：Ctrl+1 直接上屏，而不是「杀」
    let result = key(&mut r, 0x31, Some('\u{1}'), ctrl);
    assert_eq!(result.0, KeyOutcome::Consumed);
    assert_eq!(result.1.as_deref(), Some("第1项"));
    // 整段输入被短语吃掉，组句结束
    assert!(result.2.candidates.items.is_empty());
}

#[test]
fn ctrl_enter_commits_the_raw_letters() {
    let normal = KeyModifiers::default();
    let ctrl = KeyModifiers {
        ctrl: true,
        ..normal
    };
    // 组句中 Ctrl+回车：原样上屏敲的字母并结束组句
    let mut r = router(5);
    compose(&mut r, "ni", normal);
    let result = key(&mut r, 0x0D, None, ctrl);
    assert_eq!(result.0, KeyOutcome::Consumed);
    assert_eq!(result.1.as_deref(), Some("ni"));
    assert!(result.2.candidates.items.is_empty());

    // 没在组句：归应用
    let mut r = router(5);
    assert_eq!(key(&mut r, 0x0D, None, ctrl).0, KeyOutcome::Passthrough);
}

#[test]
fn backtick_cycles_english_candidate_case() {
    let normal = KeyModifiers::default();
    let english = |frame: &Frame| {
        frame
            .candidates
            .items
            .iter()
            .find(|c| c.kind == CandidateKind::English)
            .map(|c| c.text.clone())
    };
    let mut r = router(9);
    compose(&mut r, "hello", normal);
    // 反引号：原样 → 全大写 → 首字母大写 → 原样，三轮都不上屏
    for expected in ["HELLO", "Hello", "hello"] {
        let result = key(&mut r, 0xC0, Some('`'), normal);
        assert_eq!(result.0, KeyOutcome::Consumed);
        assert_eq!(result.1, None);
        assert_eq!(english(&result.2).as_deref(), Some(expected));
    }
}
