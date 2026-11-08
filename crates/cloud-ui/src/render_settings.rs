//! 候选窗口 / 状态条 / 脚本窗口共用的渲染设置。

use cloud_ui_types::{ItemNumberStyle, ThemeFile};

/// 窗口的渲染设置。
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSettings {
    /// 拼音串字体：字族名（空为系统界面字体）+ 字号（点）。
    pub pinyin_font: (String, f32),

    /// 候选词字体。
    pub candidate_font: (String, f32),

    /// 序号字体。
    pub item_number_font: (String, f32),

    /// 翻译 Tip 的字体（候选窗底部那一行左侧）。
    pub translate_font: (String, f32),

    /// u 模式符号窗口的补充字体（主字体缺字时用）。
    pub symbol_font: String,

    /// u 模式日文假名的补充字体（主字体缺字时用）。
    pub japanese_font: String,

    /// 候选窗口的最小宽度（物理像素）：竖排、横排都生效。
    pub min_width_pixels: f32,

    /// 序号的写法。
    pub item_number_style: ItemNumberStyle,

    /// 脚本要的缩放倍数（`cloudime.candidate.set_scale`）：本组句内有效，
    /// `None` = 按用户 `Ctrl + 滚轮` 的级数（用户滚一下就由滚轮接管）。
    pub scale: Option<f32>,

    /// 展开（「展示更多候选项」）成网格时每格的最大宽度（点；`0` 表示不限）：
    /// 超过就截尾加「…」，格子不再跟着候选词变长。
    pub max_cell_width: f32,

    /// 在候选项显示辅助码（`use_input_scheme.cfg` 的 `candidate_auxiliary`，缺省关）：
    /// 候选词右边隔一个空格画上那个候选的辅助码（Core 已经把码填在候选里，见 `Candidate::auxiliary`）。
    pub candidate_auxiliary: bool,

    /// 竖排时，候选窗在光标上方就自动反向排列（拼音行贴近光标在最下）。
    pub auto_reverse_candidate_order: bool,

    /// 横排时压紧成两行：翻译 Tip 放到拼音行右侧、页码放到候选行右侧。
    pub compact_candidate_window: bool,

    /// 当前主题（`Themes\` 里那份的三个窗口配色）；读不到就是缺省主题。
    pub theme: ThemeFile,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            pinyin_font: (String::new(), 11.0),
            candidate_font: (String::new(), 13.0),
            item_number_font: (String::new(), 11.0),
            translate_font: (String::new(), 11.0),
            symbol_font: String::new(),
            japanese_font: String::new(),
            min_width_pixels: 0.0,
            item_number_style: ItemNumberStyle::default(),
            scale: None,
            max_cell_width: 0.0,
            candidate_auxiliary: false,
            auto_reverse_candidate_order: false,
            compact_candidate_window: false,
            theme: Default::default(),
        }
    }
}
