//! 单击中 / 英切换键的判定，喂的是击键 sink 的 `OnTestKeyDown` / `OnTestKeyUp`（被吃掉的键也经过它们，
//! 与 `WH_KEYBOARD` 钩子不同）。按下切换键到抬起之间没插进别的键，就是一次单击。
//!
//! 切换键来自 `[shortcut] switch_mode`，单击 Shift / 单击 Ctrl 可以都勾；Ctrl + Alt + Space 是组合键，走保留键。
//! 系统热键（如 Ctrl + Space）的第二个键被系统截走、到不了这里，看起来就像单击了 Ctrl：
//! 系统热键生效时调 [`KeyTap::cancel`] 作废这次按下。
//!
//! **不记「现在有几个别的键按着」**：TSF 会对同一个键调 `OnTestKeyDown` 与 `OnKeyDown` 各一次、
//! 抬起侧却未必成对（注入键、别的钩子截走的键都只有一边），计数一旦漂高就再也回不到 0，
//! 表现是**中英再也切不了**（真机上踩过）。现在改成：装填之后只要别的键有**按下或抬起**就作废，
//! 只有一个 Cell<Option<…>>，没有会漂的状态。真机上另踩过：敲 `Shift + 标点` 时手滑先按下标点、
//! Shift 晚一拍才按下去（标点 down → Shift down → 标点 up → Shift up），标点那次抬起作废掉，
//! Shift 抬起不算单击；按住超过 [`MAX_TAP`] 的也不算「按下即松开」。
//!
//! 还有一种漏网（2026-10-06 从 DLL 日志抓到，`Shift + "` 会误切中英）：**TSF 对修饰键的投递顺序不保证**——
//! 标点那一键可能先到、Shift 的按下通知晚到，于是「别的键按下就作废」早了一步、作废完 Shift 又被装填，
//! 抬起时就成了干净的一次单击。兜法是另外记住「这个 Shift 被当修饰键用过」：别的键按下时如果
//! 切换键**正物理按着**（[`KeyTap::key_down`] 的 `switch_held`），抬起就不算单击。

use std::cell::Cell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::LPARAM;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_CONTROL, VK_LCONTROL, VK_LSHIFT, VK_RCONTROL, VK_RSHIFT, VK_SHIFT,
};

use cloudime_platform::{SwitchKey, SwitchKeys};

/// 一次单击最长按多久：按住更久多半是组合键按住不放，不算「按下即松开」。
const MAX_TAP: Duration = Duration::from_millis(800);

#[derive(Default)]
pub(crate) struct KeyTap {
    /// 按下了哪个切换键、之后还没有别的键动过。
    pressed: Cell<Option<SwitchKey>>,

    /// 上面那次按下发生在什么时候（配合 [`MAX_TAP`]）。
    armed_at: Cell<Option<Instant>>,

    /// 按住期间被当成修饰键用过（`Shift` 按着又敲了别的键）：抬起不算单击。
    /// 光靠「别的键按下就作废」不够——TSF 可能把 Shift 的按下通知排在别的键之后（见文件头）。
    held_as_modifier_at: Cell<Option<Instant>>,
}

impl KeyTap {
    /// 任一键按下。`lparam` 第 30 位是按下前的状态（1 = 自动重复，不算新按下）。
    /// `switch_held`：此刻勾着的切换键**正物理按着**没有（调用方查 `GetKeyState`）——
    /// 别的键按下时它若为真，说明那个切换键是被当修饰键用的，抬起时不算单击。
    pub(crate) fn key_down(&self, vk: u32, lparam: LPARAM, keys: SwitchKeys, switch_held: bool) {
        let repeat = (lparam.0 >> 30) & 1 != 0;
        let Some(key) = tap_key(keys, vk) else {
            // 别的键一动就作废（按下、自动重复都算）
            if switch_held {
                self.held_as_modifier_at.set(Some(Instant::now()));
            }
            self.cancel();
            return;
        };
        if repeat {
            return;
        }
        // 两个切换键一起按（Ctrl + Shift 是系统换布局的键）不算单击
        let pressed = match self.pressed.get() {
            Some(other) if other != key => None,
            _ => Some(key),
        };
        self.pressed.set(pressed);
        self.armed_at.set(pressed.map(|_| Instant::now()));
    }

    /// 任一键抬起；切换键单独抬起返回 `true`，一次抬起只算一次。
    ///
    /// `switch_held`：这个切换键此刻**还物理按着**没有。TSF 会把同一次按下的抬起也报两遍、还可能夹着
    /// 一条重复的按下；第一遍（键还按着）必须先当没看见——否则它会把 `held_as_modifier` 清掉，
    /// 紧接着那条重复的按下再装填，最后一条抬起就成了「干净」的单击（真机上 `Shift + "` 误切的直接原因）。
    pub(crate) fn key_up(&self, vk: u32, keys: SwitchKeys, switch_held: bool) -> bool {
        let Some(key) = tap_key(keys, vk) else {
            // 别的键抬起也作废：先按下标点、Shift 晚一拍才按下的手滑就是这样兜住的
            self.cancel();
            return false;
        };
        if switch_held {
            return false;
        }
        let tapped = self.pressed.get() == Some(key)
            && !self.held_as_modifier()
            && self
                .armed_at
                .get()
                .is_some_and(|at| at.elapsed() <= MAX_TAP);
        self.pressed.set(None);
        self.armed_at.set(None);

        tapped
    }

    /// 这个切换键刚被当修饰键用过（在 [`MAX_TAP`] 之内）。
    fn held_as_modifier(&self) -> bool {
        self.held_as_modifier_at
            .get()
            .is_some_and(|at| at.elapsed() <= MAX_TAP)
    }

    /// 作废正按着的切换键（按下之后发生了别的事，这次抬起不算单击）。
    pub(crate) fn cancel(&self) {
        self.pressed.set(None);
        self.armed_at.set(None);
    }
}

/// 这个虚拟键码是不是勾着的单击切换键（左右两个都算）；组合键 Ctrl + Alt + Space 不走单击判定。
fn tap_key(keys: SwitchKeys, vk: u32) -> Option<SwitchKey> {
    let is = |codes: [u16; 3]| codes.iter().any(|code| u32::from(*code) == vk);
    if keys.shift && is([VK_SHIFT.0, VK_LSHIFT.0, VK_RSHIFT.0]) {
        Some(SwitchKey::Shift)
    } else if keys.control && is([VK_CONTROL.0, VK_LCONTROL.0, VK_RCONTROL.0]) {
        Some(SwitchKey::Control)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按下再抬起的 lparam（第 30 位为 0 表示新按下）。
    const DOWN: LPARAM = LPARAM(0);
    const VK_SHIFT_LEFT: u32 = 0xA0;
    const VK_CONTROL_LEFT: u32 = 0xA2;

    fn only(key: SwitchKey) -> SwitchKeys {
        SwitchKeys::NONE.with(key, true)
    }

    #[test]
    fn shift_tap_fires_only_when_nothing_else_interrupts() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
        assert!(tap.key_up(VK_SHIFT_LEFT, keys, false));
        // 一次抬起只算一次
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));

        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
        tap.key_down(0x41, DOWN, keys, false); // 中间插了一个 A
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));
    }

    /// 手滑顺序：先按下标点、Shift 晚一拍才按下去，松开 Shift 不算单击。
    #[test]
    fn a_key_pressed_before_shift_suppresses_the_tap() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_down(0xBC, DOWN, keys, false); // 先敲 `,`
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false); // Shift 晚一拍
        tap.key_up(0xBC, keys, false);
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));
    }

    /// TSF 对修饰键的投递顺序不保证：先看到 `"`（此时 Shift 物理按着），Shift 的按下通知晚到，
    /// 抬起时不能算单击（真机上 `Shift + "` 会误切中英，见文件头）。
    /// TSF 会把同一次按下的抬起报两遍：第一遍时键**还按着**，要先当没看见（也不清标记），
    /// 紧随其后的重复按下才不会被当成一次新的单击——真机上 `Shift + "` 误切的直接原因。
    #[test]
    fn an_up_while_the_switch_key_is_still_held_does_not_clear_the_suppression() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false); // Shift 按下（装填）
        tap.key_down(0xDE, DOWN, keys, true); // Shift + " ：标记「被当修饰键用过」
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, true)); // 幽灵抬起（键还按着）：不作数、不清标记
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, true); // 重复的按下（重新装填）
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false)); // 真抬起：标记还在 → 不算单击
        // 标记有时限（`MAX_TAP`）：这段时间里再单击也不算
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));
    }

    #[test]
    fn a_key_typed_while_the_switch_key_is_held_suppresses_the_tap() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        // 双引号先到，这时 Shift 已经按着了
        tap.key_down(0xDE, DOWN, keys, true);
        // Shift 的按下通知晚一拍才到，这条会把它装填起来
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));
        // 标记有时限（`MAX_TAP`）：这段时间里再单击也不算——宁可漏一次切换，也不误切
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, true);
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));
    }

    /// 只有一个「按下」、没有对应「抬起」的键（注入键 / 双份通知都会这样）不该把中英卡死。
    /// 老实现用计数，这种键一漏，计数就回不到 0，单击中英切换键再也认不出来。
    #[test]
    fn an_unbalanced_key_down_does_not_block_later_taps() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        tap.key_down(0x41, DOWN, keys, false);
        tap.key_down(0x41, DOWN, keys, false); // 同一键的第二份通知
        tap.key_up(0x41, keys, false);
        // 下面这次单击照样算
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
        assert!(tap.key_up(VK_SHIFT_LEFT, keys, false));
    }

    #[test]
    fn only_checked_keys_fire() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Control);
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));
        tap.key_down(VK_CONTROL_LEFT, DOWN, keys, false);
        assert!(tap.key_up(VK_CONTROL_LEFT, keys, false));

        // 一个都不勾、只勾组合键：修饰键单击都不算
        for keys in [SwitchKeys::NONE, only(SwitchKey::CtrlAltSpace)] {
            tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
            assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));
            tap.key_down(VK_CONTROL_LEFT, DOWN, keys, false);
            assert!(!tap.key_up(VK_CONTROL_LEFT, keys, false));
        }
    }

    #[test]
    fn both_taps_work_when_both_are_checked_but_not_together() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift).with(SwitchKey::Control, true);
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
        assert!(tap.key_up(VK_SHIFT_LEFT, keys, false));
        tap.key_down(VK_CONTROL_LEFT, DOWN, keys, false);
        assert!(tap.key_up(VK_CONTROL_LEFT, keys, false));

        // Ctrl + Shift 一起按：谁抬起都不算
        tap.key_down(VK_CONTROL_LEFT, DOWN, keys, false);
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));
        assert!(!tap.key_up(VK_CONTROL_LEFT, keys, false));
    }

    #[test]
    fn a_system_hotkey_cancels_the_pending_tap() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Control);
        // Ctrl + Space 的 Space 被系统截走，这里只看到 Ctrl 按下又抬起
        tap.key_down(VK_CONTROL_LEFT, DOWN, keys, false);
        tap.cancel();
        assert!(!tap.key_up(VK_CONTROL_LEFT, keys, false));
    }

    #[test]
    fn auto_repeat_does_not_rearm() {
        let tap = KeyTap::default();
        let keys = only(SwitchKey::Shift);
        // 第 30 位为 1：自动重复，不算新按下
        let repeat = LPARAM(1 << 30);
        tap.key_down(VK_SHIFT_LEFT, DOWN, keys, false);
        assert!(tap.key_up(VK_SHIFT_LEFT, keys, false));
        tap.key_down(VK_SHIFT_LEFT, repeat, keys, false);
        assert!(!tap.key_up(VK_SHIFT_LEFT, keys, false));
    }
}
