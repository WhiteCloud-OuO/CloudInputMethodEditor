/// 用户在状态条上做的事，UI 线程发回 Router。
/// 打开设置 / 工具 / 特殊字符页不经过 Router，UI 线程自己起。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEvent {
    /// 点了「中 / 英 / A」按钮：翻转中英模式。
    ToggleLang,

    /// 点了「中文标点 / 英文标点」按钮：翻转当前模式的全角标点（中英各记一份）。
    TogglePunctuation,

    /// 点了「全角 / 半角」按钮：翻转直通字符的全角转换（不分中英，`InputSettings` 随 `ModeSync` 下发）。
    ToggleCharWidthType,

    /// 点了「简 / 繁」按钮：翻转繁体输出，写回 `[general] traditional`。
    ToggleSimpTrad,

    /// 拖动结束，内容左上角的新位置（物理像素）。
    Moved(i32, i32),
}
