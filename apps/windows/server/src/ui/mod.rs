//! Server 进程内的 UI 线程：候选窗口与悬浮状态条都在这条线程上自绘（普通置顶窗会被商店 / 任务栏搜索
//! 这些高 z-band 宿主盖住，只有本进程配合 uiAccess 签名才能盖过）。
//!
//! 两类窗口**一律用渲染**（`docs/design/windows.md` §6）：云朵渲染器把一整帧出成预乘 RGBA 位图（那块
//! 「内存区域」），[`layered`] 换通道后 `UpdateLayeredWindow` 整块一次贴上。窗口过程**不做直接绘制** ——
//! 不处理 `WM_PAINT`、窗口类没有背景刷、也没有 `InvalidateRect`：贴上去的那一帧会一直留在窗口上，
//! 只有再贴一帧新的才会变。
//!
//! HWND 线程亲和：Router 在工人线程上产出内容，经通道 + `PostThreadMessageW` 唤醒交给 UI 线程应用。
//! 线程句柄是 [`UiHandle`]，命令在 [`command`]，分层窗口合成在 [`layered`]，两块窗口共用的渲染器在 [`painter`]。
//! 两类窗口各有一个**模板**：独立窗口用 [`independent_window_template`]（自己那块窗 + 名下资产），
//! 辅助窗口（资产）用 [`auxiliary_window_template`]（分层面板 + 贴宿主摆位）。
//!
//! **窗口分两类**（规则见 `docs/design/windows.md`）：
//!
//! - **独立窗口（independent window）**：各有一块自己的分层窗与**自己的生命周期**（UI 线程建、按需显示 / 隐藏、
//!   随进程销毁）—— 候选窗口在 [`candidates`]、悬浮工具栏在 [`status`]、状态切换提示在 [`status_tip`]；
//! - **辅助窗口（auxiliary window）**：挂在某块独立窗口上的辅助信息 / 提示窗口，是那块独立窗口的**资产**（asset），
//!   在 [`auxiliary`]。**所有者唯一且不转移**（每个辅助窗口有且只有一个所有者，就是它的宿主；所有权不移交、
//!   也不被直接借用），而且**只能经桥接**（bridging）跟所有者通讯 —— 它不能直接找别的辅助窗口或别的独立窗口，
//!   要什么都得委托所有者（[`auxiliary::Bridge`]，宿主实现）。生命周期**始终绑定**到宿主：宿主先建好自己那块窗
//!   再建资产（`Assets::add`），销毁时 `Drop` 的第一件事就是把资产放掉（`Assets::retire`）；显示 / 隐藏也由宿主
//!   驱动（`Asset::sync` 里经 `owner.host()` / `owner.asset_on()` 要）。眼下只有候选窗口的
//!   「键位映射参考」（[`auxiliary::key_map`]）一件。
//! - **焦点（focus）**（[`auxiliary::Focus`]）：一组窗口（独立窗口 + 它名下的资产）里**任何时刻恰好一个**持有者。
//!   缺省在独立窗口身上（创建并显示后总是具备焦点），每块独立窗口**都**能**指定**资产获取焦点
//!   （[`auxiliary::DesignatedAssetToGetFocus`]，由独立窗口模板统一实现）—— 接手的资产响应鼠标 / 键盘，
//!   原所有者不再接受。**所有辅助窗口默认不具备焦点**（鼠标穿透、点不着），除非创建时声明
//!   [`auxiliary::Asset::able_to_get_focus`]；归还三条路（所有者收回 / 资产收起 / 资产生命周期末），
//!   后两条**自动**走。**同一所有者**的两件辅助窗口之间转手也**经桥接**（[`auxiliary::Bridge::designate_focus`]，
//!   资产之间不直接通讯）；把自己的焦点转出去叫**失焦**（unfocusing）、获得焦点叫**聚焦**（focusing）。
//!   通过**资产克隆 / 窃取**进来的资产特性照旧（能不能获得焦点跟原件一样），但**不抢焦点** —— 收进来时焦点仍
//!   留在所有者或姊妹资产身上。
//!   这是**逻辑焦点**：谁都不去抢系统的 `SetFocus`（窗口全是 `WS_EX_NOACTIVATE`，抢了会把用户正在打字的
//!   应用弄丢）。

mod auxiliary;
mod auxiliary_window_template;
mod candidates;
mod command;
mod independent_window_template;
mod layered;
mod measure;
mod monitor;
pub(crate) use monitor::{dpi_near, work_area_near};
mod painter;
mod region;
mod script_window;
mod status;
mod status_tip;
mod viewport;
mod window_class;

pub(crate) use measure::SharedMeasurer;
pub(crate) use viewport::Viewport;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use windows::Win32::Foundation::{E_FAIL, HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, DispatchMessageW, GWL_EXSTYLE, GetMessageW,
    GetWindowLongPtrW, MSG, PostThreadMessageW, SetWindowLongPtrW, TranslateMessage, WM_APP,
    WS_EX_TRANSPARENT,
};
use windows::core::{Error, Result};

use cloudime_platform::input_schemes::KeyMapLayout;
use cloudime_platform::input_schemes::KeyMapReference;
use cloudime_platform::protocol::{Frame, ScreenRect};

use self::candidates::CandidateWindow;
use self::command::UiCommand;
use self::painter::{Painter, SharedPainter};
use self::script_window::{ScriptWindowEvents, ScriptWindows};
use self::status::StatusBar;
use self::status_tip::StatusTip;
use crate::dispatch::{
    CandidateEvent, CandidateSink, RenderSettings, ScriptWindowSink, StatusEvent, StatusSink,
    StatusView,
};

/// 状态条上的操作（点格子 / 拖动结束）回给 Router 的回调，UI 线程上调。
pub type StatusEvents = Box<dyn Fn(StatusEvent) + Send>;

/// 候选窗上的鼠标操作（悬停 / 单击）回给 Router 的回调，UI 线程上调。
pub type CandidateEvents = Box<dyn Fn(CandidateEvent) + Send>;

/// 唤醒 UI 线程去排空命令队列的线程消息。
const WM_WAKE: u32 = WM_APP;

/// 状态条窗口的定时器发现 Caps Lock 亮灭变了：重画一次状态条（`WM_WAKE` 是投命令，这个是纯重画）。
const WM_STATUS_CAPS: u32 = WM_APP + 1;

/// UI 线程句柄：发命令 = 投进通道 + 一条 `WM_WAKE`。线程 detached，随进程存活。
#[derive(Clone)]
pub struct UiHandle {
    /// 命令通道的发送端。
    sender: Sender<UiCommand>,

    /// UI 线程 id，`PostThreadMessageW` 用。
    thread_id: u32,

    /// 候选窗最近画出来的内容区尺寸（点），脚本的 `cloudime.candidate.width()` 读它。
    viewport: Arc<Viewport>,
}

impl UiHandle {
    /// 起 UI 线程并等它建好候选窗口。失败返回 `Err`，调用方退化为不画。
    pub fn spawn(
        on_status: StatusEvents,
        on_candidates: CandidateEvents,
        on_script_window: ScriptWindowEvents,
    ) -> Result<Self> {
        // 用 Option<u32> 而非 Result 回报，免得 windows Error 跨线程。
        let (ready_tx, ready_rx) = mpsc::channel::<Option<u32>>();
        let (command_tx, command_rx) = mpsc::channel::<UiCommand>();
        let viewport = Arc::new(Viewport::default());
        let window_viewport = viewport.clone();
        thread::Builder::new()
            .name("cloudime-candidates".to_owned())
            .spawn(move || {
                run(
                    command_rx,
                    &ready_tx,
                    on_status,
                    on_candidates,
                    on_script_window,
                    window_viewport,
                )
            })
            .map_err(|_| Error::from(E_FAIL))?;
        match ready_rx.recv() {
            Ok(Some(thread_id)) => Ok(Self {
                sender: command_tx,
                thread_id,
                viewport,
            }),
            _ => Err(Error::from(E_FAIL)),
        }
    }

    /// 线程已退出（通道断）时静默丢弃。
    fn post(&self, command: UiCommand) {
        if self.sender.send(command).is_ok() {
            let _ = unsafe { PostThreadMessageW(self.thread_id, WM_WAKE, WPARAM(0), LPARAM(0)) };
        }
    }
}

impl CandidateSink for UiHandle {
    fn show(&self, frame: Frame, badges: Vec<Option<char>>, rect: ScreenRect) {
        self.post(UiCommand::Show(Box::new((frame, badges, rect))));
    }

    fn hide(&self) {
        self.post(UiCommand::Hide);
    }

    fn configure(&self, settings: RenderSettings) {
        self.post(UiCommand::Configure(Box::new(settings)));
    }

    fn viewport(&self) -> Option<(f32, f32)> {
        self.viewport.get()
    }

    /// 开关「辅助窗口」（`auxiliary_window`，眼下是键位映射参考）：挂在配置的组合键上。
    /// **必须在这个 impl 里覆盖** —— Router 拿的是 `Box<dyn CandidateSink>`，trait 上的默认体是空的。
    fn auxiliary_toggle(&self) {
        self.post(UiCommand::AuxiliaryToggle);
    }

    /// 换「辅助窗口」的显示方式（「始终跟随候选窗口」那种方式下，候选窗一显示就带上它）。
    fn set_auxiliary_mode(&self, mode: KeyMapReference) {
        self.post(UiCommand::AuxiliaryMode(mode));
    }

    /// 换「辅助窗口」的内容（键位映射参考的键盘版面）。
    fn set_auxiliary(&self, layout: KeyMapLayout) {
        self.post(UiCommand::AuxiliaryKeyMap(layout));
    }
}

impl ScriptWindowSink for UiHandle {
    /// 脚本窗口的操作（建 / 换内容 / 显示 / 收起 / 关）：投给 UI 线程按顺序执行。
    fn script_window(&self, request: cloudime_script::WindowRequest) {
        self.post(UiCommand::ScriptWindow(Box::new(request)));
    }
}

impl StatusSink for UiHandle {
    fn show_status(&self, view: StatusView) {
        self.post(UiCommand::StatusShow(Box::new(view)));
    }

    fn hide_status(&self) {
        self.post(UiCommand::StatusHide);
    }

    fn show_status_tip(&self, view: StatusView, caps: bool, anchor: ScreenRect) {
        self.post(UiCommand::StatusTip(Box::new((view, caps, anchor))));
    }

    fn open_settings(&self) {
        open_settings();
    }

    fn open_download(&self) {
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        let opened = std::process::Command::new("explorer")
            .arg(cloudime_update::DOWNLOAD_URL)
            .spawn();
        if let Err(error) = opened {
            tracing::warn!(%error, "打开下载页失败");
        }
    }
}

/// 起与本 exe 同目录的设置程序。设置程序已开时由新实例把它带到前台，得先把前台权让出去。
pub(crate) fn open_settings() {
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let exe = std::env::current_exe().map(|exe| exe.with_file_name("cloudime-settings.exe"));
    let spawned = exe.and_then(|exe| std::process::Command::new(exe).spawn());
    if let Err(error) = spawned {
        tracing::warn!(%error, "打开设置程序失败");
    }
}

/// 起与本 exe 同目录的特殊字符输入器（随安装包带的独立小工具，`SpecialSymbolsInserter.exe`）。
/// 它自己画成置顶、不抢焦点的窗口，点里面的字符直接注入当前输入框，所以不必把前台权让给它。
pub(crate) fn open_spec_chars() {
    let exe = std::env::current_exe().map(|exe| exe.with_file_name("SpecialSymbolsInserter.exe"));
    let spawned = exe.and_then(|exe| std::process::Command::new(exe).spawn());
    if let Err(error) = spawned {
        tracing::warn!(%error, "打开特殊字符输入器失败");
    }
}

/// 本进程 exe 的模块句柄（注册窗口类 / 建窗口用）。
pub(super) fn module_handle() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    HINSTANCE(module.0)
}

/// 给窗口开 / 关**鼠标穿透**（`WS_EX_TRANSPARENT` 那一个位）：开着鼠标落到下面的窗口（点不着）。
///
/// 窗口之间「谁接受鼠标」都动这一个位（见 `ui::auxiliary` 的焦点）：焦点不在自己身上的一方开着它。
/// 光靠窗口过程回 `HTTRANSPARENT` 不够 —— 那玩意儿只把消息转给**同线程**的下层窗口，而咱底下
/// 通常是别的进程的应用。
pub(super) fn set_mouse_transparent(hwnd: HWND, transparent: bool) {
    let current = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let style = if transparent {
        current | WS_EX_TRANSPARENT.0
    } else {
        current & !WS_EX_TRANSPARENT.0
    };
    unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize) };
}

/// 这个窗口这会儿是不是鼠标穿透（窗口过程的 `WM_NCHITTEST` 照它回答，与 [`set_mouse_transparent`] 同一个位）。
pub(super) fn mouse_transparent(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    style & WS_EX_TRANSPARENT.0 != 0
}

/// UI 线程主体：建窗口、报回线程 id、跑消息循环。
fn run(
    commands: Receiver<UiCommand>,
    ready: &Sender<Option<u32>>,
    on_status: StatusEvents,
    on_candidates: CandidateEvents,
    on_script_window: ScriptWindowEvents,
    viewport: Arc<Viewport>,
) {
    // 按物理像素定位，与应用报来的组句屏幕矩形对齐；已设过会失败，忽略。
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let thread_id = unsafe { GetCurrentThreadId() };
    // 装上时随 Configure 命令建。
    let painter: SharedPainter = Rc::new(RefCell::new(None));
    // 先建窗口再报 id：建窗口顺带建起本线程的消息队列，之后 PostThreadMessageW 才有处可投。
    let window = match CandidateWindow::new(painter.clone(), on_candidates, viewport) {
        Ok(window) => Rc::new(window),
        Err(error) => {
            tracing::error!(%error, "建候选窗口失败，Server 将不显示候选框");
            let _ = ready.send(None);
            return;
        }
    };
    // 登记给窗口过程：高亮滑动动画的定时器要按 HWND 找回它。
    window.attach();
    let status = match StatusBar::new(on_status, painter.clone()) {
        Ok(status) => Some(status),
        Err(error) => {
            tracing::error!(%error, "建悬浮状态条失败，将不显示状态条");
            None
        }
    };
    let tip = match StatusTip::new(painter.clone()) {
        Ok(tip) => Some(tip),
        Err(error) => {
            tracing::error!(%error, "建状态切换提示窗失败，将不显示提示");
            None
        }
    };
    // 脚本建的窗口（`cloudime.window.*`）：与其它窗口共用同一个渲染器，鼠标事件经 `on_script_window`
    // 送回工人线程。
    let mut script_windows = ScriptWindows::new(painter.clone(), on_script_window);
    if ready.send(Some(thread_id)).is_err() {
        return;
    }
    let mut msg = MSG::default();
    loop {
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 <= 0 {
            break;
        }
        if msg.message == WM_WAKE {
            // 一次唤醒排空整个队列，保住 Hide→Show 的先后。
            while let Ok(command) = commands.try_recv() {
                apply(
                    &window,
                    status.as_ref(),
                    tip.as_ref(),
                    &painter,
                    &mut script_windows,
                    command,
                );
            }
            continue;
        }
        if msg.message == WM_STATUS_CAPS {
            if let Some(status) = &status {
                status.render();
            }
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn apply(
    window: &CandidateWindow,
    status: Option<&StatusBar>,
    tip: Option<&StatusTip>,
    painter: &SharedPainter,
    script_windows: &mut ScriptWindows,
    command: UiCommand,
) {
    match command {
        UiCommand::Show(payload) => {
            let (frame, badges, rect) = *payload;
            window.set_content(&frame, &badges);
            // 宿主换了一帧：名下资产由它自己在 `show` 里摆好（见 `super::auxiliary`）
            window.show(to_win_rect(rect));
        }
        UiCommand::Hide => window.hide(),
        UiCommand::AuxiliaryToggle => {
            // 名下的资产由宿主（候选窗口）自己管：这里只把开关递过去
            let on = window.toggle_assets();
            tracing::info!(
                on,
                expanded = window.expanded(),
                layout = ?window.layout(),
                "辅助窗口：开关（键位映射参考）"
            );
        }
        UiCommand::AuxiliaryMode(mode) => {
            tracing::info!(?mode, "辅助窗口：换显示方式（键位映射参考）");
            window.set_asset_mode(mode);
        }
        UiCommand::AuxiliaryKeyMap(layout) => {
            tracing::debug!(
                rows = layout.rows.len(),
                zero = layout.zero_initials.len(),
                "辅助窗口：换内容（键位映射参考）"
            );
            window.set_asset_key_map(layout);
        }
        UiCommand::StatusShow(view) => {
            if let Some(status) = status {
                status.update(*view);
            }
        }
        UiCommand::StatusHide => {
            if let Some(status) = status {
                status.hide();
            }
        }
        UiCommand::StatusTip(payload) => {
            let (view, caps, rect) = *payload;
            if let Some(tip) = tip {
                tip.show(&view, caps, to_win_rect(rect));
            }
        }
        UiCommand::Configure(settings) => {
            window.configure(&settings);
            Painter::configure(painter, &settings);
        }
        UiCommand::ScriptWindow(request) => script_windows.apply(*request),
    }
}

fn to_win_rect(rect: ScreenRect) -> RECT {
    RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `auxiliary_toggle` 必须写成 `impl CandidateSink for UiHandle` 里的方法：Router 拿的是
    /// `Box<dyn CandidateSink>`，写成 `UiHandle` 的固有方法就会落到 trait 上那个**空默认体**上 ——
    /// `Ctrl+F1` 被吃掉却什么也不发生（真机踩过）。
    #[test]
    fn auxiliary_toggle_reaches_the_ui_thread_through_the_trait_object() {
        let (sender, receiver) = mpsc::channel();
        let sink: Box<dyn CandidateSink> = Box::new(UiHandle {
            sender,
            thread_id: 0,
            viewport: Arc::new(Viewport::default()),
        });
        sink.auxiliary_toggle();
        assert!(
            matches!(receiver.try_recv(), Ok(UiCommand::AuxiliaryToggle)),
            "开关命令该投进命令通道"
        );
    }
}
