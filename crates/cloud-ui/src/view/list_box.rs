//! 单列列表盒（子页面容器）：`View` 里的行模型与渲染参数。
//!
//! 行内容由这里转成 [`cloudime_render::Element`]，选中 / 悬停各带一份样式；
//! 行滚动、命中与滚动条仍由渲染器的 `Element::ListBox` 负责。
//! 可点击的行把 `action` 名字写进 [`cloudime_render::Document::regions`]，
//! 应用收到的是 `AppEvent::RegionClicked(action)`。

use cloudime_render::{
    Color, Element, FontSlot, HitRect, HitShape, ListBox, ListBoxAlign, ListBoxRow, Region,
};

use super::View;

/// 列表盒里的一行。
#[derive(Debug, Clone)]
pub struct ListItem {
    /// 主文字。
    pub title: String,

    /// 右侧次要文字；`None` = 只有标题。
    pub detail: Option<String>,

    /// 点击这一行时 `AppEvent::RegionClicked` 里的名字；`None` = 不可点击。
    pub action: Option<String>,
}

impl ListItem {
    /// 只有标题的一行；不带点击。
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            detail: None,
            action: None,
        }
    }

    /// 带右侧说明文字。
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// 点击这一行派 `RegionClicked(action)`。
    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }
}

/// 列表盒的绘制参数（点）。
#[derive(Debug, Clone, Copy)]
pub struct ListBoxStyle {
    /// 视口高度。
    pub height: f32,

    /// 每行高度。
    pub row_height: f32,

    /// 行与行之间的空隙。
    pub gap: f32,

    /// 可见行不够填满视口时，把剩余高度平均进行间隙，保证每行完整显示（缺省 true）。
    pub auto_item_gap_averaging: bool,

    pub radius: f32,
    pub background: Color,
    pub border: Color,
    pub border_width: f32,

    /// 边框内侧到内容区的间隙。
    pub content_gap: f32,

    /// 行内文字左右留白。
    pub padding_x: f32,

    /// 行悬停 / 选中底色的圆角。
    pub row_radius: f32,

    pub size: f32,
    pub muted_size: f32,
    pub font: FontSlot,
    pub text: Color,
    pub muted: Color,
    pub selection: Color,
    pub selection_text: Color,
    pub hover: Color,
}

impl Default for ListBoxStyle {
    fn default() -> Self {
        Self {
            height: 200.0,
            row_height: 30.0,
            gap: 2.0,
            auto_item_gap_averaging: true,
            radius: 10.0,
            background: Color::rgb(30, 30, 38),
            border: Color::rgb(58, 58, 70),
            border_width: 1.0,
            content_gap: 4.0,
            padding_x: 10.0,
            row_radius: 6.0,
            size: 14.0,
            muted_size: 12.0,
            font: FontSlot::Candidate,
            text: Color::rgb(236, 236, 242),
            muted: Color::rgb(150, 150, 162),
            selection: Color::rgb(48, 108, 220),
            selection_text: Color::rgb(255, 255, 255),
            hover: Color::rgb(45, 45, 56),
        }
    }
}

impl ListBoxStyle {
    /// 固定视口高度的快捷构造。
    pub fn with_height(height: f32) -> Self {
        Self {
            height,
            ..Self::default()
        }
    }

    /// 每行（含空隙）占的高度。
    pub fn row_stride(&self) -> f32 {
        self.row_height.max(1.0) + self.gap.max(0.0)
    }

    /// `rows` 行的内容总高度。
    pub fn content_height(&self, rows: usize) -> f32 {
        rows as f32 * self.row_stride()
    }

    /// 内容区可视高度（除掉边框与内间隙）。
    pub fn viewport_height(&self) -> f32 {
        (self.height - (self.border_width.max(0.0) + self.content_gap.max(0.0)) * 2.0).max(0.0)
    }

    /// `rows` 行时滚轮最多能滚多远。
    pub fn max_scroll(&self, rows: usize) -> f32 {
        (self.content_height(rows) - self.viewport_height()).max(0.0)
    }
}

/// 列表盒的每帧状态。
#[derive(Debug, Clone, Copy, Default)]
pub struct ListBoxState {
    /// 选中行（0 起）；`None` = 没有选中。
    pub selected: Option<usize>,

    /// 滚动偏移（点）；会被内容高度夹住。
    pub scroll_offset: f32,
}

pub(super) struct ListBoxInput<'a> {
    pub name: &'a str,
    pub items: &'a [ListItem],
    pub style: ListBoxStyle,
}

/// 量宽：列表盒横向铺满父容器，高度固定。
pub(super) fn measure(
    style: &ListBoxStyle,
    constraints: cloud_ui_core::Constraints,
) -> cloud_ui_core::Size {
    cloud_ui_core::Size::new(
        constraints.max_width,
        style.height.min(constraints.max_height),
    )
}

/// 把列表盒写进 document。
#[allow(clippy::too_many_arguments)]
pub(super) fn build<F>(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    rect: cloud_ui_core::Rect,
    drawn: (f32, f32),
    clip: Option<cloud_ui_core::Rect>,
    input: ListBoxInput<'_>,
    pointer: Option<(f32, f32)>,
    state: ListBoxState,
    measure: &mut F,
) where
    F: FnMut(&str, FontSlot, f32) -> (f32, f32),
{
    let style = input.style;
    let edge = style.border_width.max(0.0);
    let gap = style.content_gap.max(0.0);
    let viewport = (rect.height - (edge + gap) * 2.0).max(0.0);
    let stride = style.row_height.max(1.0) + style.gap.max(0.0);
    let content_height = input.items.len() as f32 * stride;
    let scrollbar = content_height > viewport + 0.01;
    let available = (rect.width - (edge + gap) * 2.0 - if scrollbar { 12.0 } else { 0.0 }).max(1.0);
    let row_width = available;

    let selected = state.selected.filter(|index| *index < input.items.len());
    let scroll_offset = state
        .scroll_offset
        .clamp(0.0, (content_height - viewport).max(0.0));

    let mut rows = Vec::with_capacity(input.items.len());
    for (index, item) in input.items.iter().enumerate() {
        let normal = row_elements(
            &style,
            item,
            row_width,
            None,
            style.text,
            style.muted,
            measure,
        );
        let selected_elements = if selected == Some(index) {
            row_elements(
                &style,
                item,
                row_width,
                Some(style.selection),
                style.selection_text,
                style.selection_text,
                measure,
            )
        } else {
            Vec::new()
        };
        let hovered_elements = row_elements(
            &style,
            item,
            row_width,
            Some(style.hover),
            style.text,
            style.muted,
            measure,
        );
        rows.push(ListBoxRow {
            index,
            top: index as f32 * stride,
            height: style.row_height,
            width: row_width,
            key: None,
            elements: normal,
            selected_elements,
            hovered_elements,
            hits: Vec::new(),
        });
    }

    let mut list = ListBox {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        radius: style.radius,
        fill: Some(style.background),
        edge_color: style.border,
        edge_width: style.border_width,
        content_gap: style.content_gap,
        gap: style.gap,
        auto_item_gap_averaging: style.auto_item_gap_averaging,
        align: ListBoxAlign::Left,
        rows,
        row_count: input.items.len(),
        scroll_offset,
        content_height,
        allow_select: true,
        selected_index: selected.map_or(-1, |index| index as i32),
        auto_center_selected: false,
        hovered_index: None,
        region_name: input.name.to_owned(),
    };

    // `pointer` 已经是相对列表盒左上角的坐标，直接和 0..尺寸 比。
    if let Some((x, y)) = pointer
        && x >= 0.0
        && x <= rect.width
        && y >= 0.0
        && y <= rect.height
        && let Some(index) = list.row_at(y)
    {
        list.hovered_index = Some(index as i32);
    }

    for row in &list.rows {
        let Some(action) = input
            .items
            .get(row.index)
            .and_then(|item| item.action.as_deref())
        else {
            continue;
        };
        let Some(view_top) = list.row_view_top(row) else {
            continue;
        };
        let top = view_top.max(0.0);
        let bottom = (view_top + row.height).min(viewport);
        if bottom <= top {
            continue;
        }
        let region_top = rect.y + edge + gap + top;
        let region_height = bottom - top;
        super::push_region(
            regions,
            action,
            HitShape::Rect(HitRect::new(
                rect.x + edge + gap,
                region_top,
                row_width,
                region_height,
            )),
            drawn,
            clip,
        );
    }

    elements.push(Element::ListBox(list));
}

#[allow(clippy::too_many_arguments)]
fn row_elements<F>(
    style: &ListBoxStyle,
    item: &ListItem,
    width: f32,
    background: Option<Color>,
    text: Color,
    muted: Color,
    measure: &mut F,
) -> Vec<Element>
where
    F: FnMut(&str, FontSlot, f32) -> (f32, f32),
{
    let mut elements = Vec::new();
    if let Some(color) = background {
        elements.push(Element::Rect {
            x: 0.0,
            y: 0.0,
            width,
            height: style.row_height,
            radius: style.row_radius,
            color,
        });
    }
    let (_, title_height) = measure(&item.title, style.font, style.size);
    elements.push(Element::Text {
        x: style.padding_x,
        y: ((style.row_height - title_height) / 2.0).max(0.0),
        text: item.title.clone(),
        size: style.size,
        slot: style.font,
        color: text,
        italic: false,
    });
    if let Some(detail) = &item.detail {
        let (detail_width, detail_height) = measure(detail, style.font, style.muted_size);
        elements.push(Element::Text {
            x: (width - style.padding_x - detail_width).max(style.padding_x),
            y: ((style.row_height - detail_height) / 2.0).max(0.0),
            text: detail.clone(),
            size: style.muted_size,
            slot: style.font,
            color: muted,
            italic: false,
        });
    }
    elements
}

impl View {
    /// 加一个单列列表盒（子页面容器）；行点击走 `ListItem::action` 的 `RegionClicked`。
    ///
    /// 滚轮 / 选中状态由应用自己保存，这里只按 `state` 画一帧。
    /// 指针位置（[`View::pointer`]）用来算行悬停。
    pub fn list_box(
        &mut self,
        name: impl Into<String>,
        items: &[ListItem],
        style: ListBoxStyle,
        state: ListBoxState,
    ) -> cloud_ui_core::NodeId {
        let mut node = cloud_ui_core::Node::new(cloud_ui_core::NodeKind::ListBox);
        node.width = cloud_ui_core::SizeSpec::Auto;
        node.height = cloud_ui_core::SizeSpec::Fixed(style.height);
        self.push(
            node,
            super::Widget::ListBox {
                name: name.into(),
                items: items.to_vec(),
                style,
                state,
            },
        )
    }
}
