//! 主题数据类型（`Themes\*.json`）：候选窗口、悬浮工具栏、状态切换提示三个窗口的颜色。
//!
//! 这里只放数据与读写；主题目录扫描在 `cloudime-platform`。

use std::path::Path;

use serde::{Deserialize, Serialize};

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
    /// 用 ARGB 建一个。
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

/// 候选窗口的 15 个颜色。
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
    /// 候选项右侧来源角标（「短语」「造」）。
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
}
