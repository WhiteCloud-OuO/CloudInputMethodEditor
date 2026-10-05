//! 高亮条的矩形与两矩形之间的滑动插值；纯几何，内容区坐标。

/// 高亮条的矩形，四边都用内容区像素坐标（内容区左上为原点，不含阴影边）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightRect {
    /// 左边界。
    pub left: f32,

    /// 上边界。
    pub top: f32,

    /// 右边界。
    pub right: f32,

    /// 下边界。
    pub bottom: f32,
}

impl HighlightRect {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    /// 四边各平移 `(dx, dy)`。内容区坐标与画布坐标（含阴影边）互转用。
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        Self {
            left: self.left + dx,
            top: self.top + dy,
            right: self.right + dx,
            bottom: self.bottom + dy,
        }
    }

    /// 在 `from` 与 `to` 之间按 `t` 逐边插值；`t` 先钳到 `0..=1`。
    pub fn lerp(from: Self, to: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let edge = |a: f32, b: f32| a + (b - a) * t;
        Self {
            left: edge(from.left, to.left),
            top: edge(from.top, to.top),
            right: edge(from.right, to.right),
            bottom: edge(from.bottom, to.bottom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HighlightRect;

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> HighlightRect {
        HighlightRect::new(left, top, right, bottom)
    }

    #[test]
    fn endpoints_are_the_two_rectangles() {
        let from = rect(1.0, 2.0, 3.0, 4.0);
        let to = rect(10.0, 20.0, 30.0, 40.0);
        assert_eq!(HighlightRect::lerp(from, to, 0.0), from);
        assert_eq!(HighlightRect::lerp(from, to, 1.0), to);
    }

    #[test]
    fn interpolates_every_edge() {
        let from = rect(0.0, 0.0, 10.0, 8.0);
        let to = rect(20.0, 40.0, 60.0, 88.0);
        let middle = HighlightRect::lerp(from, to, 0.5);
        assert_eq!(middle, rect(10.0, 20.0, 35.0, 48.0));
    }

    #[test]
    fn progress_is_clamped() {
        let from = rect(0.0, 0.0, 10.0, 10.0);
        let to = rect(10.0, 10.0, 20.0, 20.0);
        assert_eq!(HighlightRect::lerp(from, to, -1.0), from);
        assert_eq!(HighlightRect::lerp(from, to, 2.0), to);
    }

    /// 平移把内容区坐标搬到画布坐标（含阴影边）；再搬回来是同一个矩形。
    #[test]
    fn translates_by_the_shadow_margin() {
        let inside = rect(0.0, 4.0, 100.0, 30.0);
        let on_canvas = inside.translated(8.0, 8.0);
        assert_eq!(on_canvas, rect(8.0, 12.0, 108.0, 38.0));
        assert_eq!(on_canvas.translated(-8.0, -8.0), inside);
    }
}
