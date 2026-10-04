//! `[input]` 分节那几项开关：简拼、中英混输、符号映射。

use super::*;

/// `kf` 在缺省（开简拼）下是 开放 / 咖啡 的缩写；关掉之后必须打全才能出中文候选。
#[test]
fn turning_jian_pin_off_needs_complete_syllables() {
    let mut engine = engine();
    engine.set_input("kf");
    assert!(texts_of(&engine).contains(&"开放".to_owned()));

    engine.set_use_jian_pin(false);
    engine.set_input("kf");
    let abbreviated = engine
        .query()
        .map(|q| q.candidates.items)
        .unwrap_or_default();
    assert!(
        !abbreviated
            .iter()
            .any(|c| c.text == "开放" || c.text == "咖啡"),
        "{abbreviated:?}"
    );

    // 完整音节与末尾没打完的前缀照常
    engine.set_input("kaifa");
    assert!(texts_of(&engine).contains(&"开发".to_owned()));
    engine.set_input("kaif");
    assert!(texts_of(&engine).contains(&"开发".to_owned()));
}

/// 中英混输关掉后，中文模式不再掺英文词与英文补全。
#[test]
fn turning_mixture_input_off_hides_english_candidates() {
    let english = crate::dictionary::WordList::parse("hello\thello\t6000\n").unwrap();
    let mut engine = engine().with_english(english);
    let has_english = |engine: &mut Engine| {
        engine
            .query()
            .map(|q| {
                q.candidates
                    .items
                    .iter()
                    .any(|c| c.kind == CandidateKind::English)
            })
            .unwrap_or(false)
    };

    engine.set_input("hello");
    assert!(has_english(&mut engine), "缺省：中文模式下也出英文词");
    engine.set_mixture_input(false);
    engine.set_input("hello");
    assert!(!has_english(&mut engine), "关掉中英混输后不再出英文词");
}

/// 配置的符号映射：单键、小键盘、两键规则，以及「数字后标点保持半角」。
#[test]
fn configured_symbol_mapping_converts_and_sequences() {
    let mut engine = engine();
    engine.set_punctuation_mapping(crate::punctuation::Mapping::from_pairs([
        ("/", "、"),
        ("{kp}*", "×"),
        ("~=", "≈"),
    ]));
    assert_eq!(
        engine.map_symbol('/', false).map(|m| m.text),
        Some("、".to_owned())
    );
    // 主键区的条目不管小键盘
    assert_eq!(engine.map_symbol('/', true), None);
    assert_eq!(
        engine.map_symbol('*', true).map(|m| m.text),
        Some("×".to_owned())
    );
    // 两键规则：`~` 先按内置表上屏 `～`，敲 `=` 时把它撤掉换成 ≈
    assert_eq!(engine.punctuate('~').as_deref(), Some("～"));
    let mapped = engine.map_symbol('=', false).expect("两键规则命中");
    assert_eq!((mapped.text.as_str(), mapped.delete_before), ("≈", 1));
    // 内置的全角标点表照旧
    assert_eq!(engine.punctuate(',').as_deref(), Some("，"));
    // 数字后标点保持半角（缺省开）
    engine.note_passthrough('2');
    assert_eq!(engine.map_symbol('*', true), None);
    engine.set_half_wide_after_digit(false);
    engine.note_passthrough('2');
    assert_eq!(
        engine.map_symbol('*', true).map(|m| m.text),
        Some("×".to_owned())
    );
}

/// 稀有组缺省关闭（由 `[word_bank] rare_items` 决定），打开后才参与查询。
#[test]
fn turning_rare_dictionary_off_skips_it() {
    let rare = Dictionary::parse("龘\tda\t500\n").unwrap();
    let mut engine = engine().with_rare(rare);
    // 缺省关：挂着稀有组也不查
    assert!(!engine.rare_enabled());
    assert!(engine.rare_dictionary().is_some());

    engine.set_input("da");
    assert!(!texts_of(&engine).contains(&"龘".to_owned()));

    engine.set_rare_enabled(true);
    engine.set_input("da");
    assert!(texts_of(&engine).contains(&"龘".to_owned()));

    engine.set_rare_enabled(false);
    engine.set_input("da");
    assert!(!texts_of(&engine).contains(&"龘".to_owned()));
}
