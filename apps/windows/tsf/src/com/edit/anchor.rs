//! 候选窗口的定位锚点：组句范围 / 选区在屏幕上的矩形；拿不到时先找系统 IME 小窗，再沿用同窗口上次的位置，最后才退到鼠标。

use std::cell::Cell;
use std::mem::ManuallyDrop;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::UI::HiDpi::LogicalToPhysicalPointForPerMonitorDPI;
use windows::Win32::UI::TextServices::{
    ITfContext, ITfRange, TF_ANCHOR_END, TF_DEFAULT_SELECTION, TF_SELECTION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetCursorPos, GetWindowRect, GetWindowThreadProcessId,
    IsWindowVisible,
};
use windows::core::BOOL;

use cloudime_platform::protocol::ScreenRect;

use crate::com::log;

thread_local! {
    /// 同一窗口上一次量到的有效光标矩形。
    ///
    /// 游戏这类宿主在每段组合的**第一个键**会报 1×0 的假矩形（Civ VI 报的是屏幕右下角），
    /// 第二个键起才给得出真实位置。这时沿用同一窗口上次的有效位置，比退回鼠标稳：
    /// 鼠标在全屏游戏里常被锁在屏幕中央或根本不代表输入位置。
    /// 按进程隔离（DLL 在每个宿主进程里各有一份），换了窗口或放久了就作废。
    static LAST_VALID: Cell<Option<(HWND, RECT, Instant)>> = const { Cell::new(None) };

    /// 这一段组合正在等系统 IME 小窗出现；期间先不报位置，免得候选窗先闪到鼠标处。
    static WAITING_CICERO: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// 上次有效锚点的有效期：超过它就不再复用，免得切到同一窗口的另一个输入框时还摆在旧位置。
const LAST_VALID_TTL: Duration = Duration::from_secs(10);

/// 等系统 IME 小窗出现的最长时间；超过就退回鼠标。Civ VI 实测小窗在首键后约 250ms 出现。
const WAIT_CICERO: Duration = Duration::from_millis(500);

/// `range` 的屏幕矩形，拿不到（有些应用给全零 / 空矩形）时先用同窗口上次的有效位置，最后才退回鼠标。
pub(crate) fn anchor_rect(context: &ITfContext, ec: u32, range: &ITfRange) -> ScreenRect {
    match range_rect(context, ec, range) {
        Some(rect) => {
            remember(context, rect);
            to_screen(rect)
        }
        None => to_screen(fallback_rect(context)),
    }
}

/// 候选窗锚点：量到有效矩形 / 系统 IME 小窗 / 上次位置就返回；都拿不到时先等小窗出现
/// （返回 `None`，由轮询定时器补报），等超过 [`WAIT_CICERO`] 才退回鼠标。
pub(crate) fn candidate_anchor(
    context: &ITfContext,
    ec: u32,
    range: Option<&ITfRange>,
) -> Option<ScreenRect> {
    let rect = match range {
        Some(range) => range_rect(context, ec, range),
        None => selection_range(context, ec).and_then(|range| range_rect(context, ec, &range)),
    };
    if let Some(rect) = rect {
        remember(context, rect);
        return Some(to_screen(rect));
    }
    let window = view_window(context);
    if let Some(rect) = cicero_rect() {
        clear_pending_candidate();
        let rect = to_physical(rect, window);
        log::log(&format!(
            "候选锚点：量不到组句矩形，用 CiceroUIWndFrame l={} t={} r={} b={}",
            rect.left, rect.top, rect.right, rect.bottom
        ));
        return Some(to_screen(rect));
    }
    if let Some(rect) = reusable_last(context) {
        clear_pending_candidate();
        log::log(&format!(
            "候选锚点：量不到组句矩形，沿用上次位置 l={} t={} r={} b={}",
            rect.left, rect.top, rect.right, rect.bottom
        ));
        return Some(to_screen(rect));
    }
    // 都没有：先别报位置，等系统 IME 小窗出现（轮询定时器每 80ms 补一次）。
    let expired = WAITING_CICERO.with(|waiting| match waiting.get() {
        Some(at) => at.elapsed() >= WAIT_CICERO,
        None => {
            waiting.set(Some(Instant::now()));
            log::log("候选锚点：量不到组句矩形，等系统 IME 小窗出现");
            false
        }
    });
    if expired {
        clear_pending_candidate();
        let mouse = to_physical(mouse_anchor(), window);
        log::log(&format!(
            "候选锚点：等不到系统 IME 小窗，退回鼠标 l={} t={} r={} b={}",
            mouse.left, mouse.top, mouse.right, mouse.bottom
        ));
        return Some(to_screen(mouse));
    }
    None
}

/// 轮询定时器每拍调用：等到小窗就返回它的锚点；等超时则退回鼠标。`None` = 继续等。
pub(crate) fn retry_pending_candidate(context: &ITfContext) -> Option<ScreenRect> {
    let since = WAITING_CICERO.with(Cell::get)?;
    let window = view_window(context);
    if let Some(rect) = cicero_rect() {
        clear_pending_candidate();
        let rect = to_physical(rect, window);
        log::log(&format!(
            "候选锚点：系统 IME 小窗已出现，补报 l={} t={} r={} b={}",
            rect.left, rect.top, rect.right, rect.bottom
        ));
        return Some(to_screen(rect));
    }
    if since.elapsed() >= WAIT_CICERO {
        clear_pending_candidate();
        let mouse = to_physical(mouse_anchor(), window);
        log::log(&format!(
            "候选锚点：等不到系统 IME 小窗，补报鼠标 l={} t={} r={} b={}",
            mouse.left, mouse.top, mouse.right, mouse.bottom
        ));
        return Some(to_screen(mouse));
    }
    None
}

/// 有没有正在等系统 IME 小窗。
pub(crate) fn pending_candidate() -> bool {
    WAITING_CICERO.with(|waiting| waiting.get().is_some())
}

/// 放弃等待（组句结束 / 量到有效矩形）。
pub(crate) fn clear_pending_candidate() {
    WAITING_CICERO.with(|waiting| waiting.set(None));
}

/// 插入点（没有选区时是光标，有选区时是选区终点）的屏幕矩形。脚本要「光标位置」时用它。
pub(crate) fn insertion_rect(context: &ITfContext, ec: u32) -> ScreenRect {
    match selection_range(context, ec) {
        Some(range) => {
            // 有选区时插入符在终点（与编辑器的习惯一致）
            let _ = unsafe { range.Collapse(ec, TF_ANCHOR_END) };
            anchor_rect(context, ec, &range)
        }
        None => to_screen(fallback_rect(context)),
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

/// 记下这次量到的有效矩形，供同窗口下一段组合的第一个键复用。
fn remember(context: &ITfContext, rect: RECT) {
    let Some(window) = view_window(context).filter(|hwnd| !hwnd.is_invalid()) else {
        return;
    };
    clear_pending_candidate();
    LAST_VALID.with(|last| last.set(Some((window, rect, Instant::now()))));
}

/// 同一窗口上次的有效矩形（换了窗口或放久了就不复用）。
fn reusable_last(context: &ITfContext) -> Option<RECT> {
    let window = view_window(context).filter(|hwnd| !hwnd.is_invalid())?;
    LAST_VALID.with(|last| match last.get() {
        Some((cached, rect, at)) if cached == window && at.elapsed() < LAST_VALID_TTL => Some(rect),
        _ => None,
    })
}

/// 量不到矩形时的退路：先找系统 IME 界面小窗（CiceroUIWndFrame），再复用同窗口上次的位置，最后才退回鼠标。
fn fallback_rect(context: &ITfContext) -> RECT {
    let window = view_window(context);
    if let Some(rect) = cicero_rect() {
        let rect = to_physical(rect, window);
        log::log(&format!(
            "候选锚点：量不到组句矩形，用 CiceroUIWndFrame l={} t={} r={} b={}",
            rect.left, rect.top, rect.right, rect.bottom
        ));
        return rect;
    }
    if let Some(rect) = reusable_last(context) {
        log::log(&format!(
            "候选锚点：量不到组句矩形，沿用上次位置 l={} t={} r={} b={}",
            rect.left, rect.top, rect.right, rect.bottom
        ));
        return rect;
    }
    let mouse = to_physical(mouse_anchor(), window);
    // 游戏这类宿主常常报不出真实光标位置；记下坐标，下次排查「候选窗为什么在屏幕角上」时一眼能对上。
    log::log(&format!(
        "候选锚点：量不到组句矩形，退回鼠标 l={} t={} r={} b={}",
        mouse.left, mouse.top, mouse.right, mouse.bottom
    ));
    mouse
}

/// 本进程里 TSF 桥接层的默认 IME 界面窗口（类名 `CiceroUIWndFrame`）。
///
/// 走 IMM32 桥接的宿主（Civ VI 这类游戏）在组合开始时会让系统在默认位置摆一个小窗显示编码；
/// 游戏报不出光标矩形时，把候选窗摆在这个小窗下方，比跟着鼠标跑更接近用户正在看的地方。
/// 只认**可见**的窗口：同一进程里常有还没显示 / 已经收起、停在默认位置（如 200,200）的隐藏实例，
/// 拿它们的矩形当锚点会把候选窗摆到莫名其妙的地方。
fn cicero_rect() -> Option<RECT> {
    struct Probe {
        pid: u32,
        best: Option<(i64, RECT)>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` 是下面传进来的 `&mut Probe`，回调期间一直有效。
        let probe = unsafe { &mut *(lparam.0 as *mut Probe) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid != probe.pid || !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return true.into();
        }
        let mut class = [0u16; 64];
        let len = unsafe { GetClassNameW(hwnd, &mut class) };
        if len <= 0 || !is_cicero(&class[..len as usize]) {
            return true.into();
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err()
            || rect.right <= rect.left
            || rect.bottom <= rect.top
        {
            return true.into();
        }
        let area = i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top);
        keep_larger(&mut probe.best, area, rect);
        true.into()
    }

    let mut probe = Probe {
        pid: std::process::id(),
        best: None,
    };
    let lparam = LPARAM(std::ptr::from_mut(&mut probe).cast::<core::ffi::c_void>() as isize);
    unsafe {
        let _ = EnumWindows(Some(visit), lparam);
    }
    probe.best.map(|(_, rect)| rect)
}

/// 类名是不是 TSF 桥接层的默认 IME 界面窗口。
fn is_cicero(class: &[u16]) -> bool {
    class.iter().copied().eq("CiceroUIWndFrame".encode_utf16())
}

/// 同一个槽里只留面积更大的那个窗口。
fn keep_larger(slot: &mut Option<(i64, RECT)>, area: i64, rect: RECT) {
    if slot.is_none_or(|(best, _)| area > best) {
        *slot = Some((area, rect));
    }
}

fn range_rect(context: &ITfContext, ec: u32, range: &ITfRange) -> Option<RECT> {
    let mut rect = RECT::default();
    let mut clipped = BOOL(0);
    let view = unsafe { context.GetActiveView().ok()? };
    unsafe { view.GetTextExt(ec, range, &mut rect, &mut clipped).ok()? };
    let physical = to_physical(rect, unsafe { view.GetWnd().ok() });
    if physical.bottom <= physical.top {
        // 只有宽度没有高度量不出光标行，摆候选窗没有意义；按取不到处理。
        log::log(&format!(
            "候选锚点：GetTextExt 给的是零高度矩形 l={} t={} r={} b={} clipped={}",
            physical.left,
            physical.top,
            physical.right,
            physical.bottom,
            clipped.as_bool()
        ));
        return None;
    }
    Some(physical)
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
