use serde::{Deserialize, Serialize};

/// 屏幕坐标系里的一个矩形（**物理像素**）。DLL 在编辑会话里量到组句范围的屏幕位置后，用
/// [`super::ClientMessage::PositionCandidates`] 发给 Server；Server 据此把候选窗口摆到光标下方。
///
/// DLL 会把宿主进程坐标空间里的矩形换算成物理像素（宿主不是 Per-Monitor DPI aware 时两者不同），
/// Server 进程是 PerMonitorV2，按物理像素摆窗与渲染。
///
/// 对应 Win32 的 `RECT`，但不依赖 `windows` crate、可 serde，两端共用（候选窗口自绘搬到了 Server 进程，
/// 才能盖过微软商店 / 任务栏搜索这些高 z-band 宿主）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRect {
    /// 左边界。
    pub left: i32,

    /// 上边界。
    pub top: i32,

    /// 右边界。
    pub right: i32,

    /// 下边界。
    pub bottom: i32,
}
