//! 选词模型接线的测试：候选重排与按音节出字。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cloudime_dictionary::Dictionary;

use super::*;

/// 假选词模型：每条都盖住整段，偏爱某个文本。
struct Prefers(&'static str);

impl ChoiceModel for Prefers {
    fn align(&self, _keys: &str, _syllables: &[String]) -> Option<Vec<u32>> {
        Some(vec![1])
    }

    fn letter_units(&self, _keys: &str) -> Vec<u32> {
        vec![1]
    }

    fn score(&self, _context: &str, _typed: &[u32], texts: &[&str]) -> Vec<f32> {
        texts
            .iter()
            .map(|t| if *t == self.0 { 100.0 } else { 0.0 })
            .collect()
    }
}

/// 假选词模型：按音节出字偏爱错字，让整句备选压过正确的词；`preferred` 非空时选词分偏向它。
struct WrongChars(&'static str);

impl ChoiceModel for WrongChars {
    fn align(&self, _keys: &str, _syllables: &[String]) -> Option<Vec<u32>> {
        Some(vec![1])
    }

    fn letter_units(&self, _keys: &str) -> Vec<u32> {
        vec![1]
    }

    fn score(&self, _context: &str, _typed: &[u32], texts: &[&str]) -> Vec<f32> {
        texts
            .iter()
            .map(|text| if *text == self.0 { 100.0 } else { 0.0 })
            .collect()
    }

    fn compose(&self, _context: &str, syllables: &[(&str, bool)]) -> Option<CharTable> {
        let chars = "不差查好毫里离厘";
        let index: HashMap<char, u32> = chars
            .chars()
            .enumerate()
            .map(|(i, c)| (c, i as u32))
            .collect();
        let value = |c: char| match c {
            '不' | '查' | '好' | '里' => 0.0,
            '差' => -3.0,
            '离' => -1.0,
            '厘' => -10.0,
            _ => -20.0,
        };
        let rows = syllables
            .iter()
            .map(|_| chars.chars().map(value).collect())
            .collect();
        Some(CharTable::new(Arc::new(index), rows))
    }
}

/// 假选词模型：选词分相同，按音节出字偏爱给定的字。
struct Composes(&'static [char]);

impl ChoiceModel for Composes {
    fn align(&self, keys: &str, syllables: &[String]) -> Option<Vec<u32>> {
        (syllables.concat() == keys).then(|| vec![1; syllables.len()])
    }

    fn letter_units(&self, _keys: &str) -> Vec<u32> {
        vec![1]
    }

    fn score(&self, _context: &str, _typed: &[u32], texts: &[&str]) -> Vec<f32> {
        vec![0.0; texts.len()]
    }

    fn compose(&self, _context: &str, syllables: &[(&str, bool)]) -> Option<CharTable> {
        let index: HashMap<char, u32> = "在再见剑"
            .chars()
            .enumerate()
            .map(|(i, c)| (c, i as u32))
            .collect();
        let rows = (0..syllables.len())
            .map(|_| {
                "在再见剑"
                    .chars()
                    .map(|c| if self.0.contains(&c) { 0.0 } else { -10.0 })
                    .collect()
            })
            .collect();
        Some(CharTable::new(Arc::new(index), rows))
    }
}

fn first_texts(engine: &Engine) -> Vec<String> {
    engine
        .query()
        .unwrap()
        .candidates
        .items
        .iter()
        .take(2)
        .map(|c| c.text.clone())
        .collect()
}

fn settle(engine: &mut Engine) {
    let started = Instant::now();
    while engine.rescoring_pending() {
        assert!(engine.request_rescoring());
        while !engine.poll_rescoring() {
            assert!(started.elapsed() < Duration::from_secs(5), "后台没回结果");
            std::thread::sleep(Duration::from_millis(5));
        }
        engine.query().unwrap();
    }
}

#[test]
fn choice_waits_for_the_shell_then_moves_the_pick_up() {
    let mut engine =
        Engine::new(Dictionary::parse("开发\tkai fa\t9000\n开罚\tkai fa\t10\n").unwrap());
    engine.set_choice_model(Some(Arc::new(Prefers("开罚"))));
    for c in "kaifa".chars() {
        engine.push(c);
    }
    assert_eq!(first_texts(&engine), ["开发", "开罚"]);
    assert!(engine.rescoring_pending());
    settle(&mut engine);
    assert_eq!(first_texts(&engine), ["开罚", "开发"]);
    assert!(!engine.rescoring_pending());
}

#[test]
fn sentence_uses_the_char_table_once_it_arrives() {
    let mut engine = Engine::new(
        Dictionary::parse("在\tzai\t9000\n再\tzai\t100\n见\tjian\t9000\n剑\tjian\t50\n").unwrap(),
    );
    engine.set_choice_model(Some(Arc::new(Composes(&['再']))));
    for c in "zaijian".chars() {
        engine.push(c);
    }
    assert_eq!(first_texts(&engine)[0], "在见");
    settle(&mut engine);
    assert_eq!(first_texts(&engine)[0], "再见");
}

/// 四音节短语：整句候选会占满模型窗口，词候选排在第 4 位以后。
fn wrong_chars_engine(preferred: &'static str) -> Engine {
    let mut engine = Engine::new(
        Dictionary::parse(
            "不差毫厘\tbu cha hao li\t10\n\
             不\tbu\t10000\n\
             差\tcha\t10000\n\
             查\tcha\t10000\n\
             好\thao\t10000\n\
             毫\thao\t100\n\
             里\tli\t10000\n\
             离\tli\t10000\n\
             厘\tli\t100\n",
        )
        .unwrap(),
    );
    engine.set_choice_model(Some(Arc::new(WrongChars(preferred))));
    engine.set_choice_tier(ChoiceTier::Balanced);
    for c in "buchahaoli".chars() {
        engine.push(c);
    }
    let _ = engine.query();
    settle(&mut engine);
    engine
}

#[test]
fn fallback_only_covers_phrase_length_inputs() {
    assert!(!fallback_length(1));
    assert!(!fallback_length(2));
    assert!(!fallback_length(3));
    assert!(fallback_length(4));
    assert!(fallback_length(8));
    assert!(!fallback_length(9));
}

#[test]
fn a_short_input_keeps_the_correct_word_in_the_first_three() {
    use crate::candidate::CandidateKind;

    let engine = wrong_chars_engine("");
    let query = engine.query().unwrap();
    let first_three: Vec<&str> = query
        .candidates
        .items
        .iter()
        .take(3)
        .map(|candidate| candidate.text.as_str())
        .collect();
    assert_ne!(first_three[0], "不差毫厘", "这一例本来就该由整句候选排第一");
    assert!(
        first_three.contains(&"不差毫厘"),
        "重排后正确的词应保在前 3 条，实际 {first_three:?}"
    );
    assert_eq!(
        query.candidates.items[2].kind,
        CandidateKind::Chinese,
        "保底的是词候选，不是整句"
    );
}

#[test]
fn a_phrase_word_outside_the_window_can_still_win() {
    let engine = wrong_chars_engine("不差毫厘");
    let query = engine.query().unwrap();
    assert_eq!(
        query.candidates.items[0].text, "不差毫厘",
        "词候选补进模型窗口后，模型分高就应该能排第一"
    );
}

#[test]
fn sentence_alternates_are_capped_before_the_list() {
    use crate::candidate::CandidateKind;

    let mut engine = Engine::new(
        Dictionary::parse("是\tshi\t1000\n时\tshi\t100\n事\tshi\t10\n十\tshi\t5\n").unwrap(),
    );
    engine.set_choice_model(Some(Arc::new(Prefers("是是时时"))));
    for c in "shishishishi".chars() {
        engine.push(c);
    }
    let query = engine.query().unwrap();
    let sentences = query
        .candidates
        .items
        .iter()
        .filter(|candidate| candidate.kind == CandidateKind::Sentence)
        .count();
    assert!(sentences >= 2, "长输入应有多条整句备选");
    assert!(
        sentences <= crate::engine::SENTENCE_CANDIDATES,
        "整句备选只显示前 {} 条，实际 {sentences}",
        crate::engine::SENTENCE_CANDIDATES
    );
}

fn candidate(text: &str, kind: crate::candidate::CandidateKind) -> crate::candidate::Candidate {
    crate::candidate::Candidate {
        text: text.to_owned(),
        display: None,
        kind,
        syllables: Vec::new(),
        reading: None,
        auxiliary: None,
        auxiliary_typed: None,
    }
}

#[test]
fn a_short_input_keeps_its_best_word_in_front() {
    use crate::candidate::CandidateKind;

    let mut items = vec![
        candidate("不查好里", CandidateKind::Sentence),
        candidate("不查好离", CandidateKind::Sentence),
        candidate("不差好里", CandidateKind::Sentence),
        candidate("不差毫厘", CandidateKind::Chinese),
        candidate("不差", CandidateKind::Chinese),
    ];
    keep_word_candidate_near_top(&mut items, "不差毫厘");
    assert_eq!(items[2].text, "不差毫厘");
    assert_eq!(items.len(), 5);
}

#[test]
fn a_word_already_in_the_first_three_is_not_moved() {
    use crate::candidate::CandidateKind;

    let mut items = vec![
        candidate("不差毫厘", CandidateKind::Chinese),
        candidate("不查好里", CandidateKind::Sentence),
        candidate("不查好离", CandidateKind::Sentence),
        candidate("不差好里", CandidateKind::Sentence),
    ];
    keep_word_candidate_near_top(&mut items, "不差毫厘");
    assert_eq!(items[0].text, "不差毫厘");
}

#[test]
fn altered_readings_get_a_fixed_cost_not_the_table() {
    use cloudime_dictionary::SyllablePattern;

    let index: HashMap<char, u32> = [('再', 0), ('在', 1)].into_iter().collect();
    let table = CharTable::new(Arc::new(index), vec![vec![0.0, -6.0]]);
    let typed = [SyllablePattern::complete("zai")];
    let reading = |s: &str| vec![s.to_owned()];
    assert_eq!(char_emission(&table, &typed, 0, "再", &reading("zai")), 0.0);
    assert_eq!(
        char_emission(&table, &typed, 0, "在", &reading("zai")),
        -6.0 * EMISSION_WEIGHT
    );
    assert_eq!(
        char_emission(&table, &typed, 0, "才", &reading("cai")),
        ALTERED_EMISSION * EMISSION_WEIGHT
    );
}
