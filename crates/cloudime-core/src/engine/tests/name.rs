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

/// 名字表里的字排在词库同音字前面：表里有「建」，词库里的「健」跟在后面。
#[test]
fn dictionary_name_chars_come_after_the_table_ones() {
    let mut engine = Engine::new(Dictionary::parse("健\tjian\t900\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine.set_input("v#zhangjian");
    assert_eq!(texts(&engine), ["张建", "张健"]);
}

/// 不常见的名字用字（词库里有、名字表里没有）也能被候选到，只是排在常见名字字后面；
/// 同音的每个姓都排上（按名字字交错），不会被排名最靠前的那个姓占满。
#[test]
fn rare_given_chars_follow_the_common_ones_for_every_surname() {
    let mut engine = Engine::new(Dictionary::parse("玮\twei\t900\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine.set_input("v#wangwei");
    assert_eq!(
        texts(&engine),
        ["王伟", "王卫", "汪伟", "汪卫", "王玮", "汪玮"]
    );
}

/// 罕见姓氏也在候选里，排在常见姓氏后面（表里按排名）。
#[test]
fn rare_surnames_come_after_the_common_ones() {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap());
    engine.set_name_tables(NameTables::from_tsv("卫\twei\t214\n隗\twei\t460\n", NAMES).unwrap());
    engine.set_input("v#wei");
    assert_eq!(texts(&engine), ["卫", "隗"]);
}

/// 词库里现成的姓名整词（`王玮`）优先于读音相同但不是名字的字（`为`）。
#[test]
fn dictionary_name_words_rank_before_plain_same_sound_chars() {
    let mut engine =
        Engine::new(Dictionary::parse("王玮\twang wei\t800\n为\twei\t9000\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv("王\twang\t1\n", NAMES).unwrap());
    engine.set_input("v#wangwei");
    assert_eq!(texts(&engine), ["王伟", "王卫", "王玮", "王为"]);
}

/// 表里已经有完整姓氏时不再拿词库的单字当姓：`为`、`未` 这类不是姓，别把候选挤乱。
#[test]
fn dictionary_chars_are_not_used_as_surnames_when_the_table_matches() {
    let mut engine = Engine::new(Dictionary::parse("为\twei\t9000\n未\twei\t9000\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv("卫\twei\t214\n", NAMES).unwrap());
    engine.set_input("v#wei");
    assert_eq!(texts(&engine), ["卫"]);
}

/// 复姓在表里时，哪怕它的首字也是表里的完整姓氏（宣 / 轩辕），也优先出复姓。
#[test]
fn compound_surname_in_the_table_wins_over_its_first_character() {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap());
    engine.set_name_tables(
        NameTables::from_tsv("宣\txuan\t318\n轩辕\txuan yuan\t415\n", NAMES).unwrap(),
    );
    engine.set_input("v#xuanyuanna");
    assert_eq!(texts(&engine), ["轩辕娜"]);
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
