//! 按键分派用的虚拟键码与字符解析。

use cloudime_platform::protocol::KeyEvent;

pub(crate) const BACK: u32 = 0x08;
pub(crate) const TAB: u32 = 0x09;
pub(crate) const RETURN: u32 = 0x0D;
pub(crate) const ESCAPE: u32 = 0x1B;

/// Delete：表达式计算面板里清空算式（结果跟着回 0）；别处交给应用。
pub(crate) const DELETE: u32 = 0x2E;

/// 空格：组句里选中高亮候选；多释义选择里选高亮那条译文。
pub(crate) const SPACE: u32 = 0x20;

/// Insert：上屏组句里已选的部分、丢掉未选的拼音。
pub(crate) const INSERT: u32 = 0x2D;
pub(crate) const PRIOR: u32 = 0x21;
pub(crate) const NEXT: u32 = 0x22;
pub(crate) const END: u32 = 0x23;
pub(crate) const HOME: u32 = 0x24;
pub(crate) const LEFT: u32 = 0x25;
pub(crate) const UP: u32 = 0x26;
pub(crate) const RIGHT: u32 = 0x27;
pub(crate) const DOWN: u32 = 0x28;

/// 主键盘 `-`（VK_OEM_MINUS）与 `=`（VK_OEM_PLUS）：固定的上一页 / 下一页。
pub(crate) const OEM_MINUS: u32 = 0xBD;
pub(crate) const OEM_PLUS: u32 = 0xBB;

/// 主键盘的反引号（VK_OEM_3）：`Ctrl + 反引号` 上屏翻译 Tip 的译文。
pub(crate) const BACKQUOTE: u32 = 0xC0;

/// 固定的翻页键：主键盘 `-` 上一页、`=` 下一页；返回 -1 / +1。
/// 只认**没按 Shift 的裸键**：`Shift + =` 是 `+`、`Shift + -` 是 `_`，属上档符号，不翻页。
/// 小键盘的 `-` / `=` 不算，PageUp / PageDown 由功能键分派（`input::apply_function_key`）处理。
pub(crate) fn page_key(event: &KeyEvent) -> Option<isize> {
    if event.modifiers.shift {
        return None;
    }
    match event.virtual_key {
        OEM_MINUS => Some(-1),
        OEM_PLUS => Some(1),
        _ => None,
    }
}

/// 敲出来是数字 1–9 的键（选候选用）：按 `character` 认，Shift 出的 `!@#` 不算。
pub(crate) fn digit(event: &KeyEvent) -> Option<usize> {
    match event.character {
        Some(c) => ('1'..='9').contains(&c).then(|| c as usize - '0' as usize),
        None => digit_key(event.virtual_key),
    }
}

/// 敲出来是数字 0–9 的键（展开「更多候选项」时跳页用，`0` 表示第 10 页）：0 不在选词的 [`digit`] 里。
pub(crate) fn page_digit(event: &KeyEvent) -> Option<usize> {
    match event.character {
        Some(c) => c.is_ascii_digit().then(|| c as usize - '0' as usize),
        None => (0x30..=0x39)
            .contains(&event.virtual_key)
            .then(|| (event.virtual_key - 0x30) as usize),
    }
}

/// 主键盘区数字键 1–9 的键码（没有 `character` 时兜底认数字）。
pub(crate) fn digit_key(virtual_key: u32) -> Option<usize> {
    (0x31..=0x39)
        .contains(&virtual_key)
        .then(|| (virtual_key - 0x30) as usize)
}

/// 主键盘或小键盘数字键 1–9 的键码。按住 Ctrl 时 `character` 是控制字符（`\u{1}`），
/// 杀词的组合键只能按键码认。
pub(crate) fn digit_virtual_key(virtual_key: u32) -> Option<usize> {
    match virtual_key {
        0x31..=0x39 => Some((virtual_key - 0x30) as usize),
        0x61..=0x69 => Some((virtual_key - 0x60) as usize),
        _ => None,
    }
}

/// 小键盘区的键（数字与 `* + - . /`）：敲出来的标点一律半角。
pub(crate) fn is_keypad(virtual_key: u32) -> bool {
    (0x60..=0x6F).contains(&virtual_key)
}
