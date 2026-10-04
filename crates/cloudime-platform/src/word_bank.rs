//! 词库目录（WordBank）：随包根下 `WordBank\`。
//!
//! 随包只带一份 `Dict.db`（中文 + 英文合一份，见 `cloudime_dictionary::DictDb`）；用户「导入词库」
//! 放进来的其他 `.db` 都是附加词库，目录里有的全部加载，没有 List.dat 这种启用清单
//!（旧版留下的 `.tsv` 仍能加载）。内置的 `Dict.db` 与数据目录里的 `UserWordBank.db` 始终加载、
//! 不出现在设置页的导入列表里。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use cloudime_dictionary::{Dictionary, WORD_BANK_FILE};

/// 词库目录名（随包根下）。
pub const WORD_BANK_DIR: &str = "WordBank";

/// 能加载的扩展名，靠前的优先：同名的 `.db` 与 `.tsv` 只取 `.db`。
const EXTENSIONS: [&str; 2] = ["db", "tsv"];

/// 内置词库文件名（大小写不敏感），设置页不列出、也不能移除：
/// 随包主词库 `Dict.db`，以及用户自造词库 `UserWordBank.db`（在数据目录，正常不出现在 `WordBank\`）。
const BUILTIN_FILES: [&str; 2] = [WORD_BANK_FILE, "UserWordBank.db"];

/// 词库目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordBank {
    /// 词库目录。
    pub dir: PathBuf,
}

/// 词库目录里的一个词库文件（设置页列它）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordBankFile {
    /// 文件名（含扩展名）。
    pub file: String,

    /// 词干（不含扩展名）。
    pub stem: String,

    /// 完整路径。
    pub path: PathBuf,

    /// 词库自己的名字（读元数据；没有就用词干）。
    pub name: String,

    /// 词条数（主词库是中文条数，英文词表另算）。
    pub entries: usize,

    /// 许可证。
    pub license: String,

    /// 文件坏了（读不出来）。
    pub broken: bool,
}

/// 目录里词库文件的快照：用于发现新增、移除与同名更新，不读正文。
pub fn snapshot(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(PathBuf, Option<SystemTime>, u64)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| EXTENSIONS.contains(&extension))
        })
        .filter_map(|path| {
            let metadata = std::fs::metadata(&path).ok()?;
            Some((path, metadata.modified().ok(), metadata.len()))
        })
        .collect();
    files.sort();
    files
}

/// 指定文件的快照（短语库用它）。
pub fn snapshot_files(paths: &[&Path]) -> Vec<(PathBuf, Option<SystemTime>, u64)> {
    paths
        .iter()
        .filter_map(|path| {
            let metadata = std::fs::metadata(path).ok()?;
            Some((path.to_path_buf(), metadata.modified().ok(), metadata.len()))
        })
        .collect()
}

impl WordBank {
    /// 按随包根定位（缺省 `WordBank\`）。
    pub fn locate(bundled_root: &Path) -> Self {
        Self {
            dir: bundled_root.join(WORD_BANK_DIR),
        }
    }

    /// 随包主词库的完整路径（`WordBank\Dict.db`）。
    pub fn path(&self) -> PathBuf {
        self.dir.join(WORD_BANK_FILE)
    }

    /// 目录里的词库文件（按文件名排序，同名只留优先扩展名的），返回 `(词干, 文件名, 路径)`。
    pub fn files(&self) -> Vec<(String, String, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut files: Vec<(String, String, usize, PathBuf)> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter_map(|path| {
                let extension = path.extension()?.to_str()?;
                let rank = EXTENSIONS.iter().position(|known| *known == extension)?;
                let file = path.file_name()?.to_str()?.to_owned();
                let stem = path.file_stem()?.to_str()?.to_owned();
                Some((file, stem, rank, path))
            })
            .collect();
        files.sort();
        files.dedup_by(|a, b| a.1 == b.1);
        files
            .into_iter()
            .map(|(file, stem, _, path)| (stem, file, path))
            .collect()
    }

    /// 打开一个文件读它的名字 / 条数 / 许可证；读不出来标成坏的。
    pub fn describe(stem: &str, file: &str, path: &Path) -> WordBankFile {
        match Dictionary::from_path(path) {
            Ok(dictionary) => {
                let metadata = dictionary.metadata();
                WordBankFile {
                    file: file.to_owned(),
                    stem: stem.to_owned(),
                    path: path.to_path_buf(),
                    name: metadata
                        .map(|m| m.name.clone())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| stem.to_owned()),
                    entries: dictionary.len(),
                    license: metadata.map(|m| m.license.clone()).unwrap_or_default(),
                    broken: false,
                }
            }
            Err(error) => {
                tracing::warn!(file = %path.display(), %error, "词库读不出来");
                WordBankFile {
                    file: file.to_owned(),
                    stem: stem.to_owned(),
                    path: path.to_path_buf(),
                    name: stem.to_owned(),
                    entries: 0,
                    license: String::new(),
                    broken: true,
                }
            }
        }
    }

    /// 内置词库（`Dict.db` / `UserWordBank.db`）没有开关、一定会加载，也不让移除。
    pub fn is_builtin(file: &str) -> bool {
        BUILTIN_FILES
            .iter()
            .any(|known| known.eq_ignore_ascii_case(file))
    }

    /// 用户导入的附加词库（设置页列它）：目录里除内置 `Dict.db` / `UserWordBank.db` 外的词库。
    pub fn imported(&self) -> Vec<WordBankFile> {
        self.files()
            .into_iter()
            .filter(|(_, file, _)| !Self::is_builtin(file))
            .map(|(stem, file, path)| Self::describe(&stem, &file, &path))
            .collect()
    }

    /// 加载除主词库外的全部附加词库（目录里有的都加载）。坏文件只记日志、跳过。
    pub fn load_except(&self, main: Option<&Path>) -> Vec<Dictionary> {
        let mut loaded = Vec::new();
        for (stem, file, path) in self.files() {
            if main.is_some_and(|main| main == path) {
                continue;
            }
            if let Some(dictionary) = self.open(&stem, &file, &path) {
                loaded.push(dictionary);
            }
        }
        loaded
    }

    /// 主词库：`Dict.db` 优先，否则第一个词库；目录里一个都没有返回 `None`。
    pub fn main(&self) -> Option<(String, PathBuf)> {
        self.files()
            .into_iter()
            .find(|(stem, ..)| stem.eq_ignore_ascii_case("dict"))
            .or_else(|| self.files().into_iter().next())
            .map(|(stem, _, path)| (stem, path))
    }

    fn open(&self, stem: &str, file: &str, path: &Path) -> Option<Dictionary> {
        match Dictionary::from_path(path) {
            Ok(dictionary) => {
                tracing::info!(
                    name = %dictionary.metadata().map_or(stem, |m| m.name.as_str()),
                    file = %file,
                    entries = dictionary.len(),
                    license = %dictionary.metadata().map_or("", |m| m.license.as_str()),
                    "词库已加载"
                );
                Some(dictionary)
            }
            Err(error) => {
                tracing::warn!(file = %path.display(), %error, "词库加载失败，跳过");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (PathBuf, WordBank) {
        let dir = std::env::temp_dir().join(format!("cloudime-wordbank-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bank = WordBank {
            dir: dir.join("WordBank"),
        };
        (dir, bank)
    }

    #[test]
    fn locate_uses_the_bundled_word_bank_directory() {
        let bank = WordBank::locate(Path::new("C:/CloudIME"));
        assert_eq!(bank.dir, Path::new("C:/CloudIME/WordBank"));
        assert!(bank.path().ends_with("WordBank/Dict.db"));
    }

    #[test]
    fn main_prefers_dict_db() {
        let (dir, bank) = fixture("main");
        std::fs::create_dir_all(&bank.dir).unwrap();
        std::fs::write(bank.dir.join("idioms.tsv"), "成语\tcheng yu\t10\n").unwrap();
        std::fs::write(bank.dir.join("Dict.db"), b"not really a db").unwrap();
        let (stem, path) = bank.main().unwrap();
        assert_eq!(stem, "Dict");
        assert!(path.ends_with("Dict.db"));
        // 没有 Dict.db 时退回第一个词库
        std::fs::remove_file(bank.dir.join("Dict.db")).unwrap();
        let (stem, _) = bank.main().unwrap();
        assert_eq!(stem, "idioms");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_skips_the_main_dictionary() {
        let (dir, bank) = fixture("load");
        std::fs::create_dir_all(&bank.dir).unwrap();
        std::fs::write(bank.dir.join("dict.tsv"), "词\tci\t20\n").unwrap();
        std::fs::write(bank.dir.join("law.tsv"), "法\tfa\t10\n").unwrap();
        let main = bank.dir.join("dict.tsv");
        let extras = bank.load_except(Some(&main));
        assert_eq!(extras.len(), 1);
        assert_eq!(extras[0].lookup(&["fa"], false)[0].text, "法");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn imported_list_hides_the_builtins() {
        let (dir, bank) = fixture("imported");
        std::fs::create_dir_all(&bank.dir).unwrap();
        std::fs::write(bank.dir.join("Dict.db"), b"not really a db").unwrap();
        std::fs::write(bank.dir.join("UserWordBank.db"), b"not really a db").unwrap();
        std::fs::write(bank.dir.join("law.tsv"), "法\tfa\t10\n").unwrap();
        let files: Vec<String> = bank.imported().into_iter().map(|file| file.file).collect();
        assert_eq!(files, ["law.tsv"]);
        assert!(WordBank::is_builtin("Dict.db"));
        assert!(WordBank::is_builtin("userwordbank.db"));
        assert!(!WordBank::is_builtin("law.db"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
