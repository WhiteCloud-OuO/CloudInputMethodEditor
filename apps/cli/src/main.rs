//! 云朵输入法 CLI：Phase 1 的测试工具。
//!
//! 输入拼音，打印候选和各阶段耗时；输入序号上屏并记入用户词频。
//! 不依赖任何平台 API，是 Core 的第一个「壳」。

mod args;
mod display;
mod error;
mod eval;
mod logging;
mod repl;
mod replay;
mod rescoring;
mod tuning;

use std::time::Instant;

use clap::Parser;
use cloudime_core::{Engine, FuzzyRules};
use cloudime_dictionary::{Dictionary, WordList};
use cloudime_learning::FrequencyLearner;
use cloudime_lm::BigramModel;
use cloudime_platform::Config;

use crate::args::Args;
use crate::error::CliError;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), CliError> {
    let args = Args::parse();
    let _log_guard = logging::init()?;

    let started = Instant::now();
    let mut engine = build_engine(&args)?;
    tracing::info!(total_ms = started.elapsed().as_millis(), "Engine 就绪");
    engine.set_english_mode(args.english_mode);
    tuning::apply(&mut engine, &args.tune)?;
    if let Some(path) = &args.replay {
        let report = replay::run(&mut engine, path, args.misses)?;
        print!("{report}");
        return Ok(());
    }
    if !args.eval_text.is_empty() {
        let report = eval::run(
            &mut engine,
            &args.eval_text,
            args.eval_save.as_deref(),
            args.misses,
        )?;
        print!("{report}");
        return Ok(());
    }
    if args.inputs.is_empty() {
        repl::run(&mut engine, args.limit)?;
    } else {
        for input in &args.inputs {
            println!("> {input}");
            if args.typing {
                display::show_typing(&mut engine, input);
            } else {
                display::show(&mut engine, input, args.limit);
            }
        }
    }
    engine.learner_mut().flush();
    Ok(())
}

/// 组装 Engine：这是 Core 之外唯一知道具体 Learner 类型的地方。
fn build_engine(args: &Args) -> Result<Engine, CliError> {
    let dict_path = args
        .dict
        .clone()
        .unwrap_or_else(|| args::default_data_file("dict.tsv"));

    let started = Instant::now();
    let dictionary = Dictionary::from_path(&dict_path)?;
    let dict_load = started.elapsed();
    let english_path = args.english.clone().or_else(|| {
        let path = args::default_data_file("english.tsv");
        path.is_file().then_some(path)
    });
    let started = Instant::now();
    let english = english_path.as_ref().map(WordList::from_path).transpose()?;
    let english_load = started.elapsed();
    let learner = match &args.user_dict {
        Some(path) => FrequencyLearner::from_path(path)?,
        None => FrequencyLearner::default(),
    };
    tracing::info!(
        dict = %dict_path.display(),
        entries = dictionary.len(),
        english = english.as_ref().map_or(0, WordList::len),
        learned = learner.len(),
        dict_ms = dict_load.as_millis(),
        english_ms = english_load.as_millis(),
        "加载完成"
    );
    let mut engine = Engine::new(dictionary).with_learner(Box::new(learner));
    if !args.extra_dict.is_empty() {
        let mut extras = Vec::new();
        for path in &args.extra_dict {
            let dictionary = Dictionary::from_path(path)?;
            tracing::info!(path = %path.display(), entries = dictionary.len(), "附加词库已加载");
            extras.push(dictionary);
        }
        engine.set_extra_dictionaries(extras);
    }
    if let Some(words) = english {
        engine = engine.with_english(words);
    }
    // 语言模型可选：没有就退化成一元词频整句；打包过的 lm.qj 优先
    let packed = std::path::PathBuf::from("data/generated/lm.qj");
    let unigram = std::path::PathBuf::from("data/generated/lm-unigram.tsv");
    let bigram = std::path::PathBuf::from("data/generated/lm-bigram.tsv");
    if packed.is_file() || (unigram.is_file() && bigram.is_file()) {
        let started = Instant::now();
        let model = if packed.is_file() {
            BigramModel::from_path(&packed)?
        } else {
            BigramModel::from_paths(&unigram, &bigram)?
        };
        tracing::info!(
            words = model.word_count(),
            bigrams = model.bigram_count(),
            load_ms = started.elapsed().as_millis(),
            "语言模型已加载"
        );
        engine = engine.with_language_model(Box::new(model));
    }
    if let Some(dir) = &args.neural {
        let started = Instant::now();
        let scorer = cloudime_neural::CharScorer::load(dir)?;
        tracing::info!(
            load_ms = started.elapsed().as_millis(),
            weight = args.neural_weight.unwrap_or(cloudime_core::NEURAL_WEIGHT),
            "神经重打分已启用"
        );
        engine = if args.neural_async {
            engine.with_async_sentence_scorer(
                Box::new(scorer),
                args.neural_weight,
                args.neural_margin,
                args.neural_context,
            )
        } else {
            engine.with_sentence_scorer(
                Box::new(scorer),
                args.neural_weight,
                args.neural_margin,
                args.neural_context,
            )
        };
    }
    let config_path = args
        .config
        .clone()
        .unwrap_or_else(args::default_config_file);
    let mut config = Config::load(&config_path)?;
    if !args.fuzzy.is_empty() {
        let mut rules = FuzzyRules::default();
        for name in &args.fuzzy {
            if name == "all" {
                rules = FuzzyRules::ALL;
            } else if !rules.enable(name) {
                tracing::warn!(name, "不认识的模糊音规则，忽略");
            }
        }
        config.input.mo_hu_yin_list = cloudime_platform::fuzzy_bits(&rules);
    }
    let fuzzy = config.input.fuzzy_rules();
    if fuzzy.any() {
        tracing::info!(rules = ?fuzzy, "模糊音已启用");
    }
    engine.set_traditional_mode(
        config.input.simp_trad_chinese_chars_toggle == cloudime_platform::SimpTrad::Traditional,
    );
    engine.set_fuzzy(fuzzy);
    engine.set_use_jian_pin(config.input.use_jian_pin);
    engine.set_mixture_input(config.input.mixture_input);
    engine.set_punctuation_mapping(config.input.punctuation_mapping());
    engine.set_half_wide_after_digit(config.input.use_half_wide_punctuation_marks_after_digital);
    engine.set_association_counts(config.candidate.association_counts());
    Ok(engine)
}
