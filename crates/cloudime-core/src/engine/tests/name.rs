//! V 模式 `#` 取名：姓氏优先、单双字名、选姓后继续、词库兜底。

use super::*;
use crate::name::NameTables;

const SURNAMES: &str =
    "王\twang\t1\n张\tzhang\t3\n万\twan\t20\n汪\twang\t50\n欧\tou\t60\n欧阳\tou yang\t401\n";
const NAMES: &str = "伟\twei\n卫\twei\n建\tjian\n国\tguo\n娜\tna\n洋\tyang\n怡\tyi\n英\tying\n";

fn name_engine() -> Engine {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine
}

fn texts(engine: &Engine) -> Vec<String> {
    engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .map(|candidate| candidate.text)
        .collect()
}

#[test]
fn full_name_pinyin_prefers_surname_rank_then_name_table_order() {
    let mut engine = name_engine();
    engine.set_input("v#zhangwei");
    assert_eq!(texts(&engine), ["张伟", "张卫"]);

    engine.set_input("v#wangwei");
    assert_eq!(texts(&engine), ["王伟", "王卫", "汪伟", "汪卫"]);
}

#[test]
fn exact_syllable_beats_a_longer_one_with_the_same_start() {
    let mut engine = name_engine();
    engine.set_input("v#zhangyi");
    // `yi` 是完整音节：怡 在前；英（ying）只是同前缀，排在后面。
    assert_eq!(texts(&engine), ["张怡", "张英"]);
}

#[test]
fn shorter_surname_is_not_read_as_surname_plus_one_given_letter() {
    let mut engine = name_engine();
    engine.set_input("v#wang");
    assert_eq!(texts(&engine), ["王", "汪"]);

    // 少打一个 g：万 是完整姓氏，王 / 汪 是还没打完的姓氏，都该出。
    engine.set_input("v#wan");
    let all = texts(&engine);
    assert!(all.contains(&"万".to_owned()));
    assert!(all.contains(&"王".to_owned()));
    assert!(all.contains(&"汪".to_owned()));
}

#[test]
fn compound_surname_wins_over_its_first_character() {
    let mut engine = name_engine();
    engine.set_input("v#ouyangna");
    assert_eq!(texts(&engine), ["欧阳娜"]);
}

#[test]
fn surname_only_then_continue_with_given_name() {
    let mut engine = name_engine();
    engine.set_input("v#wang");
    assert_eq!(texts(&engine), ["王", "汪"]);
    assert!(engine.name_selection_is_surname("王"));
    assert!(!engine.name_selection_is_surname("王伟"));

    assert!(engine.select_name_surname("王"));
    assert_eq!(engine.composition().text(), "v#王");
    let all = texts(&engine);
    assert!(all.starts_with(&[
        "王伟".to_owned(),
        "王卫".to_owned(),
        "王建".to_owned(),
        "王国".to_owned()
    ]));
    assert!(all.contains(&"王建国".to_owned()));

    let candidate = engine.query().unwrap().candidates.items[0].clone();
    assert_eq!(engine.commit(&candidate), Some("王伟".to_owned()));
}

#[test]
fn literal_surname_lists_both_single_and_double_given_names() {
    let mut engine = name_engine();
    engine.set_input("v#王");
    let all = texts(&engine);
    assert!(all.starts_with(&["王伟".to_owned(), "王卫".to_owned()]));
    assert!(all.contains(&"王建国".to_owned()));
}

#[test]
fn rare_single_surname_falls_back_to_dictionary() {
    let mut engine = Engine::new(Dictionary::parse("爨\tcuan\t100\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine.set_input("v#cuanwei");
    assert_eq!(texts(&engine), ["爨伟", "爨卫"]);
}

#[test]
fn rare_compound_surname_falls_back_to_dictionary() {
    let mut engine = Engine::new(Dictionary::parse("澹台\tdan tai\t100\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine.set_input("v#dantaijian");
    assert_eq!(texts(&engine), ["澹台建"]);
}

#[test]
fn name_table_blocks_dictionary_fallback_for_the_same_reading() {
    let mut engine = Engine::new(Dictionary::parse("健\tjian\t900\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine.set_input("v#zhangjian");
    assert_eq!(texts(&engine), ["张建"]);
}

#[test]
fn name_mode_requires_the_name_prefix() {
    let mut engine = name_engine();
    engine.set_input("v#");
    assert!(engine.name_mode());
    assert!(texts(&engine).is_empty());
    engine.set_input("v@");
    assert!(!engine.name_mode());
}
