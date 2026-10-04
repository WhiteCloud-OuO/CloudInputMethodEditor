use serde::{Deserialize, Serialize};

/// `[status_bar]` 分节：悬浮状态条记住的位置（常开，只跟「当前输入法是不是云朵输入法」走）。
/// 按钮见 `data\icons-arrangement.cfg`；与任务栏的中 / 英指示器（语言栏按钮）并存，各是一条。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusBarConfig {
    /// 记住的屏幕横坐标（内容左上角物理像素）；没拖动过为 `None`，首次按屏幕右下角摆放。
    pub x: Option<i32>,

    /// 记住的屏幕纵坐标（内容左上角物理像素）。
    pub y: Option<i32>,
}
