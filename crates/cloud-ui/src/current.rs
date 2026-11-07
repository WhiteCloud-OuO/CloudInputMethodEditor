//! 线程本地的「当前窗口」：给不接收 owner 参数的同步 API（如 [`crate::MessageDialog`]）用。
//!
//! 只有**独立窗口**在 UI 线程上处理自己的回调 / 绘制时才会进入这个上下文；窗口之外没有当前窗口。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::HWND;

use crate::painter::SharedPainter;
use crate::text_edit::TextHostWindow;
use cloudime_render::Color;

/// 当前正在处理 UI 工作的独立窗口。
#[derive(Clone)]
pub(crate) struct CurrentWindow {
    /// 独立窗口句柄；对话框以它为 owner。
    pub(crate) hwnd: HWND,

    /// 这块窗口共享的渲染器。
    pub(crate) painter: SharedPainter,

    /// 独立窗口是不是置顶；对话框跟随。
    pub(crate) top_most: bool,

    /// 这块窗口的 RichEdit 宿主：对话框打开时直接挡住它们。
    /// 这些是独立顶层窗口，所有者的 `EnableWindow(false)` 管不到。
    pub(crate) edit_hosts: Vec<Rc<TextHostWindow>>,

    /// 跟着窗口一起平移的其它顶层窗口（例如蒙版）：dialog 拖拽带着所有者走时用。
    pub(crate) followers: Vec<HWND>,
}

thread_local! {
    static CURRENT: RefCell<Option<CurrentWindow>> = const { RefCell::new(None) };
    /// 最近一帧应用窗口的背景色：`MessageDialog` 在 `update` 里调时拿它判断深浅主题。
    static BACKGROUND: Cell<Option<Color>> = const { Cell::new(None) };
}

/// 进入一个独立窗口的 UI 上下文；`Drop` 时恢复上一层。
pub(crate) struct CurrentGuard {
    previous: Option<CurrentWindow>,
}

impl Drop for CurrentGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            *current.borrow_mut() = self.previous.take();
        });
    }
}

/// 把 `hwnd` 设为当前窗口，直到返回的 guard 被丢弃。
pub(crate) fn enter(
    hwnd: HWND,
    painter: SharedPainter,
    top_most: bool,
    edit_hosts: Vec<Rc<TextHostWindow>>,
    followers: Vec<HWND>,
) -> CurrentGuard {
    CURRENT.with(|current| {
        let previous = current.replace(Some(CurrentWindow {
            hwnd,
            painter,
            top_most,
            edit_hosts,
            followers,
        }));
        CurrentGuard { previous }
    })
}

/// 当前窗口；没有（不在独立窗口的 UI 回调里）时为 `None`。
pub(crate) fn get() -> Option<CurrentWindow> {
    CURRENT.with(|current| current.borrow().clone())
}

/// 记下最近一帧应用窗口的背景色（消息对话框据此选深 / 浅主题）。
pub(crate) fn set_background(color: Option<Color>) {
    BACKGROUND.with(|background| background.set(color));
}

/// 最近一帧应用窗口的背景色。
pub(crate) fn background() -> Option<Color> {
    BACKGROUND.with(Cell::get)
}
