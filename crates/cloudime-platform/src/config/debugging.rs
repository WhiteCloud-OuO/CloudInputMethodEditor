//! `[debugging]` 分节：调试页的实验性开关。
//!
//! 这一节的东西都还在试，开关的语义以后可能变；两个开关缺省都关（`Default` 就是全 `false`）。

use serde::{Deserialize, Serialize};

/// `[debugging]` 分节。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DebuggingConfig {
    /// 自动隐藏悬浮工具栏（实验性，缺省关）：开启时前台是全屏应用（游戏 / 看视频）就收起；
    /// 切到别的输入法、云朵被禁用时始终收起（与这一项无关）。
    pub auto_hide_float_tool_bar: bool,

    /// 不处于输入状态时自动禁用输入法（实验性，缺省关）：焦点不在可输入文本区域（没有文本焦点、只读视图、
    /// 密码框这类上下文）时按「禁用」走，回到文本区域再恢复（判断与动作都在 DLL，见 TSF 那一节）。
    pub auto_disable_without_text_input: bool,

    /// 模型优先使用图形处理器加载（实验性，缺省关）：为之后的 DirectML 推理保留的偏好；
    /// 当前版本还没有 GPU 推理路径，打开只记录偏好，仍走 CPU。
    pub prefer_gpu_model: bool,
}
