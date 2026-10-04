use super::*;

fn candidate(text: &str) -> Candidate {
    Candidate {
        text: text.to_owned(),
        kind: cloudime_core::CandidateKind::Chinese,
        syllables: Vec::new(),
        reading: None,
    }
}

#[test]
fn records_and_round_trips_through_tsv() {
    let mut learner = FrequencyLearner::default();
    learner.record(&candidate("开发"));
    learner.record(&candidate("开发"));
    learner.record(&candidate("中文"));
    assert_eq!(learner.weight("开发"), 2);
    assert!(learner.is_dirty());

    let dir = std::env::temp_dir().join(format!("cloudime-learning-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");
    learner.save_to(&path).unwrap();
    assert!(!learner.is_dirty());

    let mut restored = FrequencyLearner::from_path(&path).unwrap();
    assert_eq!(restored.weight("开发"), 2);
    assert_eq!(restored.weight("中文"), 1);
    assert_eq!(restored.weight("没有"), 0);

    // flush 写回加载时的路径
    restored.record(&candidate("中文"));
    restored.flush();
    assert_eq!(
        FrequencyLearner::from_path(&path).unwrap().weight("中文"),
        2
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn choices_are_keyed_by_input_and_round_trip() {
    let dir = std::env::temp_dir().join("cloudime-user-choices-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");
    let _ = std::fs::remove_file(dir.join(USER_CHOICES_FILE));

    let mut learner = FrequencyLearner::from_path(&path).unwrap();
    learner.record_choice("ba", "吧");
    learner.record_choice("ba", "吧");
    learner.record_choice("bazhege", "把");
    learner.record_raw("nihooma");
    assert_eq!(learner.raw_count("nihooma"), 1);
    assert_eq!(learner.choice_weight("ba", "吧"), 2);
    assert_eq!(learner.choice_weight("ba", "把"), 0);
    assert_eq!(learner.choice_weight("bazhege", "把"), 1);
    learner.flush();

    let reloaded = FrequencyLearner::from_path(&path).unwrap();
    assert_eq!(reloaded.choice_count(), 3);
    assert_eq!(reloaded.choice_weight("ba", "吧"), 2);
    assert_eq!(reloaded.raw_count("nihooma"), 1);
    assert_eq!(reloaded.choice_weight("bazhege", "把"), 1);
}

#[test]
fn unrecord_reverses_each_kind_of_record() {
    let mut learner = FrequencyLearner::default();
    let candidate = Candidate {
        text: "开放".into(),
        kind: cloudime_core::CandidateKind::Chinese,
        syllables: vec!["kai".into(), "fang".into()],
        reading: None,
    };
    learner.record(&candidate);
    learner.record_choice("kaifa", "开放");
    learner.record_transition(Context::START, "开放", 2);
    learner.unrecord("开放");
    learner.unrecord_choice("kaifa", "开放");
    learner.unrecord_transition(Context::START, "开放", 2);
    assert_eq!(learner.weight("开放"), 0);
    assert_eq!(learner.choice_weight("kaifa", "开放"), 0);
    assert_eq!(learner.choice_count(), 0);
    assert!(learner.user_ngram().is_none());
    // 没记过的撤销不会变成负数
    learner.unrecord("没有");
    learner.unrecord_choice("x", "没有");
    assert_eq!(learner.weight("没有"), 0);
}

#[test]
fn choices_decay_when_over_the_cap() {
    let mut learner = FrequencyLearner::default();
    learner.record_choice("a", "甲");
    learner.record_choice("a", "甲");
    learner.record_choice("a", "甲");
    learner.record_choice("a", "乙");
    learner.decay_choices();
    assert_eq!(learner.choice_weight("a", "甲"), 1);
    assert_eq!(learner.choice_weight("a", "乙"), 0);
    assert_eq!(learner.choice_count(), 1);
}

#[test]
fn transitions_round_trip_through_ngram_tsv() {
    let dir = std::env::temp_dir().join("cloudime-user-ngram-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");
    let _ = std::fs::remove_file(dir.join(USER_NGRAM_FILE));

    let mut learner = FrequencyLearner::from_path(&path).unwrap();
    assert!(learner.user_ngram().is_none());
    learner.record_transition(Context::START, "我", 1);
    learner.record_transition(Context::after("我"), "想", 1);
    learner.record_transition(Context::after("我"), "想", 1);
    learner.record_transition(Context::after_two("我", "想"), "去", 1);
    assert_eq!(learner.user_ngram().unwrap().pair(Some("我"), "想"), 2);
    learner.flush();

    let reloaded = FrequencyLearner::from_path(&path).unwrap();
    // 二元 <s>我 / 我想 / 想去，三元 <s>我想 / 我想去
    assert_eq!(reloaded.ngram_transition_count(), 5);
    assert_eq!(reloaded.user_ngram().unwrap().pair(None, "我"), 1);
    assert_eq!(reloaded.user_ngram().unwrap().count("想"), 2);
    assert_eq!(
        reloaded
            .user_ngram()
            .unwrap()
            .triple(Some("我"), "想", "去"),
        1
    );
}

#[test]
fn typos_round_trip_and_unrecord() {
    let dir = std::env::temp_dir().join("cloudime-user-typos-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");
    let _ = std::fs::remove_file(dir.join(USER_TYPOS_FILE));

    let mut learner = FrequencyLearner::from_path(&path).unwrap();
    learner.record_typo("gan", "guan");
    learner.record_typo("gan", "guan");
    learner.record_typo("shou", "shuo");
    learner.unrecord_typo("shou", "shuo");
    learner.unrecord_typo("mei", "you");
    assert_eq!(learner.typo_count("gan", "guan"), 2);
    assert_eq!(learner.typo_count("shou", "shuo"), 0);
    assert_eq!(learner.typo_count_total(), 1);
    learner.flush();

    let reloaded = FrequencyLearner::from_path(&path).unwrap();
    assert_eq!(reloaded.typo_count("gan", "guan"), 2);
    assert_eq!(reloaded.typo_count_total(), 1);
}

#[test]
fn broken_lines_are_skipped_instead_of_failing_the_load() {
    let dir = std::env::temp_dir().join("cloudime-broken-lines-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");
    std::fs::write(&path, "开发\t3\n没有次数\n中文\tabc\n中文\t1\n").unwrap();
    std::fs::write(dir.join(USER_NGRAM_FILE), "<s>\t我\t3\n坏行\n我\t想\t2\n").unwrap();
    std::fs::write(dir.join(USER_CHOICES_FILE), "ba\t吧\t2\nba\n").unwrap();
    std::fs::write(dir.join(USER_TYPOS_FILE), "gan\tguan\tx\ngan\tguan\t1\n").unwrap();
    std::fs::write(dir.join(USER_WORDS_FILE), "账套\tzhang tao\t100\n只有词\n").unwrap();
    // 编码坏掉的字节也不能让整个文件读不了
    std::fs::write(dir.join(USER_ENGLISH_FILE), b"gist\t2\n\xff\xfe\t1\n").unwrap();

    let learner = FrequencyLearner::from_path(&path).unwrap();
    assert_eq!(learner.weight("开发"), 3);
    assert_eq!(learner.weight("中文"), 1);
    assert_eq!(learner.user_ngram().unwrap().pair(Some("我"), "想"), 2);
    assert_eq!(learner.choice_weight("ba", "吧"), 2);
    assert_eq!(learner.typo_count("gan", "guan"), 1);
    assert_eq!(learner.word_count(), 1);
    assert_eq!(learner.english_count(), 2);
    assert!(learner.path.is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn saving_is_atomic_and_leaves_no_temporary_files() {
    let dir = std::env::temp_dir().join("cloudime-atomic-save-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");
    let mut learner = FrequencyLearner::from_path(&path).unwrap();
    learner.record(&candidate("开发"));
    learner.record_choice("kaifa", "开发");
    learner.record_transition(Context::START, "开发", 1);
    learner.record_typo("gan", "guan");
    learner.learn_word("账套", &["zhang".into(), "tao".into()], 1.0);
    learner.learn_english("gist");
    assert!(learner.has_unsaved());
    learner.flush();
    assert!(!learner.has_unsaved());
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 6, "{names:?}");
    assert!(
        names.iter().any(|name| name == "UserWordBank.db"),
        "自造词库也落盘了：{names:?}"
    );
    assert!(names.iter().all(|name| !name.contains(".tmp")), "{names:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_file_is_empty_table() {
    let learner = FrequencyLearner::from_path("/nonexistent/cloudime-user.tsv").unwrap();
    assert!(learner.is_empty());
}

#[test]
fn user_word_bank_can_live_outside_the_frequency_directory() {
    let dir = std::env::temp_dir().join("cloudime-user-bank-path-test");
    let _ = std::fs::remove_dir_all(&dir);
    let data = dir.join("data");
    let bank_dir = dir.join("install").join("WordBank");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&bank_dir).unwrap();
    let frequency = data.join("user.tsv");
    let bank = bank_dir.join("UserWordBank.db");

    let mut learner = FrequencyLearner::from_path_with_user_word_bank(&frequency, &bank).unwrap();
    learner.learn_word("账套", &["zhang".into(), "tao".into()], 1.0);
    learner.flush();

    // 自造词库落在指定路径，词频目录里不再留 UserWordBank.db
    assert!(bank.is_file(), "自造词库应落在指定路径");
    assert!(!data.join("UserWordBank.db").exists());

    let reloaded = FrequencyLearner::from_path_with_user_word_bank(&frequency, &bank).unwrap();
    assert_eq!(reloaded.bank_count(), 1);
    assert!(reloaded.is_user_word("账套"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn forget_removes_the_user_word_and_every_trace_of_learning() {
    let mut learner = FrequencyLearner::default();
    learner.learn_word("账套", &["zhang".into(), "tao".into()], 1.0);
    learner.record(&candidate("账套"));
    learner.record_choice("zhangtao", "账套");
    learner.record_choice("zt", "账套");
    learner.record_choice("zt", "周天");
    learner.record_transition(Context::START, "账套", 1);
    learner.record_transition(Context::after("账套"), "建好", 1);
    let forgotten = learner.forget("账套");
    assert!(forgotten.user_word && forgotten.learning);
    assert!(learner.user_words().is_none());
    assert_eq!(learner.weight("账套"), 0);
    assert_eq!(learner.choice_weight("zt", "账套"), 0);
    assert_eq!(learner.choice_weight("zt", "周天"), 1);
    assert!(learner.user_ngram().is_none());
    // 词库词、没学过：什么都没清
    assert!(learner.forget("开发").is_nothing());
    // 只学过、不是用户词
    learner.record(&candidate("开发"));
    let forgotten = learner.forget("开发");
    assert!(!forgotten.user_word && forgotten.learning);
    // 个人英文词
    learner.learn_english("gist");
    assert!(learner.forget_english("Gist"));
    assert!(!learner.forget_english("gist"));
    assert!(learner.user_english().is_none());
}

#[test]
fn user_words_round_trip_through_tsv_and_form_a_dictionary() {
    let dir = std::env::temp_dir().join("cloudime-user-words-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");

    let mut learner = FrequencyLearner::from_path(&path).unwrap();
    assert!(learner.user_words().is_none());
    learner.learn_word("账套", &["zhang".to_owned(), "tao".to_owned()], 1.0);
    learner.learn_word("账套", &["zhang".to_owned(), "tao".to_owned()], 1.0);
    assert_eq!(learner.word_count(), 1);
    let hits = learner
        .user_words()
        .unwrap()
        .lookup(&["zhang", "tao"], false);
    assert_eq!(hits[0].text, "账套");
    learner.flush();

    let reloaded = FrequencyLearner::from_path(&path).unwrap();
    assert_eq!(reloaded.word_count(), 1);
    let hits = reloaded.user_words().unwrap().lookup(&["zhang"], true);
    assert_eq!(hits[0].text, "账套");
}

#[test]
fn english_words_round_trip_through_tsv_and_form_a_word_list() {
    let dir = std::env::temp_dir().join("cloudime-user-english-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");
    let _ = std::fs::remove_file(FrequencyLearner::english_path(&path));
    let mut learner = FrequencyLearner::from_path(&path).unwrap();
    assert!(learner.user_english().is_none());
    learner.learn_english("gist");
    learner.learn_english("Gist");
    learner.learn_english("python");
    assert_eq!(learner.english_count(), 2);
    // 第一次敲的写法留下，次数合并
    assert_eq!(learner.user_english().unwrap().get("gist"), Some("gist"));
    assert_eq!(learner.user_english().unwrap().complete("g", 3), ["gist"]);
    learner.flush();
    let reloaded = FrequencyLearner::from_path(&path).unwrap();
    assert_eq!(reloaded.english_count(), 2);
    assert_eq!(
        reloaded.user_english().unwrap().get("python"),
        Some("python")
    );
    let saved = std::fs::read_to_string(FrequencyLearner::english_path(&path)).unwrap();
    assert!(saved.contains("gist\t2"));
}

#[test]
fn auto_words_grow_weight_and_other_words_grow_slower() {
    let mut learner = FrequencyLearner::default();
    let frequency = |learner: &FrequencyLearner| {
        learner
            .user_words()
            .unwrap()
            .lookup(&["qing", "jian"], false)
            .first()
            .unwrap()
            .frequency
    };
    // 自造词：初始权重当用户词库的词频，每次重选 ×1.2
    learner.learn_word("青简", &["qing".to_owned(), "jian".to_owned()], 3000.0);
    assert_eq!(learner.word_count(), 1);
    assert!(learner.is_user_word("青简"));
    assert!(!learner.is_user_word("创造"));
    assert_eq!(learner.rank_weight("青简"), 1.0, "权重写在词频里，不再叠加");
    assert_eq!(frequency(&learner), 3000);
    learner.record(&candidate("青简"));
    assert_eq!(frequency(&learner), 3600, "3000 × 1.2");
    learner.record(&candidate("青简"));
    assert_eq!(frequency(&learner), 4320, "再 ×1.2");

    // 撤销退回一档，退到初始就不再退
    learner.unrecord("青简");
    assert_eq!(frequency(&learner), 3600, "4320 ÷ 1.2");
    learner.unrecord("青简");
    assert_eq!(frequency(&learner), 3000, "3600 ÷ 1.2");
    learner.unrecord("青简");
    assert_eq!(frequency(&learner), 3000, "已经回到初始，不再退");

    // 词库里已有的词：每选一次 ×1.15，撤销靠 counts 回落
    assert!((learner.rank_weight("开发") - 1.0).abs() < 1e-12);
    learner.record(&candidate("开发"));
    assert!((learner.rank_weight("开发") - DICT_GROWTH).abs() < 1e-12);
    learner.record(&candidate("开发"));
    assert!((learner.rank_weight("开发") - DICT_GROWTH * DICT_GROWTH).abs() < 1e-12);
    learner.unrecord("开发");
    assert!((learner.rank_weight("开发") - DICT_GROWTH).abs() < 1e-12);

    // 删掉自造词，权重也一起没了
    assert!(learner.forget("青简").user_word);
    assert_eq!(learner.rank_weight("青简"), 1.0);
    assert_eq!(learner.word_count(), 0);
}

#[test]
fn english_bank_words_join_the_personal_english_list() {
    let mut learner = FrequencyLearner::default();
    learner.learn_english_word("nihao", 1000.0);
    assert!(learner.is_user_word("nihao"));
    assert_eq!(learner.user_english().unwrap().get("nihao"), Some("nihao"));
    assert_eq!(learner.bank_count(), 1);
    // 中文自造词不混进英文表
    learner.learn_word("青简", &["qing".to_owned(), "jian".to_owned()], 3000.0);
    assert!(learner.user_english().unwrap().get("qingjian").is_none());
    let hits = learner
        .user_words()
        .unwrap()
        .lookup(&["qing", "jian"], false);
    assert_eq!(hits[0].text, "青简");
    assert_eq!(learner.bank_count(), 2);
}

#[test]
fn legacy_bank_without_language_loads_as_chinese() {
    let dir = std::env::temp_dir().join("cloudime-bank-legacy-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("user.tsv");
    let connection = rusqlite::Connection::open(FrequencyLearner::bank_path(&path)).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE words (text TEXT PRIMARY KEY, pinyin TEXT NOT NULL, base REAL NOT NULL, repeats INTEGER NOT NULL);
             INSERT INTO words (text, pinyin, base, repeats) VALUES ('青简', 'qing jian', 3000, 1);",
        )
        .unwrap();
    drop(connection);
    // 旧库（没有 language 列）当中文读，权重照样算
    let learner = FrequencyLearner::from_path(&path).unwrap();
    assert!(learner.is_user_word("青简"));
    assert_eq!(
        learner
            .user_words()
            .unwrap()
            .lookup(&["qing", "jian"], false)[0]
            .frequency,
        3600
    );
    let _ = std::fs::remove_dir_all(&dir);
}
