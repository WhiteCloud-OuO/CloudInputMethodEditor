//! 查词、切分、光标与上屏消耗。

use super::*;

#[test]
fn weight_orders_exact_longer_and_prefix_hits() {
    // 覆盖满（`kai fa` 读法）的最前，先按末音节完整匹配（开发）再按 `exact`（开发者比 开放 短），
    // 再按权重（开放 20000 > 开饭 800）；前缀词 开 覆盖少，排最后
    assert_eq!(texts("kaifa"), ["开发", "开发者", "开放", "开饭", "开"]);
}

#[test]
fn partial_last_syllable_expands() {
    // 末音节是没打完的 `f`，所有命中 full_last 都为假，按 `exact` 与权重排：开放(20000) > 开发(9000)
    // > 开饭(800)，更长的 开发者 覆盖同一段但 `exact` 为假，跟在其后；前缀词 开 最后
    assert_eq!(texts("kaif"), ["开放", "开发", "开饭", "开发者", "开"]);
}

#[test]
fn initials_match_abbreviated_words_and_commit_consumes_letters() {
    let all = texts("kf");
    // 完整覆盖两个声母的 开放（20000）靠覆盖压过只覆盖 `k` 的前缀词 开
    assert_eq!(all[0], "开放");
    assert!(
        all.contains(&"开放".to_owned())
            && all.contains(&"开发".to_owned())
            && all.contains(&"咖啡".to_owned())
    );

    let mut engine = engine();
    engine.set_input("kfzhe");
    let kaifa = engine
        .query()
        .unwrap()
        .candidates
        .items
        .iter()
        .find(|c| c.text == "开发")
        .unwrap()
        .clone();
    engine.commit(&kaifa);
    assert_eq!(engine.composition().text(), "zhe");
}

#[test]
fn unparsable_tail_is_kept_aside() {
    let mut engine = engine();
    // kaiv 唯一的纠法是删掉刚敲的 v，那不算纠错：v 留作尾巴等下一键（kaifv 会被纠成 kaifa）
    engine.set_input("kaiv");
    let query = engine.query().unwrap();
    assert!(query.correction.is_none());
    assert_eq!(query.tail, "v");
    assert_eq!(query.marked_text(), "kai'v");
    assert_eq!(query.candidates.items[0].text, "开");

    let kaifa = query
        .candidates
        .items
        .iter()
        .find(|c| c.text == "开发")
        .unwrap()
        .clone();
    engine.commit(&kaifa);
    assert_eq!(engine.composition().text(), "v");
    // 剩下的 v 进表达式模式：没候选但也不报错
    assert!(engine.query().unwrap().candidates.items.is_empty());
}

#[test]
fn cursor_edits_requery_from_the_start_and_map_into_marked_text() {
    let mut engine = engine();
    engine.set_input("kaifa");
    engine.move_cursor_left();
    engine.move_cursor_left();
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), "kai'fa");
    assert_eq!(query.marked_cursor(), 3); // kai|'fa

    engine.push('n');
    let query = engine.query().unwrap();
    assert_eq!(query.text, "kainfa");
    assert_eq!(query.marked_cursor(), 5); // kai'n|'fa

    engine.set_input("xi'an");
    engine.move_cursor_left();
    engine.move_cursor_left();
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), "xi'an");
    assert_eq!(query.marked_cursor(), 3); // xi'|an，紧跟用户自己敲的 '
}

#[test]
fn punctuation_follows_committed_text() {
    let mut engine = engine();
    assert_eq!(engine.punctuate(',').as_deref(), Some("，"));
    engine.note_passthrough('3');
    assert_eq!(engine.punctuate('.'), None);
    engine.set_input("kaifa");
    let kaifa = engine.query().unwrap().candidates.items[0].clone();
    engine.commit(&kaifa);
    assert_eq!(engine.punctuate('.').as_deref(), Some("。"));
}

#[test]
fn marked_text_joins_best_segmentation_with_apostrophes() {
    let mut engine = engine();
    engine.set_input("kaifa");
    assert_eq!(engine.query().unwrap().marked_text(), "kai'fa");
    engine.set_input("kf");
    assert_eq!(engine.query().unwrap().marked_text(), "k'f");
}

#[test]
fn prefix_words_sort_by_weight_with_full_matches() {
    // 覆盖满三音节的 开发者（cov7）先；前缀词 开发（`kai fa`，cov5）次之；
    // 再是 `kai f a` 等切法的 开放 / 开饭（cov4），最后是只覆盖 `kai` 的 开（cov3）
    let all = texts("kaifazhe");
    assert_eq!(all, ["开发者", "开发", "开放", "开饭", "开"]);
}

#[test]
fn ambiguous_segmentation_merges_results() {
    let all = texts("xian");
    assert_eq!(all[0], "先");
    assert!(all.contains(&"西安".to_owned()));
}

/// 同一个词同时出现在靠前与靠后的词库里时，只出靠前那本的那条（跨词库去重、靠前优先），
/// 即使靠后那本词频高得多也不顶替；同一本词库里同一个词的多种读法在这一层照旧全收。
#[test]
fn earlier_dictionary_wins_for_the_same_text() {
    // 基础词库靠前、附加词库靠后；两本都有同名的「重开」但读法不同，附加词库的词频高得多
    let mut engine = Engine::new(Dictionary::parse("重开\tzhong kai\t100\n").unwrap());
    engine.set_extra_dictionaries(vec![Dictionary::parse("重开\tzong kai\t999999\n").unwrap()]);
    engine.set_input("zk");
    let items = engine.query().unwrap().candidates.items;
    let hits: Vec<&Candidate> = items.iter().filter(|c| c.text == "重开").collect();
    assert_eq!(hits.len(), 1);
    // 留下的是基础词库那条读法，不是附加词库的 zong
    assert_eq!(hits[0].syllables, ["zhong", "kai"]);

    // 同一本里同一个词按两种读法命中：这一层不去重，两条都在（rank 再按名次挑）
    let engine = Engine::new(Dictionary::parse("重\tzhong\t100\n重\tzong\t100\n").unwrap());
    let positions = vec![vec![cloudime_dictionary::SyllablePattern::prefix("z")]];
    let hits = engine.lookup_all(&positions);
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|hit| hit.text == "重"));
}

#[test]
fn complete_syllable_that_is_also_prefix_expands_after_exact() {
    // 敲的就是完整音节 `xia` = 下：末音节完整匹配的 下 先，前缀扩展来的 先 / 想 / 西安 随后按权重
    assert_eq!(texts("xia"), ["下", "先", "想", "西安"]);
}

#[test]
fn keyboard_u_umlaut_spelling_matches_canonical_dictionary_keys() {
    let dictionary =
        Dictionary::parse("策略\tce lve\t9000\n虐待\tnve dai\t8000\n学习\txue xi\t7000\n").unwrap();
    let mut engine = Engine::new(dictionary);

    engine.set_input("celue");
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), "ce'lue");
    assert_eq!(query.candidates.items[0].text, "策略");
    assert_eq!(query.candidates.items[0].syllables, ["ce", "lve"]);
    let strategy = query.candidates.items[0].clone();
    assert_eq!(engine.commit(&strategy), "策略");
    assert!(engine.composition().is_empty());

    engine.set_input("nuedai");
    assert_eq!(engine.query().unwrap().candidates.items[0].text, "虐待");

    engine.set_input("xuexicelue");
    let sentence = &engine.query().unwrap().candidates.items[0];
    assert_eq!(sentence.text, "学习策略");
    assert_eq!(sentence.kind, CandidateKind::Sentence);
}

#[test]
fn empty_input_is_an_error() {
    assert_eq!(engine().query().unwrap_err(), ParseError::Empty);
}

#[test]
fn commit_consumes_only_the_candidate_syllables() {
    let mut engine = engine();
    engine.set_input("kaifazhe");
    let kaifa = engine
        .query()
        .unwrap()
        .candidates
        .items
        .iter()
        .find(|c| c.text == "开发")
        .unwrap()
        .clone();
    assert_eq!(engine.commit(&kaifa), "开发");
    assert_eq!(engine.composition().text(), "zhe");

    engine.set_input("kaif");
    assert_eq!(engine.commit(&kaifa), "开发");
    assert!(engine.composition().is_empty());

    engine.set_input("xi'an");
    let xian = engine
        .query()
        .unwrap()
        .candidates
        .items
        .iter()
        .find(|c| c.text == "西安")
        .unwrap()
        .clone();
    engine.commit(&xian);
    assert!(engine.composition().is_empty());
}

#[test]
fn take_raw_returns_pinyin_and_clears() {
    let mut engine = engine();
    engine.set_input("kaifa");
    assert_eq!(engine.take_raw(), "kaifa");
    assert!(engine.composition().is_empty());
}

#[test]
fn choice_key_strips_separators_and_clamps() {
    assert_eq!(choice_key("kai'fa", 6), "kaifa");
    assert_eq!(choice_key("kai'fa", 3), "kai");
    assert_eq!(choice_key("ba", 10), "ba");
}

#[test]
fn sentence_conversion_competes_with_word_hits_by_weight() {
    // 想开发：SAMPLE 里没有整词，最优路径是 想 + 开发；整句进同一个按权重排的池子，不再无条件第一
    let mut engine = engine();
    engine.set_input("xiangkaifa");
    let query = engine.query().unwrap();
    let sentence = query
        .candidates
        .items
        .iter()
        .find(|c| c.kind == CandidateKind::Sentence)
        .cloned()
        .expect("整句候选");
    assert_eq!(sentence.text, "想开发");
    assert_eq!(sentence.syllables, ["xiang", "kai", "fa"]);
    // 整句权重是路径词权重的几何平均：想 9000 与 开发 9000 → 9000
    assert_eq!(engine.commit(&sentence), "想开发");
    assert!(engine.composition().is_empty());

    // 整段本身是一个词：不出整句，词只出现一次
    let all = texts("kaifa");
    assert_eq!(all[0], "开发");
    assert_eq!(all.iter().filter(|t| *t == "开发").count(), 1);

    // 末尾只有一个字母时整句不算它：xiangkaif → 想开
    let mut engine = super::engine();
    engine.set_input("xiangkaif");
    let query = engine.query().unwrap();
    assert!(
        query
            .candidates
            .items
            .iter()
            .any(|c| c.kind == CandidateKind::Sentence && c.text == "想开")
    );
}

#[test]
fn shortcuts_follow_the_first_local_candidate() {
    let all = texts("rq");
    let shortcut = all.iter().position(|t| t.ends_with('日')).unwrap();
    assert!(shortcut <= 1);
    assert!(all.iter().any(|t| t.contains('-')));

    let mut engine = engine();
    engine.set_input("xq");
    let query = engine.query().unwrap();
    let weekday = query
        .candidates
        .items
        .iter()
        .find(|c| c.kind == CandidateKind::Shortcut)
        .unwrap()
        .clone();
    assert!(weekday.text.starts_with("星期"));
    engine.commit(&weekday);
    assert!(engine.composition().is_empty());
}

#[test]
fn shift_letters_join_the_buffer() {
    let dictionary = Dictionary::parse("C盘\tc pan\t8000\n磁盘\tci pan\t249\n").unwrap();
    let mut engine = Engine::new(dictionary);
    // 中文模式下按住 Shift 敲 C，再打 pan：大写按小写参与匹配（出「C盘」），
    // 拼音行按敲的样子显示，回车原样上屏时保留大写。
    engine.push('C');
    for c in "pan".chars() {
        engine.push(c);
    }
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "C盘");
    assert_eq!(query.marked_text(), "C'pan");
    assert_eq!(engine.take_raw(), "Cpan");
    assert!(engine.composition().is_empty());
}

#[test]
fn expression_mode_skips_pinyin_and_evaluates() {
    let mut engine = self::engine();
    assert!(!engine.expression_mode());
    engine.set_input("v1+2");
    assert!(engine.expression_mode());
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), "v1+2");
    assert_eq!(query.marked_cursor(), 4);
    assert_eq!(query.candidates.items[0].text, "3");
    assert_eq!(query.candidates.items[0].kind, CandidateKind::Shortcut);
    assert_eq!(query.candidates.items[1].text, "1+2=3");
    let result = query.candidates.items[0].clone();
    assert_eq!(engine.commit(&result), "3");
    assert!(engine.composition().is_empty());

    // 只有 v：候选为空但不报错，preedit 照显示
    engine.set_input("v");
    let query = engine.query().unwrap();
    assert!(query.candidates.items.is_empty());
    assert_eq!(query.marked_text(), "v");

    // v 开头的英文词仍能混输
    let words = WordList::parse("very\tvery\t4800\n").unwrap();
    let mut engine = self::engine().with_english(words);
    engine.set_input("very");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "very");
    assert_eq!(query.candidates.items[0].kind, CandidateKind::English);
}

/// 只认句首的 开发 与 开发 → 先 的假模型：让两词路径压过整段的词。
struct XianModel;

impl LanguageModel for XianModel {
    fn log_prob(&self, previous: Option<&str>, word: &str) -> Option<f64> {
        match (previous, word) {
            (None, "开发") => Some(-1.0),
            (Some("开发"), "先") => Some(-0.5),
            _ => None,
        }
    }
}

#[test]
fn a_word_spelling_the_sentence_keeps_its_rank_unless_its_reading_differs() {
    // 整句读出 开发 + 先 时，与词 开发先 同文本同读音：不重复插，词留在池子里（只出现一次）
    let sample = format!("{SAMPLE}开发线\tkai fa xian\t5000\n开发先\tkai fa xian\t1\n");
    let mut engine =
        Engine::new(Dictionary::parse(&sample).unwrap()).with_language_model(Box::new(XianModel));
    engine.set_input("kaifaxian");
    let all = texts_of(&engine);
    assert!(all.contains(&"开发线".to_owned()));
    assert_eq!(all.iter().filter(|t| *t == "开发先").count(), 1);

    // 同文本的词是按别的读音（xiang，靠模糊音 an-ang 对上）收的：那条错读音的词让位，整句以正确读音顶上
    let sample = format!("{SAMPLE}开发线\tkai fa xian\t5000\n开发先\tkai fa xiang\t1\n");
    let mut engine =
        Engine::new(Dictionary::parse(&sample).unwrap()).with_language_model(Box::new(XianModel));
    engine.set_fuzzy(FuzzyRules {
        an_ang: true,
        ..FuzzyRules::default()
    });
    engine.set_input("kaifaxian");
    let items = engine.query().unwrap().candidates.items;
    let sentence = items
        .iter()
        .find(|c| c.kind == CandidateKind::Sentence && c.text == "开发先")
        .expect("整句以正确读音顶上");
    assert_eq!(sentence.syllables, ["kai", "fa", "xian"]);
    assert_eq!(items.iter().filter(|c| c.text == "开发先").count(), 1);
}

#[test]
fn delete_syllable_backward_deletes_a_syllable_and_delete_to_start_deletes_to_the_start() {
    let mut engine = engine();
    // 全拼：删最优切分的最后一个音节；`'` 连同前面的音节一起删；切不动的尾巴整个删
    engine.set_input("kaifaxian");
    assert!(engine.delete_syllable_backward());
    assert_eq!(engine.composition().text(), "kaifa");
    engine.set_input("kai'fa'");
    assert!(engine.delete_syllable_backward());
    assert_eq!(engine.composition().text(), "kai'");
    engine.set_input("kaifv");
    assert!(engine.delete_syllable_backward());
    assert_eq!(engine.composition().text(), "kaif");
    assert!(engine.delete_syllable_backward());
    assert_eq!(engine.composition().text(), "kai");
    // 光标停在中间：只动光标前的
    engine.set_input("kaifaxian");
    for _ in 0..4 {
        engine.move_cursor_left();
    }
    assert!(engine.delete_syllable_backward());
    assert_eq!(engine.composition().text(), "kaixian");
    assert_eq!(engine.composition().cursor(), 3);
    assert!(engine.delete_to_start());
    assert_eq!(engine.composition().text(), "xian");
    assert_eq!(engine.composition().cursor(), 0);
    assert!(!engine.delete_syllable_backward());
    assert!(!engine.delete_to_start());
    // 英文直输段：字母一段一段删，标点一次一个
    engine.set_input("hello,world");
    assert!(engine.delete_syllable_backward());
    assert_eq!(engine.composition().text(), "hello,");
    assert!(engine.delete_syllable_backward());
    assert_eq!(engine.composition().text(), "hello");
}

#[test]
fn cursor_moves_by_syllable() {
    let mut engine = engine();
    // 光标在末尾发现前面错了：左跳音节两下跳到第二个音节后面，删音节删掉它，重敲，右跳音节回末尾
    engine.set_input("kaifa'xian'xia");
    assert!(engine.move_cursor_syllable_left());
    assert_eq!(engine.composition().cursor(), "kaifa'xian'".len());
    assert!(engine.move_cursor_syllable_left());
    assert_eq!(engine.composition().cursor(), "kaifa'".len());
    assert!(engine.move_cursor_syllable_left());
    assert_eq!(engine.composition().cursor(), "kai".len());
    assert!(engine.delete_syllable_backward());
    assert_eq!(engine.composition().text(), "fa'xian'xia");
    engine.push('x');
    engine.push('i');
    assert_eq!(engine.composition().text(), "xifa'xian'xia");
    // 右跳音节：从 xi| 起跳过一个音节到 xifa|，再跳先越过 `'` 再过一个音节
    assert!(engine.move_cursor_syllable_right());
    assert_eq!(engine.composition().cursor(), "xifa".len());
    assert!(engine.move_cursor_syllable_right());
    assert_eq!(engine.composition().cursor(), "xifa'xian".len());
    assert!(engine.move_cursor_syllable_right());
    assert_eq!(engine.composition().cursor(), "xifa'xian'xia".len());
    assert!(!engine.move_cursor_syllable_right());
    engine.move_cursor_home();
    assert!(!engine.move_cursor_syllable_left());
    // 直输段按字母段跳
    engine.set_input("hello,world");
    assert!(engine.move_cursor_syllable_left());
    assert_eq!(engine.composition().cursor(), "hello,".len());
    engine.move_cursor_home();
    assert!(engine.move_cursor_syllable_right());
    assert_eq!(engine.composition().cursor(), "hello".len());
}
