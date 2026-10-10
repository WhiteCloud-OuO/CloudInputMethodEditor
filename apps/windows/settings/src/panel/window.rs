//! 设置窗口的位置 / 尺寸记忆：上次的几何存在
//! `%LOCALAPPDATA%\CloudIME\settings-window.toml`（**本机状态**，不跟账户漫游，见
//! `cloudime_platform::dirs::settings_window_path`）。下次打开照原样摆回去；文件不在（新用户）、
//! 读不动、数据离奇，就退回 [`DEFAULT_CLIENT_SIZE`] 再居中 —— 也就是加这个功能之前的行为。
//!
//! 框架只给「客户区尺寸」（`WindowVisuals::client_size`，DIP）不给位置，所以分三块做：
//! - **尺寸**：`view` 里把恢复值交给 `client_size`（先夹进工作区），窗口一出现就是最终大小；
//! - **位置**：`create` 里装一个本线程 CBT 钩子，赶在窗口显示之前 `SetWindowPos`（与居中同一套机制）；
//! - **落盘**：窗口失去焦点（`HCBT_SETFOCUS` 换到别的窗口）或即将销毁（`HCBT_DESTROYWND`）时各存一次，
//!   于是拖完 / 缩放过、只要点了别处或正常关掉就记住了；几何没变就不重复写（同一窗口里换控件焦点
//!   也会走这条钩子）。

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO,
    MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, EnumWindows, GetClientRect, GetWindowRect, GetWindowThreadProcessId,
    HCBT_ACTIVATE, HCBT_DESTROYWND, HCBT_SETFOCUS, IsIconic, IsZoomed, SPI_GETWORKAREA,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowPos,
    SetWindowsHookExW, SystemParametersInfoW, WH_CBT,
};
use windows::core::BOOL;

/// 设置窗口打开时的客户区尺寸（DIP）。
///
/// **尺寸必须赶在窗口建出来之前就声明**：框架是「建窗 → 应用 `WindowVisuals` → `Activate`（显示）」
/// 三步，只有第一次 publication 里就给具体值，窗口才会一出现就是最终大小；晚一步（等布局把尺寸
/// 回报上来再缩，那条路删掉了）就会看到「先按系统默认宽度闪一下、再缩」。
///
/// 那一刻窗口还不存在、量不到系统默认值（试过：第一次 `view` 时枚举本进程窗口，一个都没有），
/// 所以直接写死：本机（2560×1440、100% 缩放）系统给的默认客户区是 1912×1028，「宽取 2/3、
/// 高不变」即 1275×1028。小屏由 [`clamp_to_work_area`] 兜住；没有窗口记录（新用户）就用它。
const DEFAULT_CLIENT_SIZE: (f64, f64) = (1275.0, 1028.0);

/// 记录里客户区尺寸能信的范围（DIP）。窗口自己画不出更小的，比这还小就只可能是坏数据。
const MIN_CLIENT_SIZE: (f64, f64) = (240.0, 180.0);
const MAX_CLIENT_SIZE: (f64, f64) = (20000.0, 20000.0);

/// 记录里位置（物理像素）能信的范围：正常虚拟桌面远小于此。
const MAX_POSITION: i32 = 32768;

/// 够大才算主窗口：框架自己还有小窗口，颜色对话框之类也会走 CBT 激活。
const MAIN_WINDOW_MIN_WIDTH: i32 = 320;

/// 探「这块位置还在不在显示器上」时取的标题栏一点（相对窗口左上角的物理像素）。
const PROBE: (i32, i32) = (40, 20);

/// 上次的窗口几何。
///
/// `x` / `y` 是窗口左上角（**外框**）的物理像素；`width` / `height` 是**客户区 DIP** ——
/// 尺寸跟着框架的 `client_size` 走，位置只能靠 Win32，就按 Win32 的物理像素记。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Geometry {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// 启动时读到的「要恢复的几何」；钩子拿不到组件，从这里取。只初始化一次。
static RESTORE: OnceLock<Option<Geometry>> = OnceLock::new();

/// 位置已经摆过（恢复或居中只做一次：钩子与 `view` 里的兜底都会来调）。
static PLACED: AtomicBool = AtomicBool::new(false);

/// 认下来的主窗口句柄（钩子里第一次见到够大的本进程窗口时记上）；0 = 还没见到。
static MAIN: AtomicIsize = AtomicIsize::new(0);

/// 上次写盘的那份几何；一样就不重复写（同一窗口里换控件焦点也会走落盘那条钩子）。
static SAVED: Mutex<Option<Geometry>> = Mutex::new(None);

impl Geometry {
    /// 数据可信吗：尺寸有限、在上下限之间，位置在合理范围。
    fn sane(self) -> bool {
        let size_ok =
            |value: f64, min: f64, max: f64| value.is_finite() && value >= min && value <= max;
        size_ok(self.width, MIN_CLIENT_SIZE.0, MAX_CLIENT_SIZE.0)
            && size_ok(self.height, MIN_CLIENT_SIZE.1, MAX_CLIENT_SIZE.1)
            && self.x.abs() <= MAX_POSITION
            && self.y.abs() <= MAX_POSITION
    }

    /// 写文件的内容。
    fn text(self) -> String {
        format!(
            "# 设置窗口上次的位置与大小（本机状态，删掉即回到默认）。\n\
             # x / y：窗口左上角的物理像素；width / height：客户区 DIP。\n\
             x = {}\n\
             y = {}\n\
             width = {:.1}\n\
             height = {:.1}\n",
            self.x, self.y, self.width, self.height
        )
    }

    /// 解析文件内容：四个数缺一不可，数据离奇也算读不出来。
    fn parse(source: &str) -> Option<Self> {
        let document: toml_edit::DocumentMut = source.parse().ok()?;
        let geometry = Self {
            x: number(document.get("x"))?.round() as i32,
            y: number(document.get("y"))?.round() as i32,
            width: number(document.get("width"))?,
            height: number(document.get("height"))?,
        };
        geometry.sane().then_some(geometry)
    }

    /// 从 [`cloudime_platform::dirs::settings_window_path`] 读；没有 / 读不动 / 数据离奇 → `None`。
    fn load() -> Option<Self> {
        let path = cloudime_platform::dirs::settings_window_path()?;
        let source = std::fs::read_to_string(&path).ok()?;
        let geometry = Self::parse(&source);
        if geometry.is_none() && !source.trim().is_empty() {
            crate::log::warn(format!("窗口几何读不出来，按默认摆：{}", path.display()));
        }
        geometry
    }

    /// 原子写回，返回写到的路径。
    fn store(self) -> Result<PathBuf, String> {
        let path = cloudime_platform::dirs::settings_window_path()
            .ok_or_else(|| "拿不到 LOCALAPPDATA，写不了窗口几何".to_owned())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        }
        cloudime_core::storage::write_atomic_str(&path, &self.text())
            .map_err(|error| error.to_string())?;
        Ok(path)
    }
}

/// 取一个数值键：整数与浮点都认（手改文件时少打个 `.0` 不该算坏数据）。
fn number(item: Option<&toml_edit::Item>) -> Option<f64> {
    let value = item?.as_value()?;
    value
        .as_float()
        .or_else(|| value.as_integer().map(|number| number as f64))
}

/// `create` 里调：读一次上次的几何，再装窗口钩子（位置恢复与落盘都在钩子里）。
pub(crate) fn install() {
    if let Some(geometry) = restore() {
        crate::log::info(format!("恢复窗口几何：{geometry:?}"));
    }
    let hook = unsafe { SetWindowsHookExW(WH_CBT, Some(hook), None, GetCurrentThreadId()) };
    if hook.is_err() {
        crate::log::warn("装窗口钩子失败：窗口记忆与位置恢复会失效");
    }
}

/// `view` 里调：这次窗口的客户区尺寸（恢复值或默认，都夹进工作区）。
pub(crate) fn initial_client_size() -> (f64, f64) {
    let wanted = restore().map_or(DEFAULT_CLIENT_SIZE, |geometry| {
        (geometry.width, geometry.height)
    });
    clamp_to_work_area(wanted)
}

/// `view` 里调（兜底）：万一下一次 `view` 时窗口已经出现而钩子没生效，这里再摆一次。
pub(crate) fn place_once() {
    if PLACED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(hwnd) = find_main_window() {
        MAIN.store(hwnd.0 as isize, Ordering::Relaxed);
        place(hwnd);
    }
}

/// 上次记的几何；没有记录（新用户）或读不出来 → `None`。
fn restore() -> Option<Geometry> {
    *RESTORE.get_or_init(Geometry::load)
}

/// 把窗口摆到上次的位置；没有记录或那个位置已经不在显示器上（换过屏）就居中。只做一次。
fn place(hwnd: HWND) -> bool {
    if PLACED.load(Ordering::Relaxed) {
        return false;
    }
    let placed = restore()
        .filter(|geometry| on_screen(*geometry))
        .is_some_and(|geometry| {
            unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    geometry.x,
                    geometry.y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .is_ok()
        })
        || center(hwnd);
    if placed {
        PLACED.store(true, Ordering::Relaxed);
    }
    placed
}

/// 上次的位置还在不在显示器上：探标题栏上一点，探不到就当归零。
fn on_screen(geometry: Geometry) -> bool {
    let point = POINT {
        x: geometry.x + PROBE.0,
        y: geometry.y + PROBE.1,
    };
    !unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONULL) }
        .0
        .is_null()
}

/// 把窗口挪到它所在显示器的中央；挪不动返回 `false`。
fn center(hwnd: HWND) -> bool {
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err()
        || rect.right - rect.left <= MAIN_WINDOW_MIN_WIDTH
    {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return false;
    }
    let work = info.rcWork;
    let x = work.left + ((work.right - work.left) - (rect.right - rect.left)) / 2;
    let y = work.top + ((work.bottom - work.top) - (rect.bottom - rect.top)) / 2;
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
    .is_ok()
}

/// 把当前几何写盘；与上次写的一样、或这一拍量到的不可信（最大化 / 最小化中、还没布局）就不动。
fn save(hwnd: HWND) {
    let Some(geometry) = geometry_of(hwnd) else {
        return;
    };
    if !geometry.sane() {
        return;
    }
    if SAVED
        .lock()
        .ok()
        .is_some_and(|saved| *saved == Some(geometry))
    {
        return;
    }
    match geometry.store() {
        Ok(path) => {
            if let Ok(mut saved) = SAVED.lock() {
                *saved = Some(geometry);
            }
            crate::log::info(format!("记住窗口几何：{}", path.display()));
        }
        Err(error) => crate::log::warn(format!("记住窗口几何失败：{error}")),
    }
}

/// 量当前窗口：外框左上角（物理像素）+ 客户区（换算成 DIP，与框架同一套单位）。
///
/// 最大化 / 最小化的那一套尺寸不记（`GetWindowRect` 给的是屏幕上的实际大小），下次打开要还原成
/// 普通窗口的样子；还没布局完（客户区是 0）也当量不到。
fn geometry_of(hwnd: HWND) -> Option<Geometry> {
    if unsafe { IsIconic(hwnd) }.as_bool() || unsafe { IsZoomed(hwnd) }.as_bool() {
        return None;
    }
    let mut outer = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut outer) }.ok()?;
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) }.ok()?;
    let scale = f64::from(unsafe { GetDpiForWindow(hwnd) }.max(96)) / 96.0;
    let geometry = Geometry {
        x: outer.left,
        y: outer.top,
        width: f64::from(client.right - client.left) / scale,
        height: f64::from(client.bottom - client.top) / scale,
    };
    (geometry.width > 0.0 && geometry.height > 0.0).then_some(geometry)
}

/// 本线程的 CBT 钩子：窗口位置恢复、失焦 / 销毁时落盘都在这儿 —— 组件拿不到窗口句柄，
/// 只有钩子看得见。装完一直挂着不摘：除上面那三种事件外只做一次比较就返回。
unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let hwnd = HWND(wparam.0 as *mut core::ffi::c_void);
    if code == HCBT_ACTIVATE as i32 {
        // 第一次见到够大的本进程窗口就认下来，顺手摆到上次的位置（没记录就居中）。
        if main_window().0.is_null() && is_main_window(hwnd) {
            MAIN.store(hwnd.0 as isize, Ordering::Relaxed);
        }
        if hwnd == main_window() {
            place(hwnd);
        }
    } else if code == HCBT_SETFOCUS as i32 {
        // 焦点换到别的窗口：这一轮的位置 / 尺寸定型了，存一次。
        let main = main_window();
        if !main.0.is_null() && hwnd != main {
            save(main);
        }
    } else if code == HCBT_DESTROYWND as i32 && hwnd == main_window() {
        // 正常关窗口也走这里（`WM_DESTROY` 之前），所以拖完直接关掉也记得住。
        save(hwnd);
        MAIN.store(0, Ordering::Relaxed);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// 认下来的主窗口句柄；0 表示还没见到。
fn main_window() -> HWND {
    HWND(MAIN.load(Ordering::Relaxed) as *mut core::ffi::c_void)
}

/// 够大才算主窗口：框架自己还有小窗口（消息窗之类），颜色对话框也会走 CBT 激活。
fn is_main_window(hwnd: HWND) -> bool {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok()
        && rect.right - rect.left > MAIN_WINDOW_MIN_WIDTH
}

/// 枚举本进程的窗口，找够大的那个（钩子没生效时的兜底）。
fn find_main_window() -> Option<HWND> {
    struct Probe {
        pid: u32,
        hwnd: HWND,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` 是下面传进来的 `&mut Probe`，回调期间一直有效。
        let probe = unsafe { &mut *(lparam.0 as *mut Probe) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid != probe.pid || !is_main_window(hwnd) {
            return true.into();
        }
        probe.hwnd = hwnd;
        false.into()
    }

    let mut probe = Probe {
        pid: std::process::id(),
        hwnd: HWND(std::ptr::null_mut()),
    };
    unsafe {
        let _ = EnumWindows(
            Some(visit),
            LPARAM(std::ptr::from_mut(&mut probe).cast::<core::ffi::c_void>() as isize),
        );
    }
    (!probe.hwnd.0.is_null()).then_some(probe.hwnd)
}

/// 把想要的客户区尺寸夹进主显示器工作区（DIP），别在小屏上顶出屏幕。
fn clamp_to_work_area((width, height): (f64, f64)) -> (f64, f64) {
    let mut rect = RECT::default();
    let params = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(std::ptr::from_mut(&mut rect).cast::<core::ffi::c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if params.is_err() {
        return (width, height);
    }
    let scale = f64::from(unsafe { GetDpiForSystem() }.max(96)) / 96.0;
    (
        width.min(f64::from(rect.right - rect.left) / scale),
        height.min(f64::from(rect.bottom - rect.top) / scale),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_parse_round_trip() {
        let geometry = Geometry {
            x: -1200,
            y: 64,
            width: 1275.0,
            height: 1028.0,
        };
        assert_eq!(Geometry::parse(&geometry.text()), Some(geometry));
    }

    /// 手改文件时少打个 `.0` 不该算坏数据。
    #[test]
    fn integers_are_accepted_too() {
        assert_eq!(
            Geometry::parse("x = 10\ny = 20\nwidth = 900\nheight = 700\n"),
            Some(Geometry {
                x: 10,
                y: 20,
                width: 900.0,
                height: 700.0,
            })
        );
    }

    /// 缺字段 / 不是 TOML / 数值离奇，一律当读不出来（调用方退回默认 + 居中）。
    #[test]
    fn missing_or_absurd_values_are_rejected() {
        for source in [
            "",
            "这不是 toml",
            "x = 1\ny = 2\nwidth = 900\n",
            "x = 0\ny = 0\nwidth = 10\nheight = 700\n",
            "x = 0\ny = 0\nwidth = 900\nheight = 90000\n",
            "x = 999999\ny = 0\nwidth = 900\nheight = 700\n",
            "x = 0\ny = 0\nwidth = nan\nheight = 700\n",
        ] {
            assert_eq!(Geometry::parse(source), None, "{source:?} 该被当成坏数据");
        }
    }
}
