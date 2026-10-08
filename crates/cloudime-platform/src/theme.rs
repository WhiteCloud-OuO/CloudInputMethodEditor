//! 主题文件（`Themes\*.json`）：候选窗口、悬浮工具栏、状态切换提示三个窗口的 21 个颜色。
//!
//! 两个目录，按「用户目录优先」找同名文件：
//! - 随包默认：`<安装目录>\Themes\<名>.json`（只读，升级 / 卸载会动这一棵）；
//! - 用户自己的：`%APPDATA%\CloudIME\Themes\<名>.json`（**可写**，设置页编辑 / 新建的就是这里）。
//!
//! 文件是 JSON，颜色一律 `#AARRGGBB`；缺键用缺省色（缺省对齐现在的实际观感）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 随包默认主题的文件名（新建主题就从它复制）。随包另有一份 `Panic.json`，两份都是内置主题。
pub const DEFAULT_THEME_FILE: &str = "Default.json";

/// 放主题文件的子目录名。
pub const THEMES_DIR: &str = "Themes";

/// 一个 `#AARRGGBB` 颜色（每个通道 8 位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ThemeColor {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl ThemeColor {
    /// 按 ARGB 建一个。
    pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Self {
        Self { a, r, g, b }
    }

    /// 不透明色。
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::argb(0xFF, r, g, b)
    }

    /// 解析 `#AARRGGBB` / `#RRGGBB`（`#` 可省）；长度不对或不是十六进制为 `None`。
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().trim_start_matches('#');
        let digits = u32::from_str_radix(hex, 16).ok()?;
        match hex.len() {
            8 => Some(Self::argb(
                (digits >> 24) as u8,
                (digits >> 16) as u8,
                (digits >> 8) as u8,
                digits as u8,
            )),
            6 => Some(Self::rgb(
                (digits >> 16) as u8,
                (digits >> 8) as u8,
                digits as u8,
            )),
            _ => None,
        }
    }

    /// `#AARRGGBB`。
    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.a, self.r, self.g, self.b)
    }
}

impl TryFrom<String> for ThemeColor {
    type Error = ThemeError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text).ok_or(ThemeError::Color(text))
    }
}

impl From<ThemeColor> for String {
    fn from(color: ThemeColor) -> Self {
        color.hex()
    }
}

/// 主题文件读写 / 解析失败的原因。
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("颜色写法不对：{0}（要 #AARRGGBB 或 #RRGGBB）")]
    Color(String),

    #[error("读写主题文件失败：{0}")]
    Io(#[from] std::io::Error),

    #[error("主题文件不是合法 JSON：{0}")]
    Json(#[from] serde_json::Error),
}

/// 候选窗口的 14 个颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CandidateTheme {
    /// 窗口背景。
    pub background: ThemeColor,
    /// 当前候选的高亮条底色。
    pub highlight: ThemeColor,
    /// 窗口阴影。
    pub shadow: ThemeColor,
    /// 拼音串。
    pub pinyin: ThemeColor,
    /// 拼音串里的光标。
    pub pinyin_caret: ThemeColor,
    /// 高亮的那条候选项文字。
    pub highlight_text: ThemeColor,
    /// 普通候选项文字。
    pub text: ThemeColor,
    /// 高亮的那条候选项的序号。
    pub highlight_index: ThemeColor,
    /// 普通候选项的序号。
    pub index: ThemeColor,
    /// 页码。
    pub page_number: ThemeColor,
    /// 候选项右侧来源角标（「短」/「造」）。
    pub badge: ThemeColor,
    /// 翻译 Tip 里的词性、分号、读音括号。
    pub translate_meta: ThemeColor,
    /// 翻译 Tip 里已学会的释义。
    pub translate_learned: ThemeColor,
    /// 翻译 Tip 里还没学会的释义。
    pub translate_fresh: ThemeColor,
    /// 候选项额外内容（如云翻译返回的文本）。
    pub extra: ThemeColor,
}

impl Default for CandidateTheme {
    fn default() -> Self {
        Self {
            background: ThemeColor::rgb(0xFF, 0xFF, 0xFF),
            highlight: ThemeColor::argb(0xF0, 0xC8, 0xF1, 0xFF),
            shadow: ThemeColor::argb(0x5A, 0x00, 0x00, 0x00),
            pinyin: ThemeColor::rgb(0x00, 0x00, 0x00),
            pinyin_caret: ThemeColor::argb(0xD8, 0x00, 0x00, 0x00),
            highlight_text: ThemeColor::argb(0xD8, 0x00, 0x00, 0x00),
            text: ThemeColor::argb(0xD8, 0x00, 0x00, 0x00),
            index: ThemeColor::rgb(0x00, 0x00, 0x00),
            highlight_index: ThemeColor::rgb(0x00, 0x00, 0x00),
            page_number: ThemeColor::rgb(0x88, 0x88, 0x88),
            badge: ThemeColor::rgb(0x88, 0x88, 0x88),
            translate_meta: ThemeColor::rgb(0x33, 0x33, 0x33),
            translate_learned: ThemeColor::rgb(0x33, 0x33, 0x33),
            translate_fresh: ThemeColor::rgb(0xFF, 0x7F, 0x27),
            extra: ThemeColor::rgb(0x0F, 0x6C, 0xBD),
        }
    }
}

/// 悬浮工具栏的 3 个颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarTheme {
    pub background: ThemeColor,
    pub icon: ThemeColor,
    pub shadow: ThemeColor,
}

impl Default for BarTheme {
    fn default() -> Self {
        Self {
            background: ThemeColor::rgb(0xFF, 0xFF, 0xFF),
            icon: ThemeColor::rgb(0x23, 0x1F, 0x20),
            shadow: ThemeColor::argb(0x5A, 0x00, 0x00, 0x00),
        }
    }
}

/// 状态切换提示窗口的 3 个颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TipTheme {
    pub background: ThemeColor,
    pub icon: ThemeColor,
    pub shadow: ThemeColor,
}

impl Default for TipTheme {
    fn default() -> Self {
        Self {
            background: ThemeColor::rgb(0xFF, 0xFF, 0xFF),
            icon: ThemeColor::rgb(0x23, 0x1F, 0x20),
            shadow: ThemeColor::argb(0x5A, 0x00, 0x00, 0x00),
        }
    }
}

/// 一份主题文件（三个窗口的 21 个颜色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeFile {
    pub candidate: CandidateTheme,
    pub bar: BarTheme,
    pub tip: TipTheme,
}

impl ThemeFile {
    /// 读一个主题文件。
    pub fn load(path: &Path) -> Result<Self, ThemeError> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }

    /// 写一个主题文件（建父目录；美化过的 JSON）。
    pub fn save(&self, path: &Path) -> Result<(), ThemeError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// 一个可选的「主题」的一条：名字（不含 `.json`）与文件位置。
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
    fn colors_round_trip_through_hex() {
        assert_eq!(
            ThemeColor::parse("#ff33ccff"),
            Some(ThemeColor::argb(0xFF, 0x33, 0xCC, 0xFF))
        );
        assert_eq!(
            ThemeColor::parse("33ccff"),
            Some(ThemeColor::rgb(0x33, 0xCC, 0xFF))
        );
        assert_eq!(ThemeColor::parse("#12345"), None);
        assert_eq!(ThemeColor::parse("zz"), None);
        assert_eq!(ThemeColor::rgb(0x33, 0xCC, 0xFF).hex(), "#FF33CCFF");
    }

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
