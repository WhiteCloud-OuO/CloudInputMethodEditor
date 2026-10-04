//! `[debugging]` 分节：调试页的实验性开关。
//!
//! 这一节的东西都还在试，开关的语义以后可能变；缺省值按「保持改造前行为」定。

use serde::{Deserialize, Serialize};

/// `[debugging]` 分节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DebuggingConfig {
    /// 自动隐藏悬浮工具栏（实验性）：开启时前台是全屏应用（游戏 / 看视频）就收起；
    /// 切到别的输入法、云朵被禁用时始终收起（与这一项无关）。
    pub auto_hide_float_tool_bar: bool,
}

impl Default for DebuggingConfig {
    fn default() -> Self {
        Self {
            auto_hide_float_tool_bar: true,
        }
    }
}
