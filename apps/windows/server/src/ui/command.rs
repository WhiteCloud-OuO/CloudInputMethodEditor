//! 交给 UI 线程执行的命令。

use cloudime_platform::protocol::{Frame, ScreenRect};

use crate::dispatch::{RenderSettings, StatusView};

/// 交给 UI 线程执行的命令。`Frame` 较大，装箱免得枚举过胖。
pub(super) enum UiCommand {
    /// 把候选窗口摆到组句矩形下方并按帧重绘；`badges` 是每个候选的来源角标。
    Show(Box<(Frame, Vec<Option<char>>, ScreenRect)>),

    /// 收起候选窗口。
    Hide,

    /// 显示 / 更新悬浮状态条。
    StatusShow(Box<StatusView>),

    /// 收起悬浮状态条。
    StatusHide,

    /// 换渲染器（字体变了才重建）。
    Configure(RenderSettings),
}
