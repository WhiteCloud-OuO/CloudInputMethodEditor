//! V 模式 `#` 取名：先出姓、再一段一段出名字字，选姓后不拼后面几段。

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

/// 形码取名：姓名表 + 最小五笔码表，`v#` / Ctrl+Alt+V 后按编码取字。
fn shape_name_engine() -> Engine {
    let mut engine = name_engine();
    engine.set_shape(Some(
        crate::shape::Table::parse(
            "xtay\t张\t10\nwfnh\t伟\t10\nwfnh\t倒霉蛋\t1000\niynt\t沪\t10\n",
            4,
        )
        .0,
    ));
    engine
}

/// 形码里从空组句进 v 模式，再敲 `#` 后面的内容（`set_input` 会把 v 模式清掉）。
fn set_v_input(engine: &mut Engine, input: &str) {
    engine.start_expression();
    for character in input.chars() {
        engine.push(character);
    }
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
fn before_a_surname_is_chosen_only_surnames_are_offered() {
    let mut engine = name_engine();
    // 名拼音已经打完了，也先只出姓。
    engine.set_input("v#zhangwei");
    assert_eq!(texts(&engine), ["张"]);

    engine.set_input("v#wangwei");
    assert_eq!(texts(&engine), ["王", "汪"]);
}

#[test]
fn surname_selection_keeps_the_rest_of_the_pinyin() {
    let mut engine = name_engine();
    engine.set_input("v#wangwei");
    assert!(engine.select_name_part("王"));
    assert_eq!(engine.composition().text(), "v#王wei");
    assert_eq!(texts(&engine), ["王伟", "王卫"]);

    let candidate = engine.query().unwrap().candidates.items[0].clone();
    assert_eq!(engine.commit(&candidate), Some("王伟".to_owned()));
}

#[test]
fn exact_syllable_beats_a_longer_one_with_the_same_start() {
    let mut engine = name_engine();
    engine.set_input("v#wangyi");
    assert!(engine.select_name_part("王"));
    // `yi` 是完整音节：怡 在前；英（ying）只是同前缀，排在后面。
    assert_eq!(texts(&engine), ["王怡", "王英"]);
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
    assert_eq!(texts(&engine), ["欧阳"]);
}

#[test]
fn shape_name_uses_wubi_codes_separated_by_apostrophes() {
    let mut engine = shape_name_engine();
    set_v_input(&mut engine, "#xtay'wfnh");
    assert_eq!(texts(&engine), ["张"], "还没选姓时只出姓氏编码对应的字");

    assert!(engine.select_name_part("张"));
    assert_eq!(engine.composition().text(), "v#张'wfnh");
    assert_eq!(texts(&engine), ["张伟"], "多字词不参与取名");

    let candidate = engine.query().unwrap().candidates.items[0].clone();
    assert_eq!(engine.commit(&candidate), Some("张伟".to_owned()));
}

#[test]
fn shape_name_codes_allow_z_wildcards() {
    let mut engine = shape_name_engine();
    // 每段的 z 当通配一位：xtaz 命中 xtay（张），wfnz 命中 wfnh（伟）
    set_v_input(&mut engine, "#xtaz'wfnz");
    assert_eq!(texts(&engine), ["张"]);
    assert!(engine.select_name_part("张"));
    assert_eq!(texts(&engine), ["张伟"]);
}

#[test]
fn shape_name_requires_the_apostrophe_between_codes() {
    let mut engine = shape_name_engine();
    // 两个编码没加 `'`：整串不是一段合法五笔码，不出任何候选
    set_v_input(&mut engine, "#xtaywfnh");
    assert!(texts(&engine).is_empty());
    assert!(!engine.select_name_part("张"));

    // 补上分隔符后正常取名
    set_v_input(&mut engine, "#xtay'wfnh");
    assert!(engine.select_name_part("张"));
    assert_eq!(texts(&engine), ["张伟"]);
}

#[test]
fn literal_surname_only_offers_itself_until_pinyin_is_typed() {
    let mut engine = name_engine();
    engine.set_input("v#王");
    assert_eq!(texts(&engine), ["王"]);
}

#[test]
fn rare_single_surname_falls_back_to_dictionary() {
    let mut engine = Engine::new(Dictionary::parse("爨\tcuan\t100\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine.set_input("v#cuanwei");
    assert_eq!(texts(&engine), ["爨"]);
    assert!(engine.select_name_part("爨"));
    assert_eq!(texts(&engine), ["爨伟", "爨卫"]);
}

#[test]
fn rare_compound_surname_falls_back_to_dictionary() {
    let mut engine = Engine::new(Dictionary::parse("澹台\tdan tai\t100\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine.set_input("v#dantaijian");
    assert_eq!(texts(&engine), ["澹台"]);
    assert!(engine.select_name_part("澹台"));
    assert_eq!(texts(&engine), ["澹台建"]);
}

#[test]
fn rare_given_chars_follow_the_common_ones() {
    let mut engine = Engine::new(Dictionary::parse("玮\twei\t900\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine.set_input("v#wangwei");
    assert!(engine.select_name_part("王"));
    assert_eq!(texts(&engine), ["王伟", "王卫", "王玮"]);
}

#[test]
fn dictionary_name_chars_come_after_the_table_ones() {
    let mut engine = Engine::new(Dictionary::parse("健\tjian\t900\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv(SURNAMES, NAMES).unwrap());
    engine.set_input("v#zhangjian");
    assert!(engine.select_name_part("张"));
    assert_eq!(texts(&engine), ["张建", "张健"]);
}

/// 罕见姓氏也在候选里，排在常见姓氏后面（表里按排名）。
#[test]
fn rare_surnames_come_after_the_common_ones() {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap());
    engine.set_name_tables(NameTables::from_tsv("卫\twei\t214\n隗\twei\t460\n", NAMES).unwrap());
    engine.set_input("v#wei");
    assert_eq!(texts(&engine), ["卫", "隗"]);
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
    assert_eq!(texts(&engine), ["轩辕"]);
}

/// 双字名一段一段取：选姓 → 只出第一段（`xiu`）的字 → 选完剩下的拼音接着取。
#[test]
fn given_name_is_taken_one_segment_at_a_time() {
    let mut engine = Engine::new(Dictionary::parse("石\tshi\t900\n麟\tlin\t600\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv("戴\tdai\t64\n", NAMES).unwrap());

    engine.set_input("v#daishilin");
    assert_eq!(texts(&engine), ["戴"], "还没选姓：只出姓");
    assert!(engine.select_name_part("戴"));
    assert_eq!(engine.composition().text(), "v#戴shilin");

    // 只出第一段（`shi`）的字：`石` 在，第二个音节的 `麟` 还不出。
    let all = texts(&engine);
    assert!(all.contains(&"戴石".to_owned()), "{all:?}");
    assert!(!all.iter().any(|text| text.contains('麟')), "{all:?}");
    assert_eq!(engine.name_part_after("戴石").as_deref(), Some("戴石lin"));

    assert!(engine.select_name_part("戴石"));
    assert_eq!(engine.composition().text(), "v#戴石lin");
    let all = texts(&engine);
    assert!(all.contains(&"戴石麟".to_owned()), "{all:?}");
    let candidate = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|candidate| candidate.text == "戴石麟")
        .unwrap();
    assert_eq!(engine.commit(&candidate), Some("戴石麟".to_owned()));
}

/// 名字只剩一段时，选中那个字就是完整名字，直接上屏。
#[test]
fn the_last_segment_commits_the_full_name() {
    let mut engine = Engine::new(Dictionary::parse("石\tshi\t900\n").unwrap());
    engine.set_name_tables(NameTables::from_tsv("戴\tdai\t64\n", NAMES).unwrap());
    engine.set_input("v#daishi");
    assert!(engine.select_name_part("戴"));
    assert!(texts(&engine).contains(&"戴石".to_owned()));
    assert!(
        engine.name_part_after("戴石").is_none(),
        "最后一段选中就上屏"
    );
    let candidate = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|candidate| candidate.text == "戴石")
        .unwrap();
    assert_eq!(engine.commit(&candidate), Some("戴石".to_owned()));
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
