//! 输入法的三种状态：中文 / 英文 / 禁用。
//!
//! 中 / 英由单击 `Shift` 切；禁用 / 启用由系统的「输入法 / 非输入法切换」（缺省 `Ctrl + Space`）翻。
//! 禁用是**完全不接管**：所有按键原样交给应用，悬浮状态栏收起，托盘图标显示「off」。

use serde::{Deserialize, Serialize};

/// 全局输入法状态（Server 一份，所有应用共用）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    /// 中文输入。
    #[default]
    Chinese,

    /// 英文输入（中英模式共用一份候选与学习）。
    English,

    /// 禁用：不接管任何按键，等于没装输入法。
    Disabled,
}

impl InputMode {
    /// 英文输入。
    pub fn english(self) -> bool {
        self == Self::English
    }

    /// 禁用。
    pub fn disabled(self) -> bool {
        self == Self::Disabled
    }
}
