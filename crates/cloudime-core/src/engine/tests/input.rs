//! `[input]` 分节那几项开关：简拼、中英混输、符号映射。

use super::*;

/// `kf` 在缺省（开简拼）下是 开放 / 咖啡 的缩写；关掉之后必须打全才能出中文候选。
#[test]
fn turning_jian_pin_off_needs_complete_syllables() {
    let mut engine = engine();
    engine.set_input("kf");
    assert!(texts_of(&engine).contains(&"开放".to_owned()));

    engine.set_use_jian_pin(false);
    engine.set_input("kf");
    let abbreviated = engine
        .query()
        .map(|q| q.candidates.items)
        .unwrap_or_default();
    assert!(
        !abbreviated
            .iter()
            .any(|c| c.text == "开放" || c.text == "咖啡"),
        "{abbreviated:?}"
    );

    // 完整音节与末尾没打完的前缀照常
    engine.set_input("kaifa");
    assert!(texts_of(&engine).contains(&"开发".to_owned()));
    engine.set_input("kaif");
    assert!(texts_of(&engine).contains(&"开发".to_owned()));
}

/// 特殊输入模式：`yun'duo` 先只出 `yun` 的单字，选完一个字后自动进入 `duo`。
#[test]
fn special_input_walks_one_syllable_at_a_time() {
    let mut engine = Engine::new(
        Dictionary::parse(
            "云\tyun\t1000\n运\tyun\t900\n多\tduo\t1000\n朵\tduo\t800\n云朵\tyun duo\t9000\n",
        )
        .unwrap(),
    );
    engine.set_input("yun'duo");
    assert_eq!(
        engine.query().unwrap().candidates.items[0].text,
        "云朵",
        "普通模式整段读成一个词"
    );

    engine.set_special_input(true);
    let first = engine.query().unwrap().candidates.items;
    assert!(
        first.iter().all(|candidate| {
            candidate.kind == CandidateKind::Chinese
                && candidate.text.chars().count() == 1
                && candidate.syllables.len() == 1
        }),
        "特殊模式只出单字：{first:?}"
    );
    assert!(first.iter().any(|candidate| candidate.text == "云"));

    let committed = engine.commit(&first[0]);
    assert_eq!(committed, None, "还有 duo 没选，先留在组句里");
    assert!(engine.special_input(), "模式保持到用户再次切换");

    let second = engine.query().unwrap().candidates.items;
    let duo = second
        .iter()
        .find(|candidate| candidate.text == "多")
        .expect("下一个音节 duo 的单字候选");
    assert_eq!(engine.commit(duo).as_deref(), Some("云多"));
}

/// 大写字母停在拼音串里但按顺序保留：`su'A'ni` 选完得到「苏A你」。
#[test]
fn uppercase_is_an_ordered_english_literal() {
    let mut engine = Engine::new(Dictionary::parse("苏\tsu\t1000\n你\tni\t1000\n").unwrap());
    engine.set_input("su'A'ni");
    assert_eq!(engine.composition().text(), "suni");
    let su = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|candidate| candidate.text == "苏")
        .expect("su 的单字候选");
    assert_eq!(engine.commit(&su), None);
    let ni = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|candidate| candidate.text == "你")
        .expect("ni 的单字候选");
    assert_eq!(engine.commit(&ni).as_deref(), Some("苏A你"));
}

/// 特殊输入模式里选过字：整段上屏前造词；含大写英文时 K 整体算一个读音单位。
#[test]
fn special_input_creates_a_word_from_the_whole_commit() {
    let learner = WordLearner::default();
    let shared = learner.shared.clone();
    let weights = learner.learned_weights.clone();
    let mut engine = Engine::new(
        Dictionary::parse(
            "云\tyun\t1000\n朵\tduo\t1000\n输\tshu\t1000\n入\tru\t1000\n法\tfa\t1000\n\
             去\tqu\t1000\n唱\tchang\t1000\n吧\tba\t1000\n",
        )
        .unwrap(),
    )
    .with_learner(Box::new(learner));
    engine.set_special_input_word(true);
    engine.set_special_input(true);

    engine.set_input("qu'chang'K'ba");
    assert_eq!(engine.composition().text(), "qu'changba");
    for wanted in ["去", "唱"] {
        let candidate = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|candidate| candidate.text == wanted)
            .unwrap_or_else(|| panic!("{wanted} 的单字候选"));
        assert_eq!(engine.commit(&candidate), None);
    }
    let ba = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|candidate| candidate.text == "吧")
        .expect("K 跳过后应该匹配 ba");
    assert_eq!(engine.commit(&ba).as_deref(), Some("去唱K吧"));
    // 整段（含 K）收进用户词库，K 作为一个读音单位。
    let words = shared.lock().unwrap().0.clone();
    assert!(words.contains(&"去唱K吧".to_owned()), "{words:?}");
    assert_eq!(weights.lock().unwrap().get("去唱K吧"), Some(&2000.0));
}

/// 词库里已经有的整段不重复收进用户词库，只走原有学习。
#[test]
fn special_input_word_does_not_duplicate_a_known_word() {
    let learner = WordLearner::default();
    let shared = learner.shared.clone();
    let weights = learner.learned_weights.clone();
    let mut engine = Engine::new(
        Dictionary::parse(
            "云\tyun\t1000\n朵\tduo\t1000\n输\tshu\t1000\n入\tru\t1000\n法\tfa\t1000\n\
             云朵输入法\tyun duo shu ru fa\t5000\n",
        )
        .unwrap(),
    )
    .with_learner(Box::new(learner));
    engine.set_special_input_word(true);
    engine.set_special_input(true);
    engine.set_input("yun'duo'shu'ru'fa");
    let mut text = None;
    while !engine.composition().text().is_empty() || !engine.composition().has_selected() {
        let candidate = engine.query().unwrap().candidates.items[0].clone();
        text = engine.commit(&candidate).or(text);
        if engine.composition().is_empty() {
            break;
        }
    }
    assert_eq!(text.as_deref(), Some("云朵输入法"));
    assert!(shared.lock().unwrap().0.is_empty());
    assert!(weights.lock().unwrap().is_empty());
}

/// 没有在特殊模式下选过字（或中途 Esc / Insert / Enter）时不造词。
#[test]
fn special_input_word_needs_a_special_selection() {
    let learner = WordLearner::default();
    let shared = learner.shared.clone();
    let mut engine =
        Engine::new(Dictionary::parse("云\tyun\t1000\n").unwrap()).with_learner(Box::new(learner));
    engine.set_special_input_word(true);
    engine.set_special_input(true);

    // 打开又关掉，没选字。
    engine.set_special_input(false);
    assert!(shared.lock().unwrap().0.is_empty());

    // 选了一个字后 Esc。
    engine.set_special_input(true);
    engine.set_input("yun");
    let yun = engine.query().unwrap().candidates.items[0].clone();
    engine.commit(&yun);
    engine.clear();
    assert!(shared.lock().unwrap().0.is_empty());
}

/// 中英混输关掉后，中文模式不再掺英文词与英文补全。
#[test]
fn turning_mixture_input_off_hides_english_candidates() {
    let english = crate::dictionary::WordList::parse("hello\thello\t6000\n").unwrap();
    let mut engine = engine().with_english(english);
    let has_english = |engine: &mut Engine| {
        engine
            .query()
            .map(|q| {
                q.candidates
                    .items
                    .iter()
                    .any(|c| c.kind == CandidateKind::English)
            })
            .unwrap_or(false)
    };

    engine.set_input("hello");
    assert!(has_english(&mut engine), "缺省：中文模式下也出英文词");
    engine.set_mixture_input(false);
    engine.set_input("hello");
    assert!(!has_english(&mut engine), "关掉中英混输后不再出英文词");
}

/// 配置的符号映射：单键、小键盘、两键规则，以及「数字后标点保持半角」。
#[test]
fn configured_symbol_mapping_converts_and_sequences() {
    let mut engine = engine();
    engine.set_punctuation_mapping(crate::punctuation::Mapping::from_pairs([
        ("/", "、"),
        ("{kp}*", "×"),
        ("~=", "≈"),
    ]));
    assert_eq!(
        engine.map_symbol('/', false).map(|m| m.text),
        Some("、".to_owned())
    );
    // 主键区的条目不管小键盘
    assert_eq!(engine.map_symbol('/', true), None);
    assert_eq!(
        engine.map_symbol('*', true).map(|m| m.text),
        Some("×".to_owned())
    );
    // 两键规则：`~` 先按内置表上屏 `～`，敲 `=` 时把它撤掉换成 ≈
    assert_eq!(engine.punctuate('~').as_deref(), Some("～"));
    let mapped = engine.map_symbol('=', false).expect("两键规则命中");
    assert_eq!((mapped.text.as_str(), mapped.delete_before), ("≈", 1));
    // 内置的全角标点表照旧
    assert_eq!(engine.punctuate(',').as_deref(), Some("，"));
    // 数字后标点保持半角（缺省开）
    engine.note_passthrough('2');
    assert_eq!(engine.map_symbol('*', true), None);
    engine.set_half_wide_after_digit(false);
    engine.note_passthrough('2');
    assert_eq!(
        engine.map_symbol('*', true).map(|m| m.text),
        Some("×".to_owned())
    );
}

/// 稀有组缺省关闭（由 `[word_bank] rare_items` 决定），打开后才参与查询。
#[test]
fn turning_rare_dictionary_off_skips_it() {
    let rare = Dictionary::parse("龘\tda\t500\n").unwrap();
    let mut engine = engine().with_rare(rare);
    // 缺省关：挂着稀有组也不查
    assert!(!engine.rare_enabled());
    assert!(engine.rare_dictionary().is_some());

    engine.set_input("da");
    assert!(!texts_of(&engine).contains(&"龘".to_owned()));

    engine.set_rare_enabled(true);
    engine.set_input("da");
    assert!(texts_of(&engine).contains(&"龘".to_owned()));

    engine.set_rare_enabled(false);
    engine.set_input("da");
    assert!(!texts_of(&engine).contains(&"龘".to_owned()));
}
