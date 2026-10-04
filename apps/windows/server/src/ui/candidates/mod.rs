//! 候选窗口：不抢焦点、置顶的分层窗口，跟随光标，画拼音行与候选列表，四周柔和阴影。
//! 由云朵渲染器出位图再贴（[`super::painter`]）；绘制内容在 [`RenderData`]，一行的展示形态在 [`row`]。

mod render_data;
pub(crate) mod row;

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IDC_ARROW, LoadCursorW, SW_HIDE, SW_SHOWNA,
    ShowWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use cloudime_platform::ItemNumberStyle;
use cloudime_platform::protocol::Frame;

pub(crate) use self::render_data::RenderData;
use super::layered;
use super::monitor;
use super::painter::SharedPainter;
use super::window_class::WindowClass;
use crate::dispatch::RenderSettings;

const CLASS_NAME: PCWSTR = w!("CloudIMECandidateWindow");
static CLASS: WindowClass = WindowClass::new();

/// 光标行与候选窗之间的间隙（逻辑像素）。
const CARET_GAP: i32 = 2;

/// 候选窗口。内容经 `UpdateLayeredWindow` 一次贴上，窗口过程只走默认处理。
pub(crate) struct CandidateWindow {
    hwnd: HWND,

    /// 绘制内容。
    data: RefCell<RenderData>,

    /// 上次用的 DPI（光标所在显示器）。
    dpi: Cell<u32>,

    /// 上次记进日志的缩放值（窗口 DPI、光标所在显示器 DPI）：变了才再记一条（#146）。
    logged_dpi: Cell<Option<(u32, Option<u32>)>>,

    /// 云朵渲染器；`None` 不绘制（字体库加载失败）。
    painter: SharedPainter,

    /// 序号写法（配置变了重设）。
    index_style: Cell<ItemNumberStyle>,
}

impl CandidateWindow {
    /// 建一个隐藏的候选窗口。
    pub(crate) fn new(painter: SharedPainter) -> Result<Self> {
        CLASS.ensure(|| WNDCLASSEXW {
            lpfnWndProc: Some(wndproc),
            hInstance: super::module_handle(),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        })?;
        let dpi = unsafe { GetDpiForSystem() }.max(96);
        let data = RefCell::new(RenderData::empty());
        // NOACTIVATE：显示时不抢应用焦点。
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("云朵候选"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(super::module_handle()),
                None,
            )?
        };
        Ok(Self {
            hwnd,
            data,
            dpi: Cell::new(dpi),
            logged_dpi: Cell::new(None),
            painter,
            index_style: Cell::new(ItemNumberStyle::default()),
        })
    }

    /// 刷新内容（不定位、不显示）；`badges` 是每个候选的来源角标。
    pub(crate) fn set_content(&self, frame: &Frame, badges: &[Option<char>]) {
        self.data
            .borrow_mut()
            .set(frame, badges, self.index_style.get());
    }

    /// 配置变了：换序号写法（三项字体与最小宽度在 painter 那边）。
    pub(crate) fn configure(&self, settings: &RenderSettings) {
        self.index_style.set(settings.item_number_style);
    }

    /// 按光标矩形定位并显示：贴光标下方（放不下放上方），四周留出阴影。
    pub(crate) fn show(&self, anchor: RECT) {
        self.sync_dpi(anchor);
        let rendered = {
            let data = self.data.borrow();
            self.painter.borrow_mut().as_mut().and_then(|painter| {
                painter.render_frame(&data.render_frame(), data.layout, self.dpi.get())
            })
        };
        let Some(rendered) = rendered else {
            // 字体库加载失败或渲染出错（已记日志）：不贴图、不显示。
            self.hide();
            return;
        };
        let content = (
            rendered.content_width as i32,
            rendered.content_height as i32,
        );
        if content.0 <= 0 || content.1 <= 0 {
            self.hide();
            return;
        }
        let (content_x, content_y) = place(anchor, content);
        let updated = layered::present(
            self.hwnd,
            &rendered.pixmap,
            (
                content_x - rendered.content_x as i32,
                content_y - rendered.content_y as i32,
            ),
        );
        if updated.is_ok() {
            let _ = unsafe { ShowWindow(self.hwnd, SW_SHOWNA) };
        } else {
            self.hide();
        }
    }

    pub(crate) fn hide(&self) {
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    /// 刷新光标所在显示器的 DPI；每次 `show` 前调。
    ///
    /// DPI 取光标所在显示器的：窗口藏着时改了缩放（或睡眠唤醒后多显示器重排），
    /// `GetDpiForWindow` 会停在旧值，候选字就大小不对（#146）。
    fn sync_dpi(&self, anchor: RECT) {
        let caret = POINT {
            x: anchor.left,
            y: anchor.top,
        };
        let monitor_dpi = monitor::dpi_near(caret);
        let window_dpi = unsafe { GetDpiForWindow(self.hwnd) };
        let dpi = match (monitor_dpi, window_dpi) {
            (Some(dpi), _) => dpi,
            (None, 0) => self.dpi.get(),
            (None, dpi) => dpi,
        };
        self.log_dpi(caret, window_dpi, monitor_dpi, dpi);
        self.dpi.set(dpi);
    }

    /// 缩放值变了就记一条，多显示器 / 睡眠唤醒的问题从日志里能看出取到的是哪个值（#146）。
    fn log_dpi(&self, caret: POINT, window_dpi: u32, monitor_dpi: Option<u32>, used: u32) {
        if self.logged_dpi.replace(Some((window_dpi, monitor_dpi)))
            == Some((window_dpi, monitor_dpi))
        {
            return;
        }
        tracing::info!(
            window_dpi,
            ?monitor_dpi,
            used,
            system_dpi = unsafe { GetDpiForSystem() },
            caret_x = caret.x,
            caret_y = caret.y,
            "候选窗口缩放值"
        );
    }
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// 内容左上角：贴光标下方，放不下放上方，再放不下贴屏幕内；都夹在所在显示器工作区里。
fn place(anchor: RECT, content: (i32, i32)) -> (i32, i32) {
    let work = monitor::work_area_near(POINT {
        x: anchor.left,
        y: anchor.top,
    });
    let x = anchor
        .left
        .clamp(work.left, (work.right - content.0).max(work.left));
    let below = anchor.bottom + CARET_GAP;
    let above = anchor.top - CARET_GAP - content.1;
    let y = if below + content.1 <= work.bottom {
        below
    } else if above >= work.top {
        above
    } else {
        (work.bottom - content.1).max(work.top)
    };
    (x, y)
}

/// 分层窗口无需 `WM_PAINT`，全交默认处理。
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
