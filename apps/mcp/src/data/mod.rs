//! MCP Server 的只读词库装载。

mod dictionary_info;
mod dictionary_info_response;
mod error;
mod load;
mod loaded;
mod options;

pub use dictionary_info::DictionaryInfo;
pub use dictionary_info_response::DictionaryInfoResponse;
pub use error::LoadError;
pub use load::load;
pub use loaded::Loaded;
pub use options::LoadOptions;
