//! `[candidate]` 分节：候选窗口（排布、个数、联想上限、三个字体、序号样式、最小宽度、按程序隐藏）。

use serde::{Deserialize, Serialize};

use super::LayoutMode;

/// 候选个数的下限：CSV 规定滑轨是 5~9（数字键选词只有 1~9）。
pub const MIN_CANDIDATE_COUNT: usize = 5;

/// 候选个数的上限。
pub const MAX_CANDIDATE_COUNT: usize = 9;

/// 联想候选项目上限的下限：0 表示不显示联想候选（只留覆盖整段输入的词）。
pub const MIN_ASSOCIATION_COUNTS: usize = 0;

/// 联想候选项目上限的上限。
pub const MAX_ASSOCIATION_COUNTS: usize = 4;

/// 候选框最小宽度的缺省值（物理像素，只在竖排时起作用）。
pub const DEFAULT_CANDIDATE_BOX_MINIMUM_WIDTH: u32 = 180;

/// 一种字体的用法：字族名 + 字号（点）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontChoice {
    /// 字族名（如 `微软雅黑`）；空为系统界面字体。
    pub family: String,

    /// 字号（点）。
    pub size: f32,
}

impl FontChoice {
    /// 按缺省值建一个。
    pub fn new(family: &str, size: f32) -> Self {
        Self {
            family: family.to_owned(),
            size,
        }
    }

    /// 界面上显示的样子：`微软雅黑 11pt`。
    pub fn label(&self) -> String {
        format!("{} {:.0}pt", self.family, self.size)
    }
}

impl Default for FontChoice {
    fn default() -> Self {
        Self::new(DEFAULT_FAMILY, 14.0)
    }
}

/// 三个字体的缺省字族（CSV 给的默认值）。
pub const DEFAULT_FAMILY: &str = "微软雅黑";

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

/// `[candidate]` 分节。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CandidateConfig {
    /// 使用本地整句模型（输入法内置，`.qjm`）：消耗一点处理器与内存换更准的整句输入；关掉只用词库与短语匹配。
    pub use_local_sentence_organization_model: bool,

    /// 候选项排布方向。
    pub candidate_arrangement_direction: LayoutMode,

    /// 候选项个数（5–9）。
    pub candidate_count: usize,

    /// 联想候选项目上限（0–4）：候选列表里「比读法更长的词」（联想）最多留几条，
    /// 免得单字输入时被大量联想候选淹没、看不到要选的那个字 / 词。
    pub candidate_association_counts: usize,

    /// 拼音串字体。
    pub pinyin_font: FontChoice,

    /// 候选项字体。
    pub candidate_font: FontChoice,

    /// 候选项序号字体。
    pub item_number_font: FontChoice,

    /// 翻译字体：候选窗底部那一行左侧的翻译 Tip。
    pub translate_font: FontChoice,

    /// 候选项序号样式。
    pub item_number_style: ItemNumberStyle,

    /// 组句中的拼音显示在行内、候选窗口还是两处都显示。
    pub preedit: crate::config::PreeditMode,

    /// 候选框最小宽度（物理像素，只在竖排时有效）。
    pub candidate_box_minimum_width: u32,

    /// 展示更多候选项（按 Tab）：组句里 Tab 把候选窗展开成一整屏（竖排 5 列 / 横排 5 行，
    /// 另一个方向就是 [`candidate_count`](Self::candidate_count)）；关掉时 Tab 吃掉但不展开。
    pub show_more_candidate_items: bool,

    /// 在下列程序中不显示候选框（完全不接管输入，按键原样交给应用）。
    pub program_list_of_hiding_candidate: Vec<String>,
}

impl Default for CandidateConfig {
    fn default() -> Self {
        Self {
            use_local_sentence_organization_model: true,
            candidate_arrangement_direction: LayoutMode::default(),
            candidate_count: MAX_CANDIDATE_COUNT,
            candidate_association_counts: 2,
            pinyin_font: FontChoice::new(DEFAULT_FAMILY, 11.0),
            candidate_font: FontChoice::new(DEFAULT_FAMILY, 13.0),
            item_number_font: FontChoice::new(DEFAULT_FAMILY, 11.0),
            translate_font: FontChoice::new(DEFAULT_FAMILY, 11.0),
            item_number_style: ItemNumberStyle::default(),
            preedit: crate::config::PreeditMode::default(),
            candidate_box_minimum_width: DEFAULT_CANDIDATE_BOX_MINIMUM_WIDTH,
            show_more_candidate_items: false,
            program_list_of_hiding_candidate: Vec::new(),
        }
    }
}

impl CandidateConfig {
    /// 夹到合法范围的候选项个数。
    pub fn candidate_count(&self) -> usize {
        self.candidate_count
            .clamp(MIN_CANDIDATE_COUNT, MAX_CANDIDATE_COUNT)
    }

    /// 夹到合法范围的联想候选项目上限。
    pub fn association_counts(&self) -> usize {
        self.candidate_association_counts
            .clamp(MIN_ASSOCIATION_COUNTS, MAX_ASSOCIATION_COUNTS)
    }

    /// 这个程序（exe 文件名，不区分大小写）在不在「不显示候选框」名单里。
    pub fn hides_candidate_for(&self, program: &str) -> bool {
        let program = program.trim();
        !program.is_empty()
            && self
                .program_list_of_hiding_candidate
                .iter()
                .any(|name| name.trim().eq_ignore_ascii_case(program))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_number_styles_format_digits() {
        assert_eq!(ItemNumberStyle::Decimal.format(3), "3.");
        assert_eq!(ItemNumberStyle::Circled.format(1), "①");
        assert_eq!(ItemNumberStyle::Circled.format(9), "⑨");
        assert_eq!(ItemNumberStyle::Roman.format(4), "Ⅳ");
        assert_eq!(ItemNumberStyle::Dingbat.format(7), "❼");
        assert_eq!(ItemNumberStyle::Parenthesized.format(9), "⑼");
    }

    #[test]
    fn candidate_count_is_clamped_to_five_through_nine() {
        let config = CandidateConfig {
            candidate_count: 1,
            ..CandidateConfig::default()
        };
        assert_eq!(config.candidate_count(), MIN_CANDIDATE_COUNT);
        let config = CandidateConfig {
            candidate_count: 42,
            ..CandidateConfig::default()
        };
        assert_eq!(config.candidate_count(), MAX_CANDIDATE_COUNT);
    }

    #[test]
    fn association_counts_are_clamped_to_zero_through_four() {
        let config = CandidateConfig {
            candidate_association_counts: 42,
            ..CandidateConfig::default()
        };
        assert_eq!(config.association_counts(), MAX_ASSOCIATION_COUNTS);
        let config = CandidateConfig {
            candidate_association_counts: MIN_ASSOCIATION_COUNTS,
            ..CandidateConfig::default()
        };
        assert_eq!(config.association_counts(), MIN_ASSOCIATION_COUNTS);
    }

    #[test]
    fn hiding_list_matches_program_names_case_insensitively() {
        let config = CandidateConfig {
            program_list_of_hiding_candidate: vec!["Code.exe".to_owned()],
            ..CandidateConfig::default()
        };
        assert!(config.hides_candidate_for("code.exe"));
        assert!(config.hides_candidate_for(" Code.EXE "));
        assert!(!config.hides_candidate_for("notepad.exe"));
        assert!(!config.hides_candidate_for(""));
    }
}
