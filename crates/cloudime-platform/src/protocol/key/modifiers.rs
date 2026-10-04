use serde::{Deserialize, Serialize};

/// 一次按键按下时的修饰键状态（Windows 语义）。后两位不是物理修饰键，是输入法状态：
/// `caps` 是 Caps Lock 锁定位（管大小写、亮着即临时英文大写），`english_mode` 是 Shift 单击切出来的持久中英模式。
///
/// 普通字符键通常都是 `false`，能干净序列化。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KeyModifiers {
    /// Ctrl
    pub ctrl: bool,

    /// Shift
    pub shift: bool,

    /// Alt
    pub alt: bool,

    /// Win（⊞）
    pub win: bool,

    /// Caps Lock 亮着（锁定状态，不是按着）。管字母大小写；亮着时无论中英文模式都直接出大写英文（微软拼音式）。
    #[serde(default)]
    pub caps: bool,

    /// 持久的英文模式（Windows 单击 Shift 切换，DLL 记状态）。中文模式为 `false`。
    #[serde(default)]
    pub english_mode: bool,
}

impl KeyModifiers {
    /// 有没有按着 Ctrl / Alt / Win（Shift 不算：它只改字符大小写与标点）。
    pub fn has_command_key(self) -> bool {
        self.ctrl || self.alt || self.win
    }
}
