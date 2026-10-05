//! 拼写纠错 / 敲错边 / 模糊音。

use super::*;

/// 少敲一个键的音节级变体进词图：整段都是合法音节的 `meiganxi` 里，`gan` 补成 `guan` 转出 没关系。
#[test]
fn missing_key_syllable_variant_reaches_meiguanxi() {
    let dictionary = Dictionary::parse(
        "没关系\tmei guan xi\t800000\n关系\tguan xi\t500000\n没\tmei\t900000\n干\tgan\t200000\n\
             洗\txi\t100000\n美感\tmei gan\t300000\n",
    )
    .unwrap();
    let mut engine = Engine::new(dictionary);
    engine.set_input("meiganxi");
    let items = engine.query().unwrap().candidates.items;
    assert!(items.iter().any(|c| c.text == "没关系"));
    // 少敲边只是多一种读法：原样的 美感 仍在
    assert!(items.iter().any(|c| c.text == "美感"));

    // 敲的拼音本身正好是一个词（按另一种切分）：技能 仍在，不会被读成 近藤
    let dictionary = Dictionary::parse(
        "技能\tji neng\t300000\n近藤\tjin teng\t900000\n近\tjin\t500000\n\
             机\tji\t400000\n能\tneng\t600000\n",
    )
    .unwrap();
    let mut engine = Engine::new(dictionary);
    engine.set_input("jineng");
    let items = engine.query().unwrap().candidates.items;
    assert!(items.iter().any(|c| c.text == "技能"));
    assert!(items.iter().all(|c| c.text != "近藤"));
}

/// 模糊音命中的词按敲的字母消耗拼音（`zi` 对 `zhi`），不算敲错。
#[test]
fn fuzzy_hits_consume_the_typed_syllables() {
    let dictionary =
        Dictionary::parse("知识\tzhi shi\t500000\n只是\tzhi shi\t600000\n资\tzi\t1000\n").unwrap();
    let mut engine =
        Engine::new(dictionary).with_learner(Box::new(CountingLearner(HashMap::new())));
    engine.set_fuzzy(FuzzyRules {
        z_zh: true,
        ..FuzzyRules::default()
    });
    engine.set_input("zishi");
    let zhishi = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.text == "知识")
        .unwrap();
    engine.commit(&zhishi);
    assert!(engine.composition().is_empty());
    assert_eq!(engine.learner().typo_count("zi", "zhi"), 0);
}

/// 纠错读法上屏时按敲的字母对齐、并把那一处敲错记进个人敲错表。
#[test]
fn accepted_correction_reading_aligns_and_records_the_typo() {
    let dictionary = Dictionary::parse(
        "你好吗\tni hao ma\t5000\n你好\tni hao\t9000\n你\tni\t90000\n好\thao\t80000\n吗\tma\t70000\n",
    )
    .unwrap();
    let mut engine =
        Engine::new(dictionary).with_learner(Box::new(CountingLearner(HashMap::new())));
    // a / o 换位：nihoama → nihaoma，纠错读法带出 你好吗，原样读法的 你 也保留
    engine.set_input("nihoama");
    let query = engine.query().unwrap();
    assert!(query.correction.is_none());
    let nihaoma = query
        .candidates
        .items
        .iter()
        .find(|c| c.text == "你好吗")
        .cloned()
        .expect("纠错读法带出 你好吗");
    assert_eq!(engine.commit(&nihaoma).as_deref(), Some("你好吗"));
    assert_eq!(engine.learner().typo_count("hoa", "hao"), 1);
}

/// 纠错读法只进同一个候选池，不替换原样读法；拼音行按敲的字母显示，不画删除线。
#[test]
fn correction_readings_join_the_pool_without_replacing_the_original() {
    let dictionary = Dictionary::parse(
        "你好吗\tni hao ma\t5000\n你好\tni hao\t9000\n你\tni\t90000\n好\thao\t80000\n吗\tma\t70000\n\
             和\the\t50000\n何\the\t3000\n咯\tlo\t100\n你猴\tni hou\t10\n",
    )
    .unwrap();
    let mut engine =
        Engine::new(dictionary).with_learner(Box::new(CountingLearner(HashMap::new())));
    engine.set_input("nihoama");
    let query = engine.query().unwrap();
    assert!(query.correction.is_none());
    // 原样读法的 你 与纠错读法带出的 你好 都在
    assert!(query.candidates.items.iter().any(|c| c.text == "你"));
    assert!(query.candidates.items.iter().any(|c| c.text == "你好"));
    // 拼音行只用敲的字母（没有 Corrected 段）
    assert!(
        query
            .marked_segments()
            .iter()
            .all(|s| s.kind != MarkedKind::Corrected)
    );
    // 选纠错读法的词照样能上屏
    let nihao = query
        .candidates
        .items
        .iter()
        .find(|c| c.text == "你好")
        .cloned()
        .expect("你好");
    // 你好 只吃掉拼音前段（`nihoa`，剩下 ma）：选中是并进组句，不立刻上屏
    assert_eq!(engine.commit(&nihao), None);
    assert_eq!(engine.composition().selected_text(), "你好");
    // 回车把已选文本 + 剩余拼音原样整体上屏
    assert_eq!(engine.take_raw(), "你好ma");
}

#[test]
fn transposition_reading_with_an_unfinished_last_syllable_still_yields_the_word() {
    let dictionary = Dictionary::parse(
        "明天\tming tian\t8000\n明\tming\t12000\n天\ttian\t9000\n米\tmi\t3000\n给\tgei\t5000\n\
         你\tni\t90000\n太\ttai\t4000\n啊\ta\t6000\n",
    )
    .unwrap();
    let mut engine = Engine::new(dictionary);
    // 敲反 gn 之后还在往下敲：mignt 的换位读法 ming t… 带出 明天
    for input in ["mignt", "migntia", "migntian"] {
        engine.set_input(input);
        let query = engine.query().unwrap();
        assert!(query.correction.is_none());
        assert!(
            query.candidates.items.iter().any(|c| c.text == "明天"),
            "{input}"
        );
    }
}

#[test]
fn commit_alignment_backtracks_over_typo_variants() {
    let engine = engine();
    let syllables =
        |list: &[&str]| -> Vec<String> { list.iter().map(|s| (*s).to_owned()).collect() };
    // pingyin 上屏 拼音：pin 原样只吃三个字母会剩 gyin，退回来按敲错变体 ping → pin 吃四个，整段吃光并记敲错
    let alignment = engine.align("pingyin", &syllables(&["pin", "yin"]));
    assert_eq!(alignment.consumed, 7);
    assert_eq!(alignment.typos, [("ping".to_owned(), "pin".to_owned())]);
    // 原样、没打完的前缀、前缀词、带 ' 的输入照旧
    assert_eq!(
        engine.align("nihao", &syllables(&["ni", "hao"])).consumed,
        5
    );
    assert_eq!(
        engine
            .align("mingt", &syllables(&["ming", "tian"]))
            .consumed,
        5
    );
    assert_eq!(
        engine.align("nihaoma", &syllables(&["ni", "hao"])).consumed,
        5
    );
    assert_eq!(
        engine.align("ni'hao", &syllables(&["ni", "hao"])).consumed,
        6
    );
    // 对不上的候选还是贪心对到哪算哪
    assert_eq!(engine.align("nihao", &syllables(&["ni", "ma"])).consumed, 2);
}

#[test]
fn unfinished_last_syllable_is_not_recorded_as_a_typo() {
    let engine = engine();
    let syllables =
        |list: &[&str]| -> Vec<String> { list.iter().map(|s| (*s).to_owned()).collect() };
    // shijia 选 时间 是 jian 没敲完：jia 虽是 jian 的敲错变体，也不进个人敲错表
    for (input, list) in [
        ("shijia", ["shi", "jian"]),
        ("zhegua", ["zhe", "guan"]),
        ("woxia", ["wo", "xian"]),
    ] {
        let alignment = engine.align(input, &syllables(&list));
        assert_eq!(alignment.consumed, input.len());
        assert!(alignment.typos.is_empty(), "{input}: {:?}", alignment.typos);
    }
}

#[test]
fn fuzzy_rules_add_homophones_behind_exact_hits() {
    // 词库里只有 kai fa 系列加一个 哈；敲 kaiha 没开 f/h 时只有前缀词 开
    let mut engine = Engine::new(Dictionary::parse(&format!("{SAMPLE}哈\tha\t50000\n")).unwrap());
    engine.set_input("kaiha");
    let before = texts_of(&engine);
    assert!(!before.contains(&"开发".to_owned()));
    assert!(before.contains(&"开".to_owned()));
    engine.set_fuzzy(FuzzyRules {
        f_h: true,
        ..FuzzyRules::default()
    });
    let after = texts_of(&engine);
    // f/h 模糊让 `kaiha` 的 `ha` 也读 `fa`：覆盖满的 开放（20000）先于 开发（9000），
    // 前缀词 开 覆盖少，排到它们后面
    assert_eq!(after[0], "开放");
    assert!(after.contains(&"开放".to_owned()));
    assert!(after.contains(&"开发".to_owned()));
    // 整句转换走同一套写法：xiangkaiha → 想开发（整句候选进同一个池子）
    engine.set_input("xiangkaiha");
    let query = engine.query().unwrap();
    let sentence = query
        .candidates
        .items
        .iter()
        .find(|c| c.kind == CandidateKind::Sentence)
        .cloned()
        .expect("整句候选");
    assert_eq!(sentence.text, "想开发");
    // 敲对的仍然优先：覆盖满的 开发（9000）在只覆盖 `kai` 的 开（前缀词 20000）前，且不重复
    engine.set_input("kaifa");
    let all = texts_of(&engine);
    assert_eq!(all[0], "开发");
    assert_eq!(all.iter().filter(|t| *t == "开发").count(), 1);
}
