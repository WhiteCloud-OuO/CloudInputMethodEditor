//! 候选窗口的定位锚点：组句范围 / 选区在屏幕上的矩形，拿不到时退到鼠标位置。

use std::mem::ManuallyDrop;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::HiDpi::LogicalToPhysicalPointForPerMonitorDPI;
use windows::Win32::UI::TextServices::{
    ITfContext, ITfRange, TF_ANCHOR_END, TF_DEFAULT_SELECTION, TF_SELECTION,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows::core::BOOL;

use cloudime_platform::protocol::ScreenRect;

/// `range` 的屏幕矩形，拿不到（有些应用给全零 / 空矩形）退到鼠标处。
pub(crate) fn anchor_rect(context: &ITfContext, ec: u32, range: &ITfRange) -> ScreenRect {
    let rect = range_rect(context, ec, range)
        .unwrap_or_else(|| to_physical(mouse_anchor(), view_window(context)));
    to_screen(rect)
}

/// 插入点（没有选区时是光标，有选区时是选区）的屏幕矩形。「只在候选窗口」模式应用里不放 marked text，
/// 没有组句范围可量，就用它给候选窗口定位。
pub(crate) fn caret_rect(context: &ITfContext, ec: u32) -> ScreenRect {
    match selection_range(context, ec) {
        Some(range) => anchor_rect(context, ec, &range),
        None => mouse_screen_rect(context),
    }
}

/// 插入点（没有选区时是光标，有选区时是选区终点）的屏幕矩形。脚本要「光标位置」时用它。
pub(crate) fn insertion_rect(context: &ITfContext, ec: u32) -> ScreenRect {
    match selection_range(context, ec) {
        Some(range) => {
            // 有选区时插入符在终点（与编辑器的习惯一致）
            let _ = unsafe { range.Collapse(ec, TF_ANCHOR_END) };
            anchor_rect(context, ec, &range)
        }
        None => mouse_screen_rect(context),
    }
}

/// 当前选区的范围；`GetSelection` 移交所有权，由调用方释放。
pub(crate) fn selection_range(context: &ITfContext, ec: u32) -> Option<ITfRange> {
    let mut selection = [TF_SELECTION::default()];
    let mut fetched = 0u32;
    unsafe {
        context
            .GetSelection(ec, TF_DEFAULT_SELECTION, &mut selection, &mut fetched)
            .ok()?;
    }
    if fetched == 0 {
        return None;
    }
    unsafe { ManuallyDrop::take(&mut selection[0].range) }
}

/// 没有可量的范围时的锚点：鼠标处一个零宽、约一行高的矩形。
pub(crate) fn mouse_screen_rect(context: &ITfContext) -> ScreenRect {
    to_screen(to_physical(mouse_anchor(), view_window(context)))
}

fn range_rect(context: &ITfContext, ec: u32, range: &ITfRange) -> Option<RECT> {
    let mut rect = RECT::default();
    let mut clipped = BOOL(0);
    let view = unsafe { context.GetActiveView().ok()? };
    unsafe { view.GetTextExt(ec, range, &mut rect, &mut clipped).ok()? };
    (rect.right > rect.left || rect.bottom > rect.top)
        .then(|| to_physical(rect, unsafe { view.GetWnd().ok() }))
}

/// 焦点输入框所属的窗口；拿不到为 `None`。
fn view_window(context: &ITfContext) -> Option<HWND> {
    unsafe { context.GetActiveView().ok()?.GetWnd().ok() }
}

/// 把宿主坐标系里的屏幕矩形换成物理像素。
///
/// TSF 的 `GetTextExt` / `GetCursorPos` 给的是**宿主进程**坐标空间里的屏幕坐标：宿主不是
/// Per-Monitor DPI aware 时那是它自己的逻辑坐标（相当于 96 DPI），而 Server 是 PerMonitorV2、
/// 按物理像素摆窗。混合 DPI 的多屏上不换算就会偏到别处（偏左 / 偏上或跑到另一块屏）。
/// `LogicalToPhysicalPointForPerMonitorDPI` 按宿主窗口的 DPI 上下文换算，对已 aware 的宿主是恒等。
fn to_physical(rect: RECT, window: Option<HWND>) -> RECT {
    let Some(window) = window.filter(|hwnd| !hwnd.is_invalid()) else {
        return rect;
    };
    let mut top_left = POINT {
        x: rect.left,
        y: rect.top,
    };
    let mut bottom_right = POINT {
        x: rect.right,
        y: rect.bottom,
    };
    let converted = unsafe {
        LogicalToPhysicalPointForPerMonitorDPI(Some(window), &mut top_left).as_bool()
            && LogicalToPhysicalPointForPerMonitorDPI(Some(window), &mut bottom_right).as_bool()
    };
    if converted {
        RECT {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x,
            bottom: bottom_right.y,
        }
    } else {
        rect
    }
}

fn mouse_anchor() -> RECT {
    let mut point = POINT::default();
    let _ = unsafe { GetCursorPos(&mut point) };
    RECT {
        left: point.x,
        top: point.y,
        right: point.x,
        bottom: point.y + 16,
    }
}

fn to_screen(rect: RECT) -> ScreenRect {
    ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}
