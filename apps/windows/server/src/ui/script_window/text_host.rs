//! Windowless RichEdit 的宿主：每块 `text_edit` 一个**普通**弹出窗，里面不建 `EDIT` 控件，
//! 而是通过 `Msftedit.dll!CreateTextServices` 拿 `ITextServices`，由我们自己的窗口过程把
//! 绘制 / 键盘 / 鼠标 / IME 消息转给它。
//!
//! 这样文本编辑、选区、剪贴板、IME 仍由 RichEdit 引擎负责，但窗口本身、背景、字体、文字绘制和
//! 位置都由脚本窗口控制；文字用宿主自己的 GDI 字体逐 run 绘制，不调用 `TxDraw`，避免 RichEdit
//! 的 font binding 改写 `…` 等字符的字形。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::rc::Rc;
use std::sync::OnceLock;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetDC, GetTextExtentPoint32W,
    GetTextMetricsW, HBRUSH, HDC, HGDIOBJ, InvalidateRect, PAINTSTRUCT, ReleaseDC, SYS_COLOR_INDEX,
    SelectObject, SetBkMode, SetTextColor, TEXTMETRICW, TRANSPARENT, TextOutW,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::EM_GETSEL;
use windows::Win32::UI::Controls::RichEdit::{
    CFM_CHARSET, CFM_COLOR, CFM_FACE, CFM_SIZE, CHARFORMATW, EM_SETCHARFORMAT, ITextHost,
    ITextHost_Impl, ITextServices, PARAFORMAT, SCF_ALL, TXTBACK_TRANSPARENT, TXTBACKSTYLE,
    TXTBIT_RICHTEXT, TXTBIT_SAVESELECTION, TXTBIT_VIEWINSETCHANGE,
};

use windows::Win32::UI::Input::Ime::{HIMC, ImmGetContext, ImmReleaseContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow, GetCaretBlinkTime, GetClientRect,
    IDC_IBEAM, KillTimer, LoadCursorW, SCROLL_WINDOW_FLAGS, SCROLLBAR_CONSTANTS, SW_HIDE,
    SW_SHOWNA, SWP_NOACTIVATE, SendMessageW, SetCursor, SetTimer, SetWindowPos, ShowWindow,
    WM_CHAR, WM_GETTEXT, WM_GETTEXTLENGTH, WM_IME_CHAR, WM_IME_COMPOSITION, WM_IME_COMPOSITIONFULL,
    WM_IME_CONTROL, WM_IME_ENDCOMPOSITION, WM_IME_KEYDOWN, WM_IME_KEYUP, WM_IME_NOTIFY,
    WM_IME_REQUEST, WM_IME_SELECT, WM_IME_SETCONTEXT, WM_IME_STARTCOMPOSITION, WM_INPUTLANGCHANGE,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_SETFOCUS, WM_SETFONT, WM_SIZE, WM_SYSCHAR, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{
    BOOL, Error, GUID, HSTRING, IUnknown_Vtbl, Interface, PCWSTR, Result, implement, w,
};

use super::super::window_class::WindowClass;
use super::tsf;

/// `Msftedit.dll!CreateTextServices` 的原始签名。
type CreateTextServicesFn =
    unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> windows::core::HRESULT;

const CLASS_NAME: PCWSTR = w!("CloudIMETextHost");

static CLASS: WindowClass = WindowClass::new();
static CREATE_TEXT_SERVICES: OnceLock<Option<(CreateTextServicesFn, GUID)>> = OnceLock::new();

thread_local! {
    /// HWND → 宿主窗口。窗口过程靠它找回 ITextServices。
    static HOSTS: RefCell<HashMap<isize, Rc<TextHostWindow>>> = RefCell::new(HashMap::new());
}

/// 一块 windowless RichEdit 宿主窗口。
pub(super) struct TextHostWindow {
    hwnd: HWND,
    /// TSF 上下文；初始化失败时为 `None`，退化为普通 RichEdit 输入。
    tsf: Option<tsf::TsfContext>,
    /// 本次按键是否被 TSF 吃掉（用来吞掉后续 WM_CHAR）。
    eaten: Cell<bool>,
    /// 是否已 `OnTxInPlaceActivate` / `OnTxUIActivate`。
    active: Cell<bool>,
    in_place: Cell<bool>,
    line_height: i32,
    services: ITextServices,
    /// 引擎的宿主接口；必须比 `services` 活得久。
    #[allow(dead_code)]
    host: ITextHost,
    font: HGDIOBJ,
    ellipsis_font: HGDIOBJ,
    brush: HBRUSH,
    highlight: HBRUSH,
    caret_brush: HBRUSH,
    text_color: COLORREF,
    selection_text: COLORREF,
    caret_visible: Cell<bool>,
    caret_timer: Cell<usize>,
}

impl TextHostWindow {
    /// 建一块宿主窗；`family` / `size` 是候选词字体，`text` 是初始文本。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create(
        owner: HWND,
        text: &str,
        family: Option<&str>,
        size: f32,
        background: COLORREF,
        text_color: COLORREF,
        selection_background: COLORREF,
        selection_text: COLORREF,
        caret_color: COLORREF,
        dpi: u32,
    ) -> Result<Rc<Self>> {
        ensure_class()?;
        let style = WS_POPUP;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                CLASS_NAME,
                w!(""),
                style,
                0,
                0,
                0,
                0,
                Some(owner),
                None,
                None,
                None,
            )
        }?;
        let font = create_font(family, size, dpi);
        let ellipsis_font = create_font(Some("Segoe UI"), size, dpi);
        let brush = unsafe { CreateSolidBrush(background) };
        let highlight = unsafe { CreateSolidBrush(selection_background) };
        let caret_brush = unsafe { CreateSolidBrush(caret_color) };

        let line_height = font_line_height(font, size, dpi);
        let host: ITextHost = TextHost {
            hwnd,
            line_height,
            char_format: char_format(family, size, text_color),
            para_format: PARAFORMAT {
                cbSize: size_of::<PARAFORMAT>() as u32,
                ..Default::default()
            },
        }
        .into();
        let Some((create, iid)) = create_text_services() else {
            unsafe {
                let _ = DestroyWindow(hwnd);
                let _ = DeleteObject(font);
                let _ = DeleteObject(brush.into());
            }
            return Err(Error::from(windows::Win32::Foundation::E_FAIL));
        };
        let mut unknown: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { create(std::ptr::null_mut(), host.as_raw(), &mut unknown) };
        if hr.is_err() || unknown.is_null() {
            unsafe {
                let _ = DestroyWindow(hwnd);
                let _ = DeleteObject(font);
                let _ = DeleteObject(brush.into());
            }
            return Err(Error::from(hr));
        }
        // windows crate 里 ITextServices 的 IID 是 0（元数据里没有），真正的 IID 由
        // Msftedit.dll 导出，必须用它 QueryInterface，不能直接强转。
        let mut services_ptr: *mut c_void = std::ptr::null_mut();
        let vtbl = unsafe { *(unknown as *const *const IUnknown_Vtbl) };
        let hr = unsafe { ((*vtbl).QueryInterface)(unknown, &iid, &mut services_ptr) };
        unsafe {
            let _ = ((*vtbl).Release)(unknown);
        }
        if hr.is_err() || services_ptr.is_null() {
            unsafe {
                let _ = DestroyWindow(hwnd);
                let _ = DeleteObject(font);
                let _ = DeleteObject(brush.into());
            }
            return Err(Error::from(hr));
        }
        let services: ITextServices = unsafe { ITextServices::from_raw(services_ptr) };
        let tsf = match tsf::TsfContext::create(hwnd, services.clone(), line_height) {
            Ok(context) => Some(context),
            Err(error) => {
                tracing::warn!(%error, "text_edit：建 TSF 上下文失败，退化为普通 RichEdit 输入");
                None
            }
        };
        let window = Rc::new(Self {
            hwnd,
            tsf,
            eaten: Cell::new(false),
            active: Cell::new(false),
            in_place: Cell::new(false),
            line_height,
            services,
            host,
            font,
            ellipsis_font,
            brush,
            highlight,
            caret_brush,
            text_color,
            selection_text,
            caret_visible: Cell::new(false),
            caret_timer: Cell::new(0),
        });
        HOSTS.with(|map| map.borrow_mut().insert(hwnd.0 as isize, window.clone()));
        unsafe {
            let mut result = LRESULT(0);
            let text_string = text.to_string();
            let text = HSTRING::from(text_string.as_str());
            if let Err(error) = window.services.TxSetText(&text) {
                tracing::warn!(id = owner.0 as isize, %error, "text_edit：TxSetText 失败");
            }
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            let mut format = char_format(family, size, text_color);
            let _ = window.services.TxSendMessage(
                EM_SETCHARFORMAT,
                WPARAM(SCF_ALL as usize),
                LPARAM(&mut format as *mut _ as isize),
                &mut result,
            );
        }
        Ok(window)
    }

    pub(super) fn set_bounds(&self, x: i32, y: i32, width: i32, height: i32) {
        // RichEdit 单行文本默认贴顶；宿主窗本身只留一行高并垂直居中，避免依赖
        // `TxGetViewInset` 在不同 RichEdit 版本里的差异。
        let host_height = (self.line_height + 4).min(height.max(1)).max(1);
        let y = y + (height.max(1) - host_height) / 2;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST),
                x,
                y,
                width.max(1),
                host_height,
                SWP_NOACTIVATE,
            );
        }
        if let Some(tsf) = &self.tsf {
            tsf.notify_layout();
        }
        unsafe {
            let _ = self
                .services
                .OnTxPropertyBitsChange(TXTBIT_VIEWINSETCHANGE, TXTBIT_VIEWINSETCHANGE);
        }
    }

    pub(super) fn show(&self, show: bool) {
        unsafe {
            let _ = ShowWindow(self.hwnd, if show { SW_SHOWNA } else { SW_HIDE });
        }
        if !show {
            self.deactivate_in_place();
        }
    }

    pub(super) fn focus(&self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
        }
        if let Some(tsf) = &self.tsf {
            tsf.focus();
        }
    }

    /// 窗口拿到焦点：先 in-place / UI 激活，再按 windowless host 的约定用 NULL wParam
    /// 发 `WM_SETFOCUS`。少了这两步 RichEdit 不会建插入符，拖选也不重画。
    fn activate(&self) {
        if !self.in_place.get() {
            let mut rect = RECT::default();
            unsafe {
                let _ = GetClientRect(self.hwnd, &mut rect);
                let _ = self.services.OnTxInPlaceActivate(&mut rect);
            }
            self.in_place.set(true);
        }
        if !self.active.replace(true) {
            unsafe {
                let _ = self.services.OnTxUIActivate();
            }
        }
        let mut result = LRESULT(0);
        unsafe {
            let _ = self
                .services
                .TxSendMessage(WM_SETFOCUS, WPARAM(0), LPARAM(0), &mut result);
        }
        if let Some(tsf) = &self.tsf {
            tsf.focus();
        }
        self.caret_visible.set(true);
        if self.caret_timer.get() == 0 {
            let blink = unsafe { GetCaretBlinkTime() };
            let blink = if blink == 0 || blink == u32::MAX {
                530
            } else {
                blink
            };
            let timer = unsafe { SetTimer(Some(self.hwnd), 0, blink, None) };
            self.caret_timer.set(timer);
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn deactivate(&self) {
        self.caret_visible.set(false);
        let timer = self.caret_timer.replace(0);
        if timer != 0 {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), timer);
            }
        }
        if let Some(tsf) = &self.tsf {
            tsf.blur();
        }
        if self.active.replace(false) {
            unsafe {
                let _ = self.services.OnTxUIDeactivate();
            }
        }
        let mut result = LRESULT(0);
        unsafe {
            let _ = self
                .services
                .TxSendMessage(WM_KILLFOCUS, WPARAM(0), LPARAM(0), &mut result);
        }
    }

    fn deactivate_in_place(&self) {
        self.deactivate();
        if self.in_place.replace(false) {
            unsafe {
                let _ = self.services.OnTxInPlaceDeactivate();
            }
        }
    }

    /// TSF 先吃键；返回 `true` 表示这次按键别给 RichEdit。
    fn tsf_key(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
        self.caret_visible.set(true);
        let eaten = self
            .tsf
            .as_ref()
            .map(|tsf| tsf.key(msg, wparam, lparam))
            .unwrap_or(false);
        self.eaten.set(eaten);
        eaten
    }

    fn notify_selection(&self) {
        if let Some(tsf) = &self.tsf {
            tsf.notify_selection();
        }
    }

    fn poke_caret(&self) {
        self.caret_visible.set(true);
    }

    fn text(&self) -> Vec<u16> {
        let length = self.send(WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0.max(0) as usize;
        let mut buffer = vec![0u16; length + 1];
        let copied = self
            .send(
                WM_GETTEXT,
                WPARAM(length + 1),
                LPARAM(buffer.as_mut_ptr() as isize),
            )
            .0
            .max(0) as usize;
        buffer.truncate(copied.min(length));
        buffer
    }

    fn selection(&self, length: usize) -> (usize, usize) {
        let mut start = 0u32;
        let mut end = 0u32;
        self.send(
            EM_GETSEL,
            WPARAM(&mut start as *mut u32 as usize),
            LPARAM(&mut end as *mut u32 as isize),
        );
        ((start as usize).min(length), (end as usize).min(length))
    }

    fn text_width(&self, hdc: HDC, text: &[u16]) -> i32 {
        let mut width = 0;
        let mut index = 0;
        while index < text.len() {
            let ellipsis = text[index] == 0x2026;
            let start = index;
            if ellipsis {
                index += 1;
            } else {
                while index < text.len() && text[index] != 0x2026 {
                    index += 1;
                }
            }
            let font = if ellipsis {
                self.ellipsis_font
            } else {
                self.font
            };
            unsafe {
                let old = SelectObject(hdc, font);
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(hdc, &text[start..index], &mut size);
                let _ = SelectObject(hdc, old);
                width += size.cx;
            }
        }
        width
    }

    fn draw_runs(
        &self,
        hdc: HDC,
        text: &[u16],
        client: &RECT,
        start: usize,
        end: usize,
        color: COLORREF,
    ) {
        if text.is_empty() || start >= end {
            return;
        }
        unsafe {
            let old = SelectObject(hdc, self.font);
            let mut metrics = TEXTMETRICW::default();
            let _ = GetTextMetricsW(hdc, &mut metrics);
            let baseline = client.bottom / 2 + metrics.tmAscent / 2;
            let mut x = 2 + self.text_width(hdc, &text[..start]);
            let mut index = start;
            while index < end {
                let ellipsis = text[index] == 0x2026;
                let run_start = index;
                if ellipsis {
                    index += 1;
                } else {
                    while index < end && text[index] != 0x2026 {
                        index += 1;
                    }
                }
                let font = if ellipsis {
                    self.ellipsis_font
                } else {
                    self.font
                };
                let run_old = SelectObject(hdc, font);
                let mut run_metrics = TEXTMETRICW::default();
                let _ = GetTextMetricsW(hdc, &mut run_metrics);
                let _ = SetTextColor(hdc, color);
                let _ = TextOutW(
                    hdc,
                    x,
                    baseline - run_metrics.tmAscent,
                    &text[run_start..index],
                );
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(hdc, &text[run_start..index], &mut size);
                let _ = SelectObject(hdc, run_old);
                x += size.cx;
            }
            let _ = SelectObject(hdc, old);
        }
    }

    fn caret_rect(&self, hdc: HDC, text: &[u16], client: &RECT) -> Option<RECT> {
        if !self.active.get() || !self.caret_visible.get() {
            return None;
        }
        let (start, end) = self.selection(text.len());
        if start != end {
            return None;
        }
        unsafe {
            let old = SelectObject(hdc, self.font);
            let mut metrics = TEXTMETRICW::default();
            let _ = GetTextMetricsW(hdc, &mut metrics);
            let _ = SelectObject(hdc, old);
            let x = 2 + self.text_width(hdc, &text[..start]);
            if x >= client.right {
                return None;
            }
            let baseline = client.bottom / 2 + metrics.tmAscent / 2;
            let top = baseline - metrics.tmAscent;
            Some(RECT {
                left: x,
                top,
                right: x + 1,
                bottom: (top + metrics.tmHeight).min(client.bottom),
            })
        }
    }

    fn paint(&self) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            let _ = FillRect(hdc, &client, self.brush);
            let old_font = SelectObject(hdc, self.font);
            let _ = SetBkMode(hdc, TRANSPARENT);
            let text = self.text();
            let (start, end) = self.selection(text.len());
            if start < end {
                let first = 2 + self.text_width(hdc, &text[..start]);
                let second = 2 + self.text_width(hdc, &text[..end]);
                let selection = RECT {
                    left: first.min(second),
                    top: 0,
                    right: first.max(second).max(first.min(second) + 1),
                    bottom: client.bottom.max(1),
                };
                let _ = FillRect(hdc, &selection, self.highlight);
            }
            self.draw_runs(hdc, &text, &client, 0, text.len(), self.text_color);
            if start < end {
                self.draw_runs(hdc, &text, &client, start, end, self.selection_text);
            }
            if let Some(caret) = self.caret_rect(hdc, &text, &client) {
                let _ = FillRect(hdc, &caret, self.caret_brush);
            }
            let _ = SelectObject(hdc, old_font);
            let _ = EndPaint(self.hwnd, &ps);
        }
    }

    fn send(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let mut result = LRESULT(0);
        unsafe {
            let _ = self
                .services
                .TxSendMessage(msg, wparam, lparam, &mut result);
        }
        result
    }
}

impl Drop for TextHostWindow {
    fn drop(&mut self) {
        self.deactivate_in_place();
        HOSTS.with(|map| {
            map.borrow_mut().remove(&(self.hwnd.0 as isize));
        });
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = DeleteObject(self.font);
            let _ = DeleteObject(self.ellipsis_font);
            let _ = DeleteObject(self.brush.into());
            let _ = DeleteObject(self.highlight.into());
            let _ = DeleteObject(self.caret_brush.into());
        }
    }
}

/// `CreateTextServices` 的宿主实现。绘制 / 消息都由 RichEdit 引擎回调进来。
#[implement(ITextHost)]
struct TextHost {
    hwnd: HWND,
    line_height: i32,
    char_format: CHARFORMATW,
    para_format: PARAFORMAT,
}

impl ITextHost_Impl for TextHost_Impl {
    fn TxGetDC(&self) -> HDC {
        unsafe { GetDC(Some(self.hwnd)) }
    }

    fn TxReleaseDC(&self, hdc: HDC) -> i32 {
        unsafe { ReleaseDC(Some(self.hwnd), hdc) }
    }

    fn TxShowScrollBar(&self, _fnbar: i32, _fshow: BOOL) -> BOOL {
        false.into()
    }

    fn TxEnableScrollBar(&self, _fusbflags: SCROLLBAR_CONSTANTS, _fuarrowflags: i32) -> BOOL {
        false.into()
    }

    fn TxSetScrollRange(&self, _fnbar: i32, _nminpos: i32, _nmaxpos: i32, _fredraw: BOOL) -> BOOL {
        false.into()
    }

    fn TxSetScrollPos(&self, _fnbar: i32, _npos: i32, _fredraw: BOOL) -> BOOL {
        false.into()
    }

    fn TxInvalidateRect(&self, prc: *mut RECT, fmode: BOOL) {
        unsafe {
            if prc.is_null() {
                let _ = InvalidateRect(Some(self.hwnd), None, fmode.as_bool());
            } else {
                let _ = InvalidateRect(Some(self.hwnd), Some(&*prc), fmode.as_bool());
            }
        }
    }

    fn TxViewChange(&self, _fupdate: BOOL) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn TxCreateCaret(
        &self,
        _hbmp: windows::Win32::Graphics::Gdi::HBITMAP,
        _xwidth: i32,
        _yheight: i32,
    ) -> BOOL {
        false.into()
    }

    fn TxShowCaret(&self, _fshow: BOOL) -> BOOL {
        false.into()
    }

    fn TxSetCaretPos(&self, _x: i32, _y: i32) -> BOOL {
        false.into()
    }

    fn TxSetTimer(&self, idtimer: u32, utimeout: u32) -> BOOL {
        (unsafe { SetTimer(Some(self.hwnd), idtimer as usize, utimeout, None) } != 0).into()
    }

    fn TxKillTimer(&self, idtimer: u32) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), idtimer as usize);
        }
    }

    fn TxScrollWindowEx(
        &self,
        _dx: i32,
        _dy: i32,
        _lprcscroll: *mut RECT,
        _lprcclip: *mut RECT,
        _hrgnupdate: windows::Win32::Graphics::Gdi::HRGN,
        _lprcupdate: *mut RECT,
        _fuscroll: SCROLL_WINDOW_FLAGS,
    ) {
    }

    fn TxSetCapture(&self, fcapture: BOOL) {
        unsafe {
            if fcapture.as_bool() {
                SetCapture(self.hwnd);
            } else {
                let _ = ReleaseCapture();
            }
        }
    }

    fn TxSetFocus(&self) {
        unsafe {
            let _ = SetFocus(Some(self.hwnd));
        }
    }

    fn TxSetCursor(&self, hcur: windows::Win32::UI::WindowsAndMessaging::HCURSOR, _ftext: BOOL) {
        unsafe {
            SetCursor(Some(hcur));
        }
    }

    fn TxScreenToClient(&self, lppt: *mut POINT) -> BOOL {
        unsafe {
            windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, lppt)
                .as_bool()
                .into()
        }
    }

    fn TxClientToScreen(&self, lppt: *mut POINT) -> BOOL {
        unsafe {
            windows::Win32::Graphics::Gdi::ClientToScreen(self.hwnd, lppt)
                .as_bool()
                .into()
        }
    }

    fn TxActivate(&self, _ploldstate: *mut i32) -> Result<()> {
        Ok(())
    }

    fn TxDeactivate(&self, _lnewstate: i32) -> Result<()> {
        Ok(())
    }

    fn TxGetClientRect(&self, prc: *mut RECT) -> Result<()> {
        unsafe {
            GetClientRect(self.hwnd, prc)?;
        }
        Ok(())
    }

    fn TxGetViewInset(&self, prc: *mut RECT) -> Result<()> {
        unsafe {
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            let height = (client.bottom - client.top).max(0);
            let line = self.line_height.min(height);
            let top = ((height - line) / 2).max(0);
            *prc = RECT {
                left: 2,
                top,
                right: 2,
                bottom: (height - top - line).max(0),
            };
        }
        Ok(())
    }

    fn TxGetCharFormat(&self, ppcf: *const *const CHARFORMATW) -> Result<()> {
        unsafe {
            std::ptr::write(ppcf.cast_mut(), &self.char_format);
        }
        Ok(())
    }

    fn TxGetParaFormat(&self, pppf: *const *const PARAFORMAT) -> Result<()> {
        unsafe {
            std::ptr::write(pppf.cast_mut(), &self.para_format);
        }
        Ok(())
    }

    fn TxGetSysColor(&self, nindex: SYS_COLOR_INDEX) -> COLORREF {
        system_color(nindex)
    }

    fn TxGetBackStyle(&self, pstyle: *mut TXTBACKSTYLE) -> Result<()> {
        unsafe {
            *pstyle = TXTBACK_TRANSPARENT;
        }
        Ok(())
    }

    fn TxGetMaxLength(&self, plength: *mut u32) -> Result<()> {
        unsafe {
            *plength = u32::MAX;
        }
        Ok(())
    }

    fn TxGetScrollBars(&self, pdwscrollbar: *mut u32) -> Result<()> {
        unsafe {
            *pdwscrollbar = 0;
        }
        Ok(())
    }

    fn TxGetPasswordChar(&self) -> Result<i8> {
        Ok(0)
    }

    fn TxGetAcceleratorPos(&self, pcp: *mut i32) -> Result<()> {
        unsafe {
            *pcp = -1;
        }
        Ok(())
    }

    fn TxGetExtent(&self, lpextent: *mut windows::Win32::Foundation::SIZE) -> Result<()> {
        unsafe {
            let mut client = RECT::default();
            GetClientRect(self.hwnd, &mut client)?;
            let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(self.hwnd).max(1);
            // TxGetExtent 的单位是 HIMETRIC（1 毫米 = 100），不是像素。
            *lpextent = windows::Win32::Foundation::SIZE {
                cx: ((client.right - client.left) as i64 * 2540 / dpi as i64) as i32,
                cy: ((client.bottom - client.top) as i64 * 2540 / dpi as i64) as i32,
            };
        }
        Ok(())
    }

    fn OnTxCharFormatChange(&self, _pcf: *const CHARFORMATW) -> Result<()> {
        Ok(())
    }

    fn OnTxParaFormatChange(&self, _ppf: *const PARAFORMAT) -> Result<()> {
        Ok(())
    }

    fn TxGetPropertyBits(&self, _dwmask: u32, pdwbits: *mut u32) -> Result<()> {
        unsafe {
            *pdwbits = (TXTBIT_RICHTEXT | TXTBIT_SAVESELECTION) & _dwmask;
        }
        Ok(())
    }

    fn TxNotify(&self, _inotify: u32, _pv: *mut c_void) -> Result<()> {
        Ok(())
    }

    fn TxImmGetContext(&self) -> HIMC {
        unsafe { ImmGetContext(self.hwnd) }
    }

    fn TxImmReleaseContext(&self, himc: HIMC) {
        unsafe {
            let _ = ImmReleaseContext(self.hwnd, himc);
        }
    }

    fn TxGetSelectionBarWidth(&self, lselbarwidth: *mut i32) -> Result<()> {
        unsafe {
            *lselbarwidth = 0;
        }
        Ok(())
    }
}

fn ensure_class() -> Result<()> {
    CLASS.ensure(|| WNDCLASSEXW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(wndproc),
        hInstance: super::super::module_handle(),
        hCursor: unsafe { LoadCursorW(None, IDC_IBEAM).unwrap_or_default() },
        lpszClassName: CLASS_NAME,
        ..Default::default()
    })
}

fn create_text_services() -> Option<(CreateTextServicesFn, GUID)> {
    *CREATE_TEXT_SERVICES.get_or_init(|| unsafe {
        let module = LoadLibraryW(w!("Msftedit.dll")).ok()?;
        let proc = GetProcAddress(module, windows::core::s!("CreateTextServices"))?;
        let iid = GetProcAddress(module, windows::core::s!("IID_ITextServices"))?;
        let create =
            std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateTextServicesFn>(proc);
        let iid = *(iid as *const GUID);
        Some((create, iid))
    })
}

fn char_format(family: Option<&str>, size: f32, text_color: COLORREF) -> CHARFORMATW {
    let mut format = CHARFORMATW {
        cbSize: size_of::<CHARFORMATW>() as u32,
        dwMask: CFM_COLOR | CFM_SIZE | CFM_FACE | CFM_CHARSET,
        yHeight: (size * 20.0).round().max(1.0) as i32,
        crTextColor: text_color,
        bCharSet: windows::Win32::Graphics::Gdi::DEFAULT_CHARSET,
        ..Default::default()
    };
    let name = family.unwrap_or("Segoe UI");
    for (slot, unit) in format
        .szFaceName
        .iter_mut()
        .zip(name.encode_utf16().take(31))
    {
        *slot = unit;
    }
    format
}

fn font_line_height(font: HGDIOBJ, size: f32, dpi: u32) -> i32 {
    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return ((size * dpi as f32 / 72.0).ceil() as i32).max(1);
        }
        let old = SelectObject(dc, font);
        let mut metrics = TEXTMETRICW::default();
        let ok = GetTextMetricsW(dc, &mut metrics).as_bool();
        let _ = SelectObject(dc, old);
        let _ = ReleaseDC(None, dc);
        if ok {
            (metrics.tmHeight + metrics.tmExternalLeading).max(1)
        } else {
            ((size * dpi as f32 / 72.0).ceil() as i32).max(1)
        }
    }
}

fn create_font(family: Option<&str>, size: f32, dpi: u32) -> HGDIOBJ {
    use windows::Win32::Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        FF_DONTCARE, FW_NORMAL, OUT_DEFAULT_PRECIS,
    };
    let height = -((size * dpi as f32 / 72.0).round() as i32).max(1);
    let family = family.map(HSTRING::from).unwrap_or_default();
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 as u32) | (FF_DONTCARE.0 as u32),
            &family,
        )
        .into()
    }
}

fn system_color(index: SYS_COLOR_INDEX) -> COLORREF {
    unsafe { COLORREF(windows::Win32::Graphics::Gdi::GetSysColor(index)) }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // 自定义消息转发给 RichEdit；文字和选区由宿主用 GDI 自绘。
    if msg == WM_PAINT {
        with_host(hwnd, |host| host.paint());
        return LRESULT(0);
    }
    // 输入法先看键；被吃掉就不再给 RichEdit（WM_CHAR 也跟着吞）。
    if matches!(msg, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP) {
        let eaten = with_host(hwnd, |host| host.tsf_key(msg, wparam, lparam)).unwrap_or(false);
        if eaten {
            return LRESULT(0);
        }
    }
    if msg == WM_CHAR && with_host(hwnd, |host| host.eaten.get()).unwrap_or(false) {
        return LRESULT(0);
    }
    if msg == WM_TIMER {
        let ours = with_host(hwnd, |host| {
            host.caret_timer.get() != 0 && wparam.0 == host.caret_timer.get()
        })
        .unwrap_or(false);
        if ours {
            with_host(hwnd, |host| {
                host.caret_visible.set(!host.caret_visible.get());
                unsafe {
                    let _ = InvalidateRect(Some(host.hwnd), None, false);
                }
            });
            return LRESULT(0);
        }
    }
    // 悬停移动只保留给宿主判断光标；不再转给 RichEdit，免得它每次移动都失效重绘。
    if msg == WM_MOUSEMOVE && (wparam.0 as u32 & 0x0001) == 0 {
        return LRESULT(0);
    }
    if msg == WM_SETFOCUS {
        with_host(hwnd, |host| host.activate());
        return LRESULT(0);
    }
    if msg == WM_KILLFOCUS {
        with_host(hwnd, |host| host.deactivate());
        return LRESULT(0);
    }
    if matches!(
        msg,
        WM_KEYDOWN
            | WM_KEYUP
            | WM_CHAR
            | WM_IME_STARTCOMPOSITION
            | WM_IME_COMPOSITION
            | WM_IME_COMPOSITIONFULL
            | WM_IME_ENDCOMPOSITION
            | WM_IME_SETCONTEXT
            | WM_IME_NOTIFY
            | WM_IME_CONTROL
            | WM_IME_SELECT
            | WM_IME_REQUEST
            | WM_IME_KEYDOWN
            | WM_IME_KEYUP
            | WM_IME_CHAR
            | WM_INPUTLANGCHANGE
            | WM_SYSKEYDOWN
            | WM_SYSKEYUP
            | WM_SYSCHAR
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_LBUTTONDBLCLK
            | WM_MOUSEMOVE
            | WM_TIMER
            | WM_SETFONT
            | WM_SIZE
    ) && let Some(result) = with_host(hwnd, |host| host.send(msg, wparam, lparam))
    {
        if matches!(msg, WM_LBUTTONUP | WM_KEYUP) {
            with_host(hwnd, |host| host.notify_selection());
        }
        let left_button = (wparam.0 as u32 & 0x0001) != 0;
        if matches!(
            msg,
            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_KEYDOWN | WM_CHAR
        ) || (msg == WM_MOUSEMOVE && left_button)
        {
            with_host(hwnd, |host| unsafe {
                host.poke_caret();
                let _ = InvalidateRect(Some(host.hwnd), None, false);
            });
        }
        return result;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn with_host<T>(hwnd: HWND, f: impl FnOnce(&TextHostWindow) -> T) -> Option<T> {
    let host = HOSTS.with(|map| map.borrow().get(&(hwnd.0 as isize)).cloned())?;
    Some(f(&host))
}
