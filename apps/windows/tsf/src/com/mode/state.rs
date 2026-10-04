//! 当前中 / 英模式与语言栏更新回调，文本服务与语言栏按钮（[`super::ModeButton`]）共享。

use core::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::UI::TextServices::{ITfLangBarItemSink, TF_LBI_ICON, TF_LBI_STATUS};

use cloudime_platform::SwitchKeys;
use cloudime_platform::protocol::InputMode;

/// 当前输入法状态（中文 / 英文 / 禁用）+ 语言栏更新回调，文本服务与语言栏按钮共享（STA 单线程）。
pub(crate) struct ModeState {
    /// `true` 是英文模式。
    english: Cell<bool>,

    /// 禁用：不接管任何按键，悬浮状态栏收起，托盘显示「off」。
    disabled: Cell<bool>,

    /// 禁用前的中 / 英模式：重新启用时回到它。
    resume: Cell<InputMode>,

    /// 内置英文模式开关（`[general] english_mode`）：关掉后谁都不许切到英文。
    enabled: Cell<bool>,

    /// 勾着的中英切换键（`[shortcut] switch_mode`），单击判定与语言栏提示用。
    switch_keys: Cell<SwitchKeys>,

    /// 系统登记进来的语言栏更新回调；由 [`super::ModeButton`] 的 `ITfSource` 登记 / 撤销。
    pub(super) sink: RefCell<Option<ITfLangBarItemSink>>,
}

impl ModeState {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            english: Cell::new(false),
            disabled: Cell::new(false),
            resume: Cell::new(InputMode::Chinese),
            enabled: Cell::new(true),
            switch_keys: Cell::new(SwitchKeys::default()),
            sink: RefCell::new(None),
        })
    }

    pub(crate) fn english(&self) -> bool {
        self.english.get()
    }

    pub(crate) fn disabled(&self) -> bool {
        self.disabled.get()
    }

    /// 当前三态。
    pub(crate) fn mode(&self) -> InputMode {
        if self.disabled.get() {
            InputMode::Disabled
        } else if self.english.get() {
            InputMode::English
        } else {
            InputMode::Chinese
        }
    }

    /// 整体设三态（Server 下发 / 从 compartment 读回，两处都走这里）。
    /// 非禁用的模式记下来，重新启用时回到它。
    pub(crate) fn set_mode(&self, mode: InputMode) {
        if !mode.disabled() {
            self.resume.set(mode);
        }
        self.disabled.set(mode.disabled());
        self.english.set(mode.english());
    }

    /// 禁用前的中 / 英模式。
    pub(crate) fn resume_mode(&self) -> InputMode {
        self.resume.get()
    }

    pub(crate) fn set_english(&self, english: bool) {
        self.english.set(english);
    }

    /// 内置英文模式是否可用。
    pub(crate) fn enabled(&self) -> bool {
        self.enabled.get()
    }

    pub(crate) fn switch_keys(&self) -> SwitchKeys {
        self.switch_keys.get()
    }

    /// 激活时按配置设一次。
    pub(crate) fn set_settings(&self, enabled: bool, switch_keys: SwitchKeys) {
        self.enabled.set(enabled);
        self.switch_keys.set(switch_keys);
    }

    /// 通知系统重取图标 / 文字。
    pub(crate) fn notify(&self) {
        if let Some(sink) = self.sink.borrow().as_ref() {
            let _ = unsafe { sink.OnUpdate(TF_LBI_ICON | TF_LBI_STATUS) };
        }
    }
}
