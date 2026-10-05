//! 分段上屏改成整体上屏：选中只并进组句，整段转换完 / 回车 / 标点才交给应用。

use super::CountingLearner;
use crate::{Candidate, CandidateKind, Engine};
use std::collections::HashMap;

/// 带 `zhe` 音节的小词库，够走「选一半、再选剩下一半」。
const SEGMENTED: &str = "开发\tkai fa\t9000\n者\tzhe\t5000\n开放\tkai fang\t20000\n\
                         开\tkai\t20000\n作者\tzuo zhe\t100\n";

fn segmented() -> Engine {
    Engine::new(crate::dictionary::Dictionary::parse(SEGMENTED).unwrap())
}

/// 当前查询里找一条中文候选。
fn find(engine: &mut Engine, text: &str) -> Candidate {
    engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.text == text && c.kind == CandidateKind::Chinese)
        .unwrap_or_else(|| panic!("候选里没有 {text}"))
}

#[test]
fn selecting_a_prefix_merges_into_the_composition_until_the_whole_segment_converts() {
    let mut engine = segmented();
    engine.set_input("kaifazhe");
    let kaifa = find(&mut engine, "开发");
    // 选中 开发：并进组句，不立刻交给应用
    assert_eq!(engine.commit(&kaifa), None);
    assert_eq!(engine.composition().selected_text(), "开发");
    assert_eq!(engine.composition().text(), "zhe");
    // 预编辑是「已选文本 + 未选拼音」，候选只对剩下的 zhe 出
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), "开发zhe");
    assert_eq!(query.marked_cursor(), 5);
    assert!(
        query.candidates.items.iter().all(|c| c.text != "开发"),
        "已选段不再是候选"
    );
    assert!(query.candidates.items.iter().any(|c| c.text == "者"));

    // 再选 者：整段转换完，一次性交给应用
    let zhe = find(&mut engine, "者");
    assert_eq!(engine.commit(&zhe).as_deref(), Some("开发者"));
    assert!(engine.composition().is_empty());
}

#[test]
fn enter_flushes_selected_text_and_remaining_pinyin_as_is() {
    let mut engine = segmented();
    engine.set_input("kaifazhe");
    let kaifa = find(&mut engine, "开发");
    engine.commit(&kaifa);
    // 回车把已选文本 + 剩余拼音原样整体上屏（去掉手敲的分隔符）
    assert_eq!(engine.take_raw(), "开发zhe");
    assert!(engine.composition().is_empty());
}

#[test]
fn backspace_retracts_the_last_selection_and_restores_its_pinyin() {
    let mut engine = segmented();
    engine.set_input("kaifazhe");
    let kaifa = find(&mut engine, "开发");
    engine.commit(&kaifa);
    assert_eq!(engine.composition().text(), "zhe");
    // 退格先撤回选择：中文还原成拼音，光标落在还原段之后
    assert!(engine.backspace());
    assert!(!engine.composition().has_selected());
    assert_eq!(engine.composition().text(), "kaifazhe");
    assert_eq!(engine.composition().cursor(), "kaifa".len());
    // 光标停在未选拼音中间时，退格照旧删一个字符（不再撤回选择）
    assert!(engine.backspace());
    assert_eq!(engine.composition().text(), "kaifzhe");
}

#[test]
fn retracting_a_selection_and_choosing_another_word_retracts_the_learning() {
    let mut engine = segmented().with_learner(Box::new(CountingLearner(HashMap::new())));
    engine.set_input("kaifazhe");
    let kaifa = find(&mut engine, "开发");
    engine.commit(&kaifa);
    assert_eq!(engine.learner().weight("开发"), 1);
    // 撤回选择后改选 开放：开发 的那次学习退回去
    engine.backspace();
    assert_eq!(engine.composition().text(), "kaifazhe");
    let kaifang = find(&mut engine, "开放");
    engine.commit(&kaifang);
    assert_eq!(engine.learner().weight("开发"), 0);
    assert_eq!(engine.learner().choice_weight("kaifa", "开发"), 0);
    assert_eq!(engine.learner().weight("开放"), 1);
}

#[test]
fn cursor_and_deletes_stay_out_of_the_selected_text() {
    let mut engine = segmented();
    engine.set_input("kaifazhe");
    let kaifa = find(&mut engine, "开发");
    engine.commit(&kaifa);
    assert_eq!(engine.composition().text(), "zhe");
    // 光标最多退到未选拼音开头，进不了已选文本
    for _ in 0..3 {
        assert!(engine.move_cursor_left());
    }
    assert_eq!(engine.composition().cursor(), 0);
    assert!(!engine.move_cursor_left());
    assert!(!engine.delete_to_start());
    assert_eq!(engine.composition().selected_text(), "开发");
    assert_eq!(engine.composition().text(), "zhe");
}

#[test]
fn escape_discards_selected_text_too() {
    let mut engine = segmented();
    engine.set_input("kaifazhe");
    let kaifa = find(&mut engine, "开发");
    engine.commit(&kaifa);
    engine.clear();
    assert!(engine.composition().is_empty());
    assert!(!engine.composition().has_selected());
}
