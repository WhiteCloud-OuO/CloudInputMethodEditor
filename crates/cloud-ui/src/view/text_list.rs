//! 文本列表（叶子控件）：一组字符串 + 可选模糊查询 / 排序。
//!
//! 行内容只有文本，没有子页面与行内控件；选中 / 滚动状态和应用自己保存
//! （[`TextListState`]），这里只按状态画一帧，并给可见行写 `名字#原下标` 的命中区域。

use cloud_ui_core::{Constraints, Rect, Size};
use cloudime_render::{
    Element, FontSlot, HitRect, HitShape, Region, TextListStyle, build_text_list, visible_items,
};

use super::View;

/// 文本列表的每帧状态。
#[derive(Debug, Clone, Copy, Default)]
pub struct TextListState {
    /// 选中的项（`items` 里的**原下标**，不是过滤 / 排序后的行号）。
    pub selected: Option<usize>,

    /// 滚动偏移（点）；会被内容高度夹住。
    pub scroll_offset: f32,

    /// 水平滚动偏移（点）；只有 `allow_text_wrapping = false` 且内容超宽时有用。
    pub h_scroll_offset: f32,
}

/// 量宽：横向铺满父容器；高度有固定值用固定值，否则按可见行数自动算。
pub(super) fn measure<F>(
    items: &[String],
    query: &str,
    sort: bool,
    style: &TextListStyle,
    constraints: Constraints,
    measure: &mut F,
) -> Size
where
    F: FnMut(&str, FontSlot, f32) -> (f32, f32),
{
    let shown = visible_items(items, query, sort);
    let height = match style.height {
        Some(height) => height,
        None => {
            let width = cloudime_render::text_list_measured_width(style, items, &shown, measure);
            cloudime_render::text_list_auto_height(style, shown.len(), constraints.max_width, width)
        }
    };
    Size::new(constraints.max_width, height.min(constraints.max_height))
}

/// 把文本列表写进 document，并给可见行写命中区域。
#[allow(clippy::too_many_arguments)]
pub(super) fn build<F>(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    name: &str,
    items: &[String],
    query: &str,
    sort: bool,
    style: TextListStyle,
    state: TextListState,
    rect: Rect,
    drawn: (f32, f32),
    clip: Option<Rect>,
    pointer: Option<(f32, f32)>,
    measure: &mut F,
) where
    F: FnMut(&str, FontSlot, f32) -> (f32, f32),
{
    let shown = visible_items(items, query, sort);
    let edge = style.edge_width.max(0.0);
    let gap = style.content_gap.max(0.0);
    let stride = style.stride();

    // 指针已经是相对整个 view 的坐标；换算到列表盒左上角后取行。
    let pointer = pointer.map(|(x, y)| (x - (drawn.0 + rect.x), y - (drawn.1 + rect.y)));
    let hovered = pointer
        .filter(|(x, y)| *x >= 0.0 && *x <= rect.width && *y >= 0.0 && *y <= rect.height)
        .and_then(|(_, y)| {
            let local = y - edge - gap + state.scroll_offset;
            (local >= 0.0).then(|| (local / stride) as usize)
        })
        .filter(|position| *position < shown.len())
        .map(|position| shown[position]);

    let list = build_text_list(
        &style,
        items,
        &shown,
        state.selected,
        hovered,
        state.scroll_offset,
        state.h_scroll_offset,
        (rect.x, rect.y, rect.width, rect.height),
        name,
        measure,
    );

    let viewport = list.viewport_width();
    let viewport_height = list.viewport_height();
    for (position, &index) in shown.iter().enumerate() {
        let top = (position as f32 * stride - list.clamp_scroll()).max(0.0);
        let bottom = (position as f32 * stride - list.clamp_scroll() + style.row_height)
            .min(viewport_height);
        if bottom <= top {
            continue;
        }
        super::push_region(
            regions,
            &format!("{name}#{index}"),
            HitShape::Rect(HitRect::new(
                rect.x + edge + gap,
                rect.y + edge + gap + top,
                viewport,
                bottom - top,
            )),
            drawn,
            clip,
        );
    }

    elements.push(Element::ListBox(list));
}

impl View {
    /// 加一个文本列表（叶子控件，不是容器）。
    ///
    /// `items` 是原数据，`query` 做模糊过滤、`sort` 控制无查询时是否按文本排序；
    /// `state.selected` 用**原下标**。`style.height` 为 `None` 时按行数自动高，
    /// 给固定值则有视口、按 `state.scroll_offset` 滚动。行点击走
    /// `AppEvent::RegionClicked("名字#原下标")`，滚轮 / 选中由应用自己保存进 `TextListState`。
    pub fn text_list(
        &mut self,
        name: impl Into<String>,
        items: &[String],
        query: &str,
        sort: bool,
        style: TextListStyle,
        state: TextListState,
    ) -> cloud_ui_core::NodeId {
        let mut node = cloud_ui_core::Node::new(cloud_ui_core::NodeKind::Control);
        node.width = cloud_ui_core::SizeSpec::Auto;
        node.height = match style.height {
            Some(height) => cloud_ui_core::SizeSpec::Fixed(height),
            None => cloud_ui_core::SizeSpec::Auto,
        };
        self.push(
            node,
            super::Widget::TextList {
                name: name.into(),
                items: items.to_vec(),
                query: query.to_owned(),
                sort,
                style,
                state,
            },
        )
    }
}
