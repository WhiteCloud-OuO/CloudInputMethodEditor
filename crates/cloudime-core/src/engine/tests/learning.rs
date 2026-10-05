//! 学习、撤销、统计、日志与词汇。

use super::*;

#[test]
fn erasing_the_last_commit_and_choosing_another_word_retracts_its_learning() {
    let mut engine = engine().with_learner(Box::new(CountingLearner(HashMap::new())));
    let pick = |engine: &mut Engine, input: &str, text: &str| {
        engine.set_input(input);
        let candidate = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == text && c.kind == CandidateKind::Chinese)
            .unwrap();
        engine.commit(&candidate);
    };
    // 选了 开放，退两格整个删掉，重打 kaifa 选 开发：开放 的那次不算
    pick(&mut engine, "kaifa", "开放");
    assert_eq!(engine.learner().weight("开放"), 1);
    engine.note_backspace();
    engine.note_backspace();
    pick(&mut engine, "kaifa", "开发");
    assert_eq!(engine.learner().weight("开放"), 0);
    assert_eq!(engine.learner().choice_weight("kaifa", "开放"), 0);
    assert_eq!(engine.learner().weight("开发"), 1);
    // 只退了一格不算删掉
    pick(&mut engine, "kaifa", "开放");
    engine.note_backspace();
    pick(&mut engine, "kaifa", "开发");
    assert_eq!(engine.learner().weight("开放"), 1);
    // 整个删掉之后打的是别的拼音：不撤销
    engine.note_backspace();
    engine.note_backspace();
    pick(&mut engine, "xian", "先");
    assert_eq!(engine.learner().weight("开发"), 2);
    // 删得比记着的几次上屏加起来（开放 先 开放 开发，7 个字）还多：在改别处，不撤销
    pick(&mut engine, "kaifa", "开放");
    for _ in 0..8 {
        engine.note_backspace();
    }
    pick(&mut engine, "kaifa", "开发");
    assert_eq!(engine.learner().weight("开放"), 2);
}

#[test]
fn erasing_several_commits_and_retyping_retracts_the_wrong_one() {
    let mut engine = engine().with_learner(Box::new(CountingLearner(HashMap::new())));
    let pick = |engine: &mut Engine, input: &str, text: &str| {
        engine.set_input(input);
        let candidate = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == text && c.kind == CandidateKind::Chinese)
            .unwrap();
        engine.commit(&candidate);
    };
    // 打了 开放先 才发现 开放 错了：三格删光两个词，重打 kaifa 选 开发、再打 xian 选 先
    pick(&mut engine, "kaifa", "开放");
    pick(&mut engine, "xian", "先");
    for _ in 0..3 {
        engine.note_backspace();
    }
    pick(&mut engine, "kaifa", "开发");
    assert_eq!(engine.learner().weight("开放"), 0);
    assert_eq!(engine.learner().choice_weight("kaifa", "开放"), 0);
    // 先 重打后还是 先：不算选错，计数照旧
    pick(&mut engine, "xian", "先");
    assert_eq!(engine.learner().weight("先"), 2);
    // 中间隔着标点与原样上屏的英文也一样：开放，gist先 全删掉重打
    pick(&mut engine, "kaifa", "开放");
    assert_eq!(engine.punctuate(',').as_deref(), Some("，"));
    engine.set_input("gist");
    engine.take_raw();
    pick(&mut engine, "xian", "先");
    for _ in 0..8 {
        engine.note_backspace();
    }
    pick(&mut engine, "kaifa", "开发");
    assert_eq!(engine.learner().weight("开放"), 0);
    assert_eq!(engine.learner().weight("开发"), 2);
}

#[test]
fn splitting_a_buffer_into_a_word_and_a_sentence_forms_the_whole_phrase() {
    let shared = Arc::new(Mutex::new((Vec::new(), sentence::UserNgram::default())));
    let learner = WordLearner {
        shared: Arc::clone(&shared),
        ..WordLearner::default()
    };
    let mut engine = engine().with_learner(Box::new(learner));
    let mut phrase = String::new();
    // 每次都是先自选 想，剩下的 kaifaxian 用整句候选上屏
    for round in 1..=2 {
        engine.set_input("xiangkaifaxian");
        let xiang = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == "想" && c.kind == CandidateKind::Chinese)
            .unwrap();
        engine.commit(&xiang);
        assert_eq!(engine.composition().text(), "kaifaxian");
        let rest = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.kind == CandidateKind::Sentence && c.text.ends_with('先'))
            .unwrap_or_else(|| panic!("round {round}: 整句候选"));
        assert_eq!(rest.kind, CandidateKind::Sentence, "round {round}");
        let head = rest.text.strip_suffix('先').unwrap().to_owned();
        engine.commit(&rest);
        assert!(engine.composition().is_empty());
        phrase = format!("想{}", rest.text);
        // 接缝处（想 → 开X）按自选记双份，整句内部的接续记一份；整段拼音 → 合成词 记一次选择
        let learned = shared.lock().unwrap();
        assert_eq!(
            learned.1.pair(Some("想"), &head),
            round * EXPLICIT_TRANSITION_WEIGHT
        );
        assert_eq!(learned.1.pair(Some(&head), "先"), round);
        drop(learned);
        assert_eq!(
            engine.learner().choice_weight("xiangkaifaxian", &phrase),
            round
        );
        // 第二次：整段合成 想开X先；接缝处不造两字词
        let words = shared.lock().unwrap().0.clone();
        if round == 1 {
            assert!(words.is_empty(), "{words:?}");
        } else {
            assert_eq!(words, [phrase.clone()]);
        }
        engine.note_passthrough('\n');
    }
    // 第三次整段打出来：合成词在候选里（纯按权重排，不一定第一）
    engine.set_input("xiangkaifaxian");
    assert!(texts_of(&engine).contains(&phrase), "{phrase}");
}

#[test]
fn commit_feeds_learner_and_reorders() {
    let mut engine = engine().with_learner(Box::new(CountingLearner(HashMap::new())));
    engine.set_input("kaif");
    let query = engine.query().unwrap();
    let kaifa = query
        .candidates
        .items
        .iter()
        .find(|c| c.text == "开发")
        .unwrap()
        .clone();
    assert_eq!(engine.commit(&kaifa).as_deref(), Some("开发"));
    assert!(engine.composition().is_empty());

    // 同一输入串再打：选过的 开发 压过词频更高的 开放
    engine.set_input("kaif");
    assert_eq!(engine.query().unwrap().candidates.items[0].text, "开发");
    // 换个输入串（`kai'fa`，分隔符不计）也算同一串；`kaifang` 不算
    engine.set_input("kai'f");
    assert_eq!(engine.query().unwrap().candidates.items[0].text, "开发");
    engine.set_input("kaifazhe");
    let items = engine.query().unwrap().candidates.items;
    let kaifa = items.iter().find(|c| c.text == "开发").unwrap().clone();
    // 从长输入里选前缀词：按消耗掉的那段（`kaifa`）记
    engine.commit(&kaifa);
    engine.set_input("kaifa");
    assert_eq!(engine.query().unwrap().candidates.items[0].text, "开发");
}

#[test]
fn candidate_words_are_recorded_as_seen_and_committed() {
    let book = Arc::new(Mutex::new(HashMap::new()));
    let mut engine = engine().with_vocabulary_tracker(Box::new(MemoryVocabulary(book.clone())));
    engine.set_input("kaifa");
    let query = engine.query().unwrap();
    let kaifa = query
        .candidates
        .items
        .iter()
        .find(|c| c.text == "开发")
        .cloned()
        .unwrap();
    engine.note_displayed(query.candidates.items.iter());
    assert!(book.lock().unwrap().is_empty());
    engine.commit(&kaifa);
    {
        let counts = book.lock().unwrap();
        assert_eq!(counts.get("开发"), Some(&(1, 1)));
        // 同一页的其它中文候选算看到过一轮，但没有上屏
        assert!(
            counts
                .values()
                .any(|(seen, committed)| *seen == 1 && *committed == 0)
        );
    }
    // 私密输入中的候选不记
    engine.set_private(true);
    engine.set_input("xian");
    let query = engine.query().unwrap();
    let xian = query.candidates.items[0].clone();
    engine.note_displayed(query.candidates.items.iter());
    engine.commit(&xian);
    assert!(!book.lock().unwrap().contains_key("先"));
}

#[test]
fn input_log_records_commits_with_their_context_and_retractions() {
    let entries = Arc::new(Mutex::new(Vec::new()));
    let mut engine = engine().with_input_logger(Box::new(MemoryLogger(entries.clone())));
    engine.set_input("kaifazhe");
    let query = engine.query().unwrap();
    let kaifa = query
        .candidates
        .items
        .iter()
        .find(|c| c.text == "开发")
        .cloned()
        .unwrap();
    engine.commit(&kaifa);
    {
        let entries = entries.lock().unwrap();
        let InputLogEntry::Commit(commit) = &entries[0] else {
            panic!("expected a commit");
        };
        assert_eq!(commit.id, 1);
        assert_eq!(commit.scope, "kaifazhe");
        assert_eq!(commit.keys, "kaifa");
        assert_eq!(commit.pinyin, "kai'fa'zhe");
        assert_eq!(commit.text, "开发");
        assert_eq!(commit.source, InputSource::Word);
        assert_eq!(commit.index, Some(1));
        assert_eq!(commit.top[0], "开发者");
        assert!(!commit.corrected);
    }
    // 已选的 开发 加上剩下的 zhe 一起回车原样上屏
    assert_eq!(engine.take_raw(), "开发zhe");
    // 整个退格删掉 开发 再重打同一段拼音换选 开放：记一条撤销
    engine.set_input("kaifa");
    engine.query().unwrap();
    engine.commit(&kaifa);
    for _ in 0..2 {
        engine.note_backspace();
    }
    engine.set_input("kaifa");
    let query = engine.query().unwrap();
    let kaifang = query
        .candidates
        .items
        .iter()
        .find(|c| c.text == "开放")
        .cloned()
        .unwrap();
    engine.commit(&kaifang);
    let entries = entries.lock().unwrap();
    let kinds: Vec<&str> = entries
        .iter()
        .map(|e| match e {
            InputLogEntry::Commit(c) => match c.source {
                InputSource::Raw => "raw",
                _ => "commit",
            },
            InputLogEntry::Retract { .. } => "retract",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["commit", "raw", "commit", "retract", "commit"]);
    assert!(matches!(
        &entries[3],
        InputLogEntry::Retract { of: 3, text, chosen } if text == "开发" && chosen == "开放"
    ));
}

#[test]
fn word_ranking_follows_the_previous_committed_word() {
    let dictionary =
        Dictionary::parse("把\tba\t3000000\n吧\tba\t2000000\n做了\tzuo le\t5000\n").unwrap();
    let mut engine = Engine::new(dictionary).with_language_model(Box::new(BaModel));
    let first = |engine: &mut Engine| engine.query().unwrap().candidates.items[0].text.clone();
    // 句首：模型说 把
    engine.set_input("ba");
    assert_eq!(first(&mut engine), "把");
    // 做了 上屏之后再打 ba：吧
    engine.set_input("zuole");
    let done = engine.query().unwrap().candidates.items[0].clone();
    engine.commit(&done);
    engine.set_input("ba");
    assert_eq!(first(&mut engine), "吧");
    // 标点断句后回到句首
    engine.punctuate('。');
    engine.set_input("ba");
    assert_eq!(first(&mut engine), "把");
}

#[test]
fn erasing_a_committed_sentence_retracts_its_transitions() {
    let shared = Arc::new(Mutex::new((Vec::new(), sentence::UserNgram::default())));
    let learner = WordLearner {
        shared: Arc::clone(&shared),
        ..WordLearner::default()
    };
    let mut engine = engine().with_learner(Box::new(learner));
    engine.set_input("xiangkaifa");
    let sentence = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.kind == CandidateKind::Sentence)
        .unwrap();
    assert_eq!(sentence.text, "想开发");
    engine.commit(&sentence);
    assert_eq!(shared.lock().unwrap().1.pair(Some("想"), "开发"), 1);
    // 想开发 三个字全删掉，重打前缀 xiang 选 想：整句路径的两条转移退回，想 按点选记双份
    for _ in 0..3 {
        engine.note_backspace();
    }
    engine.set_input("xiangkaifa");
    let xiang = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.text == "想" && c.kind == CandidateKind::Chinese)
        .unwrap();
    engine.commit(&xiang);
    let ngram = &shared.lock().unwrap().1;
    assert_eq!(ngram.pair(Some("想"), "开发"), 0);
    assert_eq!(ngram.pair(None, "想"), EXPLICIT_TRANSITION_WEIGHT);
}

#[test]
fn commits_feed_the_personal_bigram_and_form_words() {
    let shared = Arc::new(Mutex::new((Vec::new(), sentence::UserNgram::default())));
    let learner = WordLearner {
        shared: Arc::clone(&shared),
        ..WordLearner::default()
    };
    let mut engine = engine().with_learner(Box::new(learner));

    // 整句上屏：按路径上的词记转移，第一个词在句首
    engine.set_input("xiangkaifa");
    let sentence = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.kind == CandidateKind::Sentence)
        .unwrap();
    assert_eq!(sentence.text, "想开发");
    engine.commit(&sentence);
    {
        let ngram = &shared.lock().unwrap().1;
        assert_eq!(ngram.pair(None, "想"), 1);
        assert_eq!(ngram.pair(Some("想"), "开发"), 1);
    }

    // 标点断句；之后连续从同一段拼音里选 开发 + 先：第一次只记转移，第二次自动造词 开发先
    engine.punctuate('，');
    let select = |engine: &mut Engine, text: &str| {
        let candidate = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == text && c.kind == CandidateKind::Chinese)
            .unwrap();
        engine.commit(&candidate);
    };
    for round in 1..=2 {
        engine.set_input("kaifaxian");
        select(&mut engine, "开发");
        assert_eq!(engine.composition().text(), "xian");
        select(&mut engine, "先");
        assert!(engine.composition().is_empty());
        // 用户自己点选的词，转移按 EXPLICIT_TRANSITION_WEIGHT 份记
        let learned = shared.lock().unwrap();
        assert_eq!(
            learned.1.pair(None, "开发"),
            round * EXPLICIT_TRANSITION_WEIGHT
        );
        assert_eq!(
            learned.1.pair(Some("开发"), "先"),
            round * EXPLICIT_TRANSITION_WEIGHT
        );
        assert_eq!(learned.0.len(), usize::from(round == 2), "round {round}");
        engine.note_passthrough('\n');
    }
    assert_eq!(shared.lock().unwrap().0, ["开发先"]);

    // 分两段打的同样两次：咖啡 + 开 记一次不造词，第二次造
    for round in 1..=2 {
        engine.set_input("kafei");
        select(&mut engine, "咖啡");
        engine.set_input("kai");
        select(&mut engine, "开");
        assert_eq!(
            shared.lock().unwrap().1.pair(Some("咖啡"), "开"),
            round * EXPLICIT_TRANSITION_WEIGHT
        );
        assert_eq!(shared.lock().unwrap().0.len(), 1 + usize::from(round == 2));
        engine.punctuate('。');
    }
    assert_eq!(shared.lock().unwrap().0, ["开发先", "咖啡开"]);
}

#[test]
fn auto_word_weight_is_the_max_character_frequency() {
    let engine = engine();
    // SAMPLE 里 开 kai = 20000、没有单字 发：取最大的 20000
    assert_eq!(
        engine.initial_user_weight("开发", &["kai".into(), "fa".into()]),
        20000.0
    );
    // 一个字都查不到时退回 1
    assert_eq!(
        engine.initial_user_weight("甲乙", &["jia".into(), "yi".into()]),
        1.0
    );
}

#[test]
fn remember_sentence_learns_a_short_sentence_after_two_times() {
    let shared = Arc::new(Mutex::new((Vec::new(), sentence::UserNgram::default())));
    let learner = WordLearner {
        shared: Arc::clone(&shared),
        ..WordLearner::default()
    };
    let mut engine = engine().with_learner(Box::new(learner));
    for round in 1..=2 {
        engine.set_input("xiangkaifa");
        let sentence = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.kind == CandidateKind::Sentence)
            .unwrap();
        assert_eq!(sentence.text, "想开发");
        engine.remember_sentence(&sentence);
        // 记第一次不收录，第二次才收进用户词库
        assert_eq!(shared.lock().unwrap().0.len(), usize::from(round == 2));
        engine.commit(&sentence);
    }
    assert_eq!(shared.lock().unwrap().0, ["想开发"]);
}

#[test]
fn ctrl_enter_learns_an_english_word_after_two_times() {
    let shared = Arc::new(Mutex::new((Vec::new(), sentence::UserNgram::default())));
    let learner = WordLearner {
        shared: Arc::clone(&shared),
        ..WordLearner::default()
    };
    let mut engine = engine().with_learner(Box::new(learner));
    for round in 1..=2 {
        engine.set_input("nihao");
        assert_eq!(engine.take_raw_english(), "nihao");
        // 记第一次不收录，第二次才收进自造英文词库
        assert_eq!(shared.lock().unwrap().0.len(), usize::from(round == 2));
    }
    assert_eq!(shared.lock().unwrap().0, ["nihao"]);
}

#[test]
fn extra_dictionaries_join_lookup_and_sentences() {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap());
    engine.set_input("zhangtao");
    assert!(
        engine
            .query()
            .unwrap()
            .candidates
            .items
            .iter()
            .all(|c| c.text != "账套")
    );
    let extra = Dictionary::parse("账套\tzhang tao\t500\n").unwrap();
    engine.set_extra_dictionaries(vec![extra]);
    engine.set_input("zhangtao");
    let query = engine.query().unwrap();
    assert_eq!(query.candidates.items[0].text, "账套");
    // 整句词图也能用上附加词库里的词
    engine.set_input("kaifazhangtao");
    let sentence = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.kind == CandidateKind::Sentence)
        .unwrap();
    assert_eq!(sentence.text, "开发账套");
    engine.set_extra_dictionaries(Vec::new());
    engine.set_input("zhangtao");
    assert!(
        engine
            .query()
            .unwrap()
            .candidates
            .items
            .iter()
            .all(|c| c.text != "账套")
    );
}

/// 建一个带内存日志的引擎，返回引擎与条目列表。
fn logged_engine() -> (Engine, Arc<Mutex<Vec<InputLogEntry>>>) {
    let entries = Arc::new(Mutex::new(Vec::new()));
    let engine = engine().with_input_logger(Box::new(MemoryLogger(entries.clone())));
    (engine, entries)
}

fn commit_first(engine: &mut Engine) -> Option<String> {
    let query = engine.query().unwrap();
    let first = query.candidates.items[0].clone();
    engine.commit(&first)
}

#[test]
fn empty_take_raw_is_not_logged() {
    let (mut engine, entries) = logged_engine();
    assert_eq!(engine.take_raw(), "");
    assert!(entries.lock().unwrap().is_empty());
}

#[test]
fn input_log_records_a_retype_when_keys_change_after_backspace_in_composition() {
    let (mut engine, entries) = logged_engine();
    for c in "kaifs".chars() {
        engine.push(c);
    }
    engine.backspace();
    engine.push('a');
    // 再删一次不换快照：记的是第一次退格前的串
    engine.backspace();
    engine.push('a');
    let query = engine.query().unwrap();
    let kaifa = query
        .candidates
        .items
        .iter()
        .find(|c| c.text == "开发")
        .cloned()
        .unwrap();
    assert_eq!(engine.commit(&kaifa).as_deref(), Some("开发"));
    let entries = entries.lock().unwrap();
    let InputLogEntry::Commit(commit) = &entries[0] else {
        panic!("expected a commit");
    };
    assert_eq!(commit.keys, "kaifa");
    assert_eq!(commit.pages, 0);
    assert!(!commit.rescored);
    assert!(matches!(
        &entries[1],
        InputLogEntry::Retype { before, after, of: 1 } if before == "kaifs" && after == "kaifa"
    ));
    assert_eq!(entries.len(), 2);
}

#[test]
fn backspace_that_restores_the_same_keys_is_not_a_retype() {
    let (mut engine, entries) = logged_engine();
    for c in "kaifa".chars() {
        engine.push(c);
    }
    engine.backspace();
    engine.push('a');
    commit_first(&mut engine);
    assert_eq!(entries.lock().unwrap().len(), 1);
}

#[test]
fn input_log_records_a_retype_across_commits_when_similar_keys_are_retyped() {
    let (mut engine, entries) = logged_engine();
    let pick = |engine: &mut Engine, text: &str| {
        let candidate = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == text && c.kind == CandidateKind::Chinese)
            .unwrap();
        engine.commit(&candidate);
    };
    engine.set_input("kaifa");
    pick(&mut engine, "开发");
    for _ in 0..2 {
        engine.note_backspace();
    }
    engine.set_input("kaifan");
    pick(&mut engine, "开饭");
    let entries = entries.lock().unwrap();
    assert!(matches!(
        &entries[1],
        InputLogEntry::Retype { before, after, of: 1 } if before == "kaifa" && after == "kaifan"
    ));
    assert!(matches!(&entries[2], InputLogEntry::Commit(_)));
}

#[test]
fn input_log_flushes_passthrough_before_the_next_commit_and_marks_breaks() {
    let (mut engine, entries) = logged_engine();
    engine.set_application(Some("notepad.exe".into()));
    // 还没上屏过：失焦不记 break
    engine.break_chain();
    engine.note_passthrough(',');
    engine.note_passthrough(' ');
    engine.set_input("kaifa");
    commit_first(&mut engine);
    engine.note_passthrough('.');
    engine.break_chain();
    engine.break_chain();
    let entries = entries.lock().unwrap();
    assert!(matches!(&entries[0], InputLogEntry::Passthrough { text } if text == ", "));
    let InputLogEntry::Commit(commit) = &entries[1] else {
        panic!("expected a commit");
    };
    assert_eq!(commit.app.as_deref(), Some("notepad.exe"));
    assert!(matches!(&entries[2], InputLogEntry::Passthrough { text } if text == "."));
    assert!(matches!(
        &entries[3],
        InputLogEntry::Break { app: Some(app) } if app == "notepad.exe"
    ));
    assert_eq!(entries.len(), 4);
}

#[test]
fn input_log_records_the_session_and_page_turns() {
    let (mut engine, entries) = logged_engine();
    engine.log_session("0.0.1", "cli");
    engine.set_input("kaifa");
    engine.note_page_turn();
    engine.note_page_turn();
    commit_first(&mut engine);
    let entries = entries.lock().unwrap();
    assert!(matches!(
        &entries[0],
        InputLogEntry::Session { v: INPUT_LOG_VERSION, version, platform, model: false }
            if version == "0.0.1" && platform == "cli"
    ));
    let InputLogEntry::Commit(commit) = &entries[1] else {
        panic!("expected a commit");
    };
    assert_eq!(commit.pages, 2);
}
