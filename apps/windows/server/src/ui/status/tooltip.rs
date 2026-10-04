//! 状态条图标的悬停提示：系统 tooltip 控件。
//!
//! 每个按钮挂一个「功能 + 快捷键」的工具，`TTF_SUBCLASS` 让控件自己盯着鼠标；状态条每次重画时
//! 用 [`Tooltip::sync`] 把各按钮的格子与文字同步过来即可，不用自己跟踪悬停。
//! 外观走 comctl32 v6——Server 的 manifest（`build.rs` 的 `new_manifest`）本来就带这个依赖。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TTF_SUBCLASS, TTM_ADDTOOLW,
    TTM_DELTOOLW, TTM_SETMAXTIPWIDTH, TTS_ALWAYSTIP, TTS_NOANIMATE, TTS_NOFADE, TTS_NOPREFIX,
    TTTOOLINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SendMessageW, WINDOW_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PWSTR, w};

/// 提示文字的最大宽度（像素）：太长了系统自己折行。
const MAX_TIP_WIDTH: isize = 420;

/// 状态条上的 tooltip 控件；建不出来为 `None`（只是没有提示，不影响状态条）。
pub(super) struct Tooltip {
    /// tooltip 窗口。
    hwnd: HWND,

    /// 挂着它的状态条窗口（所有工具都挂在这一处）。
    owner: HWND,

    /// 每个工具的文字缓冲：`TTM_ADDTOOLW` 只记指针、不复制，缓冲得一直活着且不搬家
    /// ——`Vec<Vec<u16>>` 扩容只搬外层那截指针，内层缓冲的地址不变。
    texts: RefCell<Vec<Vec<u16>>>,
}

impl Tooltip {
    /// 挂在 `owner`（状态条窗口）上建一个 tooltip。
    pub(super) fn new(owner: HWND) -> Option<Self> {
        // `tooltips_class32` 是通用控件类，得先让 comctl32 注册它（manifest 已让 comctl32 走 v6）。
        let classes = INITCOMMONCONTROLSEX {
            dwSize: core::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        };
        let _ = unsafe { InitCommonControlsEx(&classes) };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("tooltips_class32"),
                w!(""),
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX | TTS_NOANIMATE | TTS_NOFADE),
                0,
                0,
                0,
                0,
                Some(owner),
                None,
                None,
                None,
            )
        }
        .ok()?;
        // 不设最大宽度，长句子会顶成一行、把提示拉得极宽。
        unsafe {
            SendMessageW(
                hwnd,
                TTM_SETMAXTIPWIDTH,
                Some(WPARAM(0)),
                Some(LPARAM(MAX_TIP_WIDTH)),
            );
        }
        Some(Self {
            hwnd,
            owner,
            texts: RefCell::new(Vec::new()),
        })
    }

    /// 按当前这排按钮重建工具：先删掉旧的，再把 `cells`（各按钮右边界，内容坐标）与提示文字挂上。
    /// `margin` 是阴影留白、`height` 是内容高度；纵列整高都算悬停区（状态条本来就只有一排按钮）。
    pub(super) fn sync(&self, cells: &[(i32, &str)], margin: i32, height: i32) {
        let mut texts = self.texts.borrow_mut();
        for id in 0..texts.len() {
            self.send(self.owner, id + 1, TTM_DELTOOLW, None, RECT::default());
        }
        texts.clear();
        let bottom = height + margin * 2;
        let mut left = margin;
        for (right, text) in cells {
            texts.push(text.encode_utf16().chain(std::iter::once(0)).collect());
            let index = texts.len() - 1;
            let rect = RECT {
                left,
                top: 0,
                right: right + margin,
                bottom,
            };
            let ptr = PWSTR(texts[index].as_mut_ptr());
            self.send(self.owner, index + 1, TTM_ADDTOOLW, Some(ptr), rect);
            left = right + margin;
        }
    }

    /// 发一条 `TTM_*`：`uId` 认工具，`text` 只有「加工具」时给。
    fn send(&self, hwnd: HWND, u_id: usize, message: u32, text: Option<PWSTR>, rect: RECT) {
        let mut info = TTTOOLINFOW {
            cbSize: core::mem::size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_SUBCLASS,
            hwnd,
            uId: u_id,
            rect,
            lpszText: text.unwrap_or_default(),
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                self.hwnd,
                message,
                Some(WPARAM(0)),
                Some(LPARAM((&mut info as *mut TTTOOLINFOW) as isize)),
            );
        }
    }
}

impl Drop for Tooltip {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}
