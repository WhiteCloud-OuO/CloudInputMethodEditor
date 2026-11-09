//! 前台全屏形态：区分 D3D 独占全屏与“普通窗口铺满显示器”。
//!
//! 独占全屏下别的进程窗口盖不上去，弹候选窗只会把游戏踢出独占态；铺满（无边框全屏 / F11）
//! 则可以正常叠加普通窗口。判定只在 UI 线程上探测，结果缓存 300ms，避免每键都查一次 shell。

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::Shell::{
    QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetForegroundWindow, GetShellWindow, GetWindowLongW, GetWindowRect, IsZoomed,
    WS_CAPTION,
};

/// 探测结果的有效期：游戏切独占通常发生在激活之后，300ms 足够覆盖这个间隙。
const CACHE_TTL: Duration = Duration::from_millis(300);

/// 前台全屏的三种形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FullscreenKind {
    /// 不是全屏。
    None,

    /// 普通窗口铺满所在显示器（无边框全屏 / F11）：候选窗可以正常叠加。
    Covering,

    /// D3D 独占全屏 / 演示模式：候选窗、状态提示一律不弹。
    D3dExclusive,
}

impl FullscreenKind {
    fn code(self) -> u8 {
        match self {
            Self::None => 1,
            Self::Covering => 2,
            Self::D3dExclusive => 3,
        }
    }
}

/// 最近一次探测结果（0 = 还没探测过）。
static CACHED: AtomicU8 = AtomicU8::new(0);

/// 探测缓存；`Instant` 不能 const，用 `Mutex<Option<_>>` 在首次探测时填。
static CACHE: Mutex<Option<(Instant, FullscreenKind)>> = Mutex::new(None);

/// 当前前台全屏形态。UI 线程调用时会按 `CACHE_TTL` 探测一次，其他线程读 [`exclusive_cached`]。
pub(crate) fn kind() -> FullscreenKind {
    let now = Instant::now();
    if let Ok(cache) = CACHE.lock()
        && let Some((at, kind)) = *cache
        && now.duration_since(at) < CACHE_TTL
    {
        return kind;
    }
    let kind = probe();
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), kind));
    }
    let previous = CACHED.swap(kind.code(), Ordering::Relaxed);
    if previous != kind.code() {
        tracing::info!(?kind, "前台全屏形态变化");
    }
    kind
}

/// 给工人线程读的缓存：只有 UI 线程探测过之后才可信；没探测过按“不是独占”处理。
/// 这样按键路径不会在后台线程调 shell 查询。
pub(crate) fn exclusive_cached() -> bool {
    CACHED.load(Ordering::Relaxed) == FullscreenKind::D3dExclusive.code()
}

fn probe() -> FullscreenKind {
    // 判据①：系统通知状态。演示模式也按独占处理（同样盖不住别的窗口）。
    if let Ok(state) = unsafe { SHQueryUserNotificationState() }
        && (state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE)
    {
        return FullscreenKind::D3dExclusive;
    }

    // 判据②：前台窗口矩形盖满它所在的显示器。
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() || hwnd == unsafe { GetShellWindow() } {
        return FullscreenKind::None;
    }
    let zoomed_with_caption = unsafe { IsZoomed(hwnd) }.as_bool()
        && unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32 & WS_CAPTION.0 == WS_CAPTION.0;
    if zoomed_with_caption {
        return FullscreenKind::None;
    }
    let mut window = RECT::default();
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if GetWindowRect(hwnd, &mut window).is_err() {
            return FullscreenKind::None;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return FullscreenKind::None;
        }
    }
    classify(window, info.rcMonitor, false)
}

fn classify(window: RECT, screen: RECT, zoomed_with_caption: bool) -> FullscreenKind {
    if zoomed_with_caption {
        return FullscreenKind::None;
    }
    if window.left <= screen.left
        && window.top <= screen.top
        && window.right >= screen.right
        && window.bottom >= screen.bottom
    {
        FullscreenKind::Covering
    } else {
        FullscreenKind::None
    }
}

/// 纯判定函数给测试用。
#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::RECT;

    use super::{FullscreenKind, classify};

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn a_covering_window_is_not_exclusive() {
        let screen = rect(0, 0, 1920, 1080);
        assert_eq!(classify(screen, screen, false), FullscreenKind::Covering);
    }

    #[test]
    fn a_smaller_window_is_not_fullscreen() {
        let screen = rect(0, 0, 1920, 1080);
        assert_eq!(
            classify(rect(100, 100, 1000, 800), screen, false),
            FullscreenKind::None
        );
    }

    #[test]
    fn a_maximized_window_with_caption_is_not_fullscreen() {
        let screen = rect(0, 0, 1920, 1080);
        assert_eq!(classify(screen, screen, true), FullscreenKind::None);
    }
}
