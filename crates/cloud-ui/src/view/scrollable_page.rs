//! scrollable_page（可滚动页面）控件容器：固定视口 + 裁剪 + 垂直滚动条。
//!
//! 容器分两类：**控件容器**（如 `panel`、本控件）与**子页面容器**；`scrollable_page`
//! 属于控件容器，里面放任意子控件。`View` 把它们摆进一块
//! [`cloudime_render::SelfDrawingPanel`]（`clip = true`），渲染器先画到独立画布再合成，
//! 超出视口的部分自然被裁掉。行列表请用 `list_box`，那里有整行 / 均匀间隙规则。
//! 滚动偏移由应用保存（[`ScrollablePageState`]），内容高度 / 可滚范围从
//! [`ScrollablePageMetrics`] 读。

use cloud_ui_core::{Edges, Rect};
use cloudime_render::{Color, Element, SelfDrawingPanel};

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
#[derive(Debug, Clone, Copy, Default)]
pub struct ScrollablePageState {
    /// 垂直滚动偏移（点）；会被内容高度夹住。
    pub offset: f32,
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

/// 在内容之后追加滚动条（轨道 + 滑块），保证它盖在子控件上方。
pub(super) fn push_scrollbar(
    panel: &mut SelfDrawingPanel,
    style: &ScrollablePageStyle,
    metrics: ScrollablePageMetrics,
    offset: f32,
) {
    if !style.scrollbar || metrics.max_scroll() <= 0.5 {
        return;
    }
    let width = style.scrollbar_width.max(1.0);
    let track_x = panel.width - width - 2.0;
    let track_y = 2.0;
    let track_height = (panel.height - 4.0).max(1.0);
    let thumb_height = (track_height * metrics.viewport_height / metrics.content_height.max(1.0))
        .max(16.0)
        .min(track_height);
    let travel = (track_height - thumb_height).max(0.0);
    let ratio = (offset / metrics.max_scroll().max(1.0)).clamp(0.0, 1.0);
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
        y: track_y + travel * ratio,
        width,
        height: thumb_height,
        radius: width * 0.5,
        color: style.thumb,
    });
}
