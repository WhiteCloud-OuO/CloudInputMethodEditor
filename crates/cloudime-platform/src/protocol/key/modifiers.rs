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

    /// 物理修饰键的位图：bit0 ctrl / bit1 alt / bit2 shift / bit3 win（一个都没按是 `0`）。
    ///
    /// 这一套位数是**跨进程约定**：清单里脚本写的 `combination_modifiers`、Server 下发给 DLL 的
    /// `InputSettings::script_key_modifiers`、DLL 判「这套组合键要不要送上来」都用它，
    /// 键位映射参考那套组合键（`InputSettings::key_map_modifiers` 与 `input_schemes::KeyMapModifiers`）
    /// 也按它算 —— 精确匹配，多按一个键就不算命中。`caps` / `english_mode` 不是物理修饰键，不进来。
    pub fn mask(self) -> u16 {
        u16::from(self.ctrl)
            | (u16::from(self.alt) << 1)
            | (u16::from(self.shift) << 2)
            | (u16::from(self.win) << 3)
    }
}
