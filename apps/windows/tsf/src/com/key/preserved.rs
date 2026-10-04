//! 组合键登记成 TSF **保留键**（preserved key）。
//! 带 Alt 的组合是系统键，不经击键 sink；保留键由 TSF 在应用之前匹配、回调 `OnPreservedKey`，UWP 里也一样。
//!
//! 三个：`Ctrl + Alt + Space` 切中英（旧的可选切换键，缺省不登记）、`Ctrl + Alt + .` 切简 / 繁、
//! `Ctrl + Alt + ,` 切中文 / 西文标点（后两个是固定内置热键）。

use windows::Win32::UI::Input::KeyboardAndMouse::{VK_OEM_COMMA, VK_OEM_PERIOD, VK_SPACE};
use windows::Win32::UI::TextServices::{
    ITfKeystrokeMgr, TF_MOD_ALT, TF_MOD_CONTROL, TF_PRESERVEDKEY,
};
use windows::core::{GUID, Result};

/// Ctrl + Alt + Space 中英切换键的保留键标识。
pub(crate) const GUID_SWITCH_MODE: GUID = GUID::from_u128(0xc4e65c2a_98c1_4c2e_919f_1327d05bc7ee);

/// Ctrl + Alt + . 简 / 繁的保留键标识。
pub(crate) const GUID_TOGGLE_SIMP_TRAD: GUID =
    GUID::from_u128(0x7a1c4d92_3b5e_4f08_a1c6_9e2d4b7f0a35);

/// Ctrl + Alt + , 中文 / 西文标点的保留键标识。
pub(crate) const GUID_TOGGLE_PUNCTUATION: GUID =
    GUID::from_u128(0x5c3e9f27_1d6a_4b82_8f47_2a6c9d0e5b18);

/// Ctrl + Alt + Space 的 `TF_PRESERVEDKEY`。不用 Ctrl + Space：中文 Windows 把它绑成系统的
/// 「输入法/非输入法切换」，系统先截走，保留键收不到。
fn switch_mode_key() -> TF_PRESERVEDKEY {
    TF_PRESERVEDKEY {
        uVKey: VK_SPACE.0 as u32,
        uModifiers: TF_MOD_CONTROL | TF_MOD_ALT,
    }
}

fn simp_trad_key() -> TF_PRESERVEDKEY {
    TF_PRESERVEDKEY {
        uVKey: VK_OEM_PERIOD.0 as u32,
        uModifiers: TF_MOD_CONTROL | TF_MOD_ALT,
    }
}

fn punctuation_key() -> TF_PRESERVEDKEY {
    TF_PRESERVEDKEY {
        uVKey: VK_OEM_COMMA.0 as u32,
        uModifiers: TF_MOD_CONTROL | TF_MOD_ALT,
    }
}

/// 登记 Ctrl + Alt + Space 为中英切换保留键（`switch_mode` 勾了它时）。
pub(crate) fn register_switch_mode(keystroke: &ITfKeystrokeMgr, tid: u32) -> Result<()> {
    let description: Vec<u16> = "切换中英文（云朵输入法）".encode_utf16().collect();
    unsafe { keystroke.PreserveKey(tid, &GUID_SWITCH_MODE, &switch_mode_key(), &description) }
}

pub(crate) fn unregister_switch_mode(keystroke: &ITfKeystrokeMgr) {
    let _ = unsafe { keystroke.UnpreserveKey(&GUID_SWITCH_MODE, &switch_mode_key()) };
}

/// 登记两个固定内置热键。
pub(crate) fn register_hotkeys(keystroke: &ITfKeystrokeMgr, tid: u32) -> Result<()> {
    let simp_trad: Vec<u16> = "简体 / 繁体（云朵输入法）".encode_utf16().collect();
    unsafe {
        keystroke.PreserveKey(tid, &GUID_TOGGLE_SIMP_TRAD, &simp_trad_key(), &simp_trad)?;
    }
    let punctuation: Vec<u16> = "中文 / 西文标点（云朵输入法）".encode_utf16().collect();
    unsafe {
        keystroke.PreserveKey(
            tid,
            &GUID_TOGGLE_PUNCTUATION,
            &punctuation_key(),
            &punctuation,
        )?;
    }
    Ok(())
}

pub(crate) fn unregister_hotkeys(keystroke: &ITfKeystrokeMgr) {
    let _ = unsafe { keystroke.UnpreserveKey(&GUID_TOGGLE_SIMP_TRAD, &simp_trad_key()) };
    let _ = unsafe { keystroke.UnpreserveKey(&GUID_TOGGLE_PUNCTUATION, &punctuation_key()) };
}
