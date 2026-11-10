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

/// 候选框最小宽度的缺省值（物理像素，竖排与横排都生效）。
pub const DEFAULT_CANDIDATE_BOX_MINIMUM_WIDTH: u32 = 180;

/// 「展开后每个候选项的最大宽度」的缺省值（物理像素）：展开成网格时每格不超过它，
/// 太长的候选项截断并显示 `…`。`0` 表示不限。
pub const DEFAULT_CANDIDATE_ITEM_MAXIMUM_WIDTH: u32 = 420;

/// 知意模型 ONNX Runtime 的缺省线程数：2 线程偏省电，4 线程在这类 CPU 上快 20–25%。
pub const DEFAULT_ZHIYI_THREADS: usize = 4;

/// 知意模型线程数下限。
pub const MIN_ZHIYI_THREADS: usize = 1;

/// 知意模型线程数上限（再多的收益递减，还会和打字抢 CPU）。
pub const MAX_ZHIYI_THREADS: usize = 32;

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

/// u 模式符号窗口缺字时补用的字体。
pub const DEFAULT_SYMBOL_FONT: &str = "Segoe UI Symbol";

/// u 模式日文假名缺字时补用的字体。
pub const DEFAULT_JAPANESE_FONT: &str = "Yu Gothic UI";

fn default_symbol_font() -> String {
    DEFAULT_SYMBOL_FONT.to_owned()
}

fn default_japanese_font() -> String {
    DEFAULT_JAPANESE_FONT.to_owned()
}

/// 候选词序号的样式；类型在 `cloud-ui-types`，这里 re-export 给配置与设置页。
pub use cloud_ui_types::ItemNumberStyle;

/// 「使用鼠标选词」：鼠标能不能点候选窗里的候选来选词 / 上屏。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseWordSelection {
    /// 关闭（缺省）：鼠标不选词。
    #[default]
    Off,

    /// 仅「展示更多候选项」展开成网格时可用。
    MoreCandidates,

    /// 全部开启：常规候选窗与展开后的网格都能用鼠标选。
    Always,
}

impl MouseWordSelection {
    /// 全部取值，设置界面按这个顺序列出。
    pub const ALL: [Self; 3] = [Self::Off, Self::MoreCandidates, Self::Always];

    /// 配置文件里的写法。
    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::MoreCandidates => "more_candidates",
            Self::Always => "always",
        }
    }

    /// 界面上的名字。
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "关闭",
            Self::MoreCandidates => "仅更多候选项时",
            Self::Always => "全部开启",
        }
    }
}

/// 本地整句 / 智能补全模型：关闭 / 基本（含章）/ 增强（知意）/ 混合（含章+知意）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalModelMode {
    /// 全部不使用，只按词库与短语匹配。
    Off,

    /// 含章·知微本地整句模型。
    #[default]
    Basic,

    /// 知意：候选页重排 + 按音节出字。
    Enhanced,

    /// 含章 + 知意同时加载：短输入只走知意，长输入两个模型一起用。
    Hybrid,
}

impl LocalModelMode {
    /// 设置界面的顺序。
    pub const ALL: [Self; 4] = [Self::Off, Self::Basic, Self::Enhanced, Self::Hybrid];

    /// 配置文件里的写法。
    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Basic => "basic",
            Self::Enhanced => "enhanced",
            Self::Hybrid => "hybrid",
        }
    }

    /// 界面 / 日志里的名字。
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "关闭",
            Self::Basic => "基本（含章）",
            Self::Enhanced => "增强（知意）",
            Self::Hybrid => "混合（含章+知意）",
        }
    }
}

/// `[candidate]` 分节。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CandidateConfig {
    /// 本地模型模式：关闭 / 基本（含章）/ 增强（知意）/ 混合（含章+知意）。
    pub local_model: LocalModelMode,

    /// 知意模型 ONNX Runtime 的线程数（1–32）；缺省 4。
    #[serde(default = "default_zhiyi_threads")]
    pub zhiyi_threads: usize,

    /// 旧配置里的布尔开关（`use_local_sentence_organization_model`）：只读兼容，不再写回。
    /// `local_model` 不是「基本」时以它为准；是「基本」时旧值 `false` 仍按「关闭」读。
    #[serde(
        default,
        rename = "use_local_sentence_organization_model",
        skip_serializing
    )]
    legacy_local_model_enabled: Option<bool>,

    /// 应用读不到光标前文时，这个应用里多少秒内上屏的字还给模型当前文；0 表示一直算。
    pub context_memory_seconds: u64,

    /// 候选项排布方向。
    pub candidate_arrangement_direction: LayoutMode,

    /// 竖排时，候选窗在光标上方就自动反向排列（拼音行贴近光标在最下）。
    pub auto_reverse_candidate_order: bool,

    /// 横排时压紧成两行：翻译 Tip 放到拼音行右侧、页码放到候选行右侧。
    pub compact_candidate_window: bool,

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

    /// 符号补充字体：u 模式符号窗口里主字体缺字时使用。
    #[serde(default = "default_symbol_font")]
    pub symbol_font: String,

    /// 日文补充字体：u 模式日文假名缺字时使用。
    #[serde(default = "default_japanese_font")]
    pub japanese_font: String,

    /// 候选项序号样式。
    pub item_number_style: ItemNumberStyle,

    /// 组句中的拼音显示在行内、候选窗口还是两处都显示。
    pub preedit: crate::config::PreeditMode,

    /// 候选框最小宽度（物理像素，竖排与横排都生效）。
    pub candidate_box_minimum_width: u32,

    /// 展示更多候选项（按 Tab）：组句里 Tab 把候选窗展开成一整屏（竖排 5 列 / 横排 5 行，
    /// 另一个方向就是 [`candidate_count`](Self::candidate_count)）；关掉时 Tab 吃掉但不展开。
    pub show_more_candidate_items: bool,

    /// 使用鼠标选词：鼠标能不能点候选窗里的候选来选词 / 上屏。
    /// `off` 关闭（缺省）/ `more_candidates` 仅「展示更多候选项」展开时 / `always` 全部开启。
    pub mouse_word_selection: MouseWordSelection,

    /// 展开（「展示更多候选项」）后每个候选项的最大宽度（物理像素，`0` 表示不限）：
    /// 展开成网格时每格不超过它，太长的候选项截断并显示 `…`。
    pub candidate_item_maximum_width: u32,

    /// 在下列程序中不显示候选框（完全不接管输入，按键原样交给应用）。
    pub program_list_of_hiding_candidate: Vec<String>,

    /// 在下列程序中让出候选绘制权：宿主读走候选串（`GetString`）后收我们自己的候选窗。
    /// 只给 UI-less / IMM32 桥接这类真的会自己画候选的宿主用；普通宿主不要加（读候选串是常态）。
    pub host_drawn_candidates: Vec<String>,
}

fn default_zhiyi_threads() -> usize {
    DEFAULT_ZHIYI_THREADS
}

impl Default for CandidateConfig {
    fn default() -> Self {
        Self {
            local_model: LocalModelMode::default(),
            zhiyi_threads: DEFAULT_ZHIYI_THREADS,
            legacy_local_model_enabled: None,
            context_memory_seconds: cloudime_core::DEFAULT_FRESH_SECONDS,
            candidate_arrangement_direction: LayoutMode::default(),
            auto_reverse_candidate_order: false,
            compact_candidate_window: false,
            candidate_count: MAX_CANDIDATE_COUNT,
            candidate_association_counts: 2,
            pinyin_font: FontChoice::new(DEFAULT_FAMILY, 11.0),
            candidate_font: FontChoice::new(DEFAULT_FAMILY, 13.0),
            item_number_font: FontChoice::new(DEFAULT_FAMILY, 11.0),
            translate_font: FontChoice::new(DEFAULT_FAMILY, 11.0),
            symbol_font: DEFAULT_SYMBOL_FONT.to_owned(),
            japanese_font: DEFAULT_JAPANESE_FONT.to_owned(),
            item_number_style: ItemNumberStyle::default(),
            preedit: crate::config::PreeditMode::default(),
            candidate_box_minimum_width: DEFAULT_CANDIDATE_BOX_MINIMUM_WIDTH,
            show_more_candidate_items: false,
            mouse_word_selection: MouseWordSelection::default(),
            candidate_item_maximum_width: DEFAULT_CANDIDATE_ITEM_MAXIMUM_WIDTH,
            program_list_of_hiding_candidate: Vec::new(),
            host_drawn_candidates: Vec::new(),
        }
    }
}

impl CandidateConfig {
    /// 实际生效的本地模型模式：新的 `local_model` 不是「基本」时以它为准，
    /// 否则兼容旧配置的布尔值（`false` = 关闭）。
    pub fn resolved_local_model(&self) -> LocalModelMode {
        if self.local_model != LocalModelMode::Basic {
            return self.local_model;
        }
        match self.legacy_local_model_enabled {
            Some(true) | None => LocalModelMode::Basic,
            Some(false) => LocalModelMode::Off,
        }
    }

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

    /// 夹到合法范围的知意线程数。
    pub fn zhiyi_threads(&self) -> usize {
        self.zhiyi_threads
            .clamp(MIN_ZHIYI_THREADS, MAX_ZHIYI_THREADS)
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

    /// 这个程序（exe 文件名，不区分大小写）在不在 `host_drawn_candidates` 名单里。
    pub fn host_draws_candidates_for(&self, program: &str) -> bool {
        let program = program.trim();
        !program.is_empty()
            && self
                .host_drawn_candidates
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
    fn local_model_modes_have_the_settings_labels() {
        assert_eq!(LocalModelMode::ALL.len(), 4);
        assert_eq!(LocalModelMode::default(), LocalModelMode::Basic);
        assert_eq!(LocalModelMode::Basic.label(), "基本（含章）");
        assert_eq!(LocalModelMode::Hybrid.key(), "hybrid");
        assert_eq!(LocalModelMode::Hybrid.label(), "混合（含章+知意）");
    }

    #[test]
    fn compact_and_reverse_default_to_off() {
        let config = CandidateConfig::default();
        assert!(!config.auto_reverse_candidate_order);
        assert!(!config.compact_candidate_window);
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
    fn mouse_word_selection_defaults_to_off() {
        assert_eq!(
            CandidateConfig::default().mouse_word_selection,
            MouseWordSelection::Off
        );
        assert_eq!(MouseWordSelection::ALL.len(), 3);
        assert_eq!(MouseWordSelection::MoreCandidates.key(), "more_candidates");
        assert_eq!(MouseWordSelection::Always.label(), "全部开启");
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

    #[test]
    fn host_drawn_list_matches_program_names_case_insensitively() {
        let config = CandidateConfig {
            host_drawn_candidates: vec!["MapleStory.exe".to_owned()],
            ..CandidateConfig::default()
        };
        assert!(config.host_draws_candidates_for("maplestory.exe"));
        assert!(config.host_draws_candidates_for(" MapleStory.EXE "));
        assert!(!config.host_draws_candidates_for("notepad.exe"));
        assert!(!config.host_draws_candidates_for(""));
    }

    #[test]
    fn zhiyi_threads_default_to_four_and_clamp() {
        assert_eq!(CandidateConfig::default().zhiyi_threads(), 4);
        let config = CandidateConfig {
            zhiyi_threads: 0,
            ..CandidateConfig::default()
        };
        assert_eq!(config.zhiyi_threads(), MIN_ZHIYI_THREADS);
        let config = CandidateConfig {
            zhiyi_threads: 99,
            ..CandidateConfig::default()
        };
        assert_eq!(config.zhiyi_threads(), MAX_ZHIYI_THREADS);
    }
}
