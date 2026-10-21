//! 页面树的布局枚举与尺寸约束。

use crate::geometry::Size;

/// 布局主轴方向。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Axis {
    /// 从左到右。
    #[default]
    Row,

    /// 从上到下。
    Column,
}

/// 页面 / 容器可选的布局算法。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LayoutKind {
    /// 单方向、不换行、不拉伸。
    Linear,

    /// 对齐、换行、拉伸、单独间距；缺省。
    #[default]
    Flex,

    /// 轨道 + 单元格。
    Grid,

    /// 可换行的流式布局。
    Responsive,
}

/// 节点是否参与正常布局流。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Position {
    /// 正常布局流。
    #[default]
    Flow,

    /// 绝对定位，脱离布局流。
    Absolute,
}

/// 父布局传给子节点的尺寸约束（点）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constraints {
    pub min_width: f32,
    pub max_width: f32,
    pub min_height: f32,
    pub max_height: f32,
}

impl Constraints {
    /// 没有任何限制。
    pub const fn loose() -> Self {
        Self {
            min_width: 0.0,
            max_width: f32::INFINITY,
            min_height: 0.0,
            max_height: f32::INFINITY,
        }
    }

    /// 宽高都固定成 `size`。
    pub const fn tight(size: Size) -> Self {
        Self {
            min_width: size.width,
            max_width: size.width,
            min_height: size.height,
            max_height: size.height,
        }
    }

    /// 把尺寸夹到约束范围内。
    pub fn clamp(self, size: Size) -> Size {
        Size::new(
            size.width.clamp(self.min_width, self.max_width),
            size.height.clamp(self.min_height, self.max_height),
        )
    }

    /// 约束是否允许宽度伸缩。
    pub fn width_is_tight(self) -> bool {
        self.min_width == self.max_width
    }

    /// 约束是否允许高度伸缩。
    pub fn height_is_tight(self) -> bool {
        self.min_height == self.max_height
    }
}

#[cfg(test)]
mod tests {
    use super::{Axis, Constraints, LayoutKind, Position};
    use crate::geometry::Size;

    #[test]
    fn defaults_match_the_page_tree_contract() {
        assert_eq!(Axis::default(), Axis::Row);
        assert_eq!(LayoutKind::default(), LayoutKind::Flex);
        assert_eq!(Position::default(), Position::Flow);
    }

    #[test]
    fn constraints_clamp_both_axes() {
        let constraints = Constraints {
            min_width: 100.0,
            max_width: 200.0,
            min_height: 40.0,
            max_height: 80.0,
        };
        assert_eq!(
            constraints.clamp(Size::new(50.0, 90.0)),
            Size::new(100.0, 80.0)
        );
        assert_eq!(
            constraints.clamp(Size::new(150.0, 60.0)),
            Size::new(150.0, 60.0)
        );
        assert!(!constraints.width_is_tight());
        assert!(!constraints.height_is_tight());
    }

    #[test]
    fn tight_constraints_report_both_axes_tight() {
        let constraints = Constraints::tight(Size::new(120.0, 32.0));
        assert_eq!(
            constraints.clamp(Size::new(1.0, 999.0)),
            Size::new(120.0, 32.0)
        );
        assert!(constraints.width_is_tight());
        assert!(constraints.height_is_tight());
    }
}
