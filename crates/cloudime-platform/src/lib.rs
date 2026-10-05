//! 平台层共用的、与具体窗口系统无关的部分：配置文件，以及将来 Core 与壳之间的协议类型。
//!
//! 这里的类型必须可序列化：Windows 上 Core 在独立 Server 进程，同一套类型两边都用。

mod config;
pub mod dirs;
mod error;
pub mod logs;
pub mod migrate;
pub mod phrase;
pub mod protocol;
pub mod resources;
pub mod word_bank;

pub use config::{
    CandidateConfig, Config, DEFAULT_CANDIDATE_BOX_MINIMUM_WIDTH, DEFAULT_FAMILY,
    DEFAULT_PUNCTUATION_MAPPING, DEFAULT_USER_WORD_BANK_FILE, DebuggingConfig, FontChoice,
    FullHalfPunctuation, GeneralConfig, InputConfig, ItemNumberStyle, LayoutMode, LogLevel,
    MAX_ASSOCIATION_COUNTS, MAX_CANDIDATE_COUNT, MAX_PAGE_SIZE, MIN_ASSOCIATION_COUNTS,
    MIN_CANDIDATE_COUNT, MO_HU_YIN_BITS, PAIRWISE_COMPLETION_BITS, PUNCTUATION_MAPPING_BITS,
    PhraseConfig, PreeditMode, SimpTrad, SwitchKey, SwitchKeys, UpdateChannel, UpdateConfig,
    WordBankConfig, fuzzy_bits, pair_bit, pairwise_completion,
};
pub use error::ConfigError;
pub use migrate::Migration;
pub use phrase::{PhraseError, PhraseStore};
pub use word_bank::{WORD_BANK_DIR, WordBank, WordBankFile};
