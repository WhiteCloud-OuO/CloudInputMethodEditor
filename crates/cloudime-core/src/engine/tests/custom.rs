use super::*;
use crate::custom_phrase::{DEFAULT_POSITION, MAX_POSITION};
use crate::{CandidateLayout, CustomPhrase};

fn phrase(code: &str, position: u32, text: &str) -> CustomPhrase {
    CustomPhrase {
        code: code.into(),
        text: text.into(),
        title: None,
        position,
    }
}

#[test]
fn custom_phrases_land_on_their_positions() {
    let mut e = engine();
    e.set_custom_phrases(vec![phrase("ee", 2, "；"), phrase("ee", 1, "：")])
        .unwrap();
    e.set_input("ee");
    for _ in 0..3 {
        let q = e.query().unwrap();
        assert_eq!(q.candidates.items[0].text, "：");
        assert_eq!(q.candidates.items[1].text, "；");
        let layout = CandidateLayout::new(q.candidates.items, 2);
        assert_eq!(layout.candidate(1).unwrap().text, "；");
    }
    let c = e.query().unwrap().candidates.items[1].clone();
    assert_eq!(e.commit(&c).as_deref(), Some("；"));
    assert!(e.composition().text().is_empty());
}

#[test]
fn custom_phrase_title_shows_in_the_candidate_but_commits_the_text() {
    let mut e = engine();
    e.set_custom_phrases(vec![CustomPhrase {
        code: "omw".into(),
        text: "On my way!".into(),
        title: Some("在路上".into()),
        position: 1,
    }])
    .unwrap();
    e.set_input("omw");
    let candidate = e.query().unwrap().candidates.items[0].clone();
    assert_eq!(candidate.text, "On my way!");
    assert_eq!(candidate.display_text(), "在路上");
    assert_eq!(e.commit(&candidate).as_deref(), Some("On my way!"));
}

#[test]
fn the_same_position_keeps_the_saved_order() {
    let mut e = engine();
    // 同码、同位置：按保存顺序占位（先存的在前），不会互相挤掉
    e.set_custom_phrases(vec![phrase("zz", 1, "乙"), phrase("zz", 1, "甲")])
        .unwrap();
    e.set_input("zz");
    let items = e.query().unwrap().candidates.items;
    assert_eq!(
        items.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
        ["乙", "甲"]
    );
}

#[test]
fn positions_past_the_candidate_list_go_last() {
    let mut e = engine();
    e.set_custom_phrases(vec![phrase("xian", MAX_POSITION, "末尾")])
        .unwrap();
    e.set_input("xian");
    // 位置 9 超出候选数（词库没那么多）：落到最后
    let items = e.query().unwrap().candidates.items;
    assert_eq!(items.last().unwrap().text, "末尾");
    assert!(items.len() > 1);
}

#[test]
fn custom_phrases_beat_dictionary_candidates_with_the_same_code() {
    let mut e = engine();
    e.set_custom_phrases(vec![phrase("xian", 1, "短语")])
        .unwrap();
    e.set_input("xian");
    let items = e.query().unwrap().candidates.items;
    assert_eq!(items[0].text, "短语");
    assert_eq!(items[0].kind, CandidateKind::Custom);
    // 词库候选仍在，只是排在短语后面
    assert!(items.iter().skip(1).any(|c| c.text == "先"));
}

#[test]
fn invalid_updates_are_rejected_and_keep_the_old_rules() {
    let mut e = engine();
    e.set_custom_phrases(vec![phrase("aa", 1, "，")]).unwrap();
    // 同码同文本不能重复
    assert!(
        e.set_custom_phrases(vec![phrase("aa", 1, "，"), phrase("aa", 2, "，")])
            .is_err()
    );
    // 输入码必须是小写字母
    assert!(e.set_custom_phrases(vec![phrase("AA", 1, "大写")]).is_err());
    // 位置必须在 1–9 内
    assert!(e.set_custom_phrases(vec![phrase("aa", 0, "越界")]).is_err());
    assert!(
        e.set_custom_phrases(vec![phrase("aa", MAX_POSITION + 1, "越界")])
            .is_err()
    );
    e.set_input("aa");
    assert_eq!(e.query().unwrap().candidates.items[0].text, "，");
}

#[test]
fn custom_long_text_exact_keys() {
    let mut e = engine();
    let text = format!("{}\n  end ", "长文本".repeat(12000));
    e.set_custom_phrases(vec![phrase("abcdefghij", 1, &text)])
        .unwrap();
    e.set_input("abcdefghij");
    let query = e.query().unwrap();
    let layout = CandidateLayout::new(query.candidates.items, 9);
    let c = layout.candidate(0).unwrap().clone();
    assert_eq!(e.commit(&c).as_deref(), Some(text.as_str()));
    e.set_custom_phrases(vec![phrase("ii", DEFAULT_POSITION, "目标")])
        .unwrap();
    e.set_input("ii");
    let layout = CandidateLayout::new(e.query().unwrap().candidates.items, 9);
    assert_eq!(layout.candidate(0).unwrap().text, "目标");
    assert!(layout.local().iter().all(|c| !c.text.is_empty()));
    assert!(
        e.last_query
            .borrow()
            .as_ref()
            .unwrap()
            .candidates
            .iter()
            .all(|s| !s.is_empty())
    );
    assert_eq!(e.composition().text(), "ii");
}

#[test]
fn punctuation_mode_does_not_change_custom_text() {
    let mut e = engine();
    e.set_full_width_punctuation(false);
    for c in [',', ';', ':', 'a', '1'] {
        assert_eq!(e.punctuate(c), None);
    }
    e.set_custom_phrases(vec![phrase("bb", 1, "；")]).unwrap();
    e.set_input("bb");
    let c = e.query().unwrap().candidates.items[0].clone();
    assert_eq!(e.commit(&c).as_deref(), Some("；"));
    e.set_full_width_punctuation(true);
    assert_eq!(e.punctuate(';').as_deref(), Some("；"));
}

#[test]
fn custom_exact_codes_override_mode_prefixes_but_not_longer_input() {
    let mut e = engine();
    e.set_custom_phrases(vec![phrase("vv", 1, "固定"), phrase("uu", 2, "文本")])
        .unwrap();
    e.set_input("vv");
    assert!(!e.expression_mode());
    assert_eq!(e.query().unwrap().candidates.items[0].text, "固定");
    e.set_input("vvv");
    assert!(e.expression_mode());
    assert!(
        e.query()
            .unwrap()
            .candidates
            .items
            .iter()
            .all(|c| c.text != "固定")
    );
    e.set_input("uu");
    e.set_english_mode(true);
    assert!(
        e.query()
            .unwrap()
            .candidates
            .items
            .iter()
            .all(|c| c.text != "文本")
    );
}

#[test]
fn custom_only_query_preserves_raw_preedit_and_logs_custom_source() {
    let mut e = engine();
    e.set_custom_phrases(vec![phrase("ii", MAX_POSITION, "目标")])
        .unwrap();
    e.set_input("ii");
    let query = e.query().unwrap();
    assert_eq!(query.marked_text(), "ii");
    assert_eq!(query.marked_cursor(), 2);
    assert_eq!(query.candidates.items.len(), 1);
    assert_eq!(query.candidates.items[0].kind, CandidateKind::Custom);
    assert_eq!(
        InputSource::from(query.candidates.items[0].kind),
        InputSource::Custom
    );
}

#[test]
fn custom_preview_handles_unicode_controls_and_limits() {
    assert_eq!(CustomPhrase::preview("甲\r\n乙\t😀😀尾", 5), "甲↵乙⇥😀…");
    assert_eq!(CustomPhrase::preview("甲\n乙", 3), "甲↵乙");
    assert_eq!(CustomPhrase::preview("甲", 0), "…");
    assert_eq!(CustomPhrase::preview("", 0), "");
}
