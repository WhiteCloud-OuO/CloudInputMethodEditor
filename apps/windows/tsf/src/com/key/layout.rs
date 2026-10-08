//! 标点 / 数字按当前键盘布局解析，测键时不改变 dead key 状态。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, GetKeyboardState, HKL, MAPVK_VK_TO_VSC, MapVirtualKeyExW, ToUnicodeEx,
};

pub(super) fn character(vk: u32) -> Option<char> {
    // 主键盘数字、小键盘数字与运算符（不解出字符的键 would_eat 会直接放行给应用）、OEM 标点和空格。
    if !matches!(vk, 0x30..=0x39 | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDE | 0x20) {
        return None;
    }
    let mut state = [0; 256];
    unsafe { GetKeyboardState(&mut state) }.ok()?;
    resolve(vk, &state, unsafe { GetKeyboardLayout(0) })
}

fn resolve(vk: u32, state: &[u8; 256], layout: HKL) -> Option<char> {
    let scan = unsafe { MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, Some(layout)) };
    let mut buffer = [0; 8];
    // Windows 10 1607 起，bit 2 可避免测键修改 dead key 状态。
    let count = unsafe { ToUnicodeEx(vk, scan, state, &mut buffer, 1 << 2, Some(layout)) };
    // 仅接受单个 UTF-16 单元；dead key、无映射和较长结果不生成字符。
    if count != 1 {
        return None;
    }
    char::from_u32(u32::from(buffer[0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        KLF_NOTELLSHELL, LoadKeyboardLayoutW, UnloadKeyboardLayout, VK_NUMLOCK, VK_SHIFT,
    };
    use windows::core::{PCWSTR, w};

    /// 这几个用例都要 `LoadKeyboardLayout`：布局一加载就进了当前用户会话（语言栏里多出「英语 / 德语」），
    /// 同名布局拿到的还是同一个 HKL，用完既得卸、又不能和别的用例抢。用一把锁串起来，
    /// 再靠 [`LoadedLayout`] 用完即卸，跑测试就不会往用户的语言列表里留东西。
    static LAYOUT_LOCK: Mutex<()> = Mutex::new(());

    fn layout_lock() -> MutexGuard<'static, ()> {
        LAYOUT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `LoadKeyboardLayoutW` 的 RAII 守卫。加载布局是用户级的副作用，Drop 时卸载；
    /// 若它仍是本线程当前布局则卸不掉，调用方先 `ActivateKeyboardLayout` 还原再让它 Drop。
    struct LoadedLayout(HKL);

    impl LoadedLayout {
        fn load(klid: PCWSTR) -> Self {
            Self(unsafe { LoadKeyboardLayoutW(klid, KLF_NOTELLSHELL) }.unwrap())
        }

        fn hkl(&self) -> HKL {
            self.0
        }
    }

    impl Drop for LoadedLayout {
        fn drop(&mut self) {
            let _ = unsafe { UnloadKeyboardLayout(self.0) };
        }
    }

    #[test]
    fn punctuation_and_digits_follow_layout() {
        let _lock = layout_lock();
        // 只加载、不激活（`LoadedLayout` 会在 Drop 时卸载），不切换用户正在使用的布局。
        let us = LoadedLayout::load(w!("00000409"));
        let de = LoadedLayout::load(w!("00000407"));
        let mut state = [0; 256];
        assert_eq!(resolve(0xBA, &state, us.hkl()), Some(';'));
        assert_eq!(resolve(0x32, &state, de.hkl()), Some('2'));
        assert_eq!(resolve(0x20, &state, us.hkl()), Some(' '));
        state[VK_SHIFT.0 as usize] = 0x80;
        assert_eq!(resolve(0xBA, &state, us.hkl()), Some(':'));
        assert_eq!(resolve(0xBB, &state, us.hkl()), Some('+'));
        assert_eq!(resolve(0xBB, &state, de.hkl()), Some('*'));
        assert_eq!(resolve(0x32, &state, us.hkl()), Some('@'));
        assert_eq!(resolve(0x32, &state, de.hkl()), Some('"'));
    }

    /// 小键盘数字与运算符也要能解出字符：V 模式（表达式模式）里敲小键盘才会进缓冲区。
    /// 关掉 NumLock 时系统上报的 vk 是导航键（VK_INSERT / VK_END…），到不了这里。
    #[test]
    fn keypad_digits_and_operators_resolve() {
        let _lock = layout_lock();
        let us = LoadedLayout::load(w!("00000409"));
        let mut state = [0; 256];
        state[VK_NUMLOCK.0 as usize] = 0x01;
        for (vk, expected) in [
            (0x60, '0'),
            (0x61, '1'),
            (0x62, '2'),
            (0x63, '3'),
            (0x64, '4'),
            (0x65, '5'),
            (0x66, '6'),
            (0x67, '7'),
            (0x68, '8'),
            (0x69, '9'),
            (0x6A, '*'),
            (0x6B, '+'),
            (0x6D, '-'),
            (0x6E, '.'),
            (0x6F, '/'),
        ] {
            assert_eq!(resolve(vk, &state, us.hkl()), Some(expected), "vk={vk:#x}");
        }
    }

    #[test]
    fn dead_key_lookup_does_not_change_next_character() {
        let _lock = layout_lock();
        let intl = LoadedLayout::load(w!("00020409"));
        let state = [0; 256];
        for _ in 0..2 {
            assert_eq!(resolve(0xDE, &state, intl.hkl()), None);
            // 每次预读后立即检查，避免两次 dead key 抵消副作用。
            assert_eq!(resolve(0x45, &state, intl.hkl()), Some('e'));
        }
        assert_eq!(resolve(0, &state, intl.hkl()), None);
        let mut buffer = [0; 8];
        // 与上面的重音检查串行：内核键盘缓冲会让并行测试互相影响。
        let dead = unsafe { ToUnicodeEx(0xDE, 0, &state, &mut buffer, 0, Some(intl.hkl())) };
        let result = resolve(0x31, &state, intl.hkl());
        // 清掉待组合的重音后再断言。
        let count = unsafe { ToUnicodeEx(0x31, 0, &state, &mut buffer, 0, Some(intl.hkl())) };
        assert!(dead < 0);
        assert_eq!(count, 2);
        assert_eq!(result, None);
    }

    #[test]
    fn other_keys_are_not_resolved() {
        for vk in [0x41, 0x70, 0x0D, 0x14] {
            assert_eq!(character(vk), None);
        }
    }

    #[test]
    fn event_reads_thread_layout_and_keyboard_state() {
        use crate::com::key::event::to_key_event;
        use crate::com::service::TextService;
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            ActivateKeyboardLayout, SetKeyboardState, VK_CAPITAL, VK_CONTROL, VK_LSHIFT,
        };
        use windows::Win32::UI::TextServices::ITfKeyEventSink;
        use windows::core::ComObject;

        let original_layout = unsafe { GetKeyboardLayout(0) };
        let mut original_state = [0; 256];
        unsafe { GetKeyboardState(&mut original_state) }.unwrap();
        let _lock = layout_lock();
        // 仅修改测试线程；即使断言失败，也先恢复布局和按键状态。
        let result = std::panic::catch_unwind(|| {
            let restore = unsafe { GetKeyboardLayout(0) };
            let service = ComObject::new(TextService::new());
            let sink: ITfKeyEventSink = service.to_interface();
            for (name, expected) in [(w!("00000409"), '+'), (w!("00000407"), '*')] {
                let layout = LoadedLayout::load(name);
                unsafe { ActivateKeyboardLayout(layout.hkl(), Default::default()) }.unwrap();
                let mut state = [0; 256];
                state[VK_SHIFT.0 as usize] = 0x80;
                state[VK_LSHIFT.0 as usize] = 0x80;
                unsafe { SetKeyboardState(&state) }.unwrap();
                for _ in 0..2 {
                    let event = to_key_event(0xBB, true);
                    assert_eq!(event.character, Some(expected));
                    assert!(event.modifiers.shift);
                    assert!(event.modifiers.english_mode);
                    assert!(
                        unsafe { sink.OnTestKeyDown(None, WPARAM(0xBB), LPARAM(0)) }
                            .unwrap()
                            .as_bool()
                    );
                }
                state[VK_SHIFT.0 as usize] = 0;
                state[VK_LSHIFT.0 as usize] = 0;
                state[VK_CAPITAL.0 as usize] = 1;
                state[VK_CONTROL.0 as usize] = 0x80;
                unsafe { SetKeyboardState(&state) }.unwrap();
                let event = to_key_event(0x31, false);
                assert!(!event.modifiers.shift);
                assert!(event.modifiers.caps);
                assert!(event.modifiers.ctrl);
                assert_eq!(event.virtual_key, 0x31);
                assert_eq!(to_key_event(0x41, false).character, Some('A'));
                // 没在组句时带 Ctrl 的键归应用（记事本的 `Ctrl+A` / `Ctrl+C` 要能用）：
                // `OnTestKeyDown` 返回 false，按键原样交给应用、也不问 Server。
                assert!(
                    !unsafe { sink.OnTestKeyDown(None, WPARAM(0x31), LPARAM(0)) }
                        .unwrap()
                        .as_bool()
                );
                // 先把当前布局还原，`LoadedLayout` 才卸得掉（卸当前布局会失败）。
                unsafe { ActivateKeyboardLayout(restore, Default::default()) }.unwrap();
            }
        });
        unsafe { ActivateKeyboardLayout(original_layout, Default::default()) }.unwrap();
        unsafe { SetKeyboardState(&original_state) }.unwrap();
        result.unwrap();
    }
}
