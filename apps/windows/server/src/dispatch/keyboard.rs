//! 脚本的 `cloudime.keyboard.press(chord)`：用 Windows `SendInput` 模拟一次组合键。
//!
//! 只支持「修饰键 + 一个普通键」的简单组合（脚本翻译窗口用 `ctrl+c` / `ctrl+enter` 够用）。
//! 调用发生在脚本工人线程；发送前会短暂等待用户松开当前按住的修饰键，避免把
//! `Ctrl+Alt+Y` 里的 Alt 带进要发送的 `Ctrl+C`。

use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN,
    VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};

/// 等用户松开修饰键的上限：快捷键按下时 Ctrl / Alt 还按着，直接注入会变成 Ctrl+Alt+C。
const MODIFIER_RELEASE_WAIT: Duration = Duration::from_millis(450);

/// 模拟一个组合键，例如 `ctrl+c`、`ctrl+enter`。成功返回 `Ok`，失败返回给脚本看的原因。
pub(crate) fn press(chord: &str) -> Result<(), String> {
    let (modifiers, key) = parse_chord(chord)?;
    wait_for_modifiers_to_release(&modifiers);

    let mut inputs = Vec::with_capacity(modifiers.len() * 2 + 2);
    for modifier in &modifiers {
        inputs.push(input(*modifier, false));
    }
    inputs.push(input(key, false));
    inputs.push(input(key, true));
    for modifier in modifiers.iter().rev() {
        inputs.push(input(*modifier, true));
    }

    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(format!(
            "模拟按键失败：SendInput 只发出 {sent}/{} 个事件（目标窗口可能没在前台）",
            inputs.len()
        ));
    }
    // 让前台应用把复制动作处理完，脚本随后读剪贴板才能看到新内容。
    // 某些应用 / 输入法处理 WM_KEYDOWN 是异步的，固定短等不够；等到应用把复制动作处理完。
    std::thread::sleep(Duration::from_millis(400));
    Ok(())
}

fn parse_chord(chord: &str) -> Result<(Vec<VIRTUAL_KEY>, VIRTUAL_KEY), String> {
    let mut modifiers = Vec::new();
    let mut key = None;
    for part in chord
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some(modifier) = modifier_key(part) {
            modifiers.push(modifier);
        } else if key.is_some() {
            return Err(format!("组合键 {chord:?} 里不止一个普通键"));
        } else {
            key = Some(named_key(part).ok_or_else(|| {
                format!("组合键里的键不认识：{part:?}（支持 c / enter / tab / esc / 方向键等）")
            })?);
        }
    }
    let key = key.ok_or_else(|| format!("组合键 {chord:?} 缺少普通键"))?;
    if modifiers.contains(&key) {
        return Err(format!("组合键 {chord:?} 的普通键不能同时是修饰键"));
    }
    Ok((modifiers, key))
}

fn modifier_key(name: &str) -> Option<VIRTUAL_KEY> {
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(VK_CONTROL),
        "alt" | "menu" => Some(VK_MENU),
        "shift" => Some(VK_SHIFT),
        "win" | "meta" | "super" => Some(VK_LWIN),
        _ => None,
    }
}

fn named_key(name: &str) -> Option<VIRTUAL_KEY> {
    let lower = name.to_ascii_lowercase();
    if lower.len() == 1 {
        let byte = lower.as_bytes()[0].to_ascii_uppercase();
        if byte.is_ascii_alphanumeric() {
            return Some(VIRTUAL_KEY(byte as u16));
        }
    }
    Some(match lower.as_str() {
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "esc" | "escape" => VK_ESCAPE,
        "space" => VK_SPACE,
        "backspace" | "back" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "insert" | "ins" => VK_INSERT,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        _ => return None,
    })
}

fn wait_for_modifiers_to_release(modifiers: &[VIRTUAL_KEY]) {
    if modifiers.is_empty() {
        return;
    }
    let start = Instant::now();
    while start.elapsed() < MODIFIER_RELEASE_WAIT {
        let held = modifiers
            .iter()
            .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0);
        if !held {
            return;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

fn input(key: VIRTUAL_KEY, released: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if released {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_parse_modifiers_and_keys() {
        let (modifiers, key) = parse_chord("ctrl+alt+y").unwrap();
        assert_eq!(modifiers, [VK_CONTROL, VK_MENU]);
        assert_eq!(key, VIRTUAL_KEY(0x59));

        let (modifiers, key) = parse_chord("ctrl+enter").unwrap();
        assert_eq!(modifiers, [VK_CONTROL]);
        assert_eq!(key, VK_RETURN);
    }

    #[test]
    fn invalid_chords_are_rejected() {
        assert!(parse_chord("ctrl").is_err());
        assert!(parse_chord("ctrl+a+b").is_err());
        assert!(parse_chord("ctrl+?").is_err());
    }
}
