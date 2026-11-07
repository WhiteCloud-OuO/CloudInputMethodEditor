//! 独立窗口宿主：普通 Win32 窗口 + 消息循环 + `Document` 渲染。
//!
//! 现有窗口模板都是 IME 悬浮窗（`WS_POPUP | TOPMOST | NOACTIVATE`）。这里补的是**普通应用窗口**：
//! 有系统边框、任务栏项和正常激活语义，让 `cloud-ui` 能作为独立 exe 的 UI 库使用。
//!
//! 用法：
//!
//! ```ignore
//! struct Demo;
//!
//! impl Application for Demo {
//!     fn update(&mut self, ctx: &mut Context) {
//!         for event in ctx.events() { /* 处理输入 */ }
//!     }
//!
//!     fn view(&mut self, ctx: &Context) -> cloudime_render::Document {
//!         let (width, height) = ctx.size();
//!         cloudime_render::Document::empty(width, height)
//!     }
//! }
//!
//! run_native(WindowOptions::default(), Demo)?;
//! ```

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use cloudime_render::{Color, Document, Element, FontSlot, Pixmap, Region, Shadow, TextBoxScroll};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, ClientToScreen, DIB_RGB_COLORS, EndPaint,
    HDC, InvalidateRect, PAINTSTRUCT, SRCCOPY, ScreenToClient, StretchDIBits, UpdateWindow,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetMessageW, GetWindowRect, HTCAPTION, HTCLIENT, HTTRANSPARENT, IDC_ARROW,
    IsIconic, IsWindowVisible, IsZoomed, LoadCursorW, MINMAXINFO, MSG, PostMessageW,
    PostQuitMessage, RegisterWindowMessageW, SIZE_MINIMIZED, SW_HIDE, SW_MAXIMIZE, SW_MINIMIZE,
    SW_RESTORE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos,
    ShowWindow, TranslateMessage, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_MOVE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_PAINT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS, WM_SHOWWINDOW, WM_SIZE, WNDCLASSEXW,
    WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_TOPMOST, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
    WS_OVERLAPPEDWINDOW, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::RenderSettings;
use crate::layered;
use crate::monitor;
use crate::painter::{Painter, SharedPainter};
use crate::text_edit::{
    ScrollInfo, TextHostWindow, WM_TEXT_CHANGED, WM_TEXT_FOCUS, WM_TEXT_KEY, WM_TEXT_SCROLL,
    WM_TEXT_WHEEL,
};
use crate::window::module_handle;
use crate::window_class::WindowClass;

mod tray;

use crate::mask::{MaskWindow, WindowMask};
use tray::TrayIcon;
pub use tray::{TrayEvent, TrayMenuItem, TrayOptions};

/// 托盘图标的回调消息。
const WM_TRAY_ICON: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x50;

thread_local! {
    /// 同一个 UI 线程上所有应用窗口，按 HWND 找；窗口过程只碰自己那块。
    static WINDOWS: RefCell<HashMap<isize, Rc<RefCell<AppWindow>>>> = RefCell::new(HashMap::new());

    /// 在窗口过程里新开的窗口先登记，等这一帧的借用结束再 `ShowWindow`（避免同步焦点消息重入）。
    static PENDING_SHOW: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}

static CLASS: WindowClass = WindowClass::new();

/// 窗口边框。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Decorations {
    /// 系统边框、标题栏和任务栏项（缺省）。
    #[default]
    System,

    /// 无边框：窗口内容完全自绘，可配合阴影与圆角。
    None,
}

/// 独立窗口的创建参数。尺寸单位是逻辑点（96 DPI 下的像素）。
#[derive(Debug, Clone)]
pub struct WindowOptions {
    /// 窗口标题。
    pub title: String,

    /// 初始客户区尺寸（点）。
    pub size: (f32, f32),

    /// 最小客户区尺寸（点）；`None` = 不限制。
    pub min_size: Option<(f32, f32)>,

    /// 系统边框，还是完全自绘无边框。
    pub decorations: Decorations,

    /// 是否置顶；缺省 false。
    pub top_most: bool,

    /// 无边框时是否在窗口外画一圈阴影；缺省 true。
    pub shadow: bool,

    /// 无边框时的圆角半径（点）；同时用于窗口阴影轮廓。
    pub corner_radius: f32,

    /// 自绘标题栏高度（点）；`Some` 时这一段空白可拖动、可双击最大化。
    ///
    /// 只有 `Decorations::None` 用；系统边框走系统标题栏。标题栏里的按钮区域仍算客户区，
    /// 所以在 `View::title_bar` 的按钮上点击不会被当成拖动。
    pub caption_height: Option<f32>,

    /// 通知区域（托盘）图标；窗口创建时注册，窗口销毁时移除。
    pub tray: Option<TrayOptions>,

    /// 窗口内容蒙版；`None` 表示没有蒙版。只覆盖客户区，不拦截鼠标 / 键盘。
    pub mask: Option<WindowMask>,

    /// 字体、主题与候选窗尺寸；独立应用一般用默认值。
    pub render_settings: RenderSettings,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: "Cloud UI".to_owned(),
            size: (800.0, 600.0),
            min_size: Some((320.0, 240.0)),
            decorations: Decorations::System,
            top_most: false,
            shadow: true,
            corner_radius: 12.0,
            caption_height: None,
            tray: None,
            mask: None,
            render_settings: RenderSettings::default(),
        }
    }
}

/// 独立窗口里的一次输入 / 尺寸变化。
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// 客户区尺寸变化（点）。
    Resized { width: f32, height: f32 },

    /// 鼠标移动（点）。
    MouseMoved { x: f32, y: f32 },

    /// 鼠标离开客户区；`View::pointer` 的悬停状态应清掉。
    MouseLeft,

    /// 鼠标按键按下 / 松开（点）。
    MouseButton {
        button: MouseButton,
        pressed: bool,
        x: f32,
        y: f32,
    },

    /// 左键在某个 `View` 按钮 / 区域里按下并松开；`name` 是 `View::button` 的名字。
    RegionClicked(String),

    /// 滚轮；`delta` 以 120 为一格。
    MouseWheel { delta: f32 },

    /// 键按下 / 松开；`vk` 是 Win32 虚拟键码。
    Key { vk: u32, pressed: bool },

    /// 字符输入。
    Text(String),

    /// `View::text_edit` 的文本变化；`name` 是控件名。
    TextChanged { name: String, text: String },

    /// `View::text_edit` 获得 / 失去焦点。
    TextFocusChanged { name: String, focused: bool },

    /// 焦点在 `View::text_edit` 里时按下按键（按键由 RichEdit 宿主接收）。
    TextKey {
        name: String,
        vk: u32,
        modifiers: u8,
    },

    /// 托盘图标事件。
    Tray(TrayEvent),

    /// 窗口获得 / 失去焦点。
    Focused(bool),
}

/// 鼠标按键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// 窗口命令：从 `Context` 里排队，等这一帧的借用结束再执行，
/// 避免 `ShowWindow` / `DestroyWindow` 同步回调把 `APP_WINDOW` 再借一次。
#[derive(Debug, Clone, Copy)]
enum WindowCommand {
    Minimize,
    ToggleMaximize,
    Show,
    Hide,
    Close,
}

/// 独立窗口应用。
pub trait Application {
    /// 处理这一帧之前累积的事件。
    fn update(&mut self, ctx: &mut Context);

    /// 返回这一帧要画的内容；尺寸按 [`Context::size`] 的客户区来。
    fn view(&mut self, ctx: &Context) -> Document;
}

/// 应用在 [`Application::update`] / [`Application::view`] 里能看到的窗口状态。
pub struct Context {
    events: Vec<AppEvent>,
    size: (f32, f32),
    dpi: u32,
    hwnd: HWND,
    close: Cell<bool>,
    command: Cell<Option<WindowCommand>>,
    mask: Cell<Option<WindowMask>>,
    painter: SharedPainter,
}

impl Context {
    /// 这一帧要处理的事件。
    pub fn events(&self) -> &[AppEvent] {
        &self.events
    }

    /// 客户区尺寸（点）。
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// 当前窗口 DPI（96 = 100%）。
    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    /// 量一段文字（点）：`View::build` 的布局用。
    pub fn measure_text(&self, text: &str, slot: FontSlot, size: f32) -> (f32, f32) {
        self.painter
            .borrow_mut()
            .as_mut()
            .map(|painter| painter.measure_text(text, slot, size))
            .unwrap_or((0.0, 0.0))
    }

    /// 请求下一帧重画。输入事件本身已经会触发重画，这里给定时器 / 异步状态用。
    pub fn request_redraw(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// 请求关闭窗口。
    pub fn close(&self) {
        self.close.set(true);
    }

    /// 窗口是否最大化 / 还原态。
    pub fn is_maximized(&self) -> bool {
        unsafe { IsZoomed(self.hwnd).as_bool() }
    }

    /// 最小化窗口。
    pub fn minimize(&self) {
        self.command.set(Some(WindowCommand::Minimize));
    }

    /// 最大化 / 还原切换。
    pub fn toggle_maximize(&self) {
        self.command.set(Some(WindowCommand::ToggleMaximize));
    }

    /// 显示窗口并提到前台。
    pub fn show(&self) {
        self.command.set(Some(WindowCommand::Show));
    }

    /// 隐藏窗口（例如收进托盘）。
    pub fn hide(&self) {
        self.command.set(Some(WindowCommand::Hide));
    }

    /// 当前窗口蒙版。
    pub fn mask(&self) -> Option<WindowMask> {
        self.mask.get()
    }

    /// 设置窗口蒙版；`None` 关闭。`update` 返回后、这一帧贴图前生效。
    pub fn set_mask(&self, mask: Option<WindowMask>) {
        self.mask.set(mask);
    }

    /// 再开一个普通窗口（同一个 UI 线程、同一个消息循环）。
    ///
    /// 新窗口关闭后不影响已有窗口；最后一个窗口关闭时 `run_native` 的消息循环退出。
    /// 建议在 `update` 里调用；失败只记一条日志并返回 `None`。
    pub fn open_window<A>(&self, options: WindowOptions, app: A) -> Option<WindowId>
    where
        A: Application + 'static,
    {
        let (hwnd, window) = match create_window(options, Box::new(app), Some(self.hwnd)) {
            Ok(created) => created,
            Err(error) => {
                tracing::warn!(%error, "打开第二个窗口失败");
                return None;
            }
        };
        let id = WindowId(hwnd.0 as isize);
        WINDOWS.with(|windows| windows.borrow_mut().insert(id.0, window));
        PENDING_SHOW.with(|pending| pending.borrow_mut().push(id.0));
        Some(id)
    }
}

/// 同一 UI 线程上的一块应用窗口。
///
/// 只能在创建它的 UI 线程上使用；窗口关掉后查询返回 `false`，关闭 / 聚焦是空操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(isize);

impl WindowId {
    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }

    /// 窗口是否还开着。
    pub fn is_open(self) -> bool {
        WINDOWS.with(|windows| windows.borrow().contains_key(&self.0))
    }

    /// 请求关闭：走 `WM_CLOSE`，和点标题栏的关闭按钮同一条路。
    pub fn close(self) {
        if self.is_open() {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// 把窗口提到前台。
    pub fn focus(self) {
        if self.is_open() {
            unsafe {
                let _ = SetForegroundWindow(self.hwnd());
            }
        }
    }

    /// 窗口是否最大化。
    pub fn is_maximized(self) -> bool {
        self.is_open() && is_zoomed(self.hwnd())
    }
}

/// 打开一个普通窗口并跑消息循环，直到所有窗口关闭。
///
/// 必须在主线程调用；窗口和 `cloud-ui` 的状态都在这个线程上，不能跨线程移动。
/// 用 [`Context::open_window`] 可以再开窗口，共用一个消息循环。
pub fn run_native<A>(options: WindowOptions, app: A) -> Result<()>
where
    A: Application + 'static,
{
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    // TSF / RichEdit 需要 STA；已经初始化成别的 apartment 时不让启动失败，窗口本身照常能跑。
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let (hwnd, window) = create_window(options, Box::new(app), None)?;
    WINDOWS.with(|windows| windows.borrow_mut().insert(hwnd.0 as isize, window));
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
    }

    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    WINDOWS.with(|windows| windows.borrow_mut().clear());
    PENDING_SHOW.with(|pending| pending.borrow_mut().clear());
    Ok(())
}

/// 应用 `View` 画的整窗背景色（第一个铺满文档的矩形）；没有就用上一帧 / 默认深色。
fn document_background(document: &Document) -> Option<Color> {
    match document.elements.first()? {
        Element::Rect {
            x,
            y,
            width,
            height,
            color,
            ..
        } if *x <= 0.0 && *y <= 0.0 && *width >= document.width && *height >= document.height => {
            Some(*color)
        }
        _ => None,
    }
}

fn create_window(
    options: WindowOptions,
    app: Box<dyn Application>,
    near: Option<HWND>,
) -> Result<(HWND, Rc<RefCell<AppWindow>>)> {
    let class_name = w!("CloudUIAppWindow");
    CLASS.ensure(|| WNDCLASSEXW {
        lpfnWndProc: Some(wndproc),
        hInstance: module_handle(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: class_name,
        ..Default::default()
    })?;

    let layered = options.decorations == Decorations::None;
    let shadow = layered && options.shadow;
    let dpi = near
        .map(|hwnd| unsafe { GetDpiForWindow(hwnd) })
        .unwrap_or_else(|| unsafe { GetDpiForSystem() })
        .max(96);
    let scale = dpi as f32 / 96.0;
    let content_width = (options.size.0 * scale).round().max(1.0) as i32;
    let content_height = (options.size.1 * scale).round().max(1.0) as i32;
    let shadow_margin = if shadow {
        (Shadow::panel().margin() * scale).round() as i32
    } else {
        0
    };

    let mut ex_style = WS_EX_APPWINDOW;
    let window_style = if layered {
        ex_style |= WS_EX_LAYERED;
        let mut style = WS_POPUP;
        if options.caption_height.is_some() {
            // 自绘标题栏要能最小化 / 最大化；不加 WS_THICKFRAME，避免系统边框和命中区。
            style |= WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        }
        style
    } else {
        WS_OVERLAPPEDWINDOW
    };
    if options.top_most {
        ex_style |= WS_EX_TOPMOST;
    }

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: content_width,
        bottom: content_height,
    };
    let (width, height) = if layered {
        (
            content_width + shadow_margin * 2,
            content_height + shadow_margin * 2,
        )
    } else {
        unsafe { AdjustWindowRectEx(&mut rect, window_style, false, ex_style)? };
        (rect.right - rect.left, rect.bottom - rect.top)
    };
    let width = width.max(1);
    let height = height.max(1);
    let work = near.map_or_else(monitor::primary_work_area, monitor::work_area_of_window);
    let x = work.left + ((work.right - work.left - width) / 2).max(0);
    let y = work.top + ((work.bottom - work.top - height) / 2).max(0);

    let title = HSTRING::from(options.title.as_str());
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class_name,
            PCWSTR(title.as_ptr()),
            window_style,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(module_handle()),
            None,
        )?
    };

    let painter: SharedPainter = Rc::new(RefCell::new(None));
    Painter::configure(&painter, &options.render_settings);
    if layered && let Some(painter) = painter.borrow_mut().as_mut() {
        painter.set_corner_radius(options.corner_radius);
    }
    let tray = options.tray.as_ref().and_then(|tray| {
        let tooltip = if tray.tooltip.is_empty() {
            options.title.as_str()
        } else {
            tray.tooltip.as_str()
        };
        match TrayIcon::new(hwnd, tooltip, tray) {
            Ok(icon) => Some(Rc::new(icon)),
            Err(error) => {
                tracing::warn!(%error, "托盘图标注册失败");
                None
            }
        }
    });
    let window = Rc::new(RefCell::new(AppWindow {
        app,
        painter,
        events: Vec::new(),
        dpi,
        min_size: options.min_size,
        regions: Vec::new(),
        pressed_region: None,
        layered,
        shadow,
        shadow_margin,
        top_most: options.top_most,
        caption_height: options.caption_height,
        pending_command: None,
        edits: Vec::new(),
        retired: Vec::new(),
        tray,
        mask: options.mask,
        mask_window: None,
        corner_radius: options.corner_radius,
        last_pointer: None,
    }));
    window.borrow_mut().sync_mask(hwnd);
    Ok((hwnd, window))
}

struct AppWindow {
    app: Box<dyn Application>,
    painter: SharedPainter,
    events: Vec<AppEvent>,
    dpi: u32,
    min_size: Option<(f32, f32)>,
    regions: Vec<Region>,
    pressed_region: Option<String>,
    layered: bool,
    shadow: bool,
    shadow_margin: i32,
    top_most: bool,
    caption_height: Option<f32>,
    pending_command: Option<WindowCommand>,
    edits: Vec<EditHost>,
    /// 已经从 document 里消失、但还没析构的宿主；等窗口借用结束再 drop。
    retired: Vec<EditHost>,
    tray: Option<Rc<TrayIcon>>,
    /// 当前蒙版配置；`None` 时蒙版窗保留但隐藏。
    mask: Option<WindowMask>,
    mask_window: Option<MaskWindow>,
    corner_radius: f32,
    /// 上一次鼠标位置（内容坐标）；连续重复的 `WM_MOUSEMOVE` 不再触发重画。
    last_pointer: Option<(f32, f32)>,
}

/// 一个 `View::text_edit` 对应的 RichEdit 宿主。
struct EditHost {
    name: String,
    host: Rc<TextHostWindow>,
    colors: EditColors,
}

/// 宿主建窗时用的颜色；主题切换后颜色对不上就重建宿主。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EditColors {
    background: u32,
    text: u32,
    selection: u32,
    selection_text: u32,
    caret: u32,
}

/// 一帧里一个编辑框的只读描述；sync 时不用反复借用 `Document`。
struct EditSpec {
    path: Vec<usize>,
    name: String,
    text: String,
    font: FontSlot,
    font_name: Option<String>,
    size: Option<f32>,
    fill: Option<Color>,
    text_color: Option<Color>,
    selection_background: Option<Color>,
    selection_color: Option<Color>,
    caret_color: Option<Color>,
    font_flag: u8,
    read_only: bool,
    deselect_on_blur: bool,
    passkey: bool,
    multiline: bool,
    auto_line_wrap: bool,
    line_spacing: u16,
    x: f32,
    y: f32,
    clip: Option<(f32, f32, f32, f32)>,
    visible: bool,
}

fn edit_specs(document: &Document) -> Vec<EditSpec> {
    let mut specs = Vec::new();
    for placement in document.text_edits() {
        let Some(Element::TextEdit(edit)) = document.element_at_path(&placement.path) else {
            continue;
        };
        if edit.region_name.is_empty() {
            continue;
        }
        specs.push(EditSpec {
            path: placement.path.clone(),
            name: edit.region_name.clone(),
            text: edit.text.clone(),
            font: edit.font,
            font_name: edit.font_name.clone(),
            size: edit.size,
            fill: edit.fill,
            text_color: edit.text_color,
            selection_background: edit.selection_background,
            selection_color: edit.selection_color,
            caret_color: edit.caret_color,
            font_flag: edit.font_flag,
            read_only: edit.read_only,
            deselect_on_blur: edit.deselect_on_blur,
            passkey: edit.passkey,
            multiline: false,
            auto_line_wrap: false,
            line_spacing: 0,
            x: placement.x,
            y: placement.y,
            clip: placement.clip,
            visible: placement.visible,
        });
    }
    for placement in document.text_boxes() {
        let Some(Element::TextBox(edit)) = document.element_at_path(&placement.path) else {
            continue;
        };
        if edit.region_name.is_empty() {
            continue;
        }
        specs.push(EditSpec {
            path: placement.path.clone(),
            name: edit.region_name.clone(),
            text: edit.text.clone(),
            font: edit.font,
            font_name: edit.font_name.clone(),
            size: edit.size,
            fill: edit.fill,
            text_color: edit.text_color,
            selection_background: edit.selection_background,
            selection_color: edit.selection_color,
            caret_color: edit.caret_color,
            font_flag: edit.font_flag,
            read_only: edit.read_only,
            deselect_on_blur: edit.deselect_on_blur,
            passkey: false,
            multiline: true,
            auto_line_wrap: edit.auto_line_wrap,
            line_spacing: edit.line_spacing,
            x: placement.x,
            y: placement.y,
            clip: placement.clip,
            visible: placement.visible,
        });
    }
    specs
}

/// RichEdit 的滚动范围转成渲染器画滚动条用的状态。
fn scroll_state(info: ScrollInfo) -> TextBoxScroll {
    let min = info.min.max(0) as u64;
    let page = info.page.max(0) as u64;
    let extent = info.max.max(info.min) as u64;
    let max = extent.saturating_sub(page).max(min);
    let value = (info.pos.max(info.min) as u64).clamp(min, max);
    TextBoxScroll {
        min,
        max,
        value,
        page,
        visible: max > min,
    }
}

impl AppWindow {
    fn push(&mut self, hwnd: HWND, event: AppEvent) {
        self.events.push(event);
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    fn paint(&mut self, hwnd: HWND, hdc: HDC) {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rect);
        }
        let margin = self.margin(hwnd);
        let zoomed = is_zoomed(hwnd);
        let width = ((rect.right - rect.left) - margin * 2).max(1);
        let height = ((rect.bottom - rect.top) - margin * 2).max(1);
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = dpi as f32 / 96.0;
        let mut ctx = Context {
            events: std::mem::take(&mut self.events),
            size: (width as f32 / scale, height as f32 / scale),
            dpi,
            hwnd,
            close: Cell::new(false),
            command: Cell::new(None),
            mask: Cell::new(self.mask),
            painter: self.painter.clone(),
        };
        let edit_hosts = self
            .edits
            .iter()
            .map(|edit| edit.host.clone())
            .collect::<Vec<_>>();
        let followers = self
            .mask_window
            .as_ref()
            .map(|window| window.hwnd())
            .into_iter()
            .collect::<Vec<_>>();
        let current = crate::current::enter(
            hwnd,
            self.painter.clone(),
            self.top_most,
            edit_hosts,
            followers,
        );
        self.app.update(&mut ctx);
        let mut document = self.app.view(&ctx);
        crate::current::set_background(document_background(&document));
        drop(current);
        if ctx.close.get() {
            self.pending_command = Some(WindowCommand::Close);
        }
        if let Some(command) = ctx.command.get() {
            self.pending_command = Some(command);
        }
        self.regions = document.regions.clone();
        self.mask = ctx.mask.get();
        self.sync_text_edits(hwnd, &mut document, dpi);
        self.sync_mask(hwnd);
        if self.layered {
            let shadow = (self.shadow && !zoomed).then_some(Shadow::panel());
            if let Some(rendered) = self.painter.borrow_mut().as_mut().and_then(|painter| {
                painter.render_document(&document, dpi, shadow.is_some(), false)
            }) {
                let mut window_rect = RECT::default();
                unsafe {
                    let _ = GetWindowRect(hwnd, &mut window_rect);
                }
                // 位图已经包含阴影白边，`UpdateLayeredWindow` 的目标点是窗口左上角；
                // 传内容区坐标会每帧把窗口往右下挪一个白边宽度。
                let position = (window_rect.left, window_rect.top);
                if let Err(error) = layered::present(hwnd, &rendered.rendered.pixmap, position, 255)
                {
                    tracing::warn!(%error, "无边框窗口贴图失败");
                }
            }
        } else if let Some(rendered) = self
            .painter
            .borrow_mut()
            .as_mut()
            .and_then(|painter| painter.render_document(&document, dpi, false, true))
        {
            blit(hdc, &rendered.rendered.pixmap, width, height);
        }
    }

    /// 内容区四周的留白：无边框阴影边；最大化时去掉，内容贴满窗口。
    fn margin(&self, hwnd: HWND) -> i32 {
        if self.layered && !is_zoomed(hwnd) {
            self.shadow_margin
        } else {
            0
        }
    }

    /// 让蒙版窗跟住内容区、可见性和层级；主窗口隐藏 / 最小化时一起收起。
    fn sync_mask(&mut self, hwnd: HWND) {
        let Some(mask) = self.mask else {
            if let Some(window) = &self.mask_window {
                window.show(false);
            }
            return;
        };
        if mask.color.a == 0 {
            if let Some(window) = &self.mask_window {
                window.show(false);
            }
            return;
        }
        if self.mask_window.is_none() {
            match MaskWindow::new(hwnd, self.top_most, mask.color) {
                Ok(window) => self.mask_window = Some(window),
                Err(error) => {
                    tracing::warn!(%error, "窗口蒙版创建失败");
                    self.mask = None;
                    return;
                }
            }
        }
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        }
        let margin = self.margin(hwnd);
        let width = (client.right - client.left - margin * 2).max(1);
        let height = (client.bottom - client.top - margin * 2).max(1);
        let mut origin = POINT {
            x: margin,
            y: margin,
        };
        unsafe {
            let _ = ClientToScreen(hwnd, &mut origin);
        }
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let radius = if self.layered && !is_zoomed(hwnd) {
            (self.corner_radius * dpi as f32 / 96.0).max(0.0)
        } else {
            0.0
        };
        let visible = unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() };
        let Some(window) = self.mask_window.as_mut() else {
            return;
        };
        window.set_color(mask.color);
        window.set_bounds(origin.x, origin.y, width, height, radius);
        window.show(visible);
        if visible {
            window.raise(self.top_most);
        }
    }

    /// 主窗口隐藏 / 最小化时，把不会自动跟着收起的顶层宿主和蒙版一起藏掉。
    fn hide_owned(&mut self) {
        for edit in &self.edits {
            edit.host.show(false);
        }
        if let Some(mask) = &self.mask_window {
            mask.show(false);
        }
    }

    /// 窗口隐藏 / 最小化时走一遍事件处理，不渲染；托盘菜单里的显示 / 退出靠它生效。
    fn pump(&mut self, hwnd: HWND) {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rect);
        }
        let margin = self.margin(hwnd);
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = dpi as f32 / 96.0;
        let mut ctx = Context {
            events: std::mem::take(&mut self.events),
            size: (
                ((rect.right - rect.left) - margin * 2).max(1) as f32 / scale,
                ((rect.bottom - rect.top) - margin * 2).max(1) as f32 / scale,
            ),
            dpi,
            hwnd,
            close: Cell::new(false),
            command: Cell::new(None),
            mask: Cell::new(self.mask),
            painter: self.painter.clone(),
        };
        let edit_hosts = self
            .edits
            .iter()
            .map(|edit| edit.host.clone())
            .collect::<Vec<_>>();
        let followers = self
            .mask_window
            .as_ref()
            .map(|window| window.hwnd())
            .into_iter()
            .collect::<Vec<_>>();
        let current = crate::current::enter(
            hwnd,
            self.painter.clone(),
            self.top_most,
            edit_hosts,
            followers,
        );
        self.app.update(&mut ctx);
        drop(current);
        if ctx.close.get() {
            self.pending_command = Some(WindowCommand::Close);
        }
        if let Some(command) = ctx.command.get() {
            self.pending_command = Some(command);
        }
        self.mask = ctx.mask.get();
        self.sync_mask(hwnd);
    }

    fn set_dpi(&mut self, hwnd: HWND, dpi: u32) {
        self.dpi = dpi;
        if self.layered {
            self.shadow_margin = if self.shadow {
                (Shadow::panel().margin() * dpi as f32 / 96.0).round() as i32
            } else {
                0
            };
        }
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rect);
        }
        let scale = dpi as f32 / 96.0;
        let margin = self.margin(hwnd);
        self.push(
            hwnd,
            AppEvent::Resized {
                width: (rect.right - margin * 2) as f32 / scale,
                height: (rect.bottom - margin * 2) as f32 / scale,
            },
        );
        self.sync_mask(hwnd);
    }

    /// 按当前 `Document` 摆放 / 增删 RichEdit 宿主（`View::text_edit` / `text_box`）。
    fn sync_text_edits(&mut self, hwnd: HWND, document: &mut Document, dpi: u32) {
        let visible = unsafe { IsWindowVisible(hwnd).as_bool() };
        // 蒙版在上面时不要再把宿主抬到蒙版之上，否则每帧 z-order 来回切会让编辑框背景闪。
        let masked = self.mask.is_some_and(|mask| mask.color.a > 0);
        let scale = dpi as f32 / 96.0;
        let offset = self.margin(hwnd);
        let specs = edit_specs(document);
        let mut old = std::mem::take(&mut self.edits);
        let mut next = Vec::new();
        for spec in specs {
            let (family, size, colors) = self.edit_settings(&spec);
            let (host, created) = match old
                .iter()
                .position(|item| item.name == spec.name && item.colors == colors)
            {
                Some(index) => (old.swap_remove(index).host, false),
                None => match self.create_text_host(hwnd, &spec, family, size, colors, dpi) {
                    Some(host) => (host, true),
                    None => continue,
                },
            };
            if host.text_string() != spec.text {
                host.set_text(&spec.text);
            }
            if created
                && spec.multiline
                && let Some(Element::TextBox(edit)) = document.element_at_path(&spec.path)
            {
                // 多行 RichEdit 要先 OnTxInPlaceActivate 才会排版；先用初始文本区让它排一次。
                let (_, _, width, height) = edit.edit_rect();
                let _ = host.set_bounds(
                    0,
                    0,
                    (width * scale).round().max(1.0) as i32,
                    (height * scale).round().max(1.0) as i32,
                    !masked,
                );
                host.ensure_layout();
            }
            if !visible || !spec.visible {
                host.show(false);
                next.push(EditHost {
                    name: spec.name.clone(),
                    host,
                    colors,
                });
                continue;
            }
            let (x, y, width, height) = match document.element_at_path(&spec.path) {
                Some(Element::TextEdit(edit)) => edit.edit_rect(),
                Some(Element::TextBox(edit)) => edit.edit_rect(),
                _ => {
                    host.show(false);
                    next.push(EditHost {
                        name: spec.name.clone(),
                        host,
                        colors,
                    });
                    continue;
                }
            };
            let mut left = spec.x + x;
            let mut top = spec.y + y;
            let mut right = left + width;
            let mut bottom = top + height;
            if let Some((clip_x, clip_y, clip_width, clip_height)) = spec.clip {
                left = left.max(clip_x);
                top = top.max(clip_y);
                right = right.min(clip_x + clip_width);
                bottom = bottom.min(clip_y + clip_height);
                if right <= left || bottom <= top {
                    host.show(false);
                    next.push(EditHost {
                        name: spec.name.clone(),
                        host,
                        colors,
                    });
                    continue;
                }
            }
            let mut point = POINT {
                x: offset + (left * scale).round() as i32,
                y: offset + (top * scale).round() as i32,
            };
            unsafe {
                let _ = ClientToScreen(hwnd, &mut point);
            }
            let width = ((right - left) * scale).round().max(1.0) as i32;
            let height = ((bottom - top) * scale).round().max(1.0) as i32;
            let changed = host.set_bounds(point.x, point.y, width, height, !masked);
            let shown = host.show(true);
            if changed || shown {
                host.refresh();
            }
            if !masked {
                host.raise();
            }
            if spec.multiline {
                // 宿主排完版才知道真实滚动范围；写回 Document，渲染器下一帧画滚动条。
                let (horizontal, vertical) = host.scroll_info();
                if let Some(Element::TextBox(edit)) = document.element_at_path_mut(&spec.path) {
                    let horizontal = if edit.auto_line_wrap {
                        TextBoxScroll::default()
                    } else {
                        scroll_state(horizontal)
                    };
                    edit.h_scroll = horizontal;
                    edit.v_scroll = scroll_state(vertical);
                }
            }
            next.push(EditHost {
                name: spec.name.clone(),
                host,
                colors,
            });
        }
        self.edits = next;
        // 不在这一帧里的旧宿主先寄存，等 `paint` 的借用结束再析构（析构会碰 TSF / 焦点）。
        self.retired.append(&mut old);
    }

    /// 编辑框宿主需要的字体与颜色（主题切换后颜色变了会触发重建宿主）。
    fn edit_settings(&self, spec: &EditSpec) -> (Option<String>, f32, EditColors) {
        let painter = self.painter.borrow();
        let (default_family, default_size, background, text, selection, selection_text) =
            match painter.as_ref() {
                Some(painter) => {
                    let (family, size) = painter.text_font(spec.font);
                    let (background, text) = painter.text_edit_colors();
                    let (selection, selection_text) = painter.text_edit_selection_colors();
                    (family, size, background, text, selection, selection_text)
                }
                None => (
                    None,
                    14.0,
                    Color::rgb(255, 255, 255),
                    Color::rgb(34, 34, 34),
                    Color::rgb(204, 232, 255),
                    Color::rgb(34, 34, 34),
                ),
            };
        let family = spec.font_name.clone().or(default_family);
        let size = spec.size.unwrap_or(default_size);
        let background = spec
            .fill
            .filter(|color| color.a == 255)
            .unwrap_or(background);
        let text = spec.text_color.unwrap_or(text);
        let selection = spec.selection_background.unwrap_or(selection);
        let selection_text = spec.selection_color.unwrap_or(selection_text);
        let caret = spec.caret_color.unwrap_or(text);
        (
            family,
            size,
            EditColors {
                background: colorref(background).0,
                text: colorref(text).0,
                selection: colorref(selection).0,
                selection_text: colorref(selection_text).0,
                caret: colorref(caret).0,
            },
        )
    }

    fn create_text_host(
        &self,
        owner: HWND,
        spec: &EditSpec,
        family: Option<String>,
        size: f32,
        colors: EditColors,
        dpi: u32,
    ) -> Option<Rc<TextHostWindow>> {
        TextHostWindow::create(
            owner,
            &spec.text,
            family.as_deref(),
            size,
            COLORREF(colors.background),
            COLORREF(colors.text),
            COLORREF(colors.selection),
            COLORREF(colors.selection_text),
            COLORREF(colors.caret),
            spec.font_flag,
            spec.read_only,
            spec.deselect_on_blur,
            spec.passkey,
            spec.multiline,
            false,
            spec.auto_line_wrap,
            spec.line_spacing,
            self.top_most,
            dpi,
        )
        .inspect_err(|error| tracing::warn!(%error, "独立窗口：建文本编辑宿主失败"))
        .ok()
    }

    /// 编辑框文本变了：把新文本交给应用。
    fn text_changed(&mut self, hwnd: HWND, host: HWND) {
        let found = self
            .edits
            .iter()
            .find(|edit| edit.host.hwnd() == host)
            .map(|edit| (edit.name.clone(), edit.host.text_string()));
        if let Some((name, text)) = found {
            self.push(hwnd, AppEvent::TextChanged { name, text });
        }
    }

    fn text_focus_changed(&mut self, hwnd: HWND, host: HWND, focused: bool) {
        let name = self
            .edits
            .iter()
            .find(|edit| edit.host.hwnd() == host)
            .map(|edit| edit.name.clone());
        if let Some(name) = name {
            self.push(hwnd, AppEvent::TextFocusChanged { name, focused });
        }
    }

    fn text_key_pressed(&mut self, hwnd: HWND, host: HWND, vk: u32, modifiers: u8) {
        let name = self
            .edits
            .iter()
            .find(|edit| edit.host.hwnd() == host)
            .map(|edit| edit.name.clone());
        if let Some(name) = name {
            self.push(
                hwnd,
                AppEvent::TextKey {
                    name,
                    vk,
                    modifiers,
                },
            );
        }
    }

    fn min_track_size(&self) -> Option<(i32, i32)> {
        let (width, height) = self.min_size?;
        let scale = self.dpi as f32 / 96.0;
        // 无边框窗口的阴影白边也算进窗口矩形里，不然「最小客户区」会少一圈。
        let margin = if self.layered {
            self.shadow_margin * 2
        } else {
            0
        };
        Some((
            (width * scale).round() as i32 + margin,
            (height * scale).round() as i32 + margin,
        ))
    }
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            let mut command = None;
            let mut retired = Vec::new();
            with_window(hwnd, |window| {
                window.paint(hwnd, hdc);
                command = window.pending_command.take();
                retired = std::mem::take(&mut window.retired);
            });
            unsafe {
                let _ = EndPaint(hwnd, &paint);
            }
            drop(retired);
            if let Some(command) = command {
                apply_window_command(hwnd, command);
            }
            flush_pending_show();
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            let (width, height) = (loword(lparam.0), hiword(lparam.0));
            let minimized = wparam.0 == SIZE_MINIMIZED as usize;
            with_window(hwnd, |window| {
                let scale = window.dpi as f32 / 96.0;
                let margin = window.margin(hwnd);
                window.push(
                    hwnd,
                    AppEvent::Resized {
                        width: (width - margin * 2).max(1) as f32 / scale,
                        height: (height - margin * 2).max(1) as f32 / scale,
                    },
                );
                window.sync_mask(hwnd);
                if minimized {
                    window.hide_owned();
                }
            });
            LRESULT(0)
        }
        WM_SHOWWINDOW => {
            let visible = wparam.0 != 0;
            with_window(hwnd, |window| {
                if visible {
                    if let Some(mask) = &window.mask_window {
                        let active = window.mask.is_some_and(|mask| mask.color.a > 0)
                            && !unsafe { IsIconic(hwnd) }.as_bool();
                        mask.show(active);
                        if active {
                            mask.raise(window.top_most);
                        }
                    }
                } else {
                    window.hide_owned();
                }
            });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = ((wparam.0 >> 16) & 0xFFFF) as u32;
            with_window(hwnd, |window| window.set_dpi(hwnd, dpi));
            let suggested = unsafe { &*(lparam.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            with_window(hwnd, |window| {
                if let Some((width, height)) = window.min_track_size() {
                    let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                    info.ptMinTrackSize.x = width;
                    info.ptMinTrackSize.y = height;
                }
                if window.caption_height.is_some() {
                    // WS_POPUP 默认最大化到整屏；按当前显示器工作区排，别盖任务栏。
                    let work = monitor::work_area_of_window(hwnd);
                    let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                    info.ptMaxPosition.x = work.left;
                    info.ptMaxPosition.y = work.top;
                    info.ptMaxSize.x = work.right - work.left;
                    info.ptMaxSize.y = work.bottom - work.top;
                }
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            // 进了客户区才登记离开消息；重复登记会重置跟踪，开销很小。
            let mut track = TRACKMOUSEEVENT {
                cbSize: core::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut track);
            }
            let (x, y) = mouse_point(hwnd, lparam.0);
            with_window(hwnd, |window| {
                // 系统在光标没动时也会重复发 WM_MOUSEMOVE；每次都重画会和贴图互相触发，
                // 形成持续重绘。位置没变就丢掉。
                if window.last_pointer == Some((x, y)) {
                    return;
                }
                window.last_pointer = Some((x, y));
                window.push(hwnd, AppEvent::MouseMoved { x, y });
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_window(hwnd, |window| {
                if window.last_pointer.is_none() {
                    return;
                }
                window.last_pointer = None;
                window.push(hwnd, AppEvent::MouseLeft);
            });
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP => {
            let (x, y) = mouse_point(hwnd, lparam.0);
            let button = match message {
                WM_LBUTTONDOWN | WM_LBUTTONUP => MouseButton::Left,
                _ => MouseButton::Right,
            };
            let pressed = matches!(message, WM_LBUTTONDOWN | WM_RBUTTONDOWN);
            with_window(hwnd, |window| {
                let hit = window
                    .regions
                    .iter()
                    .rev()
                    .find(|region| region.contains(x, y))
                    .map(|region| region.name.clone());
                if message == WM_LBUTTONDOWN {
                    window.pressed_region = hit;
                } else if message == WM_LBUTTONUP
                    && let Some(name) = window.pressed_region.take()
                    && hit.as_deref() == Some(name.as_str())
                {
                    window.push(hwnd, AppEvent::RegionClicked(name));
                }
                window.push(
                    hwnd,
                    AppEvent::MouseButton {
                        button,
                        pressed,
                        x,
                        y,
                    },
                )
            });
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            with_window(hwnd, |window| {
                window.push(hwnd, AppEvent::MouseWheel { delta })
            });
            LRESULT(0)
        }
        WM_KEYDOWN | WM_KEYUP => {
            let pressed = message == WM_KEYDOWN;
            let vk = wparam.0 as u32;
            with_window(hwnd, |window| {
                window.push(hwnd, AppEvent::Key { vk, pressed })
            });
            LRESULT(0)
        }
        WM_CHAR => {
            if let Some(text) = char::from_u32(wparam.0 as u32)
                && !text.is_control()
            {
                with_window(hwnd, |window| {
                    window.push(hwnd, AppEvent::Text(text.to_string()))
                });
            }
            LRESULT(0)
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            let focused = message == WM_SETFOCUS;
            with_window(hwnd, |window| window.push(hwnd, AppEvent::Focused(focused)));
            LRESULT(0)
        }
        WM_MOVE => {
            // 编辑框宿主按客户区坐标摆位，窗口移动后要重画一帧把它们挪过去。
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            with_window(hwnd, |window| window.sync_mask(hwnd));
            LRESULT(0)
        }
        WM_NCHITTEST => {
            if layered(hwnd) {
                let mut point = POINT {
                    x: loword(lparam.0),
                    y: hiword(lparam.0),
                };
                unsafe {
                    let _ = ScreenToClient(hwnd, &mut point);
                }
                let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
                let (margin, caption) =
                    with_window_value(hwnd, |window| (window.margin(hwnd), window.caption_height))
                        .unwrap_or((0, None));
                let mut client = RECT::default();
                unsafe {
                    let _ = GetClientRect(hwnd, &mut client);
                }
                if point.x < margin
                    || point.y < margin
                    || point.x >= client.right - margin
                    || point.y >= client.bottom - margin
                {
                    return LRESULT(HTTRANSPARENT as isize);
                }
                let content = (
                    (point.x - margin) as f32 / scale,
                    (point.y - margin) as f32 / scale,
                );
                if let Some(caption_height) = caption
                    && content.1 < caption_height
                {
                    let over_region = with_window_value(hwnd, |window| {
                        window
                            .regions
                            .iter()
                            .rev()
                            .any(|region| region.contains(content.0, content.1))
                    })
                    .unwrap_or(false);
                    if over_region {
                        LRESULT(HTCLIENT as isize)
                    } else {
                        LRESULT(HTCAPTION as isize)
                    }
                } else {
                    LRESULT(HTCLIENT as isize)
                }
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_NCLBUTTONDBLCLK => {
            let caption =
                with_window_value(hwnd, |window| window.caption_height.is_some()).unwrap_or(false);
            if caption && wparam.0 as i32 == HTCAPTION as i32 {
                apply_window_command(hwnd, WindowCommand::ToggleMaximize);
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_TEXT_CHANGED => {
            let host = HWND(wparam.0 as *mut _);
            with_window(hwnd, |window| window.text_changed(hwnd, host));
            LRESULT(0)
        }
        WM_TEXT_FOCUS => {
            let host = HWND(wparam.0 as *mut _);
            with_window(hwnd, |window| {
                window.text_focus_changed(hwnd, host, lparam.0 != 0)
            });
            LRESULT(0)
        }
        WM_TEXT_KEY => {
            let host = HWND(wparam.0 as *mut _);
            let key = (lparam.0 & 0xFFFF) as u32;
            let modifier = ((lparam.0 >> 16) & 0xFF) as u8;
            with_window(hwnd, |window| {
                window.text_key_pressed(hwnd, host, key, modifier)
            });
            LRESULT(0)
        }
        WM_TEXT_SCROLL | WM_TEXT_WHEEL => {
            // 多行编辑框内部滚动了：重新取滚动范围并重画。
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_TRAY_ICON => {
            if let Some(event) = tray::click_event(lparam.0) {
                with_window(hwnd, |window| {
                    window.push(hwnd, AppEvent::Tray(event.clone()))
                });
                if event == TrayEvent::RightClick {
                    let tray = with_window_value(hwnd, |window| window.tray.clone()).flatten();
                    if let Some(tray) = tray
                        && let Some(menu) = tray.show_menu()
                    {
                        with_window(hwnd, |window| window.push(hwnd, AppEvent::Tray(menu)));
                    }
                }
                // 隐藏 / 最小化时没有 WM_PAINT，托盘菜单的显示 / 退出必须当场处理。
                let needs_pump = !unsafe { IsWindowVisible(hwnd).as_bool() }
                    || unsafe { IsIconic(hwnd).as_bool() };
                if needs_pump {
                    pump_window(hwnd);
                }
            }
            LRESULT(0)
        }
        message if message == taskbar_created_message() => {
            let tray = with_window_value(hwnd, |window| window.tray.clone()).flatten();
            if let Some(tray) = tray {
                tray.readd();
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // 宿主析构会碰 TSF / 焦点，可能重入窗口过程；先摘出来，离开借用后再 drop。
            let state = WINDOWS
                .with(|windows| windows.borrow().get(&(hwnd.0 as isize)).cloned())
                .map(|window| {
                    let mut window = window.borrow_mut();
                    let mut retired = std::mem::take(&mut window.edits);
                    retired.append(&mut window.retired);
                    (retired, window.tray.take(), window.mask_window.take())
                });
            if let Some((retired, tray, mask)) = state {
                drop(retired);
                drop(tray);
                drop(mask);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let empty = WINDOWS.with(|windows| {
                let mut windows = windows.borrow_mut();
                windows.remove(&(hwnd.0 as isize));
                windows.is_empty()
            });
            if empty {
                unsafe { PostQuitMessage(0) };
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn with_window(hwnd: HWND, action: impl FnOnce(&mut AppWindow)) {
    WINDOWS.with(|windows| {
        let window = windows.borrow().get(&(hwnd.0 as isize)).cloned();
        let Some(window) = window else {
            return;
        };
        match window.try_borrow_mut() {
            Ok(mut window) => action(&mut window),
            // 同步消息（拖窗口、焦点变化、宿主析构）可能在上一条消息的借用里重入；跳过这一条，
            // 下一次 WM_PAINT / 输入消息会补上状态。
            Err(_) => tracing::warn!(hwnd = hwnd.0 as isize, "窗口过程重入，跳过一条消息"),
        }
    });
}

fn with_window_value<T>(hwnd: HWND, action: impl FnOnce(&AppWindow) -> T) -> Option<T> {
    WINDOWS.with(|windows| {
        let window = windows.borrow().get(&(hwnd.0 as isize)).cloned();
        window.and_then(|window| window.try_borrow().ok().map(|window| action(&window)))
    })
}

fn layered(hwnd: HWND) -> bool {
    with_window_value(hwnd, |window| window.layered).unwrap_or(false)
}

fn colorref(color: Color) -> COLORREF {
    COLORREF(color.r as u32 | ((color.g as u32) << 8) | ((color.b as u32) << 16))
}

fn loword(value: isize) -> i32 {
    (value as u32 & 0xFFFF) as u16 as i16 as i32
}

fn hiword(value: isize) -> i32 {
    ((value as u32 >> 16) & 0xFFFF) as u16 as i16 as i32
}

fn mouse_point(hwnd: HWND, lparam: isize) -> (f32, f32) {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = dpi as f32 / 96.0;
    let margin = with_window_value(hwnd, |window| window.margin(hwnd)).unwrap_or(0);
    (
        (loword(lparam) - margin) as f32 / scale,
        (hiword(lparam) - margin) as f32 / scale,
    )
}

fn is_zoomed(hwnd: HWND) -> bool {
    unsafe { IsZoomed(hwnd).as_bool() }
}

/// Explorer 重启后会广播 `TaskbarCreated`，收到时重新注册托盘图标。
fn taskbar_created_message() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

fn apply_window_command(hwnd: HWND, command: WindowCommand) {
    match command {
        WindowCommand::Minimize => unsafe {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        },
        WindowCommand::ToggleMaximize => {
            let command = if is_zoomed(hwnd) {
                SW_RESTORE
            } else {
                SW_MAXIMIZE
            };
            unsafe {
                let _ = ShowWindow(hwnd, command);
            }
        }
        WindowCommand::Show => unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            let _ = SetForegroundWindow(hwnd);
        },
        WindowCommand::Hide => unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        },
        WindowCommand::Close => unsafe {
            let _ = DestroyWindow(hwnd);
        },
    }
}

/// 把这一帧里 `open_window` 登记的新窗口真正显示出来。
fn flush_pending_show() {
    let pending = PENDING_SHOW.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
    for id in pending {
        let open = WINDOWS.with(|windows| windows.borrow().contains_key(&id));
        if !open {
            continue;
        }
        let hwnd = HWND(id as *mut _);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = UpdateWindow(hwnd);
        }
    }
}

/// 隐藏 / 最小化窗口收到托盘事件时，直接在消息里跑一遍 `Application::update`。
fn pump_window(hwnd: HWND) {
    let mut command = None;
    with_window(hwnd, |window| {
        window.pump(hwnd);
        command = window.pending_command.take();
    });
    if let Some(command) = command {
        apply_window_command(hwnd, command);
    }
    flush_pending_show();
}

fn blit(hdc: HDC, pixmap: &Pixmap, width: i32, height: i32) {
    let mut pixels = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.data().chunks_exact(4) {
        // `RenderSettings` 的独立窗口用不透明背景：丢掉 alpha，换成 GDI 的 BGRA。
        pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }
    let header = BITMAPINFOHEADER {
        biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: pixmap.width() as i32,
        biHeight: -(pixmap.height() as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO {
        bmiHeader: header,
        ..Default::default()
    };
    unsafe {
        let _ = StretchDIBits(
            hdc,
            0,
            0,
            width,
            height,
            0,
            0,
            pixmap.width() as i32,
            pixmap.height() as i32,
            Some(pixels.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}
