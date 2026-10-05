//! 高亮条移动动画：起点矩形 + 缓动进度。纯展示，不参与排序，也不影响按键。

mod animation;
mod rect;

pub use animation::HighlightAnimation;
pub use rect::HighlightRect;
