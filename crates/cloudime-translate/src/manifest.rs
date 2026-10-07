//! `LocalDictionary\dictionaries.list`：一行一个 `显示名=文件名`，给设置页的「本地词典」下拉用。

use std::path::{Path, PathBuf};

/// 清单里的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// 设置页上显示的名字（等号左边）。
    pub name: String,

    /// 文件名（等号右边，相对 `LocalDictionary\`）。
    pub file: String,
}

/// 本地词典清单。
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    /// 清单所在的目录（`LocalDictionary\`）；词典文件都从这里找。
    dir: PathBuf,

    /// 清单里的项，按文件里的顺序。
    items: Vec<Item>,
}

impl Manifest {
    /// 清单文件名。
    pub const FILE: &'static str = "dictionaries.list";

    /// 从目录读清单。文件不在 / 读不了就是空清单（设置页显示为空、翻译功能不可用），不报错。
    pub fn load(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let path = dir.join(Self::FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self {
                items: parse(&text),
                dir,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self {
                dir,
                items: Vec::new(),
            },
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "本地词典清单读不了");
                Self {
                    dir,
                    items: Vec::new(),
                }
            }
        }
    }

    /// 清单所在目录。
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 一个文件名的完整路径（清单里没有也照样拼——配置可以是手写的）。
    pub fn path_of(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    /// 文件对应的显示名；清单里没有就是 `None`（设置页回落到文件名）。
    pub fn name_of(&self, file: &str) -> Option<&str> {
        self.items
            .iter()
            .find(|item| item.file == file)
            .map(|item| item.name.as_str())
    }
}

/// 拆清单文本：`显示名=文件名` 的行；空行与 `#` 跳过，坏行警告后跳过。
fn parse(text: &str) -> Vec<Item> {
    let mut items = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, file)) = line.split_once('=') else {
            tracing::warn!(
                line = number + 1,
                "本地词典清单这行看不懂（要 `显示名=文件名`），跳过"
            );
            continue;
        };
        let (name, file) = (name.trim(), file.trim());
        if name.is_empty() || file.is_empty() {
            continue;
        }
        items.push(Item {
            name: name.to_owned(),
            file: file.to_owned(),
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_names_and_files_in_order() {
        let items = parse("# 注释\n\n英语词典=glossary-en.qj\n西语词典 = glossary-es.qj \n坏行\n");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "英语词典");
        assert_eq!(items[0].file, "glossary-en.qj");
        assert_eq!(items[1].name, "西语词典");
        assert_eq!(items[1].file, "glossary-es.qj");
    }

    #[test]
    fn a_missing_manifest_reads_as_empty() {
        let manifest = Manifest::load(std::env::temp_dir().join("cloudime-no-such-dir"));
        assert!(manifest.is_empty());
        assert_eq!(manifest.name_of("a.qj"), None);
    }
}
