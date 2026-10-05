//! 高亮条在两行之间滑动时的动画参数。

use super::rect::HighlightRect;

/// 高亮条从 `from` 矩形滑向 [`Frame::highlighted`](crate::frame::Frame::highlighted) 所在行矩形的动画。
/// 渲染器据此在两矩形之间逐边插值；没有它就直接画在目标行。
///
/// `from` 是内容区坐标（左上为原点、不含阴影边）的显式矩形：壳把「当前视觉位置」直接传下来，
/// 连按方向键时能从这里续滑，而不是每次从上一帧的逻辑高亮行重新出发。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightAnimation {
    /// 起点矩形（内容区坐标，像素）。
    pub from: HighlightRect,

    /// 缓动后的进度，`0` 是起点、`1` 是终点；壳已钳到 `0..=1`。
    pub progress: f32,
}
