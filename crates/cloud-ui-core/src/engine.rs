//! measure / arrange 布局引擎。
//!
//! 输入是 [`PageTree`]，输出每个节点的绝对矩形。文字 / 图片的固有尺寸由 [`Measure`] 提供；
//! 核心不依赖具体字体或渲染器。

use crate::geometry::{Point, Rect, Size};
use crate::layout::{Align, Axis, Constraints, Justify};
use crate::page::{Edges, GridTrack, LayoutSpec, Node, NodeId, PageTree, SizeSpec};

/// 测量叶子节点 / 子页面的固有尺寸。
pub trait Measure {
    fn measure(&mut self, node: NodeId, constraints: Constraints) -> Size;
}

/// 没有内容测量时的空实现：`auto` 叶子会得到 0。
#[derive(Debug, Default)]
pub struct NoopMeasure;

impl Measure for NoopMeasure {
    fn measure(&mut self, _node: NodeId, _constraints: Constraints) -> Size {
        Size::ZERO
    }
}

/// 布局结果：每个节点一个矩形，根尺寸单独给出。
#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub rects: Vec<Option<Rect>>,
    pub size: Size,
}

impl LayoutResult {
    pub fn rect(&self, id: NodeId) -> Option<Rect> {
        self.rects.get(id).copied().flatten()
    }
}

/// 对整棵页面树做布局。根节点的 `origin` 是 `(0, 0)`。
pub fn layout(
    tree: &PageTree,
    constraints: Constraints,
    measure: &mut impl Measure,
) -> LayoutResult {
    let mut rects = vec![None; tree.nodes.len()];
    let size = place_node(
        tree,
        tree.root,
        Point::ORIGIN,
        constraints,
        None,
        &mut rects,
        measure,
    );
    LayoutResult { rects, size }
}

/// 计算节点的外框尺寸；不会写入结果。
fn measure_node(
    tree: &PageTree,
    id: NodeId,
    constraints: Constraints,
    measure: &mut impl Measure,
) -> Size {
    let node = &tree.nodes[id];
    if !node.visible {
        return Size::ZERO;
    }
    let width = resolve_axis(node.width, constraints.max_width);
    let height = resolve_axis(node.height, constraints.max_height);
    if let (Some(width), Some(height)) = (width, height) {
        return clamp_size(node, Size::new(width, height));
    }

    let intrinsic = intrinsic_size(tree, id, constraints, measure);
    clamp_size(
        node,
        Size::new(
            width.unwrap_or(intrinsic.width),
            height.unwrap_or(intrinsic.height),
        ),
    )
}

/// 节点在给定约束下的内容固有尺寸。
fn intrinsic_size(
    tree: &PageTree,
    id: NodeId,
    constraints: Constraints,
    measure: &mut impl Measure,
) -> Size {
    let node = &tree.nodes[id];
    let width = resolve_axis(node.width, constraints.max_width);
    let height = resolve_axis(node.height, constraints.max_height);
    if let (Some(width), Some(height)) = (width, height) {
        return clamp_size(node, Size::new(width, height));
    }

    if !node.is_layout_host() || node.children.is_empty() {
        let measured = measure.measure(
            id,
            Constraints {
                min_width: 0.0,
                max_width: width.unwrap_or(constraints.max_width),
                min_height: 0.0,
                max_height: height.unwrap_or(constraints.max_height),
            },
        );
        return clamp_size(
            node,
            Size::new(
                width.unwrap_or(measured.width) + node.padding.horizontal(),
                height.unwrap_or(measured.height) + node.padding.vertical(),
            ),
        );
    }

    let content_width = width
        .map(|width| node.content_width(width))
        .unwrap_or(constraints.max_width);
    let content_height = height
        .map(|height| node.content_height(height))
        .unwrap_or(constraints.max_height);
    let mut temp = vec![None; tree.nodes.len()];
    let used = layout_children(
        tree,
        id,
        Point::ORIGIN,
        Size::new(content_width, content_height),
        &mut temp,
        measure,
    );
    clamp_size(
        node,
        Size::new(
            width.unwrap_or(used.width + node.padding.horizontal()),
            height.unwrap_or(used.height + node.padding.vertical()),
        ),
    )
}

/// 在 `origin` 放置节点并递归布局子节点。
fn place_node(
    tree: &PageTree,
    id: NodeId,
    origin: Point,
    constraints: Constraints,
    forced: Option<Size>,
    rects: &mut [Option<Rect>],
    measure: &mut impl Measure,
) -> Size {
    let node = &tree.nodes[id];
    if !node.visible {
        if let Some(slot) = rects.get_mut(id) {
            *slot = None;
        }
        return Size::ZERO;
    }
    let size = forced
        .map(|size| clamp_size(node, size))
        .unwrap_or_else(|| measure_node(tree, id, constraints, measure));
    if let Some(slot) = rects.get_mut(id) {
        *slot = Some(Rect::new(origin.x, origin.y, size.width, size.height));
    }
    if node.children.is_empty() {
        return size;
    }
    let content = node.content_size(size);
    layout_children(
        tree,
        id,
        Point::new(origin.x + node.padding.left, origin.y + node.padding.top),
        content,
        rects,
        measure,
    );
    size
}

fn layout_children(
    tree: &PageTree,
    id: NodeId,
    origin: Point,
    content: Size,
    rects: &mut [Option<Rect>],
    measure: &mut impl Measure,
) -> Size {
    let node = &tree.nodes[id];
    let flow: Vec<NodeId> = node
        .children
        .iter()
        .copied()
        .filter(|child| {
            tree.nodes[*child].visible && tree.nodes[*child].position == crate::Position::Flow
        })
        .collect();
    let absolutely_positioned: Vec<NodeId> = node
        .children
        .iter()
        .copied()
        .filter(|child| {
            tree.nodes[*child].visible && tree.nodes[*child].position == crate::Position::Absolute
        })
        .collect();

    let used = match &node.layout {
        LayoutSpec::Linear { gap } => {
            layout_linear(tree, &flow, origin, content, *gap, rects, measure)
        }
        LayoutSpec::Flex {
            direction,
            wrap,
            justify,
            align,
            gap,
        } => layout_flex(
            tree, &flow, origin, content, *direction, *wrap, *justify, *align, *gap, rects, measure,
        ),
        LayoutSpec::Grid { columns, rows, gap } => layout_grid(
            tree, &flow, origin, content, columns, rows, *gap, rects, measure,
        ),
        LayoutSpec::Responsive {
            gap,
            min_item_width: _,
        } => layout_responsive(tree, &flow, origin, content, *gap, rects, measure),
    };

    for child in absolutely_positioned {
        place_absolute(tree, child, origin, content, rects, measure);
    }
    used
}

fn layout_linear(
    tree: &PageTree,
    children: &[NodeId],
    origin: Point,
    content: Size,
    gap: f32,
    rects: &mut [Option<Rect>],
    measure: &mut impl Measure,
) -> Size {
    let constraints = constraints_for(content);
    let mut x = 0.0;
    let mut height: f32 = 0.0;
    for (index, child) in children.iter().enumerate() {
        if index > 0 {
            x += gap;
        }
        let child_constraints = Constraints {
            min_width: 0.0,
            max_width: content.width.max(0.0),
            min_height: 0.0,
            max_height: content.height.max(0.0),
        };
        let size = measure_node(tree, *child, constraints, measure);
        place_node(
            tree,
            *child,
            Point::new(origin.x + x, origin.y),
            child_constraints,
            Some(size),
            rects,
            measure,
        );
        x += size.width;
        height = height.max(size.height);
    }
    Size::new(x, height)
}

#[allow(clippy::too_many_arguments)]
fn layout_flex(
    tree: &PageTree,
    children: &[NodeId],
    origin: Point,
    content: Size,
    direction: Axis,
    wrap: bool,
    justify: Justify,
    align: Align,
    gap: f32,
    rects: &mut [Option<Rect>],
    measure: &mut impl Measure,
) -> Size {
    if children.is_empty() {
        return Size::ZERO;
    }
    let main = match direction {
        Axis::Row => content.width,
        Axis::Column => content.height,
    };
    let constraints = constraints_for(content);

    let items: Vec<(NodeId, Size, Edges)> = children
        .iter()
        .map(|child| {
            let node = &tree.nodes[*child];
            let size = measure_node(tree, *child, constraints, measure);
            (*child, size, node.margin)
        })
        .collect();

    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut current_line = Vec::new();
    let mut line_main = 0.0;
    for (index, (_, size, margin)) in items.iter().enumerate() {
        let item_main = main_size(*size, direction) + main_margin(*margin, direction);
        let next = if current_line.is_empty() {
            item_main
        } else {
            line_main + gap + item_main
        };
        if wrap && !current_line.is_empty() && next > main {
            lines.push(std::mem::take(&mut current_line));
            line_main = 0.0;
        }
        line_main += if current_line.is_empty() {
            item_main
        } else {
            gap + item_main
        };
        current_line.push(index);
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }

    let mut cursor_cross = 0.0;
    let mut used_main: f32 = 0.0;
    let mut used_cross: f32 = 0.0;
    for (line_index, line) in lines.iter().enumerate() {
        if line_index > 0 {
            cursor_cross += gap;
        }
        let line_main_size: f32 = line
            .iter()
            .map(|index| {
                let (_, size, margin) = items[*index];
                main_size(size, direction) + main_margin(margin, direction)
            })
            .sum::<f32>()
            + gap * line.len().saturating_sub(1) as f32;
        let line_cross_size = line
            .iter()
            .map(|index| {
                let (_, size, margin) = items[*index];
                cross_size(size, direction) + cross_margin(margin, direction)
            })
            .fold(0.0, f32::max);
        let free = if main.is_finite() {
            (main - line_main_size).max(0.0)
        } else {
            0.0
        };
        let grow_total: f32 = line
            .iter()
            .map(|index| tree.nodes[items[*index].0].grow.max(0.0))
            .sum();
        let mut targets = Vec::with_capacity(line.len());
        for index in line {
            let (child, size, margin) = items[*index];
            let mut target = size;
            if free > 0.0 && grow_total > 0.0 {
                let extra = free * tree.nodes[child].grow.max(0.0) / grow_total;
                target = match direction {
                    Axis::Row => Size::new(size.width + extra, size.height),
                    Axis::Column => Size::new(size.width, size.height + extra),
                };
            }
            let child_align = tree.nodes[child].align_self.unwrap_or(align);
            if let Align::Stretch = child_align {
                target = match direction {
                    Axis::Row => {
                        Size::new(target.width, (line_cross_size - margin.vertical()).max(0.0))
                    }
                    Axis::Column => Size::new(
                        (line_cross_size - margin.horizontal()).max(0.0),
                        target.height,
                    ),
                };
            }
            targets.push(target);
        }
        let target_main = targets
            .iter()
            .zip(line.iter())
            .map(|(target, index)| {
                let (_, _, margin) = items[*index];
                main_size(*target, direction) + main_margin(margin, direction)
            })
            .sum::<f32>()
            + gap * line.len().saturating_sub(1) as f32;
        let justify_free = if grow_total > 0.0 && free > 0.0 {
            0.0
        } else {
            (main - target_main).max(0.0)
        };
        let (start, between) = justify_offset(justify, justify_free, line.len());
        let mut cursor_main = start;
        for (position, index) in line.iter().enumerate() {
            if position > 0 {
                cursor_main += between;
            }
            let (child, size, margin) = items[*index];
            let target = targets[position];
            let child_align = tree.nodes[child].align_self.unwrap_or(align);
            let cross_offset = match child_align {
                Align::Start => 0.0,
                Align::Center => (line_cross_size - cross_size(size, direction)).max(0.0) / 2.0,
                Align::End => (line_cross_size - cross_size(size, direction)).max(0.0),
                Align::Stretch => 0.0,
            };
            let origin = match direction {
                Axis::Row => Point::new(
                    origin.x + cursor_main + margin.left,
                    origin.y + cursor_cross + cross_offset + margin.top,
                ),
                Axis::Column => Point::new(
                    origin.x + cursor_cross + cross_offset + margin.left,
                    origin.y + cursor_main + margin.top,
                ),
            };
            place_node(
                tree,
                child,
                origin,
                constraints_for(content),
                Some(target),
                rects,
                measure,
            );
            cursor_main += main_size(target, direction) + main_margin(margin, direction) + gap;
            used_main = used_main.max(cursor_main);
        }
        cursor_cross += line_cross_size;
        used_cross = cursor_cross;
    }
    if direction == Axis::Row {
        Size::new(used_main.min(main).max(0.0), used_cross)
    } else {
        Size::new(used_cross, used_main.min(main).max(0.0))
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_grid(
    tree: &PageTree,
    children: &[NodeId],
    origin: Point,
    content: Size,
    columns: &[GridTrack],
    rows: &[GridTrack],
    gap: f32,
    rects: &mut [Option<Rect>],
    measure: &mut impl Measure,
) -> Size {
    if children.is_empty() {
        return Size::ZERO;
    }
    let constraints = constraints_for(content);
    let mut items: Vec<(NodeId, Size, usize, usize)> = Vec::new();
    let mut next_row = 0;
    let mut next_column = 0;
    for child in children {
        let node = &tree.nodes[*child];
        let size = measure_node(tree, *child, constraints, measure);
        let column = node.grid_column.unwrap_or(next_column);
        let row = node.grid_row.unwrap_or(next_row);
        if node.grid_column.is_none() {
            next_column += 1;
            if next_column >= columns.len().max(1) {
                next_column = 0;
                next_row += 1;
            }
        }
        items.push((*child, size, column, row));
    }
    let column_count = columns.len().max(
        items
            .iter()
            .map(|(_, _, column, _)| column + 1)
            .max()
            .unwrap_or(1),
    );
    let row_count = rows.len().max(
        items
            .iter()
            .map(|(_, _, _, row)| row + 1)
            .max()
            .unwrap_or(1),
    );
    let widths = resolve_tracks(
        columns,
        column_count,
        content.width,
        |index| {
            items
                .iter()
                .filter(|(_, _, column, _)| *column == index)
                .map(|(_, size, _, _)| size.width)
                .fold(0.0, f32::max)
        },
        gap,
    );
    let heights = resolve_tracks(
        rows,
        row_count,
        content.height,
        |index| {
            items
                .iter()
                .filter(|(_, _, _, row)| *row == index)
                .map(|(_, size, _, _)| size.height)
                .fold(0.0, f32::max)
        },
        gap,
    );
    let mut x = origin.x;
    for width in &widths {
        x += width + gap;
    }
    let used_width = (x - origin.x - gap).max(0.0);
    for (child, _, column, row) in items {
        let x = origin.x
            + widths[..column.min(widths.len())].iter().sum::<f32>()
            + gap * column.min(widths.len()) as f32;
        let y = origin.y
            + heights[..row.min(heights.len())].iter().sum::<f32>()
            + gap * row.min(heights.len()) as f32;
        place_node(
            tree,
            child,
            Point::new(x, y),
            constraints_for(Size::new(
                widths.get(column).copied().unwrap_or(0.0),
                heights.get(row).copied().unwrap_or(0.0),
            )),
            None,
            rects,
            measure,
        );
    }
    let used_height = heights.iter().sum::<f32>() + gap * heights.len().saturating_sub(1) as f32;
    Size::new(used_width, used_height)
}

fn layout_responsive(
    tree: &PageTree,
    children: &[NodeId],
    origin: Point,
    content: Size,
    gap: f32,
    rects: &mut [Option<Rect>],
    measure: &mut impl Measure,
) -> Size {
    let constraints = constraints_for(content);
    let items: Vec<(NodeId, Size)> = children
        .iter()
        .map(|child| (*child, measure_node(tree, *child, constraints, measure)))
        .collect();
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut line = Vec::new();
    let mut line_width = 0.0;
    for (index, (_, size)) in items.iter().enumerate() {
        let next = if line.is_empty() {
            size.width
        } else {
            line_width + gap + size.width
        };
        if !line.is_empty() && content.width.is_finite() && next > content.width {
            lines.push(std::mem::take(&mut line));
            line_width = 0.0;
        }
        line_width += if line.is_empty() {
            size.width
        } else {
            gap + size.width
        };
        line.push(index);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    let mut y = 0.0;
    let mut used_width: f32 = 0.0;
    for (line_index, line) in lines.iter().enumerate() {
        if line_index > 0 {
            y += gap;
        }
        let height = line
            .iter()
            .map(|index| items[*index].1.height)
            .fold(0.0, f32::max);
        let mut x = 0.0;
        for index in line {
            let (child, size) = items[*index];
            place_node(
                tree,
                child,
                Point::new(origin.x + x, origin.y + y),
                constraints,
                Some(size),
                rects,
                measure,
            );
            x += size.width + gap;
        }
        used_width = used_width.max((x - gap).max(0.0));
        y += height;
    }
    Size::new(used_width, y)
}

fn place_absolute(
    tree: &PageTree,
    id: NodeId,
    content_origin: Point,
    content: Size,
    rects: &mut [Option<Rect>],
    measure: &mut impl Measure,
) {
    let node = &tree.nodes[id];
    let constraints = constraints_for(content);
    let measured = measure_node(tree, id, constraints, measure);
    let width = resolve_axis(node.width, content.width)
        .or_else(|| match (node.insets.left, node.insets.right) {
            (Some(left), Some(right)) => Some((content.width - left - right).max(0.0)),
            _ => None,
        })
        .unwrap_or(measured.width);
    let height = resolve_axis(node.height, content.height)
        .or_else(|| match (node.insets.top, node.insets.bottom) {
            (Some(top), Some(bottom)) => Some((content.height - top - bottom).max(0.0)),
            _ => None,
        })
        .unwrap_or(measured.height);
    let x = node
        .insets
        .left
        .or_else(|| node.insets.right.map(|right| content.width - right - width))
        .unwrap_or(0.0);
    let y = node
        .insets
        .top
        .or_else(|| {
            node.insets
                .bottom
                .map(|bottom| content.height - bottom - height)
        })
        .unwrap_or(0.0);
    place_node(
        tree,
        id,
        Point::new(content_origin.x + x, content_origin.y + y),
        constraints_for(Size::new(width, height)),
        Some(Size::new(width, height)),
        rects,
        measure,
    );
}

fn resolve_tracks(
    tracks: &[GridTrack],
    count: usize,
    available: f32,
    auto_size: impl Fn(usize) -> f32,
    gap: f32,
) -> Vec<f32> {
    let mut sizes = vec![0.0; count.max(tracks.len())];
    let mut fixed = 0.0;
    let mut fractions = 0.0;
    for (index, size) in sizes.iter_mut().enumerate() {
        let track = tracks.get(index).copied().unwrap_or(GridTrack::Auto);
        let value = match track {
            GridTrack::Fixed(value) => value,
            GridTrack::Percent(percent) if available.is_finite() => available * percent,
            GridTrack::Auto | GridTrack::Percent(_) => auto_size(index),
            GridTrack::Fraction(fraction) => {
                fractions += fraction.max(0.0);
                0.0
            }
        };
        *size = value;
        fixed += value;
    }
    let gaps = gap * sizes.len().saturating_sub(1) as f32;
    if fractions > 0.0 && available.is_finite() {
        let remaining = (available - fixed - gaps).max(0.0);
        for (index, size) in sizes.iter_mut().enumerate() {
            if let Some(GridTrack::Fraction(fraction)) = tracks.get(index) {
                *size = remaining * fraction.max(0.0) / fractions;
            }
        }
    }
    sizes
}

fn resolve_axis(spec: SizeSpec, available: f32) -> Option<f32> {
    match spec {
        SizeSpec::Fixed(value) => Some(value),
        SizeSpec::Percent(percent) if available.is_finite() => Some(available * percent),
        SizeSpec::Auto | SizeSpec::Percent(_) => None,
    }
}

fn clamp_size(node: &Node, size: Size) -> Size {
    Size::new(
        size.width.clamp(node.min_width, node.max_width),
        size.height.clamp(node.min_height, node.max_height),
    )
}

fn constraints_for(size: Size) -> Constraints {
    Constraints {
        min_width: 0.0,
        max_width: size.width.max(0.0),
        min_height: 0.0,
        max_height: size.height.max(0.0),
    }
}

fn main_size(size: Size, direction: Axis) -> f32 {
    match direction {
        Axis::Row => size.width,
        Axis::Column => size.height,
    }
}

fn cross_size(size: Size, direction: Axis) -> f32 {
    match direction {
        Axis::Row => size.height,
        Axis::Column => size.width,
    }
}

fn main_margin(margin: Edges, direction: Axis) -> f32 {
    match direction {
        Axis::Row => margin.horizontal(),
        Axis::Column => margin.vertical(),
    }
}

fn cross_margin(margin: Edges, direction: Axis) -> f32 {
    match direction {
        Axis::Row => margin.vertical(),
        Axis::Column => margin.horizontal(),
    }
}

fn justify_offset(justify: Justify, free: f32, count: usize) -> (f32, f32) {
    match justify {
        Justify::Start => (0.0, 0.0),
        Justify::Center => (free / 2.0, 0.0),
        Justify::End => (free, 0.0),
        Justify::SpaceBetween if count > 1 => (0.0, free / (count - 1) as f32),
        Justify::SpaceAround if count > 0 => (free / count as f32 / 2.0, free / count as f32),
        Justify::SpaceEvenly if count > 0 => (free / (count + 1) as f32, free / (count + 1) as f32),
        Justify::SpaceBetween | Justify::SpaceAround | Justify::SpaceEvenly => (0.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::{LayoutResult, Measure, NoopMeasure, layout};
    use crate::geometry::{Rect, Size};
    use crate::layout::{Align, Axis, Constraints, Justify, Position};
    use crate::page::{GridTrack, LayoutSpec, Node, NodeKind, PageTree, SizeSpec};

    struct FixedMeasure;
    impl Measure for FixedMeasure {
        fn measure(&mut self, _node: usize, constraints: Constraints) -> Size {
            Size::new(constraints.max_width.min(10.0), 20.0)
        }
    }

    fn fixed(width: f32, height: f32) -> Node {
        let mut node = Node::new(NodeKind::Control);
        node.width = SizeSpec::Fixed(width);
        node.height = SizeSpec::Fixed(height);
        node
    }

    fn rect(result: &LayoutResult, id: usize) -> Rect {
        result.rect(id).expect("node should have a rect")
    }

    #[test]
    fn linear_lays_children_out_in_a_row_with_gap() {
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.nodes[0].layout = LayoutSpec::Linear { gap: 4.0 };
        tree.nodes[0].width = SizeSpec::Fixed(100.0);
        tree.nodes[0].height = SizeSpec::Fixed(40.0);
        let first = tree.push(fixed(20.0, 10.0));
        let second = tree.push(fixed(30.0, 12.0));
        tree.nodes[0].children = vec![first, second];
        let result = layout(
            &tree,
            Constraints::tight(Size::new(100.0, 40.0)),
            &mut NoopMeasure,
        );
        assert_eq!(rect(&result, first), Rect::new(0.0, 0.0, 20.0, 10.0));
        assert_eq!(rect(&result, second), Rect::new(24.0, 0.0, 30.0, 12.0));
    }

    #[test]
    fn flex_justify_center_and_cross_center() {
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.nodes[0].layout = LayoutSpec::Flex {
            direction: Axis::Row,
            wrap: false,
            justify: Justify::Center,
            align: Align::Center,
            gap: 10.0,
        };
        tree.nodes[0].width = SizeSpec::Fixed(100.0);
        tree.nodes[0].height = SizeSpec::Fixed(50.0);
        let first = tree.push(fixed(20.0, 10.0));
        let second = tree.push(fixed(20.0, 20.0));
        tree.nodes[0].children = vec![first, second];
        let result = layout(
            &tree,
            Constraints::tight(Size::new(100.0, 50.0)),
            &mut NoopMeasure,
        );
        // 两项 + 10 gap = 50，剩余 50，从 25 开始；交叉轴按最高的一项（20）居中。
        assert_eq!(rect(&result, first), Rect::new(25.0, 5.0, 20.0, 10.0));
        assert_eq!(rect(&result, second), Rect::new(55.0, 0.0, 20.0, 20.0));
    }

    #[test]
    fn flex_grow_uses_the_remaining_main_axis() {
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.nodes[0].layout = LayoutSpec::Flex {
            direction: Axis::Row,
            wrap: false,
            justify: Justify::Start,
            align: Align::Start,
            gap: 0.0,
        };
        tree.nodes[0].width = SizeSpec::Fixed(100.0);
        tree.nodes[0].height = SizeSpec::Fixed(20.0);
        let first = tree.push(fixed(20.0, 10.0));
        tree.nodes[first].grow = 1.0;
        let second = tree.push(fixed(20.0, 10.0));
        tree.nodes[0].children = vec![first, second];
        let result = layout(
            &tree,
            Constraints::tight(Size::new(100.0, 20.0)),
            &mut NoopMeasure,
        );
        assert_eq!(rect(&result, first), Rect::new(0.0, 0.0, 80.0, 10.0));
        assert_eq!(rect(&result, second), Rect::new(80.0, 0.0, 20.0, 10.0));
    }

    #[test]
    fn grid_places_children_in_tracks() {
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.nodes[0].layout = LayoutSpec::Grid {
            columns: vec![GridTrack::Fixed(30.0), GridTrack::Fixed(40.0)],
            rows: vec![GridTrack::Auto],
            gap: 5.0,
        };
        tree.nodes[0].width = SizeSpec::Fixed(100.0);
        tree.nodes[0].height = SizeSpec::Fixed(50.0);
        let first = tree.push(fixed(10.0, 10.0));
        let second = tree.push(fixed(20.0, 20.0));
        tree.nodes[0].children = vec![first, second];
        let result = layout(
            &tree,
            Constraints::tight(Size::new(100.0, 50.0)),
            &mut NoopMeasure,
        );
        assert_eq!(rect(&result, first), Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(rect(&result, second), Rect::new(35.0, 0.0, 20.0, 20.0));
    }

    #[test]
    fn responsive_wraps_to_the_next_line() {
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.nodes[0].layout = LayoutSpec::Responsive {
            gap: 5.0,
            min_item_width: None,
        };
        tree.nodes[0].width = SizeSpec::Fixed(50.0);
        tree.nodes[0].height = SizeSpec::Fixed(80.0);
        let first = tree.push(fixed(30.0, 10.0));
        let second = tree.push(fixed(30.0, 10.0));
        tree.nodes[0].children = vec![first, second];
        let result = layout(
            &tree,
            Constraints::tight(Size::new(50.0, 80.0)),
            &mut NoopMeasure,
        );
        assert_eq!(rect(&result, first), Rect::new(0.0, 0.0, 30.0, 10.0));
        assert_eq!(rect(&result, second), Rect::new(0.0, 15.0, 30.0, 10.0));
    }

    #[test]
    fn absolute_node_is_outside_the_flow() {
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.nodes[0].layout = LayoutSpec::Linear { gap: 0.0 };
        tree.nodes[0].width = SizeSpec::Fixed(100.0);
        tree.nodes[0].height = SizeSpec::Fixed(50.0);
        let flow = tree.push(fixed(10.0, 10.0));
        let mut absolute = fixed(20.0, 20.0);
        absolute.position = Position::Absolute;
        absolute.insets.right = Some(5.0);
        absolute.insets.top = Some(6.0);
        let absolute = tree.push(absolute);
        tree.nodes[0].children = vec![flow, absolute];
        let result = layout(
            &tree,
            Constraints::tight(Size::new(100.0, 50.0)),
            &mut NoopMeasure,
        );
        assert_eq!(rect(&result, flow), Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(rect(&result, absolute), Rect::new(75.0, 6.0, 20.0, 20.0));
    }

    #[test]
    fn auto_size_uses_the_measure_callback() {
        let mut tree = PageTree::new(Node::new(NodeKind::Page));
        tree.nodes[0].layout = LayoutSpec::Linear { gap: 0.0 };
        let child = tree.push(Node::new(NodeKind::Control));
        tree.nodes[0].children = vec![child];
        let result = layout(&tree, Constraints::loose(), &mut FixedMeasure);
        assert_eq!(result.size, Size::new(10.0, 20.0));
        assert_eq!(rect(&result, child), Rect::new(0.0, 0.0, 10.0, 20.0));
    }
}
