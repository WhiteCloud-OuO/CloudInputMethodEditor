/// 状态条一次要显示的内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusView {
    /// 英文模式（`false` 中文）。
    pub english: bool,

    /// 当前模式的标点转全角开着（中英各记一份状态）。
    pub full_width_punctuation: bool,

    /// 直通字符转全角开着（不分中英）。
    pub full_width_chars: bool,

    /// 繁体输出开着。
    pub traditional: bool,

    /// 配置里记住的内容左上角物理像素；`None` 首次按屏幕右下角摆。
    pub anchor: Option<(i32, i32)>,

    /// 前台全屏时自动收起（`[debugging] auto_hide_float_tool_bar`）。
    pub auto_hide_fullscreen: bool,
}
