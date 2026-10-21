//! 候选窗口的输出端。

use cloudime_platform::input_schemes::{KeyMapLayout, KeyMapReference};
use cloudime_platform::protocol::{Frame, ScreenRect};

pub use cloud_ui::RenderSettings;

/// Router 只产出帧，画交给它；Windows 上由 UI 线程实现。
pub trait CandidateSink: Send {
    /// 把候选窗口摆到 `rect`（组句范围的屏幕矩形）下方并按 `frame` 重绘。
    /// `badges` 与 `frame.candidates.items` 一一对应，是每个候选右侧的来源角标（没有为 `None`）。
    fn show(&self, frame: Frame, badges: Vec<Option<char>>, rect: ScreenRect);

    fn hide(&self);

    /// 换字体（重建渲染器）：装上时与配置热加载后调，只在设置变了时调。
    fn configure(&self, settings: RenderSettings);

    /// 候选窗最近一次画出来的**内容区尺寸**（点）；没画过 / 已经收起就是 `None`。
    /// 脚本的 `cloudime.candidate.width()` 读它 —— 折行时按窗口现在的宽度来，不用猜。
    /// 不画的实现（[`NoopSink`]）用不上，默认 `None`。
    fn viewport(&self) -> Option<(f32, f32)> {
        None
    }

    /// 开关「辅助窗口」（`auxiliary_window`，眼下是键位映射参考）：挂在配置的组合键上。
    /// 不画的实现（[`NoopSink`]）默认什么都不做。
    fn auxiliary_toggle(&self) {}

    /// 换「辅助窗口」的显示方式（`use_input_scheme.cfg` 的 `key_map_reference`）：
    /// 「始终跟随候选窗口」那种方式下，候选窗一显示就把资产带上。
    /// 不画的实现（[`NoopSink`]）默认什么都不做。
    fn set_auxiliary_mode(&self, _mode: KeyMapReference) {}

    /// 换「辅助窗口」显示的内容（键位映射参考的键盘版面，按当前方案准备好）；方案变了重送一次。
    /// 不画的实现（[`NoopSink`]）默认什么都不做。
    fn set_auxiliary(&self, _layout: KeyMapLayout) {}

    /// u 模式符号窗口：`Some` 显示并换内容，`None` 收起。与配置的键位映射参考互不影响。
    /// 不画的实现（[`NoopSink`]）默认什么都不做。
    fn set_symbol_aux(&self, _layout: Option<KeyMapLayout>) {}
}

/// 不画候选窗口的空实现。
pub struct NoopSink;

impl CandidateSink for NoopSink {
    fn show(&self, _frame: Frame, _badges: Vec<Option<char>>, _rect: ScreenRect) {}

    fn hide(&self) {}

    fn configure(&self, _settings: RenderSettings) {}
}
