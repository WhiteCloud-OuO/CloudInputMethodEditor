//! 「键位映射参考」的两个用户级设置：**什么时候显示**（[`KeyMapReference`]）与**用哪套组合键**
//! （[`KeyMapHotkey`]）。两者都记在方案目录的 `use_input_scheme.cfg`（读写见 [`super`]），
//! 规范见 `docs/design/input-schemes.md` §4.1。
//!
//! 窗口里画什么（[`KeyMapLayout`]）也在这里：三排键盘格子 + 一张零声母表。

/// 键盘三排（物理键，按键盘顺序）：第一、二排各 10 个，第三排 7 个。
pub const KEYBOARD_ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl;", "zxcvbnm"];

/// 键位映射参考里的一格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCap {
    /// 格子**左上角**那颗键：键盘格子里是一个字母（`Q` / `;`），零声母格子里是两个键（`AY`）。
    pub key: String,

    /// 格子**右上角**的声母（`zh` / `ch` / `sh`，棕色显示）；这个键上没有声母就是空的。
    pub initial: String,

    /// 格子中间那几行**韵母**，一义一行（用换行符分开）；没有就是空的。
    ///
    /// 里面的 ü 一律写作 `ü`（方案文件写 `v`：`van` 显示成 `üan`）。
    pub mapping: String,

    /// u 模式符号窗口的 Shift 符号；普通键位映射参考为空。
    pub shifted: String,
}

/// 键位映射参考的版面：三排键盘格子 + 一张零声母表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyMapLayout {
    /// 三排键盘格子（顺序见 [`KEYBOARD_ROWS`]）。
    pub rows: Vec<Vec<KeyCap>>,

    /// 零声母表：`[Shengmu]` 里的每一项一格（两键 + 韵母），最多 12 项（6 列 × 2 行）。
    pub zero_initials: Vec<KeyCap>,

    /// 面板底部的一行提示（u 模式日文假名用）；普通键位映射参考没有。
    pub footer: Option<String>,

    /// u 模式符号窗口：普通符号画在格子下方、Shift 符号画在上方；普通键位映射参考为 `false`。
    pub symbol: bool,
}

/// 什么时候显示「键位映射参考」（`use_input_scheme.cfg` 的 `key_map_reference`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyMapReference {
    /// 关。
    #[default]
    Off,

    /// 组句里按 [`KeyMapHotkey`] 那套组合键弹出 / 收起。
    Hotkey,

    /// 候选窗口一出现就跟着显示（不用按键）。
    Always,
}

impl KeyMapReference {
    /// 设置页下拉里的顺序。
    pub const ALL: [Self; 3] = [Self::Off, Self::Hotkey, Self::Always];

    /// 配置文件里的写法。
    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Hotkey => "hotkey",
            Self::Always => "always",
        }
    }

    /// 界面上的名字。
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "关闭",
            Self::Hotkey => "按下组合键时",
            Self::Always => "始终跟随候选窗口",
        }
    }

    /// 从配置里读。老版本写过 `1` / `0`（那时只有开关两种状态）：`1` = 「按下组合键时」，
    /// 与加这一项之前的行为一致（`Ctrl + F1`），别让升级把用户的设置丢掉。
    pub fn of(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "hotkey" | "1" | "true" => Self::Hotkey,
            "always" => Self::Always,
            _ => Self::Off,
        }
    }
}

/// 组合键的修饰键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyMapModifiers {
    #[default]
    Ctrl,
    CtrlAlt,
    CtrlShift,
    AltShift,
}

impl KeyMapModifiers {
    /// 设置页下拉里的顺序。
    pub const ALL: [Self; 4] = [Self::Ctrl, Self::CtrlAlt, Self::CtrlShift, Self::AltShift];

    /// 配置文件里的写法。
    pub fn key(self) -> &'static str {
        match self {
            Self::Ctrl => "ctrl",
            Self::CtrlAlt => "ctrl+alt",
            Self::CtrlShift => "ctrl+shift",
            Self::AltShift => "alt+shift",
        }
    }

    /// 界面上的名字。
    pub fn label(self) -> &'static str {
        match self {
            Self::Ctrl => "Ctrl",
            Self::CtrlAlt => "Ctrl+Alt",
            Self::CtrlShift => "Ctrl+Shift",
            Self::AltShift => "Alt+Shift",
        }
    }

    /// 从配置里读；认不出来就是缺省（`Ctrl`）。
    pub fn of(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "ctrl+alt" => Self::CtrlAlt,
            "ctrl+shift" => Self::CtrlShift,
            "alt+shift" => Self::AltShift,
            _ => Self::Ctrl,
        }
    }

    /// 修饰键位图，与 DLL 那边（`InputSettings` 的 `script_key_modifiers` 一套位数、
    /// bit0 ctrl / bit1 alt / bit2 shift / bit3 win）对齐 —— 精确匹配，多一个键就不算。
    pub fn mask(self) -> u16 {
        match self {
            Self::Ctrl => 0b0001,
            Self::CtrlAlt => 0b0011,
            Self::CtrlShift => 0b0101,
            Self::AltShift => 0b0110,
        }
    }
}

/// 组合键的功能键（只开放 F1–F12）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyMapKey {
    #[default]
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl KeyMapKey {
    /// 设置页下拉里的顺序。
    pub const ALL: [Self; 12] = [
        Self::F1,
        Self::F2,
        Self::F3,
        Self::F4,
        Self::F5,
        Self::F6,
        Self::F7,
        Self::F8,
        Self::F9,
        Self::F10,
        Self::F11,
        Self::F12,
    ];

    /// 配置文件里的写法。
    pub fn key(self) -> &'static str {
        match self {
            Self::F1 => "f1",
            Self::F2 => "f2",
            Self::F3 => "f3",
            Self::F4 => "f4",
            Self::F5 => "f5",
            Self::F6 => "f6",
            Self::F7 => "f7",
            Self::F8 => "f8",
            Self::F9 => "f9",
            Self::F10 => "f10",
            Self::F11 => "f11",
            Self::F12 => "f12",
        }
    }

    /// 界面上的名字。
    pub fn label(self) -> &'static str {
        match self {
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
        }
    }

    /// 从配置里读；认不出来就是缺省（`F1`）。
    pub fn of(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|key| key.key() == value.trim().to_ascii_lowercase())
            .unwrap_or_default()
    }

    /// 虚拟键码（`VK_F1` = `0x70` … `VK_F12` = `0x7B`）。
    pub fn virtual_key(self) -> u32 {
        0x70 + self as u32
    }
}

/// 键位映射参考的组合键：修饰键 + 一个功能键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyMapHotkey {
    pub modifiers: KeyMapModifiers,
    pub key: KeyMapKey,
}

impl KeyMapHotkey {
    /// 写进配置文件的写法（`ctrl+f1`）。
    pub fn value(self) -> String {
        format!("{}+{}", self.modifiers.key(), self.key.key())
    }

    /// 从配置里读。从**最后一个** `+` 处切开，`ctrl+alt+f1` 才算得对；认不出来用缺省。
    pub fn of(value: &str) -> Self {
        let text = value.trim().to_ascii_lowercase();
        let (modifiers, key) = text.rsplit_once('+').unwrap_or(("", text.as_str()));
        Self {
            modifiers: KeyMapModifiers::of(modifiers),
            key: KeyMapKey::of(key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三个选项、四套修饰键、十二个键在设置页与配置文件之间来回一致。
    #[test]
    fn every_option_round_trips() {
        for mode in KeyMapReference::ALL {
            assert_eq!(KeyMapReference::of(mode.key()), mode, "{mode:?}");
            assert!(!mode.label().is_empty());
        }
        for modifiers in KeyMapModifiers::ALL {
            assert_eq!(KeyMapModifiers::of(modifiers.key()), modifiers);
        }
        for key in KeyMapKey::ALL {
            assert_eq!(KeyMapKey::of(key.key()), key);
        }
        for modifiers in KeyMapModifiers::ALL {
            for key in KeyMapKey::ALL {
                let hotkey = KeyMapHotkey { modifiers, key };
                assert_eq!(KeyMapHotkey::of(&hotkey.value()), hotkey);
            }
        }
        let hotkey = KeyMapHotkey {
            modifiers: KeyMapModifiers::CtrlAlt,
            key: KeyMapKey::F9,
        };
        assert_eq!(hotkey.value(), "ctrl+alt+f9");
        assert_eq!(hotkey.modifiers.mask(), 0b0011);
        assert_eq!(hotkey.key.virtual_key(), 0x78, "VK_F9");
    }

    /// 老版本写过的 `1` / `0` 仍读得出来：`1` = 「按下组合键时」（加这一项之前就是这个行为）。
    #[test]
    fn the_old_boolean_still_reads() {
        assert_eq!(KeyMapReference::of("1"), KeyMapReference::Hotkey);
        assert_eq!(KeyMapReference::of("true"), KeyMapReference::Hotkey);
        assert_eq!(KeyMapReference::of("0"), KeyMapReference::Off);
        assert_eq!(KeyMapReference::of(""), KeyMapReference::Off);
        assert_eq!(KeyMapReference::of("说什么"), KeyMapReference::Off);
        // 组合键认不出来就用缺省（`Ctrl + F1`）
        assert_eq!(
            KeyMapHotkey::of("alt+f13"),
            KeyMapHotkey::default(),
            "既不认这套修饰键、也不认这个键 → 整个退回缺省"
        );
        assert_eq!(KeyMapHotkey::of(""), KeyMapHotkey::default());
    }
}
