//! Tab 展开「更多候选项」与分页的三端约定。
use std::sync::{Arc, Mutex};

use crate::dispatch::{CandidateEvent, CandidateSink, RenderSettings, Router, RouterConfig};
use cloudime_core::{CandidateKind, CustomPhrase, Engine};
use cloudime_dictionary::{Dictionary, WordList};
use cloudime_platform::LayoutMode;
use cloudime_platform::protocol::{
    ClientMessage, Frame, KeyEvent, KeyModifiers, KeyOutcome, PROTOCOL_VERSION, ScreenRect,
    ServerMessage, SessionId,
};
use cloudime_translate::Sense;

use crate::speech;

/// 记下每次 `show` 的帧与矩形，验证「位置变了但帧没变」的重画。
#[derive(Default)]
struct Recorded {
    shows: Mutex<Vec<(Frame, ScreenRect)>>,
}

struct RecordingSink(Arc<Recorded>);

impl CandidateSink for RecordingSink {
    fn show(&self, frame: Frame, _badges: Vec<Option<char>>, rect: ScreenRect) {
        self.0.shows.lock().unwrap().push((frame, rect));
    }

    fn hide(&self) {}

    fn configure(&self, _settings: RenderSettings) {}
}

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
                    title: None,
                    position,
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
fn page_keys_move_by_page_and_boundaries_hold() {
    for size in [1, 4, 5, 9] {
        let mut router = router(size);
        let normal = KeyModifiers::default();
        // 没在组句：翻页键交给应用
        assert_eq!(
            key(&mut router, 0x22, None, normal).0,
            KeyOutcome::Passthrough
        );
        assert_eq!(
            key(&mut router, 0x21, None, normal).0,
            KeyOutcome::Passthrough
        );
        compose(&mut router, "qq", normal);
        assert_eq!(key(&mut router, 0x21, None, normal).2.page, 0);
        let last = 9_usize.div_ceil(size) - 1;
        for page in 1..=last {
            let result = key(&mut router, 0x22, None, normal);
            assert_eq!(result.0, KeyOutcome::Consumed);
            assert_eq!((result.2.page, result.2.highlight), (page, 0));
        }
        // 到底了再往下不动
        let frame = key(&mut router, 0x22, None, normal).2;
        assert_eq!(frame.page, last);
        let expected = frame.candidates.items[0].text.clone();
        assert_eq!(
            key(&mut router, b'1' as u32, Some('1'), normal).1,
            Some(expected)
        );
        // 新一段组句：上一页回到第一页
        compose(&mut router, "qq", normal);
        key(&mut router, 0x22, None, normal);
        assert_eq!(key(&mut router, 0x21, None, normal).2.page, 0);
    }
}

/// 组句里 Tab 展开「更多候选项」：一屏 = 5 × 候选项个数（竖排 5 列），序号不再画，
/// 数字键改成跳页、方向键在格子里走；再按一次收起。
#[test]
fn tab_expands_the_candidate_window() {
    let normal = KeyModifiers::default();
    let mut r = router(5);
    // 「展示更多候选项」缺省关，这里显式打开来测展开。
    r.config.show_more_candidate_items = true;
    compose(&mut r, "qq", normal);
    // 收起时一页 5 个、没有网格
    let expanded = key(&mut r, 9, None, normal).2;
    assert_eq!(expanded.columns, 5);
    // 一屏 5 × 5 = 25 格，9 个短语全进来了（不再分页）
    assert_eq!(expanded.candidates.items.len(), 9);
    assert_eq!((expanded.page, expanded.page_count), (0, 1));
    // 左→右、上→下铺：第 6 个在第二行第一格
    assert_eq!(expanded.candidates.items[5].text, "第6项");
    // 数字键改成跳页：没有第 3 页就夹到最后一页，仍然吃掉这一键
    assert_eq!(key(&mut r, 0x33, Some('3'), normal).0, KeyOutcome::Consumed);
    assert_eq!(key(&mut r, 0x30, Some('0'), normal).2.page, 0);
    // 展开时序号没了，Ctrl + 数字不再杀词，交还应用
    let ctrl = KeyModifiers {
        ctrl: true,
        ..normal
    };
    assert_eq!(
        key(&mut r, 0x31, Some('\u{1}'), ctrl).0,
        KeyOutcome::Passthrough
    );
    // 方向键在网格里走：高亮 0 往下是同一列下一格（第 1 个 → 第 6 个）
    assert_eq!(key(&mut r, 0x28, None, normal).2.highlight, 5);
    // 再按 Tab 收起：又回到一页 5 个，高亮落在它现在那一页（第 6 个 → 第 2 页首格）
    let collapsed = key(&mut r, 9, None, normal).2;
    assert_eq!(collapsed.columns, 0);
    assert_eq!((collapsed.page, collapsed.highlight), (1, 0));
    assert_eq!(collapsed.page_count, 2);
    assert_eq!(collapsed.candidates.items.len(), 4);
}

/// `[` `]` 挪拼音光标：展开态与两种收起态都一样（方向键在候选窗里让给了高亮与翻页）。
#[test]
fn brackets_move_the_preedit_cursor_in_every_state() {
    let normal = KeyModifiers::default();
    let cursor_after_brackets = |r: &mut Router| {
        let end = key(r, 0x23, None, normal).2.cursor; // End：光标到末尾
        let left = key(r, 0xDB, Some('['), normal);
        assert_eq!(left.0, KeyOutcome::Consumed);
        assert_eq!(left.1, None, "`[` 是挪光标、不上屏标点");
        assert_ne!(left.2.cursor, end, "`[` 把拼音光标往左挪");
        assert_eq!(key(r, 0xDD, Some(']'), normal).2.cursor, end, "`]` 挪回来");
    };

    // 展开态
    let mut r = router(5);
    compose(&mut r, "ni'hao", normal);
    key(&mut r, 9, None, normal);
    cursor_after_brackets(&mut r);

    // 竖排收起
    let mut r = router(5);
    compose(&mut r, "ni'hao", normal);
    cursor_after_brackets(&mut r);

    // 横排收起
    let mut r = router(5);
    r.config.layout = LayoutMode::Horizontal;
    compose(&mut r, "ni'hao", normal);
    cursor_after_brackets(&mut r);
}
/// 鼠标点在候选窗上：**收起态与展开态都能点着上屏、都能悬停跟手**。
#[test]
fn mouse_click_and_hover_work_in_both_states() {
    let normal = KeyModifiers::default();
    let poll_commit = |r: &mut Router| match r
        .handle(ClientMessage::Poll {
            session: SessionId(1),
        })
        .unwrap()
    {
        ServerMessage::Update { frame, commit, .. } => (frame, commit),
        _ => panic!("update"),
    };

    // 收起态：点第 2 格 → 上屏「第2项」，组句结束
    let mut r = router(5);
    compose(&mut r, "qq", normal);
    r.handle_candidate_event(CandidateEvent::Commit(1));
    let (frame, commit) = poll_commit(&mut r);
    assert_eq!(commit.as_deref(), Some("第2项"));
    assert!(frame.is_empty());

    // 展开态：点第 7 格（5 列网格的第二行第二格）
    let mut r = router(5);
    compose(&mut r, "qq", normal);
    key(&mut r, 9, None, normal);
    r.handle_candidate_event(CandidateEvent::Commit(6));
    assert_eq!(poll_commit(&mut r).1.as_deref(), Some("第7项"));

    // 悬停：收起态与展开态都跟手
    let mut r = router(5);
    compose(&mut r, "qq", normal);
    r.handle_candidate_event(CandidateEvent::Hover(3));
    assert_eq!(poll_commit(&mut r).0.highlight, 3);
    key(&mut r, 9, None, normal);
    r.handle_candidate_event(CandidateEvent::Hover(1));
    assert_eq!(poll_commit(&mut r).0.highlight, 1);
}

/// 鼠标指着某一格、高亮也在那一格时，再编辑拼音不把高亮拉回第 1 格（收起态与展开态一样）。
/// 鼠标移出候选窗后恢复原样。
#[test]
fn typing_keeps_the_highlight_under_the_mouse() {
    let normal = KeyModifiers::default();
    let poll_highlight = |r: &mut Router| match r
        .handle(ClientMessage::Poll {
            session: SessionId(1),
        })
        .unwrap()
    {
        ServerMessage::Update { frame, .. } => frame.highlight,
        _ => panic!("update"),
    };
    // 六个「ni」候选：鼠标指第 4 格，退格后候选还在（`n` 也出这几个），能看出高亮有没有被拉回去
    let make = || {
        let engine = Engine::new(
            Dictionary::parse(
                "你\tni\t100\n拟\tni\t90\n尼\tni\t80\n呢\tni\t70\n泥\tni\t60\n倪\tni\t50\n",
            )
            .unwrap(),
        );
        let mut r = Router::new(
            engine,
            RouterConfig {
                page_size: 5,
                ..Default::default()
            },
        );
        r.handle(ClientMessage::OpenSession {
            session: SessionId(1),
            app: None,
            protocol: PROTOCOL_VERSION,
        });
        r
    };

    for expanded in [false, true] {
        let mut r = make();
        compose(&mut r, "ni", normal);
        if expanded {
            key(&mut r, 9, None, normal);
        }
        r.handle_candidate_event(CandidateEvent::Hover(3));
        assert_eq!(poll_highlight(&mut r), 3, "展开={expanded}");
        // 编辑拼音（退格）：高亮钉在鼠标那一格
        assert_eq!(key(&mut r, 0x08, None, normal).2.highlight, 3);
        // 鼠标移出候选窗：不再钉着，编辑拼音照常归零
        r.handle_candidate_event(CandidateEvent::HoverLeft);
        assert_eq!(key(&mut r, 0x49, Some('i'), normal).2.highlight, 0);
    }
}

#[test]
fn right_click_does_nothing_without_a_dictionary() {
    let normal = KeyModifiers::default();
    let poll = |r: &mut Router| match r
        .handle(ClientMessage::Poll {
            session: SessionId(1),
        })
        .unwrap()
    {
        ServerMessage::Update { frame, commit, .. } => (frame, commit),
        _ => panic!("update"),
    };
    let make = |enabled: bool| {
        let engine = Engine::new(Dictionary::parse("你\tni\t100\n").unwrap());
        let mut r = Router::new(
            engine,
            RouterConfig {
                page_size: 5,
                translate_enabled: enabled,
                ..Default::default()
            },
        );
        r.handle(ClientMessage::OpenSession {
            session: SessionId(1),
            app: None,
            protocol: PROTOCOL_VERSION,
        });
        r
    };
    // 右键 = Ctrl + 反引号：这里既没选词典、词条也查不到，两条路径都该什么都不做（不 panic、不动组句）
    for enabled in [false, true] {
        let mut r = make(enabled);
        compose(&mut r, "ni", normal);
        r.handle_candidate_event(CandidateEvent::Translate(Some(0)));
        r.handle_candidate_event(CandidateEvent::Translate(Some(99)));
        r.handle_candidate_event(CandidateEvent::Translate(None));
        let (frame, commit) = poll(&mut r);
        assert_eq!(commit, None, "enabled={enabled}");
        assert_eq!(frame.candidates.items.len(), 1, "enabled={enabled}");
        assert_eq!(frame.highlight, 0, "enabled={enabled}");
    }
}

/// 释义选择那一屏（`Ctrl + 反引号` / 右键之后）：方向键挪高亮、空格选高亮那条上屏、
/// 数字选第几条、Esc 退出。真机上要一份真词典才进得去，这里直接把选择界面摆进 Router。
#[test]
fn the_sense_chooser_moves_with_arrows_and_picks_with_space() {
    let normal = KeyModifiers::default();
    let sense = |pos: &str, text: &str| Sense {
        pos: Some(pos.to_owned()),
        text: text.to_owned(),
        reading: None,
    };
    let chooser = || {
        let mut r = router(5);
        compose(&mut r, "ni", normal);
        r.translate.begin_choices(
            0,
            "你".to_owned(),
            vec![
                sense("adj.", "sad"),
                sense("n.", "sorrow"),
                sense("v.", "grieve"),
            ],
        );
        r
    };
    let highlight = |r: &Router| r.self_drawn_frame().highlight;

    // 一进选择界面：高亮在第一条，自绘帧换成释义（Tip 没了、页码没了、展开态也收回一列）
    let mut r = chooser();
    key(&mut r, 9, None, normal);
    let frame = r.self_drawn_frame();
    assert_eq!((frame.highlight, frame.columns), (0, 0));
    assert!(frame.tip.is_none());
    assert_eq!(frame.tip_choices.as_ref().unwrap().senses.len(), 3);

    // 方向键：四个方向都挪高亮，到两头就不动
    assert_eq!(key(&mut r, 0x28, None, normal).0, KeyOutcome::Consumed);
    assert_eq!(highlight(&r), 1);
    assert_eq!(key(&mut r, 0x27, None, normal).0, KeyOutcome::Consumed);
    assert_eq!(highlight(&r), 2);
    assert_eq!(key(&mut r, 0x28, None, normal).0, KeyOutcome::Consumed);
    assert_eq!(highlight(&r), 2);
    assert_eq!(key(&mut r, 0x26, None, normal).0, KeyOutcome::Consumed);
    assert_eq!(key(&mut r, 0x25, None, normal).0, KeyOutcome::Consumed);
    assert_eq!(highlight(&r), 0);

    // 空格：上屏高亮那条，选择界面跟着关掉
    key(&mut r, 0x28, None, normal);
    let result = key(&mut r, 0x20, Some(' '), normal);
    assert_eq!(result.0, KeyOutcome::Consumed);
    assert_eq!(result.1.as_deref(), Some("sorrow"));
    assert!(r.translate.choices().is_none());

    // 数字键照旧直接选第几条
    let mut r = chooser();
    assert_eq!(
        key(&mut r, 0x33, Some('3'), normal).1.as_deref(),
        Some("grieve")
    );

    // Esc 退出，什么也不上屏
    let mut r = chooser();
    assert_eq!(key(&mut r, 0x1B, None, normal).0, KeyOutcome::Consumed);
    assert!(r.translate.choices().is_none());
    assert!(r.self_drawn_frame().tip_choices.is_none());
}

/// 一条释义直接念、多条不念（多条要进选择界面里一条条念）。
#[test]
fn only_a_single_sense_is_spoken_outside_the_chooser() {
    let sense = |text: &str| Sense {
        pos: None,
        text: text.to_owned(),
        reading: None,
    };
    let one = [sense("sorrow")];
    let many = [sense("sad"), sense("sorrow")];
    assert_eq!(
        crate::dispatch::translate::single_sense(Some(&one)),
        Some("sorrow")
    );
    assert_eq!(crate::dispatch::translate::single_sense(Some(&many)), None);
    assert_eq!(crate::dispatch::translate::single_sense(Some(&[])), None);
    assert_eq!(crate::dispatch::translate::single_sense(None), None);
}

/// `Shift + 反引号` 发音：念高亮候选的译文。一条释义直接念；多条要进选择界面后念高亮那条；
/// 没有译文就没动作；关掉翻译 Tip 时这一键不认，照旧打 `~`。
#[test]
fn shift_backquote_speaks_the_sense() {
    let normal = KeyModifiers::default();
    let shift = KeyModifiers {
        shift: true,
        ..normal
    };
    let sense = |text: &str| Sense {
        pos: None,
        text: text.to_owned(),
        reading: None,
    };
    let make = |enabled: bool| {
        let mut r = Router::new(
            Engine::new(Dictionary::parse("你\tni\t100\n").unwrap()),
            RouterConfig {
                page_size: 5,
                translate_enabled: enabled,
                ..Default::default()
            },
        );
        r.handle(ClientMessage::OpenSession {
            session: SessionId(1),
            app: None,
            protocol: PROTOCOL_VERSION,
        });
        r
    };

    // 没有译文（这里根本没选词典）：吃掉这一键，什么也不念、也不上屏
    let mut r = make(true);
    compose(&mut r, "ni", normal);
    speech::take_recorded();
    let result = key(&mut r, 0xC0, Some('~'), shift);
    assert_eq!(result.0, KeyOutcome::Consumed);
    assert_eq!(result.1, None);
    assert_eq!(speech::take_recorded(), None);

    // 多条释义：候选窗里不念，进了选择界面才念高亮那条
    r.translate.begin_choices(
        0,
        "你".to_owned(),
        vec![sense("sad"), sense("sorrow"), sense("grieve")],
    );
    assert_eq!(key(&mut r, 0xC0, Some('~'), shift).0, KeyOutcome::Consumed);
    assert_eq!(speech::take_recorded().as_deref(), Some("sad"));
    key(&mut r, 0x28, None, normal);
    key(&mut r, 0xC0, Some('~'), shift);
    assert_eq!(speech::take_recorded().as_deref(), Some("sorrow"));
    // 发音只是念，不选也不退出选择界面
    assert!(r.translate.choices().is_some());

    // 关掉翻译 Tip：这一键不认，与不按 Shift 打 `~` 的结果一样
    let mut plain_router = make(false);
    compose(&mut plain_router, "ni", normal);
    let plain = key(&mut plain_router, 0xC0, Some('~'), normal).1;
    let mut shifted_router = make(false);
    compose(&mut shifted_router, "ni", normal);
    assert_eq!(key(&mut shifted_router, 0xC0, Some('~'), shift).1, plain);
    assert_eq!(speech::take_recorded(), None);
}

/// 鼠标中键 = `Shift + 反引号`：念高亮候选的译文。没有译文、或关掉翻译 Tip 时都不出声；
/// 在释义选择界面里念高亮那条（不选、不退出）。
#[test]
fn middle_click_speaks_like_shift_backquote() {
    let normal = KeyModifiers::default();
    let sense = |text: &str| Sense {
        pos: None,
        text: text.to_owned(),
        reading: None,
    };
    let make = |enabled: bool| {
        let mut r = Router::new(
            Engine::new(Dictionary::parse("你\tni\t100\n").unwrap()),
            RouterConfig {
                page_size: 5,
                translate_enabled: enabled,
                ..Default::default()
            },
        );
        r.handle(ClientMessage::OpenSession {
            session: SessionId(1),
            app: None,
            protocol: PROTOCOL_VERSION,
        });
        r
    };

    // 没有译文：不出声，也没动组句（候选还在）
    let mut r = make(true);
    compose(&mut r, "ni", normal);
    speech::take_recorded();
    r.handle_candidate_event(CandidateEvent::Speak);
    assert_eq!(speech::take_recorded(), None);
    assert_eq!(r.self_drawn_frame().candidates.items.len(), 1);

    // 释义选择界面里：念高亮那条，且不选、不退出（悬停挪高亮后再念就是新的那条）
    let mut r = make(true);
    compose(&mut r, "ni", normal);
    r.translate.begin_choices(
        0,
        "你".to_owned(),
        vec![sense("sad"), sense("sorrow"), sense("grieve")],
    );
    r.handle_candidate_event(CandidateEvent::Speak);
    assert_eq!(speech::take_recorded().as_deref(), Some("sad"));
    r.handle_candidate_event(CandidateEvent::Hover(1));
    r.handle_candidate_event(CandidateEvent::Speak);
    assert_eq!(speech::take_recorded().as_deref(), Some("sorrow"));
    assert!(r.translate.choices().is_some());

    // 关掉翻译 Tip：中键什么也不做
    let mut r = make(false);
    compose(&mut r, "ni", normal);
    r.handle_candidate_event(CandidateEvent::Speak);
    assert_eq!(speech::take_recorded(), None);
}

/// 滚轮翻页：不带修饰键翻页（下滚下一页、上滚上一页），且**高亮条留在窗口同一格**（不像键盘翻页
/// 那样回到页首）；到头就停；释义选择界面里不翻页。
/// （Ctrl + 滚轮是缩放窗口，在窗口里就地处理，不走 Router。）
#[test]
fn wheel_pages_keeping_the_highlight_cell() {
    let normal = KeyModifiers::default();
    let mut r = router(5);
    compose(&mut r, "qq", normal);
    // 9 条短语：一页 5 个、共两页。先把高亮挪到页内第 3 格
    key(&mut r, 0x28, None, normal);
    key(&mut r, 0x28, None, normal);
    assert_eq!(r.self_drawn_frame().highlight, 2);

    r.handle_candidate_event(CandidateEvent::Page(1));
    let frame = r.self_drawn_frame();
    // 帧里的 `highlight` 是**页内**格号：翻页后它不变，就是「高亮条没动」
    assert_eq!(
        (frame.page, frame.highlight),
        (1, 2),
        "翻到第二页，仍在页内第 3 格"
    );
    // 到头了再往下滚不动（第二页就是最后一页）
    r.handle_candidate_event(CandidateEvent::Page(1));
    assert_eq!(r.self_drawn_frame().page, 1);
    r.handle_candidate_event(CandidateEvent::Page(-1));
    let frame = r.self_drawn_frame();
    assert_eq!((frame.page, frame.highlight), (0, 2));

    // 最后一页不满 5 个时格号夹到最后一个：页内第 5 格翻到只剩 4 个的第二页 → 落到最后一格
    let mut r = router(5);
    compose(&mut r, "qq", normal);
    r.handle_candidate_event(CandidateEvent::Hover(4));
    assert_eq!(r.self_drawn_frame().highlight, 4);
    r.handle_candidate_event(CandidateEvent::Page(1));
    let frame = r.self_drawn_frame();
    assert_eq!((frame.page, frame.highlight), (1, 3));

    // 键盘翻页（PageDown / `=`）照旧：高亮落到新页第一个
    let mut r = router(5);
    compose(&mut r, "qq", normal);
    key(&mut r, 0x28, None, normal);
    let frame = key(&mut r, 0x22, None, normal).2;
    assert_eq!((frame.page, frame.highlight), (1, 0));

    // 释义选择界面里没有翻页这回事：滚轮什么也不做
    let sense = |text: &str| Sense {
        pos: None,
        text: text.to_owned(),
        reading: None,
    };
    r.translate
        .begin_choices(0, "你".to_owned(), vec![sense("sad"), sense("sorrow")]);
    r.handle_candidate_event(CandidateEvent::Page(1));
    assert!(r.translate.choices().is_some());
}

/// 收起态的方向键分工：**沿着列表方向的那个轴挪高亮、另一个轴翻页**（竖排是上下挪高亮 / 左右翻页，
/// 横排是左右挪高亮 / 上下翻页）；拼音光标两种收起态都由 `[` `]` 挪。
#[test]
fn collapsed_arrows_follow_the_list_direction() {
    let normal = KeyModifiers::default();
    let make = |layout: LayoutMode, text: &str| {
        let mut r = router(5);
        r.config.layout = layout;
        compose(&mut r, text, normal);
        r
    };

    // 竖排：上下挪高亮、左右翻页（9 条短语、一页 5 个）
    let mut r = make(LayoutMode::Vertical, "qq");
    assert_eq!(
        key(&mut r, 0x28, None, normal).2.highlight,
        1,
        "竖排 ↓ 挪高亮"
    );
    assert_eq!(
        key(&mut r, 0x26, None, normal).2.highlight,
        0,
        "竖排 ↑ 挪高亮"
    );
    let frame = key(&mut r, 0x27, None, normal).2;
    assert_eq!((frame.page, frame.highlight), (1, 0), "竖排 → 翻下一页");
    assert_eq!(key(&mut r, 0x25, None, normal).2.page, 0, "竖排 ← 翻上一页");

    // 横排：左右挪高亮、上下翻页
    let mut r = make(LayoutMode::Horizontal, "qq");
    assert_eq!(
        key(&mut r, 0x27, None, normal).2.highlight,
        1,
        "横排 → 挪高亮"
    );
    assert_eq!(
        key(&mut r, 0x25, None, normal).2.highlight,
        0,
        "横排 ← 挪高亮"
    );
    let frame = key(&mut r, 0x28, None, normal).2;
    assert_eq!((frame.page, frame.highlight), (1, 0), "横排 ↓ 翻下一页");
    assert_eq!(key(&mut r, 0x26, None, normal).2.page, 0, "横排 ↑ 翻上一页");

    // 两种收起态：`[` `]` 都是挪拼音光标（不组句时才是 `【】`）
    for layout in [LayoutMode::Vertical, LayoutMode::Horizontal] {
        let mut r = make(layout, "ni'hao");
        let end = key(&mut r, 0x23, None, normal).2.cursor;
        let left = key(&mut r, 0xDB, Some('['), normal);
        assert_eq!(left.0, KeyOutcome::Consumed, "{layout:?}");
        assert_eq!(left.1, None, "{layout:?}：`[` 挪光标、不上屏标点");
        assert_ne!(left.2.cursor, end, "{layout:?}：`[` 往左挪");
        assert_eq!(
            key(&mut r, 0xDD, Some(']'), normal).2.cursor,
            end,
            "{layout:?}"
        );
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

/// 方向键之后，同一次按键的异步编辑会话补报的组句矩形也会让候选窗重画一遍：
/// 帧（含高亮）一模一样，只有矩形变了。壳侧 `set_content` 会收到这个「高亮没变」的帧，
/// 曾经的实现会顺手把正在跑的高亮滑动清掉，这正是「第一次按方向键不动画」的来源。
#[test]
fn a_later_candidate_rect_resends_an_identical_frame() {
    let rect = |right| ScreenRect {
        left: 100,
        top: 100,
        right,
        bottom: 120,
    };
    let mut r = router(9);
    let recorded = Arc::new(Recorded::default());
    r.set_candidate_sink(Box::new(RecordingSink(recorded.clone())));
    let normal = KeyModifiers::default();
    compose(&mut r, "qq", normal);
    // 光标矩形还没报来时窗口不显示；报来后画出高亮第 0 项。
    let position = |r: &mut Router, right| {
        r.handle(ClientMessage::PositionCandidates {
            session: SessionId(1),
            rect: rect(right),
        });
    };
    position(&mut r, 140);
    assert_eq!(recorded.shows.lock().unwrap().len(), 1);
    // 第一次方向键：高亮 0 → 1，起滑动。
    assert_eq!(key(&mut r, 0x28, None, normal).2.highlight, 1);
    assert_eq!(recorded.shows.lock().unwrap().len(), 2);
    // 上一次按键补报的组句矩形：位置变了、帧一模一样。
    position(&mut r, 160);
    let shows = recorded.shows.lock().unwrap();
    assert_eq!(shows.len(), 3);
    assert_eq!(shows[1].0, shows[2].0);
    assert_ne!(shows[1].1, shows[2].1);
}
