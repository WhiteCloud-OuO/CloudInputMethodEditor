use cloudime_platform::{Config, ItemNumberStyle, LayoutMode, PreeditMode, SimpTrad};

use super::RenderSettings;

/// Router 要用的配置项。
#[derive(Debug, Clone, PartialEq)]
pub struct RouterConfig {
    /// 候选项个数（`[candidate] candidate_count`，5–9）。
    pub page_size: usize,

    /// 候选排布（`[candidate] candidate_arrangement_direction`）。
    pub layout: LayoutMode,

    /// 拼音串字体：字族名（空为系统界面字体）+ 字号（点）。
    pub pinyin_font: (String, f32),

    /// 候选词字体。
    pub candidate_font: (String, f32),

    /// 序号字体。
    pub item_number_font: (String, f32),

    /// 序号的写法（`[candidate] item_number_style`）。
    pub item_number_style: ItemNumberStyle,

    /// 竖排时窗口的最小宽度（`[candidate] candidate_box_minimum_width`，物理像素）。
    pub min_width_pixels: f32,

    /// 拼音显示位置（`[candidate] preedit`）。
    pub preedit: PreeditMode,

    /// 在「不显示候选框」名单里的程序（`[candidate] program_list_of_hiding_candidate`，exe 文件名）。
    pub hiding_candidates: Vec<String>,

    /// 中文模式下不在组句时的标点转全角；不进配置文件，状态条 / 右键菜单可切，会话内有效、重启回缺省（开）。
    /// 初值来自 `[input] full_half_punctuation_marks_toggle`（follow 中文全角、英文半角）。
    pub full_width_punctuation: bool,

    /// 英文模式的那一份，中英各记一份；会话内有效、重启回缺省（半角）。
    pub english_full_width_punctuation: bool,

    /// 直通给应用的可打印 ASCII 转全角（状态条上的「全角 / 半角」开关）；同标点，会话内有效、重启回缺省（半角）。
    pub full_width_chars: bool,

    /// 繁体输出（`[input] simp_trad_chinese_chars_toggle`）；状态条「简 / 繁」按钮与设置页共用这一份。
    pub traditional: bool,

    /// 中文模式的符号映射（`[input] punctuation_marks_mapping`）。
    pub punctuation_mapping: cloudime_core::Mapping,

    /// 符号成对补全的位图（`[input] punctuation_marks_pairwise_completion`）。
    pub pairwise_completion: u32,

    /// 状态条记住的位置（`[status_bar] x` / `y`，内容左上角物理像素）。
    pub status_pos: Option<(i32, i32)>,

    /// 在屏幕上显示悬浮工具栏（`[status_bar] show_status_bar`）：关掉后桌面上不再出现那条工具条。
    pub show_status_bar: bool,

    /// 自动隐藏悬浮工具栏（`[debugging] auto_hide_float_tool_bar`）：前台全屏时收起。
    /// 切到别的输入法、云朵被禁用时始终收起，与这一项无关。
    pub auto_hide_float_tool_bar: bool,
}

impl RouterConfig {
    /// 交给 UI 线程的渲染设置。
    pub fn render_settings(&self) -> RenderSettings {
        RenderSettings {
            pinyin_font: self.pinyin_font.clone(),
            candidate_font: self.candidate_font.clone(),
            item_number_font: self.item_number_font.clone(),
            min_width_pixels: self.min_width_pixels,
            item_number_style: self.item_number_style,
        }
    }

    /// 这个程序在不在「不显示候选框」名单里。
    pub fn hides_candidate_for(&self, program: &str) -> bool {
        let program = program.trim();
        !program.is_empty()
            && self
                .hiding_candidates
                .iter()
                .any(|name| name.trim().eq_ignore_ascii_case(program))
    }
}

impl From<&Config> for RouterConfig {
    fn from(config: &Config) -> Self {
        let input = &config.input;
        let candidate = &config.candidate;
        let punctuation = input.full_half_punctuation_marks_toggle;
        Self {
            page_size: candidate.candidate_count(),
            layout: candidate.candidate_arrangement_direction,
            pinyin_font: (
                candidate.pinyin_font.family.clone(),
                candidate.pinyin_font.size,
            ),
            candidate_font: (
                candidate.candidate_font.family.clone(),
                candidate.candidate_font.size,
            ),
            item_number_font: (
                candidate.item_number_font.family.clone(),
                candidate.item_number_font.size,
            ),
            item_number_style: candidate.item_number_style,
            min_width_pixels: candidate.candidate_box_minimum_width as f32,
            preedit: candidate.preedit,
            hiding_candidates: candidate.program_list_of_hiding_candidate.clone(),
            // 「标点全 / 半角」的初值：follow 中文全角、英文半角，full / half 一律；之后还能用状态条那一格会话内切
            full_width_punctuation: punctuation.full_width(false),
            english_full_width_punctuation: punctuation.full_width(true),
            full_width_chars: false,
            traditional: input.simp_trad_chinese_chars_toggle == SimpTrad::Traditional,
            punctuation_mapping: input.punctuation_mapping(),
            pairwise_completion: input.punctuation_marks_pairwise_completion,
            status_pos: config.status_bar.x.zip(config.status_bar.y),
            show_status_bar: config.status_bar.show_status_bar,
            auto_hide_float_tool_bar: config.debugging.auto_hide_float_tool_bar,
        }
    }
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self::from(&Config::default())
    }
}
