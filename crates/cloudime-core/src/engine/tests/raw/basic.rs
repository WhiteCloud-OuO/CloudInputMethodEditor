//! 全拼、直输与编辑状态不应借用候选显示串。
use super::assert_raw;
use crate::CandidateKind;
use crate::dictionary::Dictionary;
use crate::engine::tests::engine;

#[test]
fn raw_preedit_preserves_typed_text_without_automatic_separators() {
    for input in ["", "nihao", "xi'an", "no-Way", "gpt-6", "v1+2"] {
        let mut engine = engine();
        engine.set_input(input);
        let _ = engine.query();
        assert_raw(&mut engine, input, input.len());
    }
    let mut engine = engine();
    engine.set_input("NiHao");
    assert_eq!(engine.composition().text(), "nihao");
    assert_raw(&mut engine, "NiHao", 5);
}

#[test]
fn take_raw_drops_typed_apostrophes() {
    let mut engine = engine();
    engine.set_input("xi'an");
    assert_eq!(
        engine.raw_preedit().text,
        "xi'an",
        "预编辑仍保留敲进去的分隔符"
    );
    assert_eq!(engine.take_raw(), "xian");
}

#[test]
fn raw_preedit_keeps_suffix_after_editing() {
    let mut engine = engine();
    engine.set_input("nihao");
    for _ in 0..3 {
        engine.move_cursor_left();
    }
    engine.push('x');
    engine.delete_forward();
    assert_raw(&mut engine, "nixao", 3);
}

#[test]
fn raw_preedit_only_contains_the_uncommitted_remainder() {
    let mut engine = engine();
    engine.set_input("kaifazhe");
    let candidate = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.text == "开发" && c.kind == CandidateKind::Chinese)
        .unwrap();
    assert_eq!(engine.commit(&candidate), "开发");
    assert_raw(&mut engine, "zhe", 3);
}

#[test]
fn raw_preedit_cursor_covers_home_middle_and_end() {
    for position in 0..=5 {
        let mut engine = engine();
        engine.set_input("nihao");
        engine.move_cursor_home();
        for _ in 0..position {
            engine.move_cursor_right();
        }
        assert_raw(&mut engine, "nihao", position);
    }
}

#[test]
fn raw_preedit_does_not_accept_or_record_a_spelling_correction() {
    let dictionary = Dictionary::parse("你好吗\tni hao ma\t50000\n你好\tni hao\t10000\n").unwrap();
    let mut engine = crate::Engine::new(dictionary);
    engine.set_input("nihoama");
    assert!(engine.query().unwrap().correction.is_none());
    assert_raw(&mut engine, "nihoama", 7);
}
