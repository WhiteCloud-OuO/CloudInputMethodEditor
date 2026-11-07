//! 主题文件（`Themes\*.json`）：候选窗口、悬浮工具栏、状态切换提示三个窗口的 21 个颜色。
//!
//! 两个目录，按「用户目录优先」找同名文件：
//! - 随包默认：`<安装目录>\Themes\<名>.json`（只读，升级 / 卸载会动这一棵）
//! - 用户自己的：`%APPDATA%\CloudIME\Themes\<名>.json`（**可写**，设置页编辑 / 新建的就是这里）
//!
//! 文件是 JSON，颜色一律 `#AARRGGBB`；缺键用缺省色（缺省对齐现在的实际观感）。
//! 数据类型与读写方法在 `cloud-ui-types`；这里负责目录扫描与查找。

use std::path::{Path, PathBuf};

pub use cloud_ui_types::{BarTheme, CandidateTheme, ThemeColor, ThemeError, ThemeFile, TipTheme};

/// 随包默认主题的文件名（新建主题就从它复制）。随包另有一份 `Panic.json`，两份都是内置主题。
pub const DEFAULT_THEME_FILE: &str = "Default.json";

/// 放主题文件的子目录名。
pub const THEMES_DIR: &str = "Themes";

/// 一个可选的「主题」的一条：名字（不带 `.json`）与文件位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeEntry {
    /// 主题名（文件名去掉 `.json`）。
    pub name: String,
    /// 文件完整路径。
    pub path: PathBuf,
    /// 在用户目录（可写）还是随包目录（只读）。
    pub user: bool,
}

/// 主题文件的两个目录：`(用户目录, 随包目录)`，可能各自拿不到。
pub fn theme_dirs(bundled_root: Option<&Path>) -> (Option<PathBuf>, Option<PathBuf>) {
    (
        crate::dirs::user_dir().map(|dir| dir.join(THEMES_DIR)),
        bundled_root.map(|root| root.join(THEMES_DIR)),
    )
}

/// 列出一份主题文件是不是 `.json`。
fn is_theme_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

/// 列出所有主题：**用户目录优先**（同名盖掉随包的），按名字排序。
pub fn list_themes(bundled_root: Option<&Path>) -> Vec<ThemeEntry> {
    let (user_dir, bundled_dir) = theme_dirs(bundled_root);
    let mut entries = Vec::new();
    let mut scan = |dir: &Path, user: bool| {
        let Ok(list) = std::fs::read_dir(dir) else {
            return;
        };
        for file in list.flatten() {
            let path = file.path();
            if !is_theme_file(&path) {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if entries.iter().any(|entry: &ThemeEntry| entry.name == name) {
                continue; // 用户目录先扫，同名的随包主题让位
            }
            entries.push(ThemeEntry {
                name: name.to_owned(),
                path,
                user,
            });
        }
    };
    if let Some(dir) = &user_dir {
        scan(dir, true);
    }
    if let Some(dir) = &bundled_dir {
        scan(dir, false);
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

/// 找一个主题文件（用户目录优先）；`name` 可带或不带 `.json`。
pub fn find_theme(bundled_root: Option<&Path>, name: &str) -> Option<ThemeEntry> {
    let stem = name.trim().trim_end_matches(".json");
    list_themes(bundled_root)
        .into_iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(stem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_file_fills_in_the_missing_keys() {
        let theme: ThemeFile =
            serde_json::from_str(r##"{"candidate":{"background":"#FF112233"}}"##).unwrap();
        assert_eq!(
            theme.candidate.background,
            ThemeColor::rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(theme.candidate.text, CandidateTheme::default().text);
        assert_eq!(theme.bar, BarTheme::default());
        assert_eq!(theme.tip.background, TipTheme::default().background);
    }

    #[test]
    fn the_default_theme_round_trips_through_json() {
        let theme = ThemeFile::default();
        let json = serde_json::to_string(&theme).unwrap();
        assert_eq!(serde_json::from_str::<ThemeFile>(&json).unwrap(), theme);
    }

    /// 随包的 `Themes/<DEFAULT_THEME_FILE>` 必须就是内置缺省：它是「新建主题」的底，也是装机后
    /// 找不到主题文件时的回落，两者一旦漂移，同一个「默认外观」会随文件在不在而不同。
    /// 只按 `DEFAULT_THEME_FILE` 在**随包目录**里找（不碰用户目录，免得读到用户自己那份同名的）。
    #[test]
    fn the_shipped_default_theme_is_the_built_in_default() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (_, bundled) = theme_dirs(Some(root.as_path()));
        let path = bundled
            .expect("随包目录应该能算出来")
            .join(DEFAULT_THEME_FILE);
        let theme = ThemeFile::load(&path).expect("随包的默认主题应该能读");
        assert_eq!(theme, ThemeFile::default());
    }
}
