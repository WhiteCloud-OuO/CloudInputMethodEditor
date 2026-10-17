//! 渲染结果：位图加内容区（阴影边之外的那块）的位置。

use tiny_skia::Pixmap;

use crate::document::HitShape;
use crate::frame::HighlightRect;

pub struct Rendered {
    /// 预乘 RGBA 位图，含阴影边。
    pub pixmap: Pixmap,

    /// 内容区左上角在位图里的像素坐标。
    pub content_x: u32,

    pub content_y: u32,

    /// 内容区像素宽高（窗口该有的大小）。
    pub content_width: u32,

    pub content_height: u32,

    /// 渲染用的倍数，壳把像素换回点用。
    pub scale: f32,

    /// 每行 / 每格的高亮条矩形（内容区坐标），下标与帧的 `rows` 对齐。
    /// 壳按它算连按时滑动的起点；没有高亮的帧为空。
    pub highlight_rects: Vec<HighlightRect>,
}

impl Rendered {
    /// 内容区宽高换回点。
    pub fn content_size_points(&self) -> (f32, f32) {
        (
            self.content_width as f32 / self.scale,
            self.content_height as f32 / self.scale,
        )
    }
}

/// 脚本内容出图的结果：位图 + 每个元素在内容区里的命中区域（点，与 `Document::elements` 同序）。
pub struct RenderedDocument {
    /// 位图（含阴影边）与内容区位置。
    pub rendered: Rendered,

    /// 每个元素的可命中区域（按形状：圆角 / 圆 / 线段 / 路径填充都抠出来）；
    /// 画不出来 / 没有面积的元素是 `None`。
    pub hits: Vec<Option<HitShape>>,
}
