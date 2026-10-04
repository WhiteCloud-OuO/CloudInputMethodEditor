//! 状态条上一个按钮的动作。
//! `icons-arrangement.cfg` 的 `button=` 名字由状态条的 `ACTIONS` 表映射到这里。

/// 状态条上一个按钮点下去做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusAction {
    /// 中 / 英 / 大写锁定：切中英模式。
    ToggleLang,

    /// 中文标点 / 英文标点：切当前模式的全角标点。
    TogglePunctuation,

    /// 全角 / 半角：切直通字符的全角转换。
    ToggleCharWidthType,

    /// 简 / 繁：切繁体输出。
    ToggleSimpTrad,

    /// 设置：打开设置程序（UI 线程直接起进程，不经 Router）。
    OpenOptions,

    /// 工具页：程序还没做，点了只记日志。
    OpenWidgets,

    /// 特殊字符页：程序还没做，点了只记日志。
    OpenSpecChars,
}
