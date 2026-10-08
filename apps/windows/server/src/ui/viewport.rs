//! 候选窗最后一次画出来的内容区尺寸（点）：给脚本的 `cloudime.candidate.width()` 读。
//!
//! UI 线程每画一帧写一次、工人线程在脚本问的时候读，跨线程，所以用两个原子量存 f32 位模式
//! （而不是 `Rc<Cell>`）。`0` 表示「还没画过 / 已经收起」。

use std::sync::atomic::{AtomicU32, Ordering};

/// 候选窗内容区尺寸（点）的共享格子。
#[derive(Default)]
pub(crate) struct Viewport {
    width: AtomicU32,
    height: AtomicU32,
}

impl Viewport {
    /// 记下这次画出来的内容区尺寸（点）。
    pub(crate) fn set(&self, width: f32, height: f32) {
        self.width.store(width.to_bits(), Ordering::Relaxed);
        self.height.store(height.to_bits(), Ordering::Relaxed);
    }

    /// 取最近一次的内容区尺寸（点）；没画过（或已经收起）返回 `None`。
    pub(crate) fn get(&self) -> Option<(f32, f32)> {
        let width = f32::from_bits(self.width.load(Ordering::Relaxed));
        if width <= 0.0 {
            return None;
        }
        let height = f32::from_bits(self.height.load(Ordering::Relaxed));
        Some((width, height))
    }

    /// 窗口收了（或这一帧没内容）：当作现在没有窗口。
    pub(crate) fn clear(&self) {
        self.set(0.0, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::Viewport;

    /// 没画过 / 收了都算没有；画过之后给的就是记进去的尺寸。
    #[test]
    fn viewport_reports_the_last_rendered_size() {
        let viewport = Viewport::default();
        assert_eq!(viewport.get(), None);

        viewport.set(240.0, 96.0);
        assert_eq!(viewport.get(), Some((240.0, 96.0)));

        viewport.clear();
        assert_eq!(viewport.get(), None);
    }
}
