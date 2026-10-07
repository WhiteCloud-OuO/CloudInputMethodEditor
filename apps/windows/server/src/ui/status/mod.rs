//! 悬浮状态条：桌面上常驻、可拖动的浮条，画一排图标按钮。
//! 按钮来自 exe 旁 `data\icons-arrangement.cfg`（排布见 [`arrangement`]），由云朵渲染器画（[`super::painter`]）。
//!
//! 按下鼠标先 `DragDetect`：挪出拖动阈值就交给系统的移动循环（`WM_NCLBUTTONDOWN` + `HTCAPTION`），
//! 结束时 `WM_EXITSIZEMOVE` 报新位置；没挪就是点击，按 x 落进哪个按钮（[`placement`]）。
//! `WM_MOUSEACTIVATE` 回 `MA_NOACTIVATE` 不抢焦点。Caps Lock 亮灭要换图标，另有一个短定时器盯着，
//! 变了就请 UI 线程的消息循环重画一次（窗口过程手上只有摆放状态，画不了）。

mod arrangement;
mod fullscreen;
mod placement;
mod tools;
mod tooltip;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    DragDetect, GetKeyState, ReleaseCapture, VK_CAPITAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, HTCAPTION, HTCLIENT, IDC_ARROW,
    KillTimer, LoadCursorW, MA_NOACTIVATE, PostThreadMessageW, SendMessageW, SetTimer,
    WM_EXITSIZEMOVE, WM_LBUTTONDOWN, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use cloudime_render::StatusCell;

use self::arrangement::{Arrangement, ButtonState};
use self::placement::{Placement, StatusAction};
use self::tooltip::Tooltip;
use super::StatusEvents;
use super::painter::SharedPainter;
use super::window_class::WindowClass;
use super::{WM_STATUS_CAPS, layered, monitor};
use crate::dispatch::StatusView;

const CLASS_NAME: PCWSTR = w!("CloudIMEStatusBar");
static CLASS: WindowClass = WindowClass::new();

/// 状态条与屏幕边缘的间隙（逻辑像素）。
const EDGE_GAP: i32 = 8;

/// Caps Lock 的检查间隔（毫秒）：只读一次键盘状态，换图标要跟手，比全屏那个 1 秒的密。
const CAPS_INTERVAL_MS: u32 = 250;

/// 状态条窗口上 Caps Lock 检查的定时器编号（[`fullscreen::TIMER_ID`] 是 1）。
const CAPS_TIMER_ID: usize = 2;

/// 状态切换提示只显示悬浮工具栏的前四个按钮：中 / 英、全 / 半角、中 / 西文标点、简 / 繁。
pub(super) const TIP_BUTTONS: usize = 4;

/// 图标名 → 点击动作（`icons-arrangement.cfg` 的 `button=`）。同一个按钮的几个状态名指向同一个动作，
/// 状态条上按钮的顺序与显隐由 cfg 的 `pos` 决定，这里只回答「这个图标名意味着什么」。
const ACTIONS: [(&str, StatusAction); 12] = [
    ("ch", StatusAction::ToggleLang),
    ("en", StatusAction::ToggleLang),
    ("caps", StatusAction::ToggleLang),
    ("half", StatusAction::ToggleCharWidthType),
    ("full", StatusAction::ToggleCharWidthType),
    ("ch_marks", StatusAction::TogglePunctuation),
    ("en_marks", StatusAction::TogglePunctuation),
    ("simp_ch", StatusAction::ToggleSimpTrad),
    ("trad_ch", StatusAction::ToggleSimpTrad),
    ("options", StatusAction::OpenOptions),
    ("widgets", StatusAction::OpenWidgets),
    ("spec_chars", StatusAction::OpenSpecChars),
];

thread_local! {
    /// 本线程活着的状态条：HWND → 摆放状态。窗口过程按 HWND 查，查不到（已析构）就忽略。
    static PLACEMENTS: RefCell<HashMap<isize, Rc<Placement>>> = RefCell::new(HashMap::new());
}

/// 悬浮状态条窗口。
pub(super) struct StatusBar {
    hwnd: HWND,

    /// 最近一次要显示的内容；还没显示过时为 `None`。
    data: RefCell<Option<StatusView>>,

    /// 上次用的 DPI。
    dpi: Cell<u32>,

    /// 摆放状态，与窗口过程共享。
    placement: Rc<Placement>,

    /// 云朵渲染器；`None` 不绘制（字体库加载失败）。
    painter: SharedPainter,

    /// 图标按钮的排布（exe 旁 `data\icons-arrangement.cfg`，改动后重绘时重读）。
    arrangement: RefCell<Arrangement>,

    /// 图标按钮的悬停提示；建不出来就没有提示。
    tooltip: Option<Tooltip>,
}

impl StatusBar {
    /// 建一个隐藏的状态条窗口。
    pub(super) fn new(events: StatusEvents, painter: SharedPainter) -> Result<Self> {
        CLASS.ensure(|| WNDCLASSEXW {
            lpfnWndProc: Some(wndproc),
            hInstance: super::module_handle(),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        })?;
        let dpi = unsafe { GetDpiForSystem() }.max(96);
        // NOACTIVATE：显示时不抢应用焦点。
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("悬浮工具栏"),
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
        let placement = Rc::new(Placement::new(hwnd, layered::shadow_margin(dpi), events));
        PLACEMENTS.with(|map| map.borrow_mut().insert(hwnd.0 as isize, placement.clone()));
        Ok(Self {
            hwnd,
            data: RefCell::new(None),
            dpi: Cell::new(dpi),
            placement,
            painter,
            arrangement: RefCell::new(Arrangement::load()),
            tooltip: Tooltip::new(hwnd),
        })
    }

    /// 显示 / 更新：按记住的位置（首次用 `view.anchor`，都没有就右下角）摆放并重绘。
    pub(super) fn update(&self, view: StatusView) {
        if self.placement.pos.get().is_none() {
            self.placement.pos.set(view.anchor);
        }
        *self.data.borrow_mut() = Some(view);
        self.sync_dpi();
        self.render();
    }

    pub(super) fn hide(&self) {
        let _ = unsafe { KillTimer(Some(self.hwnd), CAPS_TIMER_ID) };
        fullscreen::hide(self.hwnd, &self.placement.fullscreen_hidden);
    }

    /// 刷新 DPI。DPI 优先取所在位置显示器的，理由同候选窗口（#146）。
    fn sync_dpi(&self) {
        let monitor_dpi = self
            .placement
            .pos
            .get()
            .and_then(|(x, y)| monitor::dpi_near(POINT { x, y }));
        let dpi = match (monitor_dpi, unsafe { GetDpiForWindow(self.hwnd) }) {
            (Some(dpi), _) => dpi,
            (None, 0) => self.dpi.get(),
            (None, dpi) => dpi,
        };
        self.dpi.set(dpi);
    }

    /// 渲染器要的一排按钮：按 cfg 的 `pos` 从左到右，每个按钮按当前状态挑图标；`limit` 只取前几个
    /// （状态切换提示只要前四个），画满状态条时传 [`TIP_BUTTONS`] 之外的大数（`usize::MAX`）。
    fn status_cells(
        view: &StatusView,
        arrangement: &Arrangement,
        caps: bool,
        limit: usize,
    ) -> Vec<StatusCell> {
        let state = ButtonState {
            english: view.english,
            caps,
            full_width_punctuation: view.full_width_punctuation,
            full_width_chars: view.full_width_chars,
            traditional: view.traditional,
        };
        arrangement
            .buttons
            .iter()
            .take(limit)
            .map(|button| StatusCell::icon(button.svg(&state)))
            .collect()
    }

    /// 画好贴上并显示；顺带记下各按钮边界给点击用。没有可画的按钮、渲染不出来就隐藏。
    ///
    /// Caps Lock 变了（窗口过程的定时器报来）也走这里：重画一遍。
    pub(super) fn render(&self) {
        let view = *self.data.borrow();
        let Some(view) = view else {
            self.hide();
            return;
        };
        let mut arrangement = self.arrangement.borrow_mut();
        if arrangement.refresh() {
            tracing::info!(buttons = arrangement.buttons.len(), "状态条图标排布已重读");
        }
        let cells = Self::status_cells(&view, &arrangement, self.placement.caps.get(), usize::MAX);
        let rendered = match (cells.is_empty(), self.painter.borrow_mut().as_mut()) {
            (false, Some(painter)) => painter.render_status(&cells, self.dpi.get()),
            _ => None,
        };
        let Some(rendered) = rendered else {
            // cfg 里一个按钮都没有（读不到排布 / 全被 `pos=-1` 藏起来）、字体库加载失败或渲染出错
            //（已记日志）：不贴图、不显示。
            drop(arrangement);
            self.hide();
            return;
        };
        let bitmap = &rendered.rendered;
        let content = (bitmap.content_width as i32, bitmap.content_height as i32);
        if content.0 <= 0 || content.1 <= 0 {
            drop(arrangement);
            self.hide();
            return;
        }
        let margin = bitmap.content_x as i32;
        self.placement.margin.set(margin);
        *self.placement.cells.borrow_mut() = rendered
            .cell_edges
            .iter()
            .zip(arrangement.buttons.iter().map(|button| button.action))
            .map(|(edge, action)| (edge.round() as i32, action))
            .collect();
        // 悬停提示：跟点击命中用同一套格子，文字按动作给。
        if let Some(tooltip) = &self.tooltip {
            let tips: Vec<(i32, &str)> = rendered
                .cell_edges
                .iter()
                .zip(arrangement.buttons.iter().map(|button| button.action))
                .map(|(edge, action)| (edge.round() as i32, tip_text(action)))
                .collect();
            tooltip.sync(&tips, margin, content.1);
        }
        let anchor = self.anchor(content, margin);
        let updated = layered::present(
            self.hwnd,
            &bitmap.pixmap,
            (anchor.0 - margin, anchor.1 - margin),
            255,
        );
        drop(arrangement);
        if updated.is_ok() {
            fullscreen::show(
                self.hwnd,
                &self.placement.fullscreen_hidden,
                view.auto_hide_fullscreen,
            );
            // Caps Lock 亮灭不经过 Server（DLL 那边按键根本没送来），只能自己盯着
            let _ = unsafe { SetTimer(Some(self.hwnd), CAPS_TIMER_ID, CAPS_INTERVAL_MS, None) };
        } else {
            self.hide();
        }
    }

    /// 内容左上角：记住的位置，没有就右下角，再夹进工作区；顺带记下。
    fn anchor(&self, content: (i32, i32), margin: i32) -> (i32, i32) {
        let anchor = self
            .placement
            .pos
            .get()
            .unwrap_or_else(|| default_anchor(content, margin));
        let anchor = clamp_anchor(anchor, content, margin);
        self.placement.pos.set(Some(anchor));
        anchor
    }
}

impl Drop for StatusBar {
    fn drop(&mut self) {
        PLACEMENTS.with(|map| map.borrow_mut().remove(&(self.hwnd.0 as isize)));
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// 状态切换提示用的那一小排图标（悬浮工具栏的前四个）。内部自己管图标排布，与状态条互不干扰
/// —— `Arrangement` 只在本模块可见，提示窗通过它拿图标。
pub(super) struct StatusIcons {
    arrangement: Arrangement,
}

impl StatusIcons {
    pub(super) fn load() -> Self {
        Self {
            arrangement: Arrangement::load(),
        }
    }

    /// 按当前状态出一排图标；排布文件动过会重读（与状态条一样）。
    pub(super) fn cells(&mut self, view: &StatusView, caps: bool) -> Vec<StatusCell> {
        self.arrangement.refresh();
        StatusBar::status_cells(view, &self.arrangement, caps, TIP_BUTTONS)
    }
}

/// 状态条按钮的悬停提示：功能名，第二行是快捷键（没有就只一行）。
fn tip_text(action: StatusAction) -> &'static str {
    match action {
        StatusAction::ToggleLang => "中 / 英切换\nShift",
        StatusAction::TogglePunctuation => "中文 / 西文标点\nCtrl + Alt + 逗号",
        StatusAction::ToggleCharWidthType => "全角 / 半角字符\nShift + 空格",
        StatusAction::ToggleSimpTrad => "简体 / 繁体\nCtrl + Alt + 句号",
        StatusAction::OpenOptions => "设置",
        StatusAction::OpenWidgets => "工具",
        StatusAction::OpenSpecChars => "特殊字符输入器",
    }
}

/// 首次出现的位置：主显示器工作区右下角，留出边距与阴影。
fn default_anchor(content: (i32, i32), margin: i32) -> (i32, i32) {
    let work = monitor::primary_work_area();
    let gap = ((EDGE_GAP * margin) / 16).max(EDGE_GAP);
    (
        work.right - margin - gap - content.0,
        work.bottom - margin - gap - content.1,
    )
}

/// 把内容左上角夹进所在显示器的工作区，使整块内容可见。
fn clamp_anchor(anchor: (i32, i32), content: (i32, i32), margin: i32) -> (i32, i32) {
    let work = monitor::work_area_near(POINT {
        x: anchor.0,
        y: anchor.1,
    });
    let x = anchor.0.clamp(
        work.left + margin,
        (work.right - margin - content.0).max(work.left + margin),
    );
    let y = anchor.1.clamp(
        work.top + margin,
        (work.bottom - margin - content.1).max(work.top + margin),
    );
    (x, y)
}

/// Caps Lock 亮着没有：与语言栏按钮同一套判定（低位为 1 表示锁定键亮着）。
fn caps_lock_on() -> bool {
    let state = unsafe { GetKeyState(VK_CAPITAL.0 as i32) };
    state & 1 != 0
}

fn placement_of(hwnd: HWND) -> Option<Rc<Placement>> {
    // clone 出来放开借用，再调回调。
    PLACEMENTS.with(|map| map.borrow().get(&(hwnd.0 as isize)).cloned())
}

/// 按下：拖动交给系统移动循环，没拖就是点击；点击不激活；拖动结束报位置。
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN => {
            let mut point = POINT::default();
            let _ = unsafe { GetCursorPos(&mut point) };
            if unsafe { DragDetect(hwnd, point) }.as_bool() {
                let _ = unsafe { ReleaseCapture() };
                unsafe {
                    SendMessageW(
                        hwnd,
                        WM_NCLBUTTONDOWN,
                        Some(WPARAM(HTCAPTION as usize)),
                        Some(LPARAM(0)),
                    )
                };
            } else if let Some(placement) = placement_of(hwnd) {
                // lparam 低 16 位是客户区 x（有符号）。
                placement.on_click((lparam.0 & 0xFFFF) as i16 as i32);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == fullscreen::TIMER_ID => {
            if let Some(placement) = placement_of(hwnd) {
                fullscreen::on_timer(hwnd, &placement.fullscreen_hidden);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == CAPS_TIMER_ID => {
            if let Some(placement) = placement_of(hwnd) {
                let caps = caps_lock_on();
                if caps != placement.caps.get() {
                    placement.caps.set(caps);
                    // 重画要状态条那半边（窗口过程手上只有摆放状态），交给 UI 线程的消息循环
                    let _ = unsafe {
                        PostThreadMessageW(
                            GetCurrentThreadId(),
                            WM_STATUS_CAPS,
                            WPARAM(0),
                            LPARAM(0),
                        )
                    };
                }
            }
            LRESULT(0)
        }
        WM_EXITSIZEMOVE => {
            if let Some(placement) = placement_of(hwnd) {
                placement.on_moved();
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
