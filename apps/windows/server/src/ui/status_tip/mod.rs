//! 状态切换提示：输入法状态（中 / 英、Caps Lock、全 / 半角、简 / 繁、中文 / 西文标点）一变，
//! 在输入光标附近弹一个停留 1 秒的小条 —— 样式与悬浮工具栏同一套图标，只显示它的前四个按钮，
//! 纯展示、点不着（`WS_EX_TRANSPARENT`）。
//!
//! 「在不在输入状态」由 Router 判断（DLL 报来的 TSF 焦点状态），这里只管画、摆与计时收起。

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HTTRANSPARENT, KillTimer, MA_NOACTIVATE,
    SW_HIDE, SW_SHOWNA, SetTimer, ShowWindow, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use super::layered;
use super::monitor;
use super::painter::SharedPainter;
use super::status::StatusIcons;
use super::window_class::WindowClass;
use crate::dispatch::StatusView;

const CLASS_NAME: PCWSTR = w!("CloudIMEStatusTip");
static CLASS: WindowClass = WindowClass::new();

/// 提示停留时长（毫秒）。
const LINGER_MS: u32 = 1000;

/// 停留定时器编号。
const TIMER_ID: usize = 1;

/// 与光标矩形的间距（逻辑像素，按 DPI 折成物理像素）。
const GAP: i32 = 6;

/// 状态切换提示窗口。
pub(super) struct StatusTip {
    hwnd: HWND,

    /// 云朵渲染器；`None` 不绘制（字体库加载失败）。
    painter: SharedPainter,

    /// 上次用的 DPI。
    dpi: Cell<u32>,

    /// 前四个状态按钮的图标；内部自己管排布（与悬浮状态条各一份）。
    icons: RefCell<StatusIcons>,
}

impl StatusTip {
    /// 建一个隐藏的提示窗。失败返回 `Err`，调用方降级为不显示提示。
    pub(super) fn new(painter: SharedPainter) -> Result<Self> {
        CLASS.ensure(|| WNDCLASSEXW {
            lpfnWndProc: Some(wndproc),
            hInstance: super::module_handle(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        })?;
        // TRANSPARENT：纯展示，鼠标点它等于点在底下的应用上。NOACTIVATE：显示时不抢应用焦点。
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TRANSPARENT,
                CLASS_NAME,
                w!("状态切换提示"),
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
            painter,
            dpi: Cell::new(unsafe { GetDpiForSystem() }.max(96)),
            icons: RefCell::new(StatusIcons::load()),
        })
    }

    /// 在光标矩形附近画一屏并显示，1 秒后自己收起。画不出来就保持收起。
    pub(super) fn show(&self, view: &StatusView, caps: bool, anchor: RECT) {
        // DPI 取光标所在显示器的（理由同候选窗，见 #146）。
        let point = POINT {
            x: anchor.left,
            y: anchor.top,
        };
        let dpi = monitor::dpi_near(point)
            .or_else(|| {
                let dpi = unsafe { GetDpiForWindow(self.hwnd) };
                (dpi != 0).then_some(dpi)
            })
            .unwrap_or(self.dpi.get())
            .max(1);
        self.dpi.set(dpi);
        let cells = self.icons.borrow_mut().cells(view, caps);
        let rendered = match (cells.is_empty(), self.painter.borrow_mut().as_mut()) {
            (false, Some(painter)) => {
                painter.render_status(&cells, dpi, crate::ui::painter::StatusKind::Tip)
            }
            _ => None,
        };
        let Some(rendered) = rendered else {
            // cfg 里读不到排布、字体库加载失败或渲染出错（已记日志）：不贴图、不显示。
            self.hide();
            return;
        };
        let bitmap = &rendered.rendered;
        let content = (bitmap.content_width as i32, bitmap.content_height as i32);
        if content.0 <= 0 || content.1 <= 0 {
            self.hide();
            return;
        }
        // 位图四周还有阴影留白，摆的是内容左上角，贴图位置要把留白减掉。
        let margin = bitmap.content_x as i32;
        let (x, y) = place(anchor, content, (GAP * dpi as i32) / 96);
        let updated = layered::present(self.hwnd, &bitmap.pixmap, (x - margin, y - margin), 255);
        if updated.is_err() {
            self.hide();
            return;
        }
        let _ = unsafe { ShowWindow(self.hwnd, SW_SHOWNA) };
        let _ = unsafe { SetTimer(Some(self.hwnd), TIMER_ID, LINGER_MS, None) };
    }

    /// 收起提示（没显示过也无妨）。
    pub(super) fn hide(&self) {
        let _ = unsafe { KillTimer(Some(self.hwnd), TIMER_ID) };
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }
}

impl Drop for StatusTip {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// 内容左上角：贴光标下方，放不下放上方，再放不下贴屏幕内；都夹在所在显示器工作区里。
fn place(anchor: RECT, content: (i32, i32), gap: i32) -> (i32, i32) {
    let work = monitor::work_area_near(POINT {
        x: anchor.left,
        y: anchor.top,
    });
    let x = anchor
        .left
        .clamp(work.left, (work.right - content.0).max(work.left));
    let below = anchor.bottom + gap;
    let above = anchor.top - gap - content.1;
    let y = if below + content.1 <= work.bottom {
        below
    } else if above >= work.top {
        above
    } else {
        (work.bottom - content.1).max(work.top)
    };
    (x, y)
}

/// 分层窗口无需 `WM_PAINT`；到点收起。
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // 纯展示：鼠标一律穿透到下面的窗口。
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_TIMER if wparam.0 == TIMER_ID => {
            let _ = unsafe { KillTimer(Some(hwnd), TIMER_ID) };
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
