//! scrollable_page（可滚动页面）控件容器：固定视口 + 裁剪 + 垂直滚动条。
//!
//! 容器分两类：**控件容器**（如 `panel`、本控件）与**子页面容器**；`scrollable_page`
//! 属于控件容器，里面放任意子控件。`View` 把它们摆进一块
//! [`cloudime_render::SelfDrawingPanel`]（`clip = true`），渲染器先画到独立画布再合成，
//! 超出视口的部分自然被裁掉。行列表请用 `list_box`，那里有整行 / 均匀间隙规则。
//! 滚动偏移是共享句柄（[`ScrollablePageState`]），拖动滚动条 / 滚轮由窗口壳直接写；
//! 内容高度 / 可滚范围从 [`ScrollablePageMetrics`] 读。

use cloud_ui_core::{Edges, Rect};
use cloudime_render::{Color, Element, ScrollHandle, SelfDrawingPanel};

/// 可滚动页面的外观参数（点）。
#[derive(Debug, Clone, Copy)]
pub struct ScrollablePageStyle {
    /// 视口高度。
    pub height: f32,

    /// 内容四周的留白。
    pub padding: Edges,

    /// 子控件之间的空隙（加在列布局上）。
    pub gap: f32,

    /// 底板颜色；`None` = 透明。
    pub background: Option<Color>,

    /// 底板圆角。
    pub radius: f32,

    /// 是否画右侧滚动条。
    pub scrollbar: bool,

    /// 滚动条宽度。
    pub scrollbar_width: f32,

    /// 滚动条轨道颜色。
    pub track: Color,

    /// 滚动条滑块颜色。
    pub thumb: Color,
}

impl Default for ScrollablePageStyle {
    fn default() -> Self {
        Self {
            height: 200.0,
            padding: Edges::default(),
            gap: 8.0,
            background: None,
            radius: 0.0,
            scrollbar: true,
            scrollbar_width: 6.0,
            track: Color::rgba(120, 120, 130, 60),
            thumb: Color::rgba(160, 160, 175, 160),
        }
    }
}

impl ScrollablePageStyle {
    /// 固定视口高度的快捷构造。
    pub fn with_height(height: f32) -> Self {
        Self {
            height,
            ..Self::default()
        }
    }

    /// 带内边距的快捷构造。
    pub fn padded(height: f32, padding: f32, gap: f32) -> Self {
        Self {
            height,
            padding: Edges::all(padding),
            gap,
            ..Self::default()
        }
    }
}

/// 可滚动页面的每帧状态。
///
/// 滚动偏移是 [`ScrollHandle`]：建一次、跨帧持有，每帧把 `clone()` 传给
/// [`View::scrollable_page`]。窗口壳拖动滚动条 / 滚轮时直接写句柄。
#[derive(Debug, Clone, Default)]
pub struct ScrollablePageState {
    /// 垂直滚动偏移句柄（点）；由 `build` 夹紧。
    pub offset: ScrollHandle,
}

/// 一次布局后的可滚范围。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScrollablePageMetrics {
    /// 视口在文档里的矩形（点）；应用用它判断滚轮 / 点击落在哪个滚动容器上。
    pub rect: Rect,

    /// 内容总高度（点）。
    pub content_height: f32,

    /// 视口高度（点）。
    pub viewport_height: f32,
}

impl ScrollablePageMetrics {
    /// 最多能滚多远。
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.viewport_height).max(0.0)
    }

    /// 把偏移夹在合法范围内。
    pub fn clamp(&self, offset: f32) -> f32 {
        offset.clamp(0.0, self.max_scroll())
    }
}

/// 建一块空的裁剪面板（背景）；子控件由 `View` 往里填。
pub(super) fn panel(style: &ScrollablePageStyle, rect: Rect) -> SelfDrawingPanel {
    let mut draw = Vec::new();
    if let Some(color) = style.background {
        draw.push(Element::Rect {
            x: 0.0,
            y: 0.0,
            width: rect.width,
            height: rect.height,
            radius: style.radius,
            color,
        });
    }
    SelfDrawingPanel {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        clip: true,
        draw,
        children: Vec::new(),
        regions: Vec::new(),
    }
}

/// 滚动条在面板里的两条矩形（左上角 + 宽高，面板坐标）。
#[derive(Debug, Clone, Copy)]
pub(super) struct ScrollbarRects {
    pub track: (f32, f32, f32, f32),
    pub thumb: (f32, f32, f32, f32),
}

/// 按当前偏移算滚动条的轨道 / 滑块；不需要滚动条时给 `None`。
///
/// 绘制（[`push_scrollbar`]）和窗口壳的命中（`Document.scroll_targets`）共用这一份几何，
/// 保证点中的位置和画出来的位置一致。
pub(super) fn scrollbar_rects(
    style: &ScrollablePageStyle,
    width: f32,
    height: f32,
    metrics: ScrollablePageMetrics,
    offset: f32,
) -> Option<ScrollbarRects> {
    if !style.scrollbar || metrics.max_scroll() <= 0.5 {
        return None;
    }
    let bar_width = style.scrollbar_width.max(1.0);
    let track_x = width - bar_width - 2.0;
    let track_y = 2.0;
    let track_height = (height - 4.0).max(1.0);
    let thumb_height = (track_height * metrics.viewport_height / metrics.content_height.max(1.0))
        .max(16.0)
        .min(track_height);
    let travel = (track_height - thumb_height).max(0.0);
    let ratio = (offset / metrics.max_scroll().max(1.0)).clamp(0.0, 1.0);
    Some(ScrollbarRects {
        track: (track_x, track_y, bar_width, track_height),
        thumb: (track_x, track_y + travel * ratio, bar_width, thumb_height),
    })
}

/// 在内容之后追加滚动条（轨道 + 滑块），保证它盖在子控件上方。
pub(super) fn push_scrollbar(
    panel: &mut SelfDrawingPanel,
    style: &ScrollablePageStyle,
    metrics: ScrollablePageMetrics,
    offset: f32,
) {
    let Some(rects) = scrollbar_rects(style, panel.width, panel.height, metrics, offset) else {
        return;
    };
    let (track_x, track_y, width, track_height) = rects.track;
    let (_, thumb_y, _, thumb_height) = rects.thumb;
    panel.children.push(Element::Rect {
        x: track_x,
        y: track_y,
        width,
        height: track_height,
        radius: width * 0.5,
        color: style.track,
    });
    panel.children.push(Element::Rect {
        x: track_x,
        y: thumb_y,
        width,
        height: thumb_height,
        radius: width * 0.5,
        color: style.thumb,
    });
}
