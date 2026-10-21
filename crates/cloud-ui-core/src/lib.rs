//! 页面树与布局核心：几何、约束和布局算法。
//!
//! 这个 crate 不依赖 Windows、脚本运行时或渲染器；`cloud-ui` 提供窗口壳，
//! `cloudime-script` 提供 JSON / Lua 绑定，`cloudime-render` 只接收布局后的几何。
//! 第 0 步先放基础类型；页面树节点、解析器和 measure / arrange 引擎在后续步骤加入。

mod engine;
mod geometry;
mod layout;
mod page;

pub use engine::{LayoutResult, Measure, NoopMeasure, layout};
pub use geometry::{Point, Rect, Size};
pub use layout::{Align, Axis, Constraints, Justify, LayoutKind, Position};
pub use page::{Edges, GridTrack, Insets, LayoutSpec, Node, NodeId, NodeKind, PageTree, SizeSpec};
