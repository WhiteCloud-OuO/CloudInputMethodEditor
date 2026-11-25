//! 形码输入：前缀查询、按权重排序、上屏消耗、大小写与 v 模式。

use super::*;

const SHAPE_TABLE: &str = "\
a\t工\t20
a\t戈\t10
aa\t式\t20
aaa\t工\t10
aaaa\t工\t20
aaaa\t恭恭敬敬\t10
w\t人\t20
ww\t从\t10
wqvb\t你好\t20
wqvb\t您好\t10
fcms\t云朵\t10
";

/// 挂上形码码表的引擎（拼音词库只给 z 模式用）。
fn shape_engine() -> Engine {
    let mut engine =
        Engine::new(Dictionary::parse("云朵\tyun duo\t9000\n你好\tni hao\t8000\n").unwrap());
    let mut table = crate::shape::Table::parse(SHAPE_TABLE, 4).0;
    let problems = table.load_level_one("a\t工\ng\t一\nw\t人\n");
    assert!(problems.is_empty(), "{problems:?}");
    engine.set_shape(Some(table));
    engine
}

fn candidates(engine: &Engine) -> Vec<String> {
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
fn typing_a_prefix_lists_its_candidates_by_weight() {
    let mut engine = shape_engine();
    engine.set_input("a");
    assert_eq!(candidates(&engine), ["工", "式", "戈", "恭恭敬敬"]);
    let query = engine.query().unwrap();
    assert_eq!(query.tail, "a", "拼音行显示敲的编码");
    assert_eq!(query.marked_text(), "a");
    assert!(query.segmentations.is_empty(), "形码没有拼音切分");
    assert!(query.candidates.items.iter().all(|candidate| {
        candidate.kind == CandidateKind::Chinese && candidate.syllables.len() == 1
    }));

    // 四键是完整码，只剩这一条编码下的候选
    engine.set_input("aaaa");
    assert_eq!(candidates(&engine), ["工", "恭恭敬敬"]);
}

#[test]
fn selecting_a_candidate_consumes_the_whole_code() {
    let mut engine = shape_engine();
    engine.set_input("wqvb");
    let first = engine.query().unwrap().candidates.items[0].clone();
    assert_eq!(first.text, "你好");
    assert_eq!(engine.commit(&first).as_deref(), Some("你好"));
    assert!(engine.composition().is_empty());
}

#[test]
fn shifted_letters_are_still_code_keys() {
    let mut engine = shape_engine();
    // Shift / Caps Lock 卡着敲的编码键按小写参与匹配，拼音行按原样显示
    engine.set_input("Wqvb");
    assert_eq!(engine.composition().scope(), "wqvb");
    assert_eq!(engine.composition().typed_scope(), "Wqvb");
    let query = engine.query().unwrap();
    assert_eq!(query.tail, "Wqvb");
    assert_eq!(
        query
            .candidates
            .items
            .iter()
            .map(|candidate| candidate.text.as_str())
            .collect::<Vec<_>>(),
        ["你好", "您好"]
    );
}

#[test]
fn v_is_a_code_key_instead_of_the_expression_prefix() {
    let mut engine = shape_engine();
    engine.set_input("v");
    assert!(
        !engine.expression_mode(),
        "形码下 v 只是一个编码键，不进表达式"
    );
    assert!(!engine.v_mode_shortcut_active());
    assert!(engine.query().is_err(), "码表里没有 v，按切不动的串处理");
}

#[test]
fn backspacing_returns_to_the_previous_prefix() {
    let mut engine = shape_engine();
    engine.set_input("aaa");
    assert_eq!(candidates(&engine), ["工", "恭恭敬敬"]);
    assert!(engine.backspace());
    assert_eq!(candidates(&engine), ["式", "工", "恭恭敬敬"]);
}

#[test]
fn moving_the_cursor_into_the_code_keeps_the_preedit_raw() {
    let mut engine = shape_engine();
    engine.set_input("wqvb");
    engine.move_cursor_left();
    engine.move_cursor_left();
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), "wqvb", "形码不插 `'` 分音节");
    assert_eq!(query.marked_cursor(), 2, "光标落在敲了两个键的位置");
    assert_eq!(query.candidates.items[0].text, "你好");
}

#[test]
fn unmounting_the_table_returns_to_full_pinyin() {
    let mut engine = shape_engine();
    engine.set_input("a");
    assert!(engine.query().is_ok());
    engine.clear();
    engine.set_shape(None);
    engine.set_input("a");
    // 空词库里 a 切不出候选；这里只确认没有再走形码那条路（没有「工」）
    assert!(
        engine
            .query()
            .map(|query| query.candidates.items.is_empty())
            .unwrap_or(true)
    );
}

#[test]
fn a_single_key_puts_the_level_one_shortcut_first() {
    let mut engine = shape_engine();
    engine.set_input("g");
    let query = engine.query().unwrap();
    let first = &query.candidates.items[0];
    assert_eq!(first.text, "一");
    assert_eq!(first.auxiliary.as_deref(), Some("①"), "一级简码带 [①] 角标");
    assert_eq!(first.kind, CandidateKind::Chinese);
    assert_eq!(
        engine.commit(first).as_deref(),
        Some("一"),
        "一级简码只吃一个键"
    );
    assert!(engine.composition().is_empty());
}

#[test]
fn a_wildcard_z_shows_as_a_star_and_matches_one_letter() {
    let mut engine = shape_engine();
    engine.set_input("wqzb");
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), "wq*b", "z 通配在拼音行显示成 *");
    assert_eq!(query.candidates.items[0].text, "你好");
}

#[test]
fn z_mode_runs_pinyin_and_annotates_the_wubi_code() {
    let mut engine = shape_engine();
    engine.set_input("znihao");
    assert!(engine.shape_z_mode());
    assert_eq!(engine.shape_code_len(), None, "z 模式不算纯五笔");
    let query = engine.query().unwrap();
    assert_eq!(query.marked_text(), "znihao", "z 模式不插拼音分隔符");
    let candidate = query
        .candidates
        .items
        .iter()
        .find(|candidate| candidate.text == "你好")
        .expect("拼音引擎出 你好");
    assert_eq!(
        candidate.auxiliary.as_deref(),
        Some("wqvb"),
        "右侧标五笔完整码"
    );
    assert_eq!(engine.commit(candidate).as_deref(), Some("你好"));
    assert!(engine.composition().is_empty(), "选完整段（含前导 z）");
}

#[test]
fn english_mixed_input_appends_english_candidates_after_chinese() {
    let mut engine = shape_engine();
    engine = engine.with_english(
        cloudime_dictionary::WordList::parse("hello\thello\t1000\nhelp\thelp\t500\n").unwrap(),
    );
    engine.set_shape_mixture_input(true);

    engine.set_input("hel");
    let items = engine.query().unwrap().candidates.items;
    let english: Vec<&str> = items
        .iter()
        .filter(|candidate| candidate.kind == CandidateKind::English)
        .map(|candidate| candidate.text.as_str())
        .collect();
    assert_eq!(english, ["hello", "help"]);

    // 有中文候选时英文排在中文后面
    engine.set_input("w");
    let items = engine.query().unwrap().candidates.items;
    let last_chinese = items
        .iter()
        .rposition(|candidate| candidate.kind == CandidateKind::Chinese)
        .unwrap();
    assert!(
        items[last_chinese + 1..]
            .iter()
            .all(|candidate| candidate.kind == CandidateKind::English),
        "英文只追加在中文候选之后"
    );

    // 关掉英文混输就没有英文候选
    engine.set_shape_mixture_input(false);
    assert!(
        engine
            .query()
            .unwrap()
            .candidates
            .items
            .iter()
            .all(|candidate| candidate.kind != CandidateKind::English)
    );
}

/// 假选词模型：盖住整段的候选里偏爱给定的文本。
struct PrefersChoice(&'static str);

impl ChoiceModel for PrefersChoice {
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
}

/// 假整句打分器：偏爱给定的文本。
struct PrefersSentence(&'static str);

impl SentenceScorer for PrefersSentence {
    fn score(&self, _context: &str, texts: &[&str]) -> Vec<f64> {
        texts
            .iter()
            .map(|text| if *text == self.0 { -1.0 } else { -20.0 })
            .collect()
    }
}

/// 等到选词模型的分回来并重画当前页。
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

/// 纯五笔不接本地模型：挂了选词模型也照旧按码表权重出，连打分请求都不排。
#[test]
fn pure_shape_candidates_ignore_the_local_model() {
    let mut engine = shape_engine();
    engine.set_choice_model(Some(Arc::new(PrefersChoice("式"))));
    engine.set_input("a");
    assert_eq!(candidates(&engine), ["工", "式", "戈", "恭恭敬敬"]);
    assert!(!engine.rescoring_pending(), "纯五笔不该给模型排任务");
}

/// z 模式仍是拼音查询：选词模型在临时拼音里照常重排候选。
#[test]
fn z_mode_uses_the_choice_model() {
    let mut engine =
        Engine::new(Dictionary::parse("开发\tkai fa\t9000\n开罚\tkai fa\t10\n").unwrap());
    engine.set_shape(Some(crate::shape::Table::parse(SHAPE_TABLE, 4).0));
    engine.set_choice_model(Some(Arc::new(PrefersChoice("开罚"))));
    engine.set_input("zkaifa");
    assert_eq!(candidates(&engine), ["开发", "开罚"]);
    assert!(engine.rescoring_pending());
    settle(&mut engine);
    assert_eq!(candidates(&engine), ["开罚", "开发"]);
    assert!(!engine.rescoring_pending());
}

/// z 模式仍是拼音查询：本地整句模型照常给整句路径重排。
#[test]
fn z_mode_uses_the_sentence_model() {
    let dictionary = "在\tzai\t9000\n再\tzai\t8500\n见\tjian\t9000\n剑\tjian\t8500\n";
    let mut plain = Engine::new(Dictionary::parse(dictionary).unwrap());
    plain.set_shape(Some(crate::shape::Table::parse(SHAPE_TABLE, 4).0));
    plain.set_input("zzaijian");
    assert_eq!(candidates(&plain)[0], "在见");

    let mut engine = Engine::new(Dictionary::parse(dictionary).unwrap()).with_sentence_scorer(
        Box::new(PrefersSentence("在剑")),
        None,
        None,
        None,
    );
    engine.set_shape(Some(crate::shape::Table::parse(SHAPE_TABLE, 4).0));
    engine.set_input("zzaijian");
    assert_eq!(candidates(&engine)[0], "在剑");
}
