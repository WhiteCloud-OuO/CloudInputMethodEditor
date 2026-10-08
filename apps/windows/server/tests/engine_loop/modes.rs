//! 标点全半角、字符全半角、表达式模式。

use crate::support::*;

#[test]
fn chinese_punctuation_converts_outside_and_commits_first_inside_composition() {
    let mut router = router();
    // 没在组句：逗号转全角；数字后的点保持半角。
    let comma = KeyEvent::new(0xBC, Some(','), Default::default());
    assert_eq!(
        press(&mut router, comma),
        (
            KeyOutcome::Consumed,
            Some("，".to_owned()),
            Frame::default()
        )
    );
    press(&mut router, digit(3));
    let period = KeyEvent::new(0xBE, Some('.'), Default::default());
    assert_eq!(press(&mut router, period).0, KeyOutcome::Passthrough);
    assert_eq!(press(&mut router, period).1, Some("。".to_owned()));

    // 小键盘的点不跟在数字后面也保持半角。
    let keypad_period = KeyEvent::new(0x6E, Some('.'), Default::default());
    assert_eq!(press(&mut router, keypad_period).0, KeyOutcome::Passthrough);

    // 组句中：先把高亮候选上屏，再按组句外语义把标点转全角。
    type_letters(&mut router, "ni");
    let (outcome, commit, frame) = press(&mut router, comma);
    assert_eq!(
        (outcome, commit.as_deref()),
        (KeyOutcome::Consumed, Some("你，"))
    );
    assert!(frame.is_empty(), "标点把整段拼音一起上屏了");

    // 状态条上关掉全角：原样交给应用。
    router.handle(ClientMessage::Commit { session: SESSION });
    router.handle_status_event(StatusEvent::TogglePunctuation);
    assert_eq!(press(&mut router, comma).0, KeyOutcome::Passthrough);
}

#[test]
fn expression_mode_takes_digits_and_operators() {
    let mut router = router();
    // v 开头进表达式模式：数字不选词、运算符进算式，Shift + 6 是 `^`。
    type_letters(&mut router, "v");
    press(&mut router, digit(1));
    press(&mut router, punct('+'));
    let (outcome, commit, frame) = press(&mut router, digit(2));
    assert_eq!((outcome, commit), (KeyOutcome::Consumed, None));
    assert_eq!(preedit(&frame), "v1+2");
    assert_eq!(candidate_texts(&frame), ["3", "1+2=3"]);
    press(&mut router, digit_with(6, SHIFT));
    let (_, _, frame) = press(&mut router, digit(2));
    assert_eq!(preedit(&frame), "v1+2^2");
    assert_eq!(candidate_texts(&frame), ["5", "1+2^2=5"]);
    // 空格上屏首选并清空。
    let (outcome, commit, frame) = press(&mut router, punct(' '));
    assert_eq!(
        (outcome, commit),
        (KeyOutcome::Consumed, Some("5".to_owned()))
    );
    assert!(preedit(&frame).is_empty());
}

#[test]
fn expression_mode_spells_chinese_numerals() {
    let mut router = router();
    type_letters(&mut router, "v");
    for n in [1, 2, 3] {
        press(&mut router, digit(n));
    }
    let (_, _, frame) = press(&mut router, punct('.'));
    assert_eq!(preedit(&frame), "v123.");
    let (_, _, frame) = press(&mut router, digit(5));
    assert_eq!(
        candidate_texts(&frame),
        [
            "一百二十三点五",
            "壹佰贰拾叁点伍",
            "一百二十三元五角",
            "壹佰贰拾叁元伍角"
        ]
    );
    router.handle(ClientMessage::Key {
        session: SESSION,
        event: KeyEvent::new(0x1B, None, Default::default()),
    });
    type_letters(&mut router, "v");
    for n in [1, 2, 3] {
        press(&mut router, digit(n));
    }
    let (_, _, frame) = press(&mut router, punct('+'));
    // `v123+` 算不出来就没有候选，回车上屏原文。
    assert!(candidate_texts(&frame).is_empty());
    let (_, _, frame) = press(&mut router, KeyEvent::new(0x08, None, Default::default()));
    assert_eq!(
        candidate_texts(&frame),
        [
            "一百二十三",
            "壹佰贰拾叁",
            "一百二十三元整",
            "壹佰贰拾叁元整"
        ]
    );
    press(&mut router, KeyEvent::new(0x28, None, Default::default()));
    let (_, commit, _) = press(&mut router, punct(' '));
    assert_eq!(commit.as_deref(), Some("壹佰贰拾叁"));
}

#[test]
fn expression_mode_other_punctuation_commits_then_applies() {
    // `,` 是函数的参数分隔符（算式的一部分）：进缓冲区，不上屏
    let mut comma = router();
    type_letters(&mut comma, "v");
    press(&mut comma, digit(1));
    let (outcome, commit, frame) = press(&mut comma, punct(','));
    assert_eq!((outcome, commit), (KeyOutcome::Consumed, None));
    assert_eq!(preedit(&frame), "v1,");

    // 别的标点（`;`）不是算式的一部分：先把首选上屏，再按没在组句处理。
    // 上屏的是数字 3，而「数字后标点使用半角」缺省开着，所以分号保持半角（配置可关，见输入页）
    let mut other = router();
    type_letters(&mut other, "v");
    press(&mut other, digit(1));
    press(&mut other, punct('+'));
    press(&mut other, digit(2));
    let (outcome, commit, frame) = press(&mut other, punct(';'));
    assert_eq!(
        (outcome, commit),
        (KeyOutcome::Consumed, Some("3;".to_owned()))
    );
    assert!(preedit(&frame).is_empty());
}

/// `?` 恢复为普通标点：中文模式出全角问号，英文模式半角。
#[test]
fn question_mark_is_plain_punctuation() {
    let mut router = router();
    let (outcome, commit, frame) = press(&mut router, punct('?'));
    assert_eq!(outcome, KeyOutcome::Consumed);
    assert_eq!(commit.as_deref(), Some("？"));
    assert!(preedit(&frame).is_empty());
    let (outcome, commit, _) = press(&mut router, KeyEvent::new(0xBF, Some('?'), ENGLISH));
    assert_eq!((outcome, commit), (KeyOutcome::Passthrough, None));
}

#[test]
fn punctuation_toggle_is_remembered_per_mode() {
    let mut router = router();
    let comma = KeyEvent::new(0xBC, Some(','), Default::default());
    let english_comma = KeyEvent::new(0xBC, Some(','), ENGLISH);
    // 中文模式下切成半角。
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::Chinese,
    });
    router.handle_status_event(StatusEvent::TogglePunctuation);
    assert_eq!(press(&mut router, comma).0, KeyOutcome::Passthrough);
    // 英文模式缺省半角；点那一格切成全角，英文模式下真转。
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::English,
    });
    assert_eq!(press(&mut router, english_comma).0, KeyOutcome::Passthrough);
    router.handle_status_event(StatusEvent::TogglePunctuation);
    assert_eq!(press(&mut router, english_comma).1, Some("，".to_owned()));
    // 切回中文：还是中文自己记住的半角；再切回英文：还是英文记住的全角。
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::Chinese,
    });
    assert_eq!(press(&mut router, comma).0, KeyOutcome::Passthrough);
    router.handle(ClientMessage::ModeChanged {
        session: SESSION,
        mode: InputMode::English,
    });
    assert_eq!(press(&mut router, english_comma).1, Some("，".to_owned()));
    // 英文模式里敲标点：字母不再进缓冲区（都直通了），标点也按英文那份转。
    type_english(&mut router, "hello");
    let (_, commit, _) = press(&mut router, english_comma);
    assert_eq!(commit.as_deref(), Some("，"));
}

/// 中文模式下的 Shift 大写固定收进组句缓冲区：按小写参与匹配，拼音行按敲的样子显示，回车原样上屏时还原大写。
#[test]
fn shift_letters_join_the_buffer() {
    let mut router = router();
    type_letters(&mut router, "ni");
    let (outcome, commit, frame) = press(&mut router, letter_with('A', SHIFT));
    assert_eq!((outcome, commit.as_deref()), (KeyOutcome::Consumed, None));
    assert_eq!(preedit(&frame), "niA");
    let (_, commit, _) = press(&mut router, function_key(0x0D));
    assert_eq!(commit.as_deref(), Some("niA"));
}

/// 状态条「全角 / 半角」：直通给应用的可打印 ASCII 转全角，中英两模式都转。
#[test]
fn full_width_chars_convert_passthrough_chars() {
    let config = RouterConfig {
        full_width_chars: true,
        ..RouterConfig::default()
    };
    let mut router = router_with(config);

    // 中文模式没在组句：数字直通，转全角
    assert_eq!(
        press(&mut router, digit(1)),
        (
            KeyOutcome::Consumed,
            Some("１".to_owned()),
            Frame::default()
        )
    );
    // `-` 组句外本来是直通的（全角标点表里没有它），开了全角字符就转
    let minus = KeyEvent::new(0xBD, Some('-'), Default::default());
    assert_eq!(press(&mut router, minus).1, Some("－".to_owned()));
    // 中文标点优先：逗号还是 `，`（走标点表），不被字符表截走
    let comma = KeyEvent::new(0xBC, Some(','), Default::default());
    assert_eq!(press(&mut router, comma).1, Some("，".to_owned()));

    // 英文模式：字母也转（DLL 那边开了这个开关才会把字母送来，见 tsf 的 eats_key）
    let (outcome, commit, _) = press(&mut router, letter_with('a', ENGLISH));
    assert_eq!(
        (outcome, commit.as_deref()),
        (KeyOutcome::Consumed, Some("ａ"))
    );
    let (_, commit, _) = press(&mut router, letter_with('A', CAPS));
    assert_eq!(commit.as_deref(), Some("Ａ"));

    // 关掉：数字与字母都回直通
    router.handle_status_event(StatusEvent::ToggleCharWidthType);
    assert_eq!(press(&mut router, digit(1)).0, KeyOutcome::Passthrough);
    assert_eq!(
        press(&mut router, letter_with('a', ENGLISH)).0,
        KeyOutcome::Passthrough
    );
}

/// 全角字符与中文标点各管一段：标点表关着时，标点也按字符表转。
#[test]
fn full_width_chars_apply_when_the_punctuation_table_is_off() {
    let config = RouterConfig {
        full_width_chars: true,
        full_width_punctuation: false,
        ..RouterConfig::default()
    };
    let mut router = router_with(config);
    // 中文标点关掉了：引号不再成对变成 `‘’`，改由字符表转成全角 `＇`
    let quote = KeyEvent::new(0xDE, Some('\''), Default::default());
    assert_eq!(
        press(&mut router, quote),
        (
            KeyOutcome::Consumed,
            Some("＇".to_owned()),
            Frame::default()
        )
    );
    let comma = KeyEvent::new(0xBC, Some(','), Default::default());
    assert_eq!(press(&mut router, comma).1, Some("，".to_owned()));
}

/// 中文模式下 Shift 大写固定进组句；「全角字符」只管直通的字母与符号，不把它从组句里拽出来。
#[test]
fn full_width_chars_do_not_pull_shifted_uppercase_out_of_composition() {
    let config = RouterConfig {
        full_width_chars: true,
        ..RouterConfig::default()
    };
    let mut router = router_with(config);
    let (outcome, commit, frame) = press(&mut router, letter_with('P', SHIFT));
    assert_eq!((outcome, commit.as_deref()), (KeyOutcome::Consumed, None));
    assert_eq!(preedit(&frame), "P");

    // Caps 亮着的字母是直通的，全角字符照样由我们插全角形
    let mut router = router_with(RouterConfig {
        full_width_chars: true,
        ..RouterConfig::default()
    });
    let (_, commit, _) = press(&mut router, letter_with('P', CAPS));
    assert_eq!(commit.as_deref(), Some("Ｐ"));
}

/// 符号成对补全：敲左符号补上右半边、光标停在中间（`caret_shift = -1`）；再敲一次右半边就跳过去。
#[test]
fn pairwise_completion_inserts_the_pair_and_skips_the_close() {
    let config = RouterConfig {
        pairwise_completion: cloudime_platform::pair_bit('（').unwrap(),
        ..RouterConfig::default()
    };
    let mut router = router_with(config);
    // 中文模式缺省全角标点：`(` 先转成 `（`，补上 `）`
    let (outcome, commit, caret, deleted, _) = press_full(&mut router, punct('('));
    assert_eq!(
        (outcome, commit.as_deref(), caret, deleted),
        (KeyOutcome::Consumed, Some("（）"), -1, 0)
    );
    // 紧接着敲 `)`：右半边已经在文档里了，跳过去，不再插一个
    let (outcome, commit, caret, deleted, _) = press_full(&mut router, punct(')'));
    assert_eq!(
        (outcome, commit.as_deref(), caret, deleted),
        (KeyOutcome::Consumed, None, 1, 0)
    );
}

/// 中文模式下的单键符号映射：`~` 出 `～`。两键规则（`~=`）已下线，不再有「撤掉上一个键」。
#[test]
fn single_key_symbol_mapping() {
    let mut router = router();
    // 缺省映射表里 `~` 有单键规则
    let (_, commit, _, deleted, _) = press_full(&mut router, punct('~'));
    assert_eq!((commit.as_deref(), deleted), (Some("～"), 0));
    // 映射表里没有的 `=` 没有全角标点转换时原样交给应用
    let mut router = router_with(RouterConfig {
        full_width_punctuation: false,
        ..RouterConfig::default()
    });
    let (outcome, commit, _) = press(&mut router, punct('='));
    assert_eq!((outcome, commit), (KeyOutcome::Passthrough, None));
}

/// 半角标点（或 `{` 这类不在全角表里的键）下的成对补全。
///
/// 这条路以前根本不调 `complete_pair`，所以 `()`、`{}` 这些半边永远补不上；
/// 中英一致：只要开了成对补全、键没被全角表转换就补，英文 + 西文符号也一样（`()`）。
/// 英文尖括号 `<>` 与英文单引号 `''` 不在位图里（中文模式下标点表会把它们转掉），不再测。
#[test]
fn pairwise_completion_also_covers_half_width_marks() {
    let all: u32 = cloudime_platform::PAIRWISE_COMPLETION_BITS
        .iter()
        .map(|(bit, ..)| *bit)
        .sum();
    let mut router = router_with(RouterConfig {
        full_width_punctuation: false,
        pairwise_completion: all,
        ..RouterConfig::default()
    });
    for (typed, pair) in [('(', "()"), ('[', "[]"), ('{', "{}"), ('"', "\"\"")] {
        let (outcome, commit, caret, _, _) = press_full(&mut router, punct(typed));
        assert_eq!(
            (outcome, commit.as_deref(), caret),
            (KeyOutcome::Consumed, Some(pair), -1),
            "敲 {typed:?}"
        );
    }

    // 中文全角模式：`{` 不在内置全角表里，一样补成 `{}`
    let mut router = router_with(RouterConfig {
        pairwise_completion: all,
        ..RouterConfig::default()
    });
    let (outcome, commit, caret, _, _) = press_full(&mut router, punct('{'));
    assert_eq!(
        (outcome, commit.as_deref(), caret),
        (KeyOutcome::Consumed, Some("{}"), -1)
    );

    // 英文 + 西文符号也补：`(` 补成 `()`、光标停在中间
    let mut router = router_with(RouterConfig {
        pairwise_completion: all,
        ..RouterConfig::default()
    });
    let (outcome, commit, caret, _, _) =
        press_full(&mut router, KeyEvent::new(0xBE, Some('('), ENGLISH));
    assert_eq!(
        (outcome, commit.as_deref(), caret),
        (KeyOutcome::Consumed, Some("()"), -1)
    );
    // 英文模式下紧接着敲右半边 `)`：右半边已在文档里，跳过去、不再插一个
    let (outcome, commit, caret, _, _) =
        press_full(&mut router, KeyEvent::new(0xBE, Some(')'), ENGLISH));
    assert_eq!(
        (outcome, commit.as_deref(), caret),
        (KeyOutcome::Consumed, None, 1)
    );
}
