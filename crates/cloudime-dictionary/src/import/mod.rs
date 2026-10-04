//! 词库导入：设置页「导入词库」只接受现成的 `.db` 词库存档，校验后原样放进词库目录。
//!
//! 三方词库由 `tools/dict-convert` 生成 `.db` 后分发，这里不做格式转换（云朵 TSV / Rime 的导入已下线）。

mod imported;

use std::path::Path;

use crate::dictionary::Dictionary;
use crate::error::DictionaryError;

pub use imported::Imported;

/// 把现成的 `.db` 词库导入 `dest_dir`：读一遍确认能当词库用，再原样复制（不重写，保住英文行与元数据）。
/// 目标名取源文件主干（`law.db` → `law.db`），同名覆盖；源文件已经在目标位置时什么都不做。
pub fn import(source: &Path, dest_dir: &Path) -> Result<Imported, DictionaryError> {
    if !has_db_extension(source) {
        return Err(DictionaryError::Corrupt("not a .db word bank file"));
    }
    let stem = stem_of(source)?;
    // 非 SQLite / 坏文件在这里报错，不让坏词库进目录
    let dictionary = Dictionary::from_path(source)?;
    if dictionary.is_empty() {
        return Err(DictionaryError::Corrupt("word bank has no usable entries"));
    }
    let name = dictionary
        .metadata()
        .map(|metadata| metadata.name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| stem.clone());
    let entries = dictionary.len();
    std::fs::create_dir_all(dest_dir)?;
    let target = dest_dir.join(format!("{stem}.db"));
    if !same_file(source, &target) {
        std::fs::copy(source, &target)?;
    }
    Ok(Imported {
        path: target,
        name,
        entries,
    })
}

/// 文件扩展名是不是 `.db`（大小写不敏感）。
fn has_db_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("db"))
}

/// 源文件名去掉 `.db` 扩展名后的主干；没有可用名字时报错。
fn stem_of(source: &Path) -> Result<String, DictionaryError> {
    source
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|stem| !stem.is_empty())
        .map(str::to_owned)
        .ok_or(DictionaryError::Corrupt("source has no usable file name"))
}

/// 两个路径是不是同一个文件（目标还不存在时按字面比）。
fn same_file(source: &Path, target: &Path) -> bool {
    match (std::fs::canonicalize(source), std::fs::canonicalize(target)) {
        (Ok(source), Ok(target)) => source == target,
        _ => source == target,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cloudime_format::Metadata;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cloudime-import-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn imports_a_db_word_bank_as_is() {
        let dir = scratch("db");
        let source = dir.join("finance.db");
        let metadata = Metadata {
            name: "财务词库".to_owned(),
            license: "MIT".to_owned(),
            ..Metadata::default()
        };
        Dictionary::parse("账套\tzhang tao\t500\n")
            .unwrap()
            .write_db(&source, &metadata)
            .unwrap();

        let imported = import(&source, &dir.join("dicts")).unwrap();
        assert_eq!(imported.name, "财务词库");
        assert_eq!(imported.entries, 1);
        assert!(imported.path.ends_with("dicts/finance.db"));
        let dictionary = Dictionary::from_path(&imported.path).unwrap();
        assert_eq!(dictionary.lookup(&["zhang", "tao"], false)[0].text, "账套");
        assert_eq!(dictionary.metadata().unwrap().name, "财务词库");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn imports_skip_itself_when_source_already_in_place() {
        let dir = scratch("inplace");
        let bank = dir.join("WordBank");
        std::fs::create_dir_all(&bank).unwrap();
        let path = bank.join("law.db");
        Dictionary::parse("法\tfa\t30\n")
            .unwrap()
            .write_db(&path, &Metadata::default())
            .unwrap();

        let imported = import(&path, &bank).unwrap();
        assert_eq!(imported.path, path);
        assert!(Dictionary::from_path(&imported.path).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_tsv_and_rime() {
        let dir = scratch("reject");
        let tsv = dir.join("finance.tsv");
        std::fs::write(&tsv, "账套\tzhang tao\t500\n").unwrap();
        assert!(matches!(
            import(&tsv, &dir.join("dicts")),
            Err(DictionaryError::Corrupt(_))
        ));

        let rime = dir.join("law.dict.yaml");
        std::fs::write(&rime, "---\nname: law\n...\n合同法\the tong fa\t120\n").unwrap();
        assert!(import(&rime, &dir.join("dicts")).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
