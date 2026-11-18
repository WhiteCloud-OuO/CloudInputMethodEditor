//! 画布几何：按运算符把矩形切给子槽。
//!
//! 切分只为「看得懂、点得到」，不追求与字形完全一致：包围结构把子槽往内缩，
//! 重叠按左右分栏显示（设计文档 7.2）。

use cloud_ui::Rect;
use cloudime_ids::IdsOp;

use crate::composer::Node;

/// 一个节点在画布上的位置。
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub path: Vec<usize>,
    pub rect: Rect,
}

/// 递归切分，返回根到每个节点的矩形（先父后子，深度优先）。
pub fn layout(root: &Node, rect: Rect, gap: f32) -> Vec<Placed> {
    let mut placed = Vec::new();
    place(root, &[], rect, gap.max(0.0), &mut placed);
    placed
}

fn place(node: &Node, path: &[usize], rect: Rect, gap: f32, out: &mut Vec<Placed>) {
    out.push(Placed {
        path: path.to_vec(),
        rect,
    });
    let Node::Op { op, children } = node else {
        return;
    };
    for (index, child) in children.iter().enumerate() {
        let child_rect = child_rect(*op, index, rect, gap);
        let mut child_path = path.to_vec();
        child_path.push(index);
        place(child, &child_path, child_rect, gap, out);
    }
}

/// 某个子槽的矩形。
fn child_rect(op: IdsOp, index: usize, rect: Rect, gap: f32) -> Rect {
    let inset = (rect.width.min(rect.height) * 0.12).max(2.0);
    match op {
        IdsOp::LeftRight | IdsOp::Overlaid | IdsOp::Subtraction => split_x(rect, 2, gap, index),
        IdsOp::LeftMiddleRight => split_x(rect, 3, gap, index),
        IdsOp::AboveBelow => split_y(rect, 2, gap, index),
        IdsOp::AboveMiddleBelow => split_y(rect, 3, gap, index),
        IdsOp::FullSurround => inset_rect(rect, inset, inset, inset, inset),
        IdsOp::SurroundAbove => inset_rect(rect, inset, 0.0, 0.0, 0.0),
        IdsOp::SurroundBelow => inset_rect(rect, 0.0, inset, 0.0, 0.0),
        IdsOp::SurroundLeft => inset_rect(rect, 0.0, 0.0, 0.0, inset),
        IdsOp::SurroundRight => inset_rect(rect, 0.0, 0.0, inset, 0.0),
        IdsOp::SurroundUpperLeft => inset_rect(rect, inset, 0.0, 0.0, inset),
        IdsOp::SurroundUpperRight => inset_rect(rect, inset, inset, 0.0, 0.0),
        IdsOp::SurroundLowerLeft => inset_rect(rect, 0.0, 0.0, inset, inset),
        IdsOp::SurroundLowerRight => inset_rect(rect, 0.0, inset, inset, 0.0),
        IdsOp::HorizontalReflection | IdsOp::Rotation => {
            let by = (rect.width.min(rect.height) * 0.08).max(2.0);
            inset_rect(rect, by, by, by, by)
        }
    }
}

/// 上下再左右：参数顺序是 上 / 下 / 右 / 左。
fn inset_rect(rect: Rect, top: f32, bottom: f32, right: f32, left: f32) -> Rect {
    Rect {
        x: rect.x + left,
        y: rect.y + top,
        width: (rect.width - left - right).max(1.0),
        height: (rect.height - top - bottom).max(1.0),
    }
}

fn split_x(rect: Rect, count: usize, gap: f32, index: usize) -> Rect {
    let width = ((rect.width - gap * (count as f32 - 1.0)) / count as f32).max(1.0);
    Rect {
        x: rect.x + (width + gap) * index as f32,
        y: rect.y,
        width,
        height: rect.height,
    }
}

fn split_y(rect: Rect, count: usize, gap: f32, index: usize) -> Rect {
    let height = ((rect.height - gap * (count as f32 - 1.0)) / count as f32).max(1.0);
    Rect {
        x: rect.x,
        y: rect.y + (height + gap) * index as f32,
        width: rect.width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composer::Composer;

    fn rect() -> Rect {
        Rect {
            x: 10.0,
            y: 20.0,
            width: 300.0,
            height: 200.0,
        }
    }

    fn approx(left: f32, right: f32) -> bool {
        (left - right).abs() < 0.01
    }

    #[test]
    fn left_right_splits_into_equal_columns() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftRight);
        let placed = layout(composer.root(), rect(), 8.0);
        assert_eq!(placed.len(), 3);
        assert!(approx(placed[1].rect.width, 146.0));
        assert!(approx(placed[2].rect.x, 10.0 + 146.0 + 8.0));
    }

    #[test]
    fn three_rows_split_into_equal_heights() {
        let mut composer = Composer::new();
        composer.split(IdsOp::AboveMiddleBelow);
        let placed = layout(composer.root(), rect(), 5.0);
        assert_eq!(placed.len(), 4);
        for row in &placed[1..] {
            assert!(approx(row.rect.height, (200.0 - 10.0) / 3.0));
        }
        assert!(approx(placed[3].rect.y, 20.0 + (190.0 / 3.0 + 5.0) * 2.0));
    }

    #[test]
    fn surround_shrinks_the_child() {
        let mut composer = Composer::new();
        composer.split(IdsOp::FullSurround);
        let placed = layout(composer.root(), rect(), 8.0);
        let child = placed[1].rect;
        assert!(child.x > rect().x);
        assert!(child.y > rect().y);
        assert!(child.width < rect().width);
        assert!(child.height < rect().height);
    }

    #[test]
    fn nested_nodes_get_paths_that_match_the_tree() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftRight);
        composer.split(IdsOp::AboveBelow);
        let placed = layout(composer.root(), rect(), 6.0);
        assert!(placed.iter().any(|placed| placed.path == vec![0, 1]));
        assert_eq!(placed.last().unwrap().path, vec![1]);
    }

    #[test]
    fn tiny_rects_stay_positive() {
        let small = Rect {
            x: 0.0,
            y: 0.0,
            width: 3.0,
            height: 3.0,
        };
        let mut composer = Composer::new();
        composer.split(IdsOp::AboveMiddleBelow);
        for placed in layout(composer.root(), small, 8.0) {
            assert!(placed.rect.width >= 1.0);
            assert!(placed.rect.height >= 1.0);
        }
    }
}
