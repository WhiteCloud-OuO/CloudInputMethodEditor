//! 结果列表：复用 `build_text_list`，自己补行命中区域与滚动条目标。

use cloud_ui::{Rect, TextListState, Theme};
use cloudime_render::{
    Element, FontSlot, HitRect, HitShape, Region, ScrollAxis, ScrollHandle, ScrollTarget,
    build_text_list,
};

/// 结果列表的区域名前缀；行名是 `ids.results#原下标`。
pub const RESULTS_NAME: &str = "ids.results";

/// 画结果列表并登记滚动条。
#[allow(clippy::too_many_arguments)]
pub fn build(
    elements: &mut Vec<Element>,
    regions: &mut Vec<Region>,
    scroll_targets: &mut Vec<ScrollTarget>,
    rect: Rect,
    items: &[String],
    state: &TextListState,
    theme: &Theme,
    pointer: Option<(f32, f32)>,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let mut style = theme.text_list();
    style.auto_height = false;
    style.height = Some(rect.height);
    style.width = Some(rect.width);
    style.allow_text_wrapping = false;

    let visible: Vec<usize> = (0..items.len()).collect();
    let mut list = build_text_list(
        &style,
        items,
        &visible,
        state.selected,
        None,
        state.scroll.get(),
        state.h_scroll.get(),
        (rect.x, rect.y, rect.width, rect.height),
        RESULTS_NAME,
        measure,
    );
    list.scroll_offset = list.clamp_scroll();
    list.h_scroll_offset = list.clamp_h_scroll();
    state.scroll.set(list.scroll_offset);
    state.h_scroll.set(list.h_scroll_offset);

    let edge = style.edge_width.max(0.0);
    let gap = style.content_gap.max(0.0);
    let hovered = pointer.and_then(|(x, y)| {
        if x < rect.x || x > rect.x + rect.width || y < rect.y || y > rect.y + rect.height {
            return None;
        }
        let position = list.row_at(y - rect.y)?;
        visible.get(position).copied()
    });
    let viewport = list.viewport_width();
    let viewport_height = list.viewport_height();
    for row in &list.rows {
        let Some(&index) = visible.get(row.index) else {
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
        regions.push(Region::new(
            format!("{RESULTS_NAME}#{index}"),
            HitShape::Rect(HitRect::new(
                rect.x + edge + gap,
                rect.y + edge + gap + top,
                viewport,
                bottom - top,
            )),
            false,
        ));
    }

    push_scroll_targets(
        scroll_targets,
        RESULTS_NAME,
        (rect.x, rect.y, rect.width, rect.height),
        &list,
        Some((state.scroll.clone(), style.stride())),
        Some((state.h_scroll.clone(), style.stride())),
    );
    elements.push(Element::ListBox(list));
    hovered
}

fn push_scroll_targets(
    targets: &mut Vec<ScrollTarget>,
    name: &str,
    area: (f32, f32, f32, f32),
    list: &cloudime_render::ListBox,
    vertical: Option<(ScrollHandle, f32)>,
    horizontal: Option<(ScrollHandle, f32)>,
) {
    if let Some((handle, step)) = vertical
        && list.scrollbar_visible()
    {
        let (tx, ty, tw, th) = list.track_rect();
        let (hx, hy, hw, hh) = list.thumb_rect();
        targets.push(ScrollTarget {
            name: name.to_owned(),
            axis: ScrollAxis::Vertical,
            area,
            track: (area.0 + tx, area.1 + ty, tw, th),
            thumb: (area.0 + hx, area.1 + hy, hw, hh),
            max: list.max_scroll_px(),
            step,
            handle,
        });
    }
    if let Some((handle, step)) = horizontal
        && list.h_scrollbar_visible()
    {
        let (tx, ty, tw, th) = list.h_track_rect();
        let (hx, hy, hw, hh) = list.h_thumb_rect();
        targets.push(ScrollTarget {
            name: name.to_owned(),
            axis: ScrollAxis::Horizontal,
            area,
            track: (area.0 + tx, area.1 + ty, tw, th),
            thumb: (area.0 + hx, area.1 + hy, hw, hh),
            max: list.max_h_scroll_px(),
            step,
            handle,
        });
    }
}
