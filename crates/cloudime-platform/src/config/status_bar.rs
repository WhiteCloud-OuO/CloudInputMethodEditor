use serde::{Deserialize, Serialize};

/// `[status_bar]` 分节：悬浮状态条显示不显示、以及它记住的位置。
/// 按钮见 `data\icons-arrangement.cfg`；与任务栏的中 / 英指示器（语言栏按钮）并存，各是一条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusBarConfig {
    /// 在屏幕上显示悬浮工具栏（缺省开）。关掉后桌面上不再出现那条工具条。
    pub show_status_bar: bool,

    /// 记住的屏幕横坐标（内容左上角物理像素）；没拖动过为 `None`，首次按屏幕右下角摆放。
    pub x: Option<i32>,

    /// 记住的屏幕纵坐标（内容左上角物理像素）。
    pub y: Option<i32>,
}

impl Default for StatusBarConfig {
    fn default() -> Self {
        Self {
            // `#[serde(default)]` 缺字段时取的就是这份：缺省显示。
            show_status_bar: true,
            x: None,
            y: None,
        }
    }
}
