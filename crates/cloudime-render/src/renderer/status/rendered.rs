//! 状态条的渲染结果：位图加各按钮的右边界。

use crate::renderer::Rendered;

pub struct RenderedStatus {
    /// 位图与内容区位置。
    pub rendered: Rendered,

    /// 各按钮右边界在内容区里的像素 x，从左到右；点击按 x 落进哪个按钮。
    pub cell_edges: Vec<f32>,
}
