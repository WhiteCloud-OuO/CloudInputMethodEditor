//! 装配 Engine：Server 里唯一知道具体 Learner / 词表类型的地方。

mod language_model;
mod spec;

use std::path::Path;
use std::time::Instant;

use cloudime_core::Engine;
use cloudime_dictionary::DictDb;
use cloudime_learning::{FrequencyLearner, InputLog, UsageStats, VocabularyBook};

use crate::error::ServerError;

pub use self::language_model::LanguageModelFiles;
pub use self::spec::AssemblySpec;

pub fn assemble(spec: &AssemblySpec) -> Result<Engine, ServerError> {
    let started = Instant::now();
    let data = DictDb::from_path(&spec.dict)?;
    let dictionary = data.chinese;
    let rare = data.rare;
    let english = data.english;
    let learner = match &spec.user_dir {
        Some(dir) => load_learner(dir),
        None => FrequencyLearner::default(),
    };
    tracing::info!(
        entries = dictionary.len(),
        rare = rare.len(),
        english = english.len(),
        learned = learner.len(),
        dictionary_ms = started.elapsed().as_millis(),
        "词库与学习数据已加载"
    );
    let mut engine = Engine::new(dictionary).with_learner(Box::new(learner));
    if !rare.is_empty() {
        tracing::info!(
            words = rare.len(),
            "稀有词库已随 Dict.db 加载（是否参与查询由 [word_bank] rare_items 决定）"
        );
        engine = engine.with_rare(rare);
    }
    if let Some(dir) = &spec.user_dir {
        engine = engine
            .with_usage_meter(Box::new(UsageStats::open(dir.join("usage.tsv"))))
            .with_vocabulary_tracker(Box::new(VocabularyBook::open(dir.join("user-vocab.tsv"))));
        if spec.input_log {
            let path = dir.join("input-log.jsonl");
            tracing::info!(path = %path.display(), "输入日志开着");
            engine = engine.with_input_logger(Box::new(InputLog::open(path)));
        }
    }
    engine.set_extra_dictionaries(
        spec.word_bank
            .as_ref()
            .map(|bank| bank.load_except(Some(&spec.dict)))
            .unwrap_or_default(),
    );
    if !english.is_empty() {
        tracing::info!(words = english.len(), "英文词表已随 Dict.db 加载");
        engine = engine.with_english(english);
    }
    if let Some(files) = &spec.language_model {
        let started = Instant::now();
        match files.load() {
            Ok(model) => {
                tracing::info!(
                    words = model.word_count(),
                    bigrams = model.bigram_count(),
                    load_ms = started.elapsed().as_millis(),
                    "语言模型已加载"
                );
                engine = engine.with_language_model(Box::new(model));
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    path = %files.path().display(),
                    "语言模型加载失败，退化成一元词频整句"
                );
            }
        }
    }
    Ok(engine)
}

/// 读不了就退回只在内存里学，不拿空表覆盖用户文件。
fn load_learner(dir: &Path) -> FrequencyLearner {
    let path = dir.join("user.tsv");
    match FrequencyLearner::from_path(&path) {
        Ok(learner) => learner,
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "学习数据读取失败，本次只在内存里学习");
            FrequencyLearner::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 语言模型读不出来就地降级，装配照样成功；只有主词库坏了才留给外层回落样例词库。
    #[test]
    fn optional_data_files_degrade_instead_of_failing() {
        let dir = std::env::temp_dir().join(format!("cloudime-assembly-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let unreadable = [0xff, 0xfe, 0xff, 0xff];
        let language_model = dir.join("lm.qj");
        std::fs::write(&language_model, unreadable).unwrap();
        let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/sample/dict.tsv");

        let degraded = AssemblySpec {
            language_model: Some(LanguageModelFiles::Packed(language_model)),
            ..AssemblySpec::new(sample)
        };
        assert!(assemble(&degraded).is_ok());

        // 主词库没得降级，报错交给 assemble_with_fallback 换样例词库
        let broken_dict = dir.join("dict.qj");
        std::fs::write(&broken_dict, unreadable).unwrap();
        assert!(assemble(&AssemblySpec::new(broken_dict)).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }
}
