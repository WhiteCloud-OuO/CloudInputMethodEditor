//! CLI 的错误类型：词库、语言模型、神经重排、配置、学习、回放、评测与调参各自的错误。
use cloudime_dictionary::DictionaryError;
use cloudime_learning::LearningError;
use cloudime_lm::LmError;
use cloudime_neural::NeuralError;
use cloudime_platform::ConfigError;
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(transparent)]
    Dictionary(#[from] DictionaryError),

    #[error(transparent)]
    Neural(#[from] NeuralError),

    #[error(transparent)]
    Learning(#[from] LearningError),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    LanguageModel(#[from] LmError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Replay(#[from] crate::replay::ReplayError),

    #[error(transparent)]
    Eval(#[from] crate::eval::EvalError),

    #[error(transparent)]
    Tune(#[from] crate::tuning::TuneError),
}
