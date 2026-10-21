//! Windows 自绘窗口壳：分层窗口、窗口模板、脚本窗口与 RichEdit 宿主。
//!
//! 底层绘制仍由 [`cloudime_render`] 负责：这里管窗口、焦点、鼠标穿透、命中测试、滚动与文本编辑，
//! 再把 `Document` 交给渲染器出图，最后经 `UpdateLayeredWindow` 整块贴上。
//!
//! 模块分层：
//!
//! - [`layered`]：预乘 RGBA 位图贴到分层窗口；
//! - [`window_class`] / [`window`] / [`monitor`]：窗口类、模块句柄、鼠标穿透、显示器工作区；
//! - [`painter`]：一份共享渲染器（字体、主题、阴影）；
//! - [`auxiliary`] / [`independent_window_template`] / [`auxiliary_window_template`]：
//!   独立窗口与资产窗口的所有权 / 焦点 / 桥接模型；
//! - `script_window`：Lua 脚本建的窗口（含 windowless RichEdit + TSF；`script` 特性，缺省开）。
//!
//! **线程**：窗口相关类型都在 UI 线程上使用（`Rc` / `RefCell` / `thread_local!`），不实现 `Send`；
//! [`RenderSettings`] 只是数据，可跨线程传递。

#[cfg(windows)]
pub mod auxiliary;
#[cfg(windows)]
pub mod auxiliary_window_template;
#[cfg(windows)]
pub mod independent_window_template;
#[cfg(windows)]
pub mod layered;
#[cfg(windows)]
pub mod monitor;
#[cfg(windows)]
pub mod painter;
#[cfg(all(windows, feature = "script"))]
pub mod script_window;
#[cfg(windows)]
pub mod window;
#[cfg(windows)]
pub mod window_class;

#[cfg(windows)]
mod region;
mod render_settings;

pub use cloud_ui_core::{
    Align, Axis, Constraints, Edges, GridTrack, Insets, Justify, LayoutKind, LayoutResult, Node,
    NodeId, NodeKind, PageTree, Point, Position, Rect, Size, SizeSpec,
};
pub use render_settings::RenderSettings;
#[cfg(all(windows, feature = "script"))]
pub use script_window::{ScriptWindowEvents, ScriptWindows};
