use std::path::PathBuf;

use cloudime_platform::WordBank;

use super::LanguageModelFiles;

/// 装配要用的数据文件。除词库外都可选：缺哪个就少哪个功能。
pub struct AssemblySpec {
    /// 主词库 `WordBank\Dict.db`（中文 + 英文合一份）；也可以是旧格式 `.db` 或 TSV。
    pub dict: PathBuf,

    /// 语言模型；没有就退化成一元词频整句。
    pub language_model: Option<LanguageModelFiles>,

    /// 词库目录（WordBank）：用户导入的附加词库从它读，主词库那一份跳过。
    pub word_bank: Option<WordBank>,

    /// 用户数据目录（`%APPDATA%\CloudIME`）；没有就都只在内存。
    pub user_dir: Option<PathBuf>,

    /// 用户自造词库路径（`[word_bank] user_file` 解析结果）；没有时退回与词频文件同目录。
    pub user_word_bank: Option<PathBuf>,

    /// 是否写输入日志（`[general] input_log`）。
    pub input_log: bool,
}

impl AssemblySpec {
    pub fn new(dict: impl Into<PathBuf>) -> Self {
        Self {
            dict: dict.into(),
            language_model: None,
            word_bank: None,
            user_dir: None,
            user_word_bank: None,
            input_log: false,
        }
    }
}
