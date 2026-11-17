//! 文本列表（叶子控件）：一组字符串 + 可选模糊查询 / 排序。
//!
//! 行内容只有文本，没有子页面与行内控件；选中 / 滚动状态和应用自己保存
//! （[`TextListState`]），这里只按状态画一帧，并给可见行写 `名字#原下标` 的命中区域。

use cloud_ui_core::{Constraints, Rect, Size};
use cloudime_render::{
    Element, FontSlot, HitRect, HitShape, Region, ScrollHandle, TextListStyle, build_text_list,
    visible_items,
};

use super::View;

/// 文本列表的每帧状态。
///
/// 两个滚动偏移都是 [`ScrollHandle`]：建一次、跨帧持有，每帧把 `clone()` 传给
/// [`View::text_list`]。窗口壳拖动滚动条 / 滚轮时直接写句柄，应用不用自己接鼠标。
#[derive(Debug, Clone, Default)]
pub struct TextListState {
    /// 选中的项（`items` 里的**原下标**，不是过滤 / 排序后的行号）。
    pub selected: Option<usize>,

    /// 竖向滚动偏移句柄（点）。
    pub scroll: ScrollHandle,

    /// 水平滚动偏移句柄（点）；只有 `allow_text_wrapping = false` 且内容超宽时有用。
    pub h_scroll: ScrollHandle,
}

/// 量宽：横向铺满父容器；`auto_height` 为真或没有固定高度时按可见行数自动算。
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
    let mut auto = || {
        let width = cloudime_render::text_list_measured_width(style, items, &shown, measure);
        cloudime_render::text_list_auto_height(style, shown.len(), constraints.max_width, width)
    };
    let height = match style.height {
        Some(height) if !style.auto_height => height,
        _ => auto(),
    };
    Size::new(
        style
            .width
            .unwrap_or(constraints.max_width)
            .min(constraints.max_width),
        height.min(constraints.max_height),
    )
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
    let scroll = state.scroll.get();

    // 指针已经是相对整个 view 的坐标；换算到列表盒左上角后取行。
    let pointer = pointer.map(|(x, y)| (x - (drawn.0 + rect.x), y - (drawn.1 + rect.y)));

    let mut list = build_text_list(
        &style,
        items,
        &shown,
        state.selected,
        None,
        scroll,
        state.h_scroll.get(),
        (rect.x, rect.y, rect.width, rect.height),
        name,
        measure,
    );

    // 夹紧后回写句柄，让应用与窗口壳看到同一个受限偏移。
    list.scroll_offset = list.clamp_scroll();
    list.h_scroll_offset = list.clamp_h_scroll();
    state.scroll.set(list.scroll_offset);
    state.h_scroll.set(list.h_scroll_offset);

    // 行位置由 `ListBox` 算（含平均间隙），悬停也按同一份位置取。
    if let Some((x, y)) = pointer
        && x >= 0.0
        && x <= rect.width
        && y >= 0.0
        && y <= rect.height
        && let Some(position) = list.row_at(y)
    {
        list.hovered_index = Some(position as i32);
    }

    let viewport = list.viewport_width();
    let viewport_height = list.viewport_height();
    for row in &list.rows {
        let Some(&index) = shown.get(row.index) else {
            continue;
        };
        let Some(view_top) = list.row_view_top(row) else {
            continue;
        };
        let top = view_top.max(0.0);
        let bottom = (view_top + row.height).min(viewport_height);
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
    /// 给固定值则有视口、按 `state.scroll` 滚动。行点击走
    /// `AppEvent::RegionClicked("名字#原下标")`；滚动偏移是共享句柄，拖动滚动条 /
    /// 滚轮由窗口壳直接写，应用不用自己接鼠标。
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
        node.width = match style.width {
            Some(width) => cloud_ui_core::SizeSpec::Fixed(width),
            None => cloud_ui_core::SizeSpec::Auto,
        };
        node.height = if style.auto_height {
            cloud_ui_core::SizeSpec::Auto
        } else {
            match style.height {
                Some(height) => cloud_ui_core::SizeSpec::Fixed(height),
                None => cloud_ui_core::SizeSpec::Auto,
            }
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
