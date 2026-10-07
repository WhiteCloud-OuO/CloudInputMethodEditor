//! 候选窗口的输出端。

use cloudime_platform::ItemNumberStyle;
use cloudime_platform::protocol::{Frame, ScreenRect};

/// 候选窗口 / 状态条的渲染设置。
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

    /// 竖排时窗口的最小宽度（物理像素）。
    pub min_width_pixels: f32,

    /// 序号的写法。
    pub item_number_style: ItemNumberStyle,
}

/// Router 只产出帧，画交给它；Windows 上由 UI 线程实现。
pub trait CandidateSink: Send {
    /// 把候选窗口摆到 `rect`（组句范围的屏幕矩形）下方并按 `frame` 重绘。
    /// `badges` 与 `frame.candidates.items` 一一对应，是每个候选右侧的来源角标（没有为 `None`）。
    fn show(&self, frame: Frame, badges: Vec<Option<char>>, rect: ScreenRect);

    fn hide(&self);

    /// 换字体（重建渲染器）：装上时与配置热加载后调，只在设置变了时调。
    fn configure(&self, settings: RenderSettings);
}

/// 不画候选窗口的空实现。
pub struct NoopSink;

impl CandidateSink for NoopSink {
    fn show(&self, _frame: Frame, _badges: Vec<Option<char>>, _rect: ScreenRect) {}

    fn hide(&self) {}

    fn configure(&self, _settings: RenderSettings) {}
}
