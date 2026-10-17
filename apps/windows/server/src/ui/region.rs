//! 区域（region）：窗口内容里可以被鼠标**点击 / 拖拽**的范围。
//!
//! 区域只管命中与拖拽开关，不负责画（画的是元素）：悬浮工具栏的每个图标、脚本窗口里的按钮，
//! 都是把「画出来的东西」放进一个区域里。坐标单位跟内容一致（脚本窗口是点、状态条是像素）。

use cloudime_render::Region;

/// 按内容坐标找最上面那个区域（后写的盖住先写的，与元素绘制顺序一致）。
pub(super) fn hit(regions: &[Region], x: f32, y: f32) -> Option<usize> {
    regions
        .iter()
        .enumerate()
        .rev()
        .find(|(_, region)| region.contains(x, y))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloudime_render::{HitRect, HitShape};

    fn rect(name: &str, x: f32, y: f32, width: f32, height: f32) -> Region {
        Region {
            name: name.to_owned(),
            shape: HitShape::Rect(HitRect::new(x, y, width, height)),
            draggable: true,
        }
    }

    /// 后写的区域盖住先写的（与元素绘制顺序一致），都没命中就是 `None`。
    #[test]
    fn the_topmost_region_wins_the_hit_test() {
        let regions = vec![
            rect("background", 0.0, 0.0, 100.0, 100.0),
            rect("button", 10.0, 10.0, 20.0, 20.0),
        ];
        assert_eq!(hit(&regions, 50.0, 50.0), Some(0));
        assert_eq!(hit(&regions, 15.0, 15.0), Some(1), "后写的在上面");
        assert_eq!(hit(&regions, 200.0, 200.0), None);
    }
}
