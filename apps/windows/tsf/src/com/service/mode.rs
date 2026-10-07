//! 输入法状态：全局一份，存在 Server 那边，所有应用共用。
//! 用户在这里切了就报给 Server（[`TextService_Impl::set_mode`]）；激活、得到焦点和每隔几拍的轮询
//! 向 Server 取当前状态跟上（[`TextService_Impl::sync_mode_from_server`]）。切状态先把组着的内容落定；
//! 指示器走语言栏按钮 + 转换模式 / 输入法开关两条 compartment；用户点任务栏或按系统 Ctrl + Space
//! 由 compartment 回调反向同步。切换键（固定单击 Shift）与内置英文模式开关由 Server 经协议下发
//! （[`TextService_Impl::apply_input_settings`]），设置里已不再提供这两项、Server 恒发固定值，DLL 不读配置文件。

use std::time::Instant;

use windows::Win32::UI::TextServices::{ITfKeystrokeMgr, ITfLangBarItemMgr};
use windows::core::Interface;

use cloudime_platform::SwitchKeys;
use cloudime_platform::protocol::{InputMode, InputSettings};

use super::TextService_Impl;
use crate::com::context;
use crate::com::key::preserved;
use crate::com::log::log;
use crate::com::mode::{self, ModeButton, sink};

/// 激活后多久内忽略系统写回的转换模式：msctf 在 TIP 激活后约 200–300 ms 会把 profile 存的
/// 转换模式写回 compartment，不忽略的话它会被当成用户点了任务栏、改掉全局模式。
pub(super) const CONVERSION_RESTORE_GUARD: std::time::Duration =
    std::time::Duration::from_millis(1200);

impl TextService_Impl {
    /// 应用中英模式的两项设置：激活时与收到下发值时都走这里。两项由 Server 经协议下发，
    /// 设置里已不再提供（切换键恒为单击 Shift、内置英文模式恒开），DLL 只管照做。
    ///
    /// 内置英文模式开关在运行中翻转时，语言栏的中 / 英按钮与转换模式回调跟着登记 / 撤掉（激活时由 `Activate`
    /// 自己按开关登记，这里只管激活之后的变化），协议值变了不用切走再切回输入法。
    pub(super) fn apply_mode_settings(&self, english_mode: bool, switch_keys: SwitchKeys) {
        let was_enabled = self.mode_state.enabled();
        self.mode_state.set_settings(english_mode, switch_keys);
        // 关掉内置英文模式时立刻回中文，别停在一个再也切不回去的英文状态（Server 那份也跟着回中文）。
        if !english_mode && self.mode_state.english() {
            self.mode_state.set_english(false);
            self.refresh_mode_indicator();
        }
        if was_enabled != english_mode && self.is_active() {
            if english_mode {
                log("打开了内置英文模式：登记中 / 英按钮");
                self.add_lang_bar_item();
                self.advise_mode_sinks();
            } else {
                log("关掉了内置英文模式：撤掉中 / 英按钮");
                self.unadvise_mode_sinks();
                self.remove_lang_bar_item();
            }
        }
        self.sync_switch_preserved_key(switch_keys.ctrl_alt_space);
        self.sync_preserved_hotkeys();
    }

    /// 两个固定内置热键（Ctrl + Alt + . 简 / 繁、Ctrl + Alt + , 标点）登记为保留键，只登记一次。
    fn sync_preserved_hotkeys(&self) {
        if self.hotkeys_preserved.get() {
            return;
        }
        let Some(thread_mgr) = self.thread_mgr.borrow().clone() else {
            return;
        };
        let Ok(keystroke) = thread_mgr.cast::<ITfKeystrokeMgr>() else {
            return;
        };
        match preserved::register_hotkeys(&keystroke, self.client_id.get()) {
            Ok(()) => {
                self.hotkeys_preserved.set(true);
                log("内置热键 Ctrl + Alt + . / Ctrl + Alt + , 已登记为保留键");
            }
            Err(error) => log(&format!("登记内置热键失败: {error}")),
        }
    }

    /// `Activate` 是否已经走完（[`super::ACTIVE`] 在它末尾才设）。
    fn is_active(&self) -> bool {
        super::ACTIVE.with(|active| active.borrow().is_some())
    }

    /// 应用 Server 下发的按键行为设置：`OpenSession` 的回包给一次，之后每一拍 `SyncMode` 也都带着。
    /// 值没变就什么都不做，所以设置窗口改完在下一拍（约 320 ms）生效，不用切走再切回输入法。
    pub(super) fn apply_input_settings(&self, input: InputSettings) {
        if self.input_settings.get() == Some(input) {
            return;
        }
        self.input_settings.set(Some(input));
        log(&format!(
            "按键行为设置：中英切换键 {}，内置英文模式 {}，Shift 字母进组句 {}",
            input.switch_mode.describe(),
            input.english_mode,
            input.shift_letter_compose
        ));
        self.apply_mode_settings(input.english_mode, input.switch_mode);
        // 「不处于输入状态时自动禁用输入法」：刚打开开关（且当前就不在输入状态）时立刻生效，
        // 关掉开关时把自己之前关的放回去。
        self.sync_auto_disable();
    }

    /// 「不处于输入状态时自动禁用输入法」（`[debugging] auto_disable_without_text_input`）：
    /// 焦点不在可输入文本区域时按「禁用」走（按键原样放行、语言栏图标「禁」、Server 收起状态条），
    /// 回到文本区域再回到原来的中 / 英（`ModeState::resume_mode` 记着）。
    ///
    /// **不写系统那条「输入法开 / 关」**（见 [`Self::refresh_mode_indicator`] 里的 `auto_disabled` 判断）：
    /// 写关之后系统就不再把文档焦点给我们了，`ITfThreadMgr::GetFocus` 也拿不到东西，焦点回到文本区域时
    /// 我们收不到任何恢复的依据——真机表现就是「禁用之后回不来」。只在本进程里按禁用走就没这个问题。
    pub(crate) fn sync_auto_disable(&self) {
        let enabled = self
            .input_settings
            .get()
            .is_some_and(|input| input.auto_disable_without_text_input);
        let disabled = enabled
            && !self
                .thread_mgr
                .borrow()
                .as_ref()
                .is_some_and(context::in_text_input);
        if disabled == self.auto_disabled.get() {
            return;
        }
        self.auto_disabled.set(disabled);
        log(&format!(
            "不处于输入状态：自动{}",
            if disabled { "禁用" } else { "恢复" }
        ));
        // 与系统 Ctrl + Space 走同一条路（落定拼音、改模式、报给 Server），只是不动系统开关。
        self.follow_system_disabled(disabled);
    }

    /// Ctrl + Alt + Space 是组合键、走 TSF 保留键（与「翻译选中文字」同一套）；没勾就撤掉登记，免得白占着。
    fn sync_switch_preserved_key(&self, want: bool) {
        if want == self.switch_preserved.get() {
            return;
        }
        let Some(thread_mgr) = self.thread_mgr.borrow().clone() else {
            return;
        };
        let Ok(keystroke) = thread_mgr.cast::<ITfKeystrokeMgr>() else {
            return;
        };
        if want {
            match preserved::register_switch_mode(&keystroke, self.client_id.get()) {
                Ok(()) => {
                    self.switch_preserved.set(true);
                    log("中英切换键 Ctrl + Alt + Space 已登记为保留键");
                }
                Err(error) => log(&format!("登记 Ctrl + Alt + Space 切换键失败: {error}")),
            }
        } else {
            preserved::unregister_switch_mode(&keystroke);
            self.switch_preserved.set(false);
        }
    }

    /// 停用时撤掉保留键登记（切换键与两个内置热键）。
    pub(super) fn drop_switch_preserved_key(&self, keystroke: &ITfKeystrokeMgr) {
        if self.switch_preserved.replace(false) {
            preserved::unregister_switch_mode(keystroke);
        }
        if self.hotkeys_preserved.replace(false) {
            preserved::unregister_hotkeys(keystroke);
        }
    }

    /// 用户在这个应用里切了模式（切换键、语言栏按钮、右键菜单）：改状态、刷指示器，报给 Server 成为全局状态。
    pub(super) fn set_mode(&self, mode: InputMode) {
        if self.switch_mode(mode) {
            self.refresh_mode_indicator();
            self.report_mode();
        }
    }

    /// 按布尔切中英（清掉禁用）。
    pub(super) fn set_english_mode(&self, english: bool) {
        self.set_mode(if english {
            InputMode::English
        } else {
            InputMode::Chinese
        });
    }

    /// 跟上 Server 的全局状态（别的应用切过、点了悬浮状态条）：改状态、刷指示器，不再回报。
    pub(super) fn adopt_mode(&self, mode: InputMode) {
        self.switch_source.set("跟随 Server 全局状态");
        if mode != self.mode_state.mode() && self.switch_mode(mode) {
            self.refresh_mode_indicator();
        }
    }

    /// 向 Server 取一次全局模式跟上，顺路取回按键行为设置；没连着就什么都不做。
    pub(super) fn sync_mode_from_server(&self) {
        let (in_text_input, caps) = super::text_focus_state();
        let reply = self
            .engine
            .borrow_mut()
            .as_mut()
            .map(|client| client.sync_mode(in_text_input, caps));
        match reply {
            Some(Ok(reply)) => {
                self.apply_input_settings(reply.input);
                if let Some(mode) = reply.mode {
                    self.adopt_mode(mode);
                }
            }
            Some(Err(error)) => {
                log(&format!("同步中英模式失败，断开，下一键重连: {error}"));
                self.disconnect();
            }
            None => {}
        }
    }

    /// 用户点了任务栏的中 / 英：只更新按钮，不回写 compartment（在它自己的 `OnChange` 里写会报
    /// 0x8000FFFF），报给 Server。
    fn follow_system_mode(&self, english: bool) {
        self.switch_source
            .set("系统转换模式（任务栏 / 系统快捷键）");
        let mode = if english {
            InputMode::English
        } else {
            InputMode::Chinese
        };
        if self.switch_mode(mode) {
            self.mode_state.notify();
            self.report_mode();
        }
    }

    /// 系统 Ctrl + Space 翻了输入法开关：关 = 禁用、开 = 回到禁用前的中 / 英模式。先把组着的拼音落定。
    fn follow_system_disabled(&self, disabled: bool) {
        self.commit_pending();
        let mode = if disabled {
            InputMode::Disabled
        } else {
            self.mode_state.resume_mode()
        };
        self.mode_state.set_mode(mode);
        log(if disabled {
            "输入法已禁用（Ctrl + Space）"
        } else {
            "输入法已启用（Ctrl + Space）"
        });
        self.mode_state.notify();
        self.report_mode();
    }

    /// 先把组着的内容原样落定，再改状态。返回是否真的改了。
    ///
    /// 下发的内置英文模式为关时（当前恒为开）不许切到英文——切换键、语言栏按钮、悬浮状态条、任务栏
    /// 四条入口都汇到这里，一处拦住就再也进不了英文模式（见 issue #81）；禁用不受它影响。
    fn switch_mode(&self, mode: InputMode) -> bool {
        if !self.mode_state.enabled() && mode.english() {
            log("内置英文模式已关闭，忽略切到英文");
            return false;
        }
        if mode == self.mode_state.mode() {
            return false;
        }
        self.commit_pending();
        self.mode_state.set_mode(mode);
        let source = self.switch_source.get();
        log(&match mode {
            InputMode::Chinese => format!("切到中文模式（来源：{source}）"),
            InputMode::English => format!("切到英文模式（来源：{source}）"),
            InputMode::Disabled => format!("切到禁用（来源：{source}）"),
        });
        true
    }

    /// 语言栏按钮换图标，写转换模式与输入法开关两条 compartment：中英跟着模式、开关跟着禁用与否，
    /// 系统的 Ctrl + Space 翻的就是它，对上了才能一按就切。
    pub(super) fn refresh_mode_indicator(&self) {
        self.mode_state.notify();
        let current = self.mode_state.mode();
        if let Some(thread_mgr) = self.thread_mgr.borrow().as_ref() {
            mode::set_indicator(thread_mgr, self.client_id.get(), current.english());
            // 「不处于输入状态时自动禁用」期间**不写**这条：写关之后系统不再给我们文档焦点，
            // 等焦点回到文本区域时也就没有恢复的依据了（见 `sync_auto_disable`）。
            // 图标仍然跟着模式显示「禁」，Server 那边也会跟着收起状态条。
            if !self.auto_disabled.get() {
                mode::set_keyboard_open(thread_mgr, self.client_id.get(), !current.disabled());
            }
        }
    }

    /// 本线程得到 / 失去键盘焦点。得到：补连接，跟上全局模式（别的应用里可能刚切过）。
    pub(super) fn set_thread_focus(&self, foreground: bool) {
        if self.shared.foreground() == foreground {
            return;
        }
        self.shared.set_foreground(foreground);
        if foreground {
            self.ensure_connected();
            self.sync_mode_from_server();
        }
    }

    /// 把用户切出的状态报给 Server，成为全局状态。
    pub(super) fn report_mode(&self) {
        let mode = self.mode_state.mode();
        if let Some(client) = self.engine.borrow_mut().as_mut()
            && let Err(error) = client.mode_changed(mode)
        {
            log(&format!("上报输入法状态失败: {error}"));
        }
    }

    /// 挂上「转换模式」与「输入法开 / 关」的回调。
    pub(super) fn advise_mode_sinks(&self) {
        let Some(thread_mgr) = self.thread_mgr.borrow().clone() else {
            return;
        };
        *self.mode_sinks.borrow_mut() = sink::advise(&thread_mgr);
    }

    pub(super) fn unadvise_mode_sinks(&self) {
        sink::unadvise(std::mem::take(&mut *self.mode_sinks.borrow_mut()));
    }

    /// 用户在任务栏点了中 / 英：读回 `NATIVE` 位，与当前不同才跟着切（相同是自己那次写触发的，防回环）。
    ///
    /// 激活后的最初一瞬不算：那时 msctf 在把 profile 存的转换模式写回来，不是用户操作。
    pub(super) fn sync_from_conversion_mode(&self) {
        if let Some(until) = self.conversion_guard_until.get()
            && Instant::now() < until
        {
            log("激活后忽略一次系统写回的转换模式（msctf 的 profile 恢复，不是用户操作）");
            return;
        }
        let Some(thread_mgr) = self.thread_mgr.borrow().clone() else {
            return;
        };
        let Ok(compartment) = mode::conversion_compartment(&thread_mgr) else {
            return;
        };
        let english = mode::is_english(&compartment);
        if english != self.mode_state.english() {
            log(&format!(
                "转换模式变了（任务栏 / 系统快捷键），english={english}"
            ));
            self.follow_system_mode(english);
        }
    }

    /// 系统的「输入法/非输入法切换」（缺省 Ctrl + Space）翻了输入法开关：关 = 禁用、开 = 中文。
    /// 这个热键的 Space 被系统截走，按着的 Ctrl 抬起时别再当成单击。
    pub(super) fn sync_from_keyboard_open(&self) {
        self.key_tap.cancel();
        let Some(thread_mgr) = self.thread_mgr.borrow().clone() else {
            return;
        };
        let Ok(compartment) = mode::openclose_compartment(&thread_mgr) else {
            return;
        };
        let disabled = !mode::is_keyboard_open(&compartment);
        if disabled != self.mode_state.disabled() {
            log(&format!(
                "输入法开关变了（系统 Ctrl + Space），disabled={disabled}"
            ));
            self.follow_system_disabled(disabled);
        }
    }

    pub(super) fn add_lang_bar_item(&self) {
        let button = ModeButton::create(self.mode_state.clone());
        if let Some(thread_mgr) = self.thread_mgr.borrow().as_ref() {
            match thread_mgr.cast::<ITfLangBarItemMgr>() {
                Ok(mgr) => {
                    if let Err(error) = unsafe { mgr.AddItem(&button) } {
                        log(&format!("登记中英指示器失败: {error}"));
                    }
                }
                Err(error) => log(&format!("取语言栏管理器失败: {error}")),
            }
        }
        *self.mode_button.borrow_mut() = Some(button);
    }

    pub(super) fn remove_lang_bar_item(&self) {
        if let Some(button) = self.mode_button.borrow_mut().take()
            && let Some(thread_mgr) = self.thread_mgr.borrow().as_ref()
            && let Ok(mgr) = thread_mgr.cast::<ITfLangBarItemMgr>()
        {
            let _ = unsafe { mgr.RemoveItem(&button) };
        }
    }
}
