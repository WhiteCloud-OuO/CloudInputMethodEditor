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

    /// 在光标矩形附近弹一个 1 秒的状态切换提示（只显示前四个状态按钮，纯展示）。
    /// 载荷：要显示的状态、Caps Lock 亮灭、光标矩形。
    StatusTip(Box<(StatusView, bool, ScreenRect)>),

    /// 换渲染器（字体 / 主题变了才重建）。
    Configure(Box<RenderSettings>),

    /// 开关「辅助窗口」（`auxiliary_window`，眼下是键位映射参考）：挂在配置的组合键上。
    AuxiliaryToggle,

    /// 换「辅助窗口」的显示方式（`use_input_scheme.cfg` 的 `key_map_reference`）。
    AuxiliaryMode(cloudime_platform::input_schemes::KeyMapReference),

    /// 换「辅助窗口」显示的内容（键位映射参考的键盘版面）。
    AuxiliaryKeyMap(cloudime_platform::input_schemes::KeyMapLayout),

    /// 脚本窗口的一次操作（`cloudime.window.*`）：建 / 换内容 / 显示 / 收起 / 关。
    ScriptWindow(Box<cloudime_script::WindowRequest>),
}
