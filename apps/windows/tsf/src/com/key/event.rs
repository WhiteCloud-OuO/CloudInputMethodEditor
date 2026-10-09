//! 把 TSF 送来的虚拟键码翻成协议的 [`KeyEvent`]，以及「组句中哪些键要吃」的判定。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_INSERT,
    VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB,
};

use cloudime_platform::protocol::{KeyEvent, KeyModifiers};

/// 采当前修饰键并解析字符（标点 / 数字使用当前键盘布局）。`english_mode` 是 DLL 记的持久中英模式，随事件带给 Server。
pub(crate) fn to_key_event(vk: u32, english_mode: bool) -> KeyEvent {
    let modifiers = current_modifiers(english_mode);
    KeyEvent::new(
        vk,
        resolve_char(vk, modifiers.shift, modifiers.caps),
        modifiers,
    )
}

pub(crate) fn is_letter(vk: u32) -> bool {
    (0x41..=0x5A).contains(&vk)
}

/// 组句中要吃的功能键：退格 / Tab / 回车 / Esc / 空格 / Insert / Delete / 数字。Tab 由 Router 用来
/// 展开 / 收起「更多候选项」（不再翻页），Delete 由它用来清空表达式计算面板的算式。
pub(crate) fn is_edit(vk: u32) -> bool {
    matches!(
        VIRTUAL_KEY(vk as u16),
        VK_BACK | VK_TAB | VK_RETURN | VK_ESCAPE | VK_SPACE | VK_INSERT | VK_DELETE
    ) || is_digit(vk)
}

/// PageUp/Down、End、Home、方向键。
pub(crate) fn is_nav(vk: u32) -> bool {
    (0x21..=0x28).contains(&vk)
}

fn is_digit(vk: u32) -> bool {
    (0x30..=0x39).contains(&vk)
}

fn current_modifiers(english_mode: bool) -> KeyModifiers {
    KeyModifiers {
        ctrl: key_down(VK_CONTROL),
        shift: key_down(VK_SHIFT),
        alt: key_down(VK_MENU),
        win: key_down(VK_LWIN) || key_down(VK_RWIN),
        caps: key_toggled(VK_CAPITAL),
        english_mode,
    }
}

/// 主动清掉 Caps Lock：补一次 Caps Lock 键，系统会翻转锁定状态。
/// Caps Lock 亮着时单击 Shift 用它（微软拼音：解锁并切英文）。
pub(crate) fn clear_caps_lock() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    };
    let key = |flags: KEYBDINPUT| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: flags },
    };
    let inputs = [
        key(KEYBDINPUT {
            wVk: VK_CAPITAL,
            ..Default::default()
        }),
        key(KEYBDINPUT {
            wVk: VK_CAPITAL,
            dwFlags: KEYEVENTF_KEYUP,
            ..Default::default()
        }),
    ];
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

/// 把一次按键（按下 + 抬起）注入回系统：给「`OnTestKeyDown` 已经声明吃、Server 又没接管」的键
/// 还给应用用。修饰键不用注入 —— 用户手上还按着，系统状态里就是按着的，应用照样看到 `Ctrl+A`。
///
/// **延后一点再注入**：物理键还按着的那一刻注入同名键，系统会把注入的按下当成「自动重复」
/// （`lParam` 的 previous-state 位置位），Chromium 系（Electron）会直接丢掉这个键 —— 表现为
/// 「拦截了又没还回去」。等物理键松开（~150ms）再注入，就不是重复键了。
pub(crate) fn send_key_deferred(vk: u32) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        send_key(vk);
    });
}

pub(crate) fn send_key(vk: u32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC,
        MapVirtualKeyW, SendInput, VIRTUAL_KEY,
    };
    let key = |flags: KEYBDINPUT| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: flags },
    };
    // 带上扫描码：Chromium 系（Electron）按扫描码映射 DOM `code` / 判定按键，只给 wVk 时
    // 它会把注入的键当无效事件丢掉（Firefox 不看扫描码，所以之前只在 Electron 里失效）。
    let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } as u16;
    let inputs = [
        key(KEYBDINPUT {
            wVk: VIRTUAL_KEY(vk as u16),
            wScan: scan,
            ..Default::default()
        }),
        key(KEYBDINPUT {
            wVk: VIRTUAL_KEY(vk as u16),
            wScan: scan,
            dwFlags: KEYEVENTF_KEYUP,
            ..Default::default()
        }),
    ];
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

/// Shift 按下没有。
pub(crate) fn shift_down() -> bool {
    key_down(VK_SHIFT)
}

/// Ctrl 按下没有。
pub(crate) fn ctrl_down() -> bool {
    key_down(VK_CONTROL)
}

/// 高位为 1（返回值为负）表示按下。
fn key_down(vk: VIRTUAL_KEY) -> bool {
    let state = unsafe { GetKeyState(vk.0 as i32) };
    state < 0
}

pub(crate) fn caps_lock_on() -> bool {
    key_toggled(VK_CAPITAL)
}

/// 低位为 1 表示锁定键亮着。
fn key_toggled(vk: VIRTUAL_KEY) -> bool {
    let state = unsafe { GetKeyState(vk.0 as i32) };
    state & 1 != 0
}

/// 字母大小写 = Shift 异或 Caps；数字 / 标点使用系统布局；功能键 `None`。
fn resolve_char(vk: u32, shift: bool, caps: bool) -> Option<char> {
    if is_letter(vk) {
        let lower = (b'a' + (vk - 0x41) as u8) as char;
        return Some(if shift != caps {
            lower.to_ascii_uppercase()
        } else {
            lower
        });
    }
    super::layout::character(vk)
}
