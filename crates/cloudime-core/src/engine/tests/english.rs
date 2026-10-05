//! 英文模式与中英混输。

use super::*;

#[test]
fn english_word_ranks_first_when_input_is_unlikely_pinyin() {
    // 词频是 Zipf×1000，与产品 english.tsv 同一尺度
    let words = WordList::parse(
        "hello\thello\t4720\nchina\tchina\t5100\nGitHub\tgithub\t3180\nkey\tkey\t5120\n",
    )
    .unwrap();
    let mut engine = engine().with_english(words);

    engine.set_input("hello"); // he l… l… o：中间有声母缩写
    let all: Vec<String> = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(all[0], "hello");

    engine.set_input("github"); // gi 不是音节 → 切不动
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "github");
    assert_eq!(query.candidates.items[0].kind, CandidateKind::English);
    let word = query.candidates.items[0].clone();
    assert_eq!(engine.commit(&word).as_deref(), Some("github"));
    assert!(engine.composition().is_empty());

    // china 是干净的 chi na，中文候选（词库里没有就只有英文）排前；这里词库没有 chi na，英文仍在第一位
    engine.set_input("china");
    assert_eq!(engine.query().unwrap().candidates.items[0].text, "china");
}

#[test]
fn usage_meter_counts_hanzi_words_and_english_words_per_commit() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let mut engine = engine().with_usage_meter(Box::new(MemoryMeter(recorded.clone())));
    engine.set_input("kaifa");
    let kaifa = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.text == "开发")
        .unwrap();
    assert_eq!(kaifa.text, "开发");
    engine.commit(&kaifa);
    // 拼音回车不算英文词，像英文的字母串才算
    engine.set_input("hao");
    engine.take_raw();
    engine.set_input("gist");
    engine.take_raw();
    let recorded = recorded.lock().unwrap();
    assert_eq!(
        recorded[0],
        Usage {
            hanzi: 2,
            words: 1,
            english_words: 0,
            commits: 1
        }
    );
    assert_eq!(recorded[1].english_words, 0);
    assert_eq!(recorded[1].commits, 1);
    assert_eq!(recorded[2].english_words, 1);
    assert_eq!(recorded.len(), 3);
}

#[test]
fn english_word_yields_to_a_chinese_word_whatever_the_user_chooses() {
    let dictionary = Dictionary::parse("可以\tke yi\t9000\n客运\tke yun\t100\n").unwrap();
    let mut engine = Engine::new(dictionary)
        .with_english(WordList::parse("key\tkey\t5120\n").unwrap())
        .with_learner(Box::new(CountingLearner(HashMap::new())));
    let first_two = |engine: &Engine| {
        let all = texts_of(engine);
        (all[0].clone(), all[1].clone())
    };
    let pick = |engine: &mut Engine, text: &str| {
        engine.set_input("key");
        let candidate = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == text)
            .unwrap();
        engine.commit(&candidate);
    };
    // 中文候选固定在前：ke'y 再不像话，中文词也在前、英文第二
    engine.set_input("key");
    assert_eq!(first_two(&engine), ("可以".into(), "key".into()));
    // 选过一次中文词同样在前
    pick(&mut engine, "可以");
    engine.set_input("key");
    assert_eq!(first_two(&engine), ("可以".into(), "key".into()));
    // 反复选英文词也不会把它顶到第一（中文优先不再可关）
    pick(&mut engine, "key");
    pick(&mut engine, "key");
    engine.set_input("key");
    assert_eq!(first_two(&engine), ("可以".into(), "key".into()));
}

#[test]
fn short_all_caps_acronym_yields_to_chinese() {
    // `mp` 整段只有两个字母、英文写法又是全大写缩写（MP）：几乎总是在打 门票，让中文先，
    // 英文词仍在候选里只是退到后面。超过两个字母的正文英文（hello / cargo）不受这条影响——
    // 曾经试过按词频一刀切，冻结日志回放实测英文首选从 82.5% 掉到 50.9%，cargo / rust 全被整句挤掉。
    let dictionary = Dictionary::parse("门票\tmen piao\t5000\n买票\tmai piao\t3000\n").unwrap();
    let words = WordList::parse("MP\tmp\t4290\nhello\thello\t4720\ncargo\tcargo\t5000\n").unwrap();
    let mut engine = Engine::new(dictionary).with_english(words);

    engine.set_input("mp");
    let all = texts_of(&engine);
    assert_eq!(all[0], "门票");
    assert!(
        all.iter().any(|t| t == "mp"),
        "英文词仍在候选里，只是让到后面"
    );
    assert!(all.iter().position(|t| t == "mp").unwrap() > 0);

    // 三个字母以上的正文英文：拼音不像话时照旧排第一
    engine.set_input("hello");
    assert_eq!(texts_of(&engine)[0], "hello");
    engine.set_input("cargo");
    assert_eq!(texts_of(&engine)[0], "cargo");
}

#[test]
fn learned_english_word_competes_by_weight() {
    // 英文候选按词频 ×(1+用户次数) 与中文候选一起排，不再固定让中文先
    let dictionary = Dictionary::parse("哦\to\t5000\n门票\tmen piao\t5000\n").unwrap();
    let words = WordList::parse("OK\tok\t5140\nMP\tmp\t4290\n").unwrap();
    let mut engine = Engine::new(dictionary)
        .with_english(words)
        .with_learner(Box::new(CountingLearner(HashMap::new())));
    let pick = |engine: &mut Engine, input: &str, text: &str| {
        engine.set_input(input);
        let candidate = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == text)
            .unwrap();
        engine.commit(&candidate);
    };

    // 同词频附近：OK 5140 压过 哦 5000；门票 5000 压过 MP 4290，但英文词都在候选里
    engine.set_input("ok");
    let all = texts_of(&engine);
    assert_eq!(all[0], "ok");
    assert!(all.contains(&"哦".to_owned()));
    engine.set_input("mp");
    let all = texts_of(&engine);
    assert_eq!(all[0], "门票");
    assert!(all.contains(&"mp".to_owned()));

    // 选过 OK 后权重乘 2，仍排第一；中文候选还在
    pick(&mut engine, "ok", "ok");
    engine.set_input("ok");
    let all = texts_of(&engine);
    assert_eq!(all[0], "ok");
    assert!(all.contains(&"哦".to_owned()));

    // 选过 MP 两次：4290×3 盖过 门票 5000，按同样的乘法涨上来
    pick(&mut engine, "mp", "mp");
    pick(&mut engine, "mp", "mp");
    engine.set_input("mp");
    let all = texts_of(&engine);
    assert_eq!(all[0], "mp");
    assert!(all.contains(&"门票".to_owned()));
}

#[test]
fn hyphen_turns_the_buffer_into_a_raw_english_segment() {
    let mut engine = engine();
    engine.set_input("no");
    assert!(!engine.raw_mode());
    engine.push('-');
    assert!(engine.raw_mode());
    engine.push('w');
    engine.push('a');
    engine.push('y');
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items.len(), 1);
    assert_eq!(query.candidates.items[0].text, "no-way");
    assert_eq!(query.candidates.items[0].kind, CandidateKind::English);
    assert_eq!(query.tail, "no-way");
    let raw = query.candidates.items[0].clone();
    assert_eq!(engine.commit(&raw).as_deref(), Some("no-way"));
    assert!(engine.composition().is_empty());
    // 表达式模式优先
    engine.set_input("v1-2");
    assert!(engine.expression_mode() && !engine.raw_mode());
}

#[test]
fn english_completions_appear_when_pinyin_is_unlikely() {
    // Zipf×1000：company 5.6 / compare 4.45 / compass 3.74 都过补全门槛 3.5
    let words = WordList::parse(
            "company\tcompany\t5600\ncompare\tcompare\t4450\ncompass\tcompass\t3740\ncomma\tcomma\t2900\nxian\txian\t4500\nxiangkai\txiangkai\t10\n",
        )
        .unwrap();
    let mut engine = engine().with_english(words);
    // compa 切成 co'm'pa，不像拼音：补全排最前，最多三条、按词频
    engine.set_input("compa");
    let all = texts_of(&engine);
    assert_eq!(&all[..3], ["company", "compare", "compass"]);
    // xian 是干净的拼音：只有精确词 xian，不补全
    engine.set_input("xian");
    let all = texts_of(&engine);
    assert_eq!(all.iter().filter(|t| t.starts_with("xian")).count(), 1);
    // 太短的前缀不补全
    engine.set_input("co");
    assert!(!texts_of(&engine).contains(&"company".to_owned()));
    // 第一个字母就切不动的（i 不是任何音节的开头）也要出补全
    engine.set_input("impo");
    assert!(engine.query().is_err());
    let words = WordList::parse("important\timportant\t4800\nimport\timport\t4600\n").unwrap();
    let mut fresh = Engine::new(Dictionary::parse(SAMPLE).unwrap()).with_english(words);
    fresh.set_input("impo");
    assert_eq!(texts_of(&fresh), ["important", "import"]);
}

#[test]
fn english_mode_suggests_from_the_word_list_and_keeps_the_typed_text() {
    let words = WordList::parse(
        "company\tcompany\t900\ncompare\tcompare\t500\nhello\thello\t1000\nhelp\thelp\t700\n",
    )
    .unwrap();
    let mut engine = engine()
        .with_english(words)
        .with_learner(Box::new(CountingLearner(HashMap::new())));
    engine.set_english_mode(true);
    assert!(engine.english_mode());
    // 大小写跟着敲的走，marked text 就是敲的字母，没有拼音切分
    engine.set_input("Comp");
    let query = engine.query().unwrap();
    assert_eq!(texts_of(&engine), ["Company", "Compare"]);
    assert_eq!(query.marked_text(), "Comp");
    assert!(query.segmentations.is_empty());
    // 数字进缓冲区也只是没候选
    engine.set_input("foo1");
    assert!(texts_of(&engine).is_empty());
    assert_eq!(engine.query().unwrap().marked_text(), "foo1");
    // 选中的词记次数，下次同样的前缀它靠前
    engine.set_input("comp");
    let compare = engine.query().unwrap().candidates.items[1].clone();
    assert_eq!(engine.commit(&compare).as_deref(), Some("compare"));
    assert!(engine.composition().is_empty());
    engine.set_input("comp");
    assert_eq!(texts_of(&engine), ["compare", "company"]);
    // 拼错一个字母也有候选；回车原样上屏敲的字母
    engine.set_input("helo");
    assert_eq!(texts_of(&engine), ["hello", "help"]);
    assert_eq!(engine.take_raw(), "helo");
    // 离开英文模式后同一串又按拼音算
    engine.set_english_mode(false);
    engine.set_input("comp");
    assert!(!engine.query().unwrap().segmentations.is_empty());
}

#[test]
fn raw_committed_english_words_are_learned_and_come_back_as_candidates() {
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct EnglishLearner {
        words: Vec<String>,
        list: Option<WordList>,
    }
    impl Learner for EnglishLearner {
        fn record(&mut self, _candidate: &Candidate) {}
        fn weight(&self, _text: &str) -> u32 {
            0
        }
        fn learn_english(&mut self, word: &str) {
            self.words.push(word.to_owned());
            let tsv: String = self
                .words
                .iter()
                .map(|w| format!("{w}\t{w}\t1\n"))
                .collect();
            self.list = WordList::parse(&tsv).ok();
        }
        fn user_english(&self) -> Option<&WordList> {
            self.list.as_ref()
        }
    }
    let _ = Rc::new(RefCell::new(()));
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap())
        .with_learner(Box::new(EnglishLearner::default()));
    // 随包词表里没有 gist：第一次只有拼音候选，回车原样上屏
    engine.set_input("gist");
    let first = engine.query().unwrap();
    assert!(
        first
            .candidates
            .items
            .iter()
            .all(|c| c.kind != CandidateKind::English)
    );
    assert_eq!(engine.take_raw(), "gist");
    // 第二次 gist 就是英文候选，而且排第一（拼音不像话）
    engine.set_input("gist");
    let second = engine.query().unwrap();
    assert_eq!(second.candidates.items[0].text, "gist");
    assert_eq!(second.candidates.items[0].kind, CandidateKind::English);
    // 能切成完整拼音的串回车不算英文词
    engine.set_input("hao");
    assert_eq!(engine.take_raw(), "hao");
    engine.set_input("hao");
    let hao = engine.query().unwrap();
    assert!(
        hao.candidates
            .items
            .iter()
            .all(|c| c.kind != CandidateKind::English)
    );
    // 英文模式下直通的词也学
    engine.set_english_mode(true);
    engine.set_input("wo");
    assert_eq!(engine.take_raw(), "wo");
    engine.set_english_mode(false);
    engine.set_input("wo");
    let wo = engine.query().unwrap();
    assert!(
        wo.candidates
            .items
            .iter()
            .any(|c| c.kind == CandidateKind::English && c.text == "wo")
    );
}

/// 句末的英文词：`kaifarust` → 开发rust 排第一，拼音行是 `kai'fa'rust`，上屏吃掉整段并把 rust 记进个人英文词表。
#[test]
fn english_word_at_the_end_of_pinyin_joins_the_sentence() {
    let words =
        WordList::parse("rust\trust\t3740\nID\tid\t4610\nto\tto\t7430\nfan\tfan\t4500\n").unwrap();
    let mut engine = engine().with_english(words);

    engine.set_input("kaifarust");
    let query = engine.query().unwrap();
    let first = query
        .candidates
        .items
        .iter()
        .find(|c| c.text == "开发rust")
        .cloned()
        .expect("尾段英文并入整句");
    assert_eq!(first.kind, CandidateKind::Sentence);
    assert_eq!(first.syllables, ["kai", "fa", "rust"]);
    assert_eq!(query.tail, "rust");
    assert_eq!(query.marked_text(), "kai'fa'rust");
    // 头段的词照常出（逐词上屏也行）
    assert!(query.candidates.items.iter().any(|c| c.text == "开发"));
    assert_eq!(engine.commit(&first).as_deref(), Some("开发rust"));
    assert!(engine.composition().is_empty());

    // 两个字母的尾段只认缩写词：ID 行，to 不行（`kaifato` 按拼音读）
    engine.set_input("kaifaid");
    assert!(
        engine
            .query()
            .unwrap()
            .candidates
            .items
            .iter()
            .any(|c| c.text == "开发ID")
    );
    engine.set_input("kaifato");
    let all: Vec<String> = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert!(!all.iter().any(|t| t.ends_with("to")), "{all:?}");

    // 尾段本身是合法拼音又不到四个字母（`fan`）：就是拼音，开饭 照旧
    engine.set_input("kaifan");
    assert!(
        engine
            .query()
            .unwrap()
            .candidates
            .items
            .iter()
            .any(|c| c.text == "开饭")
    );
}

/// 尾段也是合法拼音时两种读法比分：`wodedatabase` 英文赢，混合整句与拼音整句都进候选池；
/// `womenqubeijing` 拼音赢（北京 是常用词），输了的英文读法不出。
#[test]
fn pinyin_like_english_tail_competes_with_the_plain_reading() {
    const DICT: &str = "我\two\t900000\n的\tde\t800000\n我的\two de\t500000\n大\tda\t50000\n塔\tta\t3000\n巴\tba\t3000\n瑟\tse\t500\n\
        我们\two men\t400000\n去\tqu\t300000\n北京\tbei jing\t200000\n北\tbei\t20000\n京\tjing\t10000\n";
    let words = WordList::parse("database\tdatabase\t4310\nBeijing\tbeijing\t4500\n").unwrap();
    let mut engine = Engine::new(Dictionary::parse(DICT).unwrap()).with_english(words);

    engine.set_input("wodedatabase");
    let query = engine.query().unwrap();
    assert!(
        query
            .candidates
            .items
            .iter()
            .any(|c| c.text == "我的database" && c.kind == CandidateKind::Sentence)
    );
    // 拼音读法也在池子里
    assert!(
        query
            .candidates
            .items
            .iter()
            .any(|c| c.text.starts_with("我的大塔巴") && c.kind == CandidateKind::Sentence)
    );
    assert_eq!(query.marked_text(), "wo'de'database");

    engine.set_input("womenqubeijing");
    let query = engine.query().unwrap();
    assert!(
        query
            .candidates
            .items
            .iter()
            .any(|c| c.text == "我们去北京" && c.kind == CandidateKind::Sentence)
    );
    assert!(
        !query
            .candidates
            .items
            .iter()
            .any(|c| c.text.ends_with("Beijing"))
    );
    assert_eq!(query.marked_text(), "wo'men'qu'bei'jing");

    // 上屏英文赢了的整句：吃掉整段，英文词是最后一个词
    engine.set_input("wodedatabase");
    let mixed = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.text == "我的database")
        .unwrap();
    assert_eq!(mixed.syllables, ["wo", "de", "database"]);
    assert_eq!(engine.commit(&mixed).as_deref(), Some("我的database"));
    assert!(engine.composition().is_empty());
}
