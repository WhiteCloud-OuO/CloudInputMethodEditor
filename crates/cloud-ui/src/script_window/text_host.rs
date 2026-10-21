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

use windows::Win32::Foundation::{
    COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, RECTL, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
    CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC, DeleteObject, EndPaint,
    FillRect, GetDC, GetTextExtentPoint32W, GetTextMetricsW, HBRUSH, HDC, HGDIOBJ, InvalidateRect,
    PAINTSTRUCT, ReleaseDC, SRCCOPY, SYS_COLOR_INDEX, SelectObject, SetBkMode, SetTextColor,
    TEXTMETRICW, TRANSPARENT, TextOutW,
};
use windows::Win32::System::Com::DVASPECT_CONTENT;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::RichEdit::{
    CFE_BOLD, CFE_ITALIC, CFE_STRIKEOUT, CFE_UNDERLINE, CFM_BOLD, CFM_CHARSET, CFM_COLOR, CFM_FACE,
    CFM_ITALIC, CFM_SIZE, CFM_STRIKEOUT, CFM_UNDERLINE, CHARFORMATW, EM_SETCHARFORMAT,
    EM_SETPARAFORMAT, EM_SETSCROLLPOS, ITextHost, ITextHost_Impl, ITextServices, PARAFORMAT,
    PARAFORMAT2, PFM_LINESPACING, SCF_ALL, TXTBACK_TRANSPARENT, TXTBACKSTYLE, TXTBIT_MULTILINE,
    TXTBIT_RICHTEXT, TXTBIT_SAVESELECTION, TXTBIT_VIEWINSETCHANGE, TXTBIT_WORDWRAP,
};
use windows::Win32::UI::Controls::{EM_GETSEL, EM_SETREADONLY, EM_SETSEL};

use windows::Win32::UI::Input::Ime::{HIMC, ImmGetContext, ImmReleaseContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateCaret, CreateWindowExW, DefWindowProcW, DestroyWindow, EN_CHANGE,
    GetCaretBlinkTime, GetClientRect, HideCaret, IDC_IBEAM, KillTimer, LoadCursorW, PostMessageW,
    SCROLL_WINDOW_FLAGS, SCROLLBAR_CONSTANTS, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SendMessageW,
    SetCaretPos, SetCursor, SetTimer, SetWindowPos, ShowCaret, ShowWindow, WM_APP, WM_CHAR,
    WM_GETTEXT, WM_GETTEXTLENGTH, WM_IME_CHAR, WM_IME_COMPOSITION, WM_IME_COMPOSITIONFULL,
    WM_IME_CONTROL, WM_IME_ENDCOMPOSITION, WM_IME_KEYDOWN, WM_IME_KEYUP, WM_IME_NOTIFY,
    WM_IME_REQUEST, WM_IME_SELECT, WM_IME_SETCONTEXT, WM_IME_STARTCOMPOSITION, WM_INPUTLANGCHANGE,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_SETFOCUS, WM_SETFONT, WM_SIZE, WM_SYSCHAR,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_HSCROLL,
    WS_POPUP, WS_VSCROLL,
};
use windows::core::{
    BOOL, Error, GUID, HSTRING, IUnknown_Vtbl, Interface, PCWSTR, Result, implement, w,
};

use super::tsf;
use crate::window::module_handle;
use crate::window_class::WindowClass;

/// `Msftedit.dll!CreateTextServices` 的原始签名。
type CreateTextServicesFn =
    unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> windows::core::HRESULT;

const CLASS_NAME: PCWSTR = w!("CloudIMETextHost");

/// 单行编辑框插入符闪烁计时器的 ID。不能用 0：窗口计时器传 0 时，`SetTimer` 的返回值和
/// `WM_TIMER.wParam` 可能不一致，插入符会永远停在显示态。
const CARET_TIMER_ID: usize = 0x4341_5245;

/// 宿主窗 → 脚本窗口：编辑框文本变了，请重读并派发 `text_changed`。
pub(super) const WM_TEXT_CHANGED: u32 = WM_APP + 0x4A;

/// 宿主窗 → 脚本窗口：编辑框按键按下（`lparam` 低 16 位 key、高 8 位 modifier）。
pub(super) const WM_TEXT_KEY: u32 = WM_APP + 0x4B;

/// 宿主窗 → 脚本窗口：编辑框获得 / 失去焦点（`lparam` 1 = focused）。
pub(super) const WM_TEXT_FOCUS: u32 = WM_APP + 0x4C;

/// 宿主 → 脚本窗口：多行编辑框的滚动位置 / 可见范围变了，请刷新内部滚动条。
pub(super) const WM_TEXT_SCROLL: u32 = WM_APP + 0x4D;

/// 宿主 → 脚本窗口：多行编辑框收到滚轮（`lparam` 为带符号的 delta）。
pub(super) const WM_TEXT_WHEEL: u32 = WM_APP + 0x4E;

/// 多行编辑框的滚动范围（像素）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ScrollInfo {
    pub min: i32,
    pub max: i32,
    pub pos: i32,
    pub page: i32,
}

static CLASS: WindowClass = WindowClass::new();
static CREATE_TEXT_SERVICES: OnceLock<Option<(CreateTextServicesFn, GUID)>> = OnceLock::new();

thread_local! {
    /// HWND → 宿主窗口。窗口过程靠它找回 ITextServices。
    static HOSTS: RefCell<HashMap<isize, Rc<TextHostWindow>>> = RefCell::new(HashMap::new());
}

/// 一块 windowless RichEdit 宿主窗口。
pub(super) struct TextHostWindow {
    hwnd: HWND,
    owner: HWND,
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
    highlight: Cell<HBRUSH>,
    /// 与 `TextHost` 共享的选区背景色；脚本可在窗口打开后跟随主题更新。
    selection_background: Rc<Cell<COLORREF>>,
    caret_brush: HBRUSH,
    text_color: COLORREF,
    selection_text: COLORREF,
    caret_visible: Cell<bool>,
    caret_timer: Cell<usize>,
    /// 横向滚动量（像素，内容向左移）：单行文本超过编辑区时跟随光标。
    scroll_x: Cell<i32>,
    /// 上次摆位（物理像素）；脚本窗口每帧重画时滚动条拖动只改内部状态，不能反复
    /// SetWindowPos 所有宿主窗，否则多个 RichEdit 会一起闪。
    bounds: Cell<Option<(i32, i32, i32, i32)>>,
    /// 上次显示 / 隐藏状态；同样避免每帧重复 ShowWindow。
    shown: Cell<bool>,
    /// 多行模式：文字由 RichEdit 自己排版 / 绘制（`TxDraw`），内部滚动条由脚本窗口画。
    multiline: bool,
    /// 密码输入：只影响显示，真实文本仍交给 API。
    passkey: bool,
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
        font_flag: u8,
        read_only: bool,
        passkey: bool,
        multiline: bool,
        auto_wrap: bool,
        line_spacing: u16,
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
        let font = create_font(family, size, dpi, font_flag);
        let ellipsis_font = create_font(Some("Segoe UI"), size, dpi, font_flag);
        let brush = unsafe { CreateSolidBrush(background) };
        let selection_background = Rc::new(Cell::new(selection_background));
        let highlight = unsafe { CreateSolidBrush(selection_background.get()) };
        let caret_brush = unsafe { CreateSolidBrush(caret_color) };

        let line_height = font_line_height(font, size, dpi);
        let host: ITextHost = TextHost {
            hwnd,
            owner,
            passkey,
            multiline,
            auto_wrap,
            background,
            text_color,
            selection_background: selection_background.clone(),
            selection_text,
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
            owner,
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
            highlight: Cell::new(highlight),
            selection_background,
            caret_brush,
            text_color,
            selection_text,
            caret_visible: Cell::new(false),
            caret_timer: Cell::new(0),
            scroll_x: Cell::new(0),
            bounds: Cell::new(None),
            shown: Cell::new(false),
            multiline,
            passkey,
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
            if multiline {
                format.dwMask |= CFM_BOLD | CFM_ITALIC | CFM_UNDERLINE | CFM_STRIKEOUT;
                if font_flag & 1 != 0 {
                    format.dwEffects |= CFE_BOLD;
                }
                if font_flag & 2 != 0 {
                    format.dwEffects |= CFE_ITALIC;
                }
                if font_flag & 4 != 0 {
                    format.dwEffects |= CFE_UNDERLINE;
                }
                if font_flag & 8 != 0 {
                    format.dwEffects |= CFE_STRIKEOUT;
                }
            }
            let _ = window.services.TxSendMessage(
                EM_SETCHARFORMAT,
                WPARAM(SCF_ALL as usize),
                LPARAM(&mut format as *mut _ as isize),
                &mut result,
            );
            if read_only {
                let _ = window.services.TxSendMessage(
                    EM_SETREADONLY,
                    WPARAM(1),
                    LPARAM(0),
                    &mut result,
                );
            }
            if multiline && line_spacing > 0 {
                let mut format = PARAFORMAT2 {
                    Base: PARAFORMAT {
                        cbSize: size_of::<PARAFORMAT2>() as u32,
                        dwMask: PFM_LINESPACING,
                        ..Default::default()
                    },
                    dyLineSpacing: ((size * 1.2 + line_spacing as f32) * 20.0).round().max(1.0)
                        as i32,
                    bLineSpacingRule: 3,
                    ..Default::default()
                };
                let _ = window.services.TxSendMessage(
                    EM_SETPARAFORMAT,
                    WPARAM(0),
                    LPARAM(&mut format as *mut _ as isize),
                    &mut result,
                );
            }
            if multiline {
                let _ = window
                    .services
                    .TxSendMessage(EM_SETSEL, WPARAM(0), LPARAM(0), &mut result);
                let mut point = POINT { x: 0, y: 0 };
                let _ = window.services.TxSendMessage(
                    EM_SETSCROLLPOS,
                    WPARAM(0),
                    LPARAM(&mut point as *mut _ as isize),
                    &mut result,
                );
            }
        }
        Ok(window)
    }

    pub(super) fn set_bounds(&self, x: i32, y: i32, width: i32, height: i32) {
        // 单行 RichEdit 默认贴顶；宿主窗本身只留一行高并垂直居中，避免依赖
        // `TxGetViewInset` 在不同 RichEdit 版本里的差异。多行编辑框要用完整文本区。
        let host_height = if self.multiline {
            height.max(1)
        } else {
            (self.line_height + 4).min(height.max(1)).max(1)
        };
        let y = if self.multiline {
            y
        } else {
            y + (height.max(1) - host_height) / 2
        };
        let bounds = (x, y, width.max(1), host_height);
        if self.bounds.get() == Some(bounds) {
            return;
        }
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
        self.bounds.set(Some(bounds));
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
        if self.shown.get() == show {
            return;
        }
        self.shown.set(show);
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

    /// 改选区背景色（脚本跟随候选高亮色用）；同时重画当前选中文本。
    pub(super) fn set_selection_background(&self, color: COLORREF) {
        self.selection_background.set(color);
        let brush = unsafe { CreateSolidBrush(color) };
        let old = self.highlight.replace(brush);
        unsafe {
            let _ = DeleteObject(old.into());
            let _ = InvalidateRect(Some(self.hwnd), None, true);
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
        if self.multiline {
            self.caret_visible.set(true);
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
            return;
        }
        self.caret_visible.set(true);
        if self.caret_timer.get() == 0 {
            let blink = unsafe { GetCaretBlinkTime() };
            let blink = if blink == 0 || blink == u32::MAX {
                530
            } else {
                blink
            };
            // 必须给非零 ID：窗口计时器传 0 时，`SetTimer` 可能返回一个值，但系统投递的
            // `WM_TIMER.wParam` 是 0，跟返回值对不上，插入符就永远不会切到隐藏态。
            let timer = unsafe { SetTimer(Some(self.hwnd), CARET_TIMER_ID, blink, None) };
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

    /// 宿主窗句柄（脚本窗口的自定义 `text_changed` 消息按它找回控件）。
    pub(super) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 多行模式需要在建好、摆好尺寸后做一次 in-place 激活，RichEdit 才会排版并给出
    /// 行数 / 滚动范围；这里只激活排版，不抢焦点。
    pub(super) fn ensure_layout(&self) {
        if !self.multiline || self.in_place.get() {
            return;
        }
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
            if self.services.OnTxInPlaceActivate(&mut rect).is_ok() {
                self.in_place.set(true);
            }
        }
    }

    /// 多行编辑框当前的横向 / 纵向滚动范围。
    pub(super) fn scroll_info(&self) -> (ScrollInfo, ScrollInfo) {
        if !self.multiline {
            return (ScrollInfo::default(), ScrollInfo::default());
        }
        let mut horizontal = ScrollInfo::default();
        let mut vertical = ScrollInfo::default();
        unsafe {
            let mut enabled = BOOL(0);
            let _ = self.services.TxGetHScroll(
                &mut horizontal.min,
                &mut horizontal.max,
                &mut horizontal.pos,
                &mut horizontal.page,
                &mut enabled,
            );
            let _ = self.services.TxGetVScroll(
                &mut vertical.min,
                &mut vertical.max,
                &mut vertical.pos,
                &mut vertical.page,
                &mut enabled,
            );
        }
        (horizontal, vertical)
    }

    /// 把多行编辑框滚到指定位置（像素）。
    pub(super) fn set_scroll(&self, x: i32, y: i32) {
        if !self.multiline {
            return;
        }
        let mut point = POINT { x, y };
        let mut result = LRESULT(0);
        unsafe {
            let _ = self.services.TxSendMessage(
                EM_SETSCROLLPOS,
                WPARAM(0),
                LPARAM(&mut point as *mut _ as isize),
                &mut result,
            );
        }
        self.notify_scroll();
    }

    fn notify_scroll(&self) {
        unsafe {
            let _ = PostMessageW(
                Some(self.owner),
                WM_TEXT_SCROLL,
                WPARAM(self.hwnd.0 as usize),
                LPARAM(0),
            );
        }
    }

    fn notify_wheel(&self, delta: i32) {
        unsafe {
            let _ = PostMessageW(
                Some(self.owner),
                WM_TEXT_WHEEL,
                WPARAM(self.hwnd.0 as usize),
                LPARAM(delta as isize),
            );
        }
    }

    /// 通知脚本窗重读文本（`TxNotify(EN_CHANGE)` 与输入消息兜底共用）。
    fn notify_text_changed(&self) {
        unsafe {
            let _ = PostMessageW(
                Some(self.owner),
                WM_TEXT_CHANGED,
                WPARAM(self.hwnd.0 as usize),
                LPARAM(0),
            );
        }
    }

    /// 通知脚本窗有按键按下。
    fn notify_key_pressed(&self, key: u32, modifier: u8) {
        let packed = ((modifier as isize) << 16) | key as isize;
        unsafe {
            let _ = PostMessageW(
                Some(self.owner),
                WM_TEXT_KEY,
                WPARAM(self.hwnd.0 as usize),
                LPARAM(packed),
            );
        }
    }

    /// 通知脚本窗编辑框获得 / 失去焦点。
    fn notify_focus(&self, focused: bool) {
        unsafe {
            let _ = PostMessageW(
                Some(self.owner),
                WM_TEXT_FOCUS,
                WPARAM(self.hwnd.0 as usize),
                LPARAM(isize::from(focused)),
            );
        }
    }

    /// 当前完整文本（UTF-8；脚本侧 `text_changed` / `get_text` 用）。
    pub(super) fn text_string(&self) -> String {
        String::from_utf16_lossy(&self.text())
    }

    /// 密码框显示文本：每个 UTF-16 单元画一个 `*`，真实 `text()` 不变。
    fn display_text(&self, text: &[u16]) -> Vec<u16> {
        if self.passkey {
            vec![b'*' as u16; text.len()]
        } else {
            text.to_vec()
        }
    }

    /// 脚本 `set_text`：整份替换，光标放到末尾。
    pub(super) fn set_text(&self, text: &str) {
        let mut result = LRESULT(0);
        let value = HSTRING::from(text);
        unsafe {
            let _ = self.services.TxSetText(&value);
            let length = text.encode_utf16().count();
            let _ = self.services.TxSendMessage(
                EM_SETSEL,
                WPARAM(length),
                LPARAM(length as isize),
                &mut result,
            );
        }
        self.notify_selection();
        self.caret_visible.set(true);
        self.scroll_x.set(0);
        if self.multiline {
            self.notify_scroll();
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// 全选；`false` 时把选区收在末尾（取消选择，光标留在原处附近）。
    pub(super) fn select_all(&self, select: bool) {
        let length = self.text().len();
        let mut result = LRESULT(0);
        unsafe {
            if select {
                let _ = self
                    .services
                    .TxSendMessage(EM_SETSEL, WPARAM(0), LPARAM(-1), &mut result);
            } else {
                let (_, end) = self.selection(length);
                let _ = self.services.TxSendMessage(
                    EM_SETSEL,
                    WPARAM(end),
                    LPARAM(end as isize),
                    &mut result,
                );
            }
        }
        self.notify_selection();
        self.caret_visible.set(true);
    }

    /// 把文本里的 `find` 全部替换成 `text`。
    pub(super) fn replace_text(&self, find: &str, text: &str) {
        if find.is_empty() {
            return;
        }
        let current = self.text_string();
        self.set_text(&current.replace(find, text));
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
            let mut x = 2 - self.scroll_x.get() + self.text_width(hdc, &text[..start]);
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
            let x = 2 - self.scroll_x.get() + self.text_width(hdc, &text[..start]);
            if x >= client.right || x < 0 {
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

    /// 鼠标客户区坐标补上横向滚动量，再交给 RichEdit 做命中 / 拖选。
    fn adjust_mouse(&self, lparam: LPARAM) -> LPARAM {
        let x = (lparam.0 & 0xFFFF) as i16 as i32 + self.scroll_x.get();
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
        let x = x.clamp(i16::MIN as i32, i16::MAX as i32) as i16 as u16 as isize;
        let y = y as i16 as u16 as isize;
        LPARAM((y << 16) | x)
    }

    fn ensure_visible(&self, hdc: HDC, text: &[u16], client: &RECT) -> i32 {
        if !self.active.get() {
            return self.scroll_x.get();
        }
        let width = self.text_width(hdc, text);
        let view = (client.right - 4).max(1);
        if width <= view {
            self.scroll_x.set(0);
            return 0;
        }
        let (_, end) = self.selection(text.len());
        let caret = 2 + self.text_width(hdc, &text[..end]);
        let mut scroll = self.scroll_x.get();
        if caret - scroll > view {
            scroll = caret - view;
        } else if caret - scroll < 2 {
            scroll = caret - 2;
        }
        let max = (width - view).max(0);
        scroll = scroll.clamp(0, max);
        self.scroll_x.set(scroll);
        scroll
    }

    fn paint(&self) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            if self.multiline {
                self.paint_multiline_buffered(hdc, &client);
                let _ = EndPaint(self.hwnd, &ps);
                return;
            }
            let _ = FillRect(hdc, &client, self.brush);
            let old_font = SelectObject(hdc, self.font);
            let _ = SetBkMode(hdc, TRANSPARENT);
            let text = self.text();
            let text = self.display_text(&text);
            let scroll = self.ensure_visible(hdc, &text, &client);
            let (start, end) = self.selection(text.len());
            if start < end {
                let first = 2 - scroll + self.text_width(hdc, &text[..start]);
                let second = 2 - scroll + self.text_width(hdc, &text[..end]);
                let selection = RECT {
                    left: first.min(second).max(0),
                    top: 0,
                    right: first
                        .max(second)
                        .max(first.min(second) + 1)
                        .min(client.right),
                    bottom: client.bottom.max(1),
                };
                let _ = FillRect(hdc, &selection, self.highlight.get());
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

    /// 多行 RichEdit 的双缓冲绘制：拖动选区时 RichEdit 会频繁整窗失效，
    /// 直接往窗口 DC 先填背景再 `TxDraw` 会看到未选中文字被反复擦掉 / 重画；
    /// 先画到内存 DC，最后一次性 `BitBlt`。
    fn paint_multiline_buffered(&self, hdc: HDC, client: &RECT) {
        let width = client.right.max(1);
        let height = client.bottom.max(1);
        unsafe {
            let memory = CreateCompatibleDC(Some(hdc));
            if memory.is_invalid() {
                let _ = FillRect(hdc, client, self.brush);
                self.paint_multiline(hdc, client);
                return;
            }
            let bitmap = CreateCompatibleBitmap(hdc, width, height);
            if bitmap.is_invalid() {
                let _ = DeleteDC(memory);
                let _ = FillRect(hdc, client, self.brush);
                self.paint_multiline(hdc, client);
                return;
            }
            let old = SelectObject(memory, bitmap.into());
            let _ = FillRect(memory, client, self.brush);
            self.paint_multiline(memory, client);
            let _ = BitBlt(hdc, 0, 0, width, height, Some(memory), 0, 0, SRCCOPY);
            let _ = SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
        }
    }

    /// 多行模式让 RichEdit 自己排版 / 画文字、选区和 IME 组合串。
    fn paint_multiline(&self, hdc: HDC, client: &RECT) {
        let mut bounds = RECTL {
            left: 0,
            top: 0,
            right: client.right.max(1),
            bottom: client.bottom.max(1),
        };
        let mut update = *client;
        unsafe {
            let _ = self.services.TxDraw(
                DVASPECT_CONTENT,
                -1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hdc,
                HDC::default(),
                &mut bounds,
                std::ptr::null_mut(),
                &mut update,
                0,
                0,
                0,
            );
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
            let _ = DeleteObject(self.highlight.get().into());
            let _ = DeleteObject(self.caret_brush.into());
        }
    }
}

/// `CreateTextServices` 的宿主实现。绘制 / 消息都由 RichEdit 引擎回调进来。
#[implement(ITextHost)]
struct TextHost {
    hwnd: HWND,
    owner: HWND,
    passkey: bool,
    multiline: bool,
    auto_wrap: bool,
    background: COLORREF,
    text_color: COLORREF,
    selection_background: Rc<Cell<COLORREF>>,
    selection_text: COLORREF,
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

    fn TxNotify(&self, inotify: u32, _pv: *mut c_void) -> Result<()> {
        if inotify == EN_CHANGE {
            unsafe {
                let _ = PostMessageW(
                    Some(self.owner),
                    WM_TEXT_CHANGED,
                    WPARAM(self.hwnd.0 as usize),
                    LPARAM(0),
                );
            }
        }
        Ok(())
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
        xwidth: i32,
        yheight: i32,
    ) -> BOOL {
        if !self.multiline {
            return false.into();
        }
        unsafe { CreateCaret(self.hwnd, None, xwidth.max(1), yheight.max(1)).is_ok() }.into()
    }

    fn TxShowCaret(&self, fshow: BOOL) -> BOOL {
        if !self.multiline {
            return false.into();
        }
        unsafe {
            if fshow.as_bool() {
                let _ = ShowCaret(Some(self.hwnd));
            } else {
                let _ = HideCaret(Some(self.hwnd));
            }
        }
        true.into()
    }

    fn TxSetCaretPos(&self, x: i32, y: i32) -> BOOL {
        if !self.multiline {
            return false.into();
        }
        unsafe { SetCaretPos(x, y).is_ok() }.into()
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
            if self.multiline {
                let inset = 2.min(height / 2);
                *prc = RECT {
                    left: 2,
                    top: inset,
                    right: 2,
                    bottom: inset,
                };
                return Ok(());
            }
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
        if self.multiline {
            if nindex == COLOR_WINDOW {
                return self.background;
            }
            if nindex == COLOR_WINDOWTEXT {
                return self.text_color;
            }
            if nindex == COLOR_HIGHLIGHT {
                return self.selection_background.get();
            }
            if nindex == COLOR_HIGHLIGHTTEXT {
                return self.selection_text;
            }
        }
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
        let bars = if self.multiline {
            WS_VSCROLL.0 | if self.auto_wrap { 0 } else { WS_HSCROLL.0 }
        } else {
            0
        };
        unsafe {
            *pdwscrollbar = bars;
        }
        Ok(())
    }

    fn TxGetPasswordChar(&self) -> Result<i8> {
        Ok(if self.passkey { 42 } else { 0 })
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

    fn TxGetPropertyBits(&self, dwmask: u32, pdwbits: *mut u32) -> Result<()> {
        let mut bits = TXTBIT_RICHTEXT | TXTBIT_SAVESELECTION;
        if self.multiline {
            bits |= TXTBIT_MULTILINE;
            if self.auto_wrap {
                bits |= TXTBIT_WORDWRAP;
            }
        }
        unsafe {
            *pdwbits = bits & dwmask;
        }
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
        hInstance: module_handle(),
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

fn create_font(family: Option<&str>, size: f32, dpi: u32, font_flag: u8) -> HGDIOBJ {
    use windows::Win32::Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        FF_DONTCARE, FW_BOLD, FW_NORMAL, OUT_DEFAULT_PRECIS,
    };
    let height = -((size * dpi as f32 / 72.0).round() as i32).max(1);
    let family = family.map(HSTRING::from).unwrap_or_default();
    let weight = if font_flag & 1 != 0 {
        FW_BOLD.0 as i32
    } else {
        FW_NORMAL.0 as i32
    };
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            u32::from(font_flag & 2 != 0),
            u32::from(font_flag & 4 != 0),
            u32::from(font_flag & 8 != 0),
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

/// 当前修饰键位：1 shift / 2 ctrl / 4 alt / 8 win。
fn modifier_bits() -> u8 {
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 };
    let alt = unsafe { GetKeyState(VK_MENU.0 as i32) < 0 };
    let win = unsafe { GetKeyState(VK_LWIN.0 as i32) < 0 || GetKeyState(VK_RWIN.0 as i32) < 0 };
    u8::from(shift) | (u8::from(ctrl) << 1) | (u8::from(alt) << 2) | (u8::from(win) << 3)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // 自定义消息转发给 RichEdit；文字和选区由宿主用 GDI 自绘。
    if msg == WM_PAINT {
        with_host(hwnd, |host| host.paint());
        return LRESULT(0);
    }
    // 输入法先看键；被吃掉就不再给 RichEdit（WM_CHAR 也跟着吞）。
    if matches!(msg, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP) {
        if matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN) {
            let modifier = modifier_bits();
            with_host(hwnd, |host| {
                host.notify_key_pressed(wparam.0 as u32, modifier)
            });
        }
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
    if msg == WM_MOUSEWHEEL && with_host(hwnd, |host| host.multiline).unwrap_or(false) {
        let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        with_host(hwnd, |host| host.notify_wheel(delta));
        return LRESULT(0);
    }
    if msg == WM_MOUSEMOVE && (wparam.0 as u32 & 0x0001) == 0 {
        return LRESULT(0);
    }
    if msg == WM_SETFOCUS {
        with_host(hwnd, |host| {
            host.activate();
            host.notify_focus(true);
        });
        return LRESULT(0);
    }
    if msg == WM_KILLFOCUS {
        with_host(hwnd, |host| {
            host.deactivate();
            host.notify_focus(false);
        });
        return LRESULT(0);
    }
    let lparam = if matches!(
        msg,
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_MOUSEMOVE
    ) {
        with_host(hwnd, |host| host.adjust_mouse(lparam)).unwrap_or(lparam)
    } else {
        lparam
    };
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
            | WM_MOUSEWHEEL
            | WM_TIMER
            | WM_SETFONT
            | WM_SIZE
    ) && let Some(result) = with_host(hwnd, |host| host.send(msg, wparam, lparam))
    {
        if matches!(msg, WM_LBUTTONUP | WM_KEYUP) {
            with_host(hwnd, |host| host.notify_selection());
        }
        if matches!(
            msg,
            WM_CHAR | WM_KEYDOWN | WM_KEYUP | WM_IME_CHAR | WM_IME_COMPOSITION
        ) {
            with_host(hwnd, |host| host.notify_text_changed());
        }
        if matches!(
            msg,
            WM_MOUSEWHEEL
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_LBUTTONDBLCLK
                | WM_MOUSEMOVE
                | WM_KEYDOWN
                | WM_CHAR
                | WM_SIZE
        ) {
            with_host(hwnd, |host| host.notify_scroll());
        }
        let left_button = (wparam.0 as u32 & 0x0001) != 0;
        if matches!(
            msg,
            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_KEYDOWN | WM_CHAR
        ) || (msg == WM_MOUSEMOVE && left_button)
        {
            with_host(hwnd, |host| unsafe {
                host.poke_caret();
                // 多行 RichEdit 自己会在选区变化时失效对应区域；鼠标拖动时再整窗
                // InvalidateRect 一次，会把所有文字反复擦掉重画。
                if !host.multiline || msg != WM_MOUSEMOVE {
                    let _ = InvalidateRect(Some(host.hwnd), None, false);
                }
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
