//! 布局用的基础几何类型：点、尺寸、矩形。

/// 布局坐标 / 尺寸（点）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ORIGIN: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// 布局尺寸（点）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };

    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// 布局矩形（点），`x` / `y` 是左上角。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }

    pub const fn right(self) -> f32 {
        self.x + self.width
    }

    pub const fn bottom(self) -> f32 {
        self.y + self.height
    }

    pub const fn contains(self, point: Point) -> bool {
        point.x >= self.x && point.y >= self.y && point.x < self.right() && point.y < self.bottom()
    }
}

#[cfg(test)]
mod tests {
    use super::{Point, Rect, Size};

    #[test]
    fn rect_edges_and_hit_test_follow_the_top_left_origin() {
        let rect = Rect::new(10.0, 20.0, 100.0, 40.0);
        assert_eq!(rect.right(), 110.0);
        assert_eq!(rect.bottom(), 60.0);
        assert_eq!(rect.size(), Size::new(100.0, 40.0));
        assert!(rect.contains(Point::new(10.0, 20.0)));
        assert!(rect.contains(Point::new(109.9, 59.9)));
        assert!(!rect.contains(Point::new(110.0, 20.0)));
        assert!(!rect.contains(Point::new(10.0, 60.0)));
    }
}
