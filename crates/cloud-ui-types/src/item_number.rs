//! 候选词序号样式。独立 UI 与平台配置共用。

use serde::{Deserialize, Serialize};

/// 候选词序号的样式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemNumberStyle {
    /// `1.` 到 `9.`（缺省）。
    #[default]
    Decimal,

    /// `①` 到 `⑨`。
    Circled,

    /// `Ⅰ` 到 `Ⅸ`。
    Roman,

    /// `❶` 到 `❾`。
    Dingbat,

    /// `⑴` 到 `⑼`。
    Parenthesized,
}

impl ItemNumberStyle {
    /// 全部取值，设置界面按这个顺序列出。
    pub const ALL: [Self; 5] = [
        Self::Decimal,
        Self::Circled,
        Self::Roman,
        Self::Dingbat,
        Self::Parenthesized,
    ];

    /// 配置文件里的写法。
    pub fn key(self) -> &'static str {
        match self {
            Self::Decimal => "decimal",
            Self::Circled => "circled",
            Self::Roman => "roman",
            Self::Dingbat => "dingbat",
            Self::Parenthesized => "parenthesized",
        }
    }

    /// 界面上的名字（CSV 里那五行写法）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Decimal => "1.~9.",
            Self::Circled => "①~⑨",
            Self::Roman => "Ⅰ~Ⅸ",
            Self::Dingbat => "❶~❾",
            Self::Parenthesized => "⑴~⑼",
        }
    }

    /// 第 `digit`（1–9）个序号的写法。
    pub fn format(self, digit: usize) -> String {
        // 1–9 的几种序号都是连着的码点，按 digit 偏移取
        let offset = digit.saturating_sub(1) as u32;
        let codepoint = match self {
            Self::Decimal => return format!("{digit}."),
            Self::Circled => 0x2460,
            Self::Roman => 0x2160,
            Self::Dingbat => 0x2776,
            Self::Parenthesized => 0x2474,
        };
        char::from_u32(codepoint + offset)
            .map(|c| c.to_string())
            .unwrap_or_else(|| format!("{digit}."))
    }
}
