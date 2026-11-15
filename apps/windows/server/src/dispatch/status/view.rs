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

/// 状态切换提示要显示哪几项（`[input] show_status_change_tip = "switched"` 时只亮变的那几项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusItems(u8);

impl StatusItems {
    /// 中 / 英（Caps Lock 亮灭也归这颗按钮）。
    pub const LANG: Self = Self(0b0001);
    /// 全 / 半角字符。
    pub const CHAR_WIDTH: Self = Self(0b0010);
    /// 中文 / 西文标点。
    pub const PUNCTUATION: Self = Self(0b0100);
    /// 简 / 繁。
    pub const SIMP_TRAD: Self = Self(0b1000);

    /// 一项都不显示。
    pub fn none() -> Self {
        Self(0)
    }

    /// 四项全部显示（改动前的「开启」就是这个效果）。
    pub fn all() -> Self {
        Self(0b1111)
    }

    pub fn with(self, item: Self) -> Self {
        Self(self.0 | item.0)
    }

    pub fn contains(self, item: Self) -> bool {
        self.0 & item.0 != 0
    }

    /// 一项都没有。
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}
