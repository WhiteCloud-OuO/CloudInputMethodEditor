//! 本地词典与候选翻译 Tip：查词条的释义，并记住这个词条学没学会。
//!
//! 词典放在安装目录的 `LocalDictionary\` 下，清单文件 [`Manifest::FILE`]（`dictionaries.list`）
//! 一行一个 `显示名=文件名`；词典文件支持 `.qj`（青简那套释义容器，mmap 零拷贝）与 `.db`（SQLite）。
//! 文件本身只读，所以「上屏过几次 / 学会没有」记在用户目录的另一个库（[`Learning`]）里。
//!
//! ```text
//! LocalDictionary\
//!     dictionaries.list      英语词典=glossary-en.qj
//!     glossary-en.qj
//! %APPDATA%\CloudIME\translate.db   学习状态
//! ```

mod error;
mod glossary;
mod learning;
mod manifest;
mod model;

pub use error::TranslateError;
pub use glossary::Glossary;
pub use learning::{Entry, Learning};
pub use manifest::{Item, Manifest};
pub use model::{Sense, Tip};
