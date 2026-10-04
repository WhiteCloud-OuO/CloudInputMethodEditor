//! 词库：按拼音音节序列查词。
//!
//! 纯数据层，不依赖任何兄弟 crate。随包词库是一份 `WordBank\Dict.db`（SQLite），中文行按拼音音节查、
//! 英文行装成 [`WordList`]（见 [`DictDb`]）；TSV（下例）与老 `.qj` 也认。
//!
//! ```text
//! 词\t音节（空格分隔）\t词频
//! 开发\tkai fa\t9000
//! ```
//!
//! `#` 开头为注释行，空行忽略。
//!
//! 内存布局面向「几十万到上百万条常驻」：词文本与拼音键各放一个连续 arena，
//! 词目只存偏移与词频，键排序后二分定位、顺序扫描前缀范围。
//!
//! 旁支是英文词表 [`WordList`] 与中英合一份的 [`DictDb`]。

mod dictionary;
mod error;
pub mod import;
mod matching;
mod pattern;
mod word_list;

pub use dictionary::{DictDb, Dictionary, WORD_BANK_FILE};
pub use error::DictionaryError;
pub use matching::Match;
pub use pattern::{SyllablePattern, canonical_syllable};
pub use word_list::WordList;
