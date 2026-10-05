//! 候选窗口：不抢焦点、置顶的分层窗口，跟随光标，画拼音行与候选列表，四周柔和阴影。
//! 由云朵渲染器出位图再贴（[`super::painter`]）；绘制内容在 [`RenderData`]，一行的展示形态在 [`row`]。
//!
//! 方向键只移动高亮时，高亮条从一个格子滑到另一个（见 [`HIGHLIGHT_SLIDE_MS`]）：
//! 窗口过程收 `WM_TIMER` 按经过时间算进度、原地重贴；纯展示，不改窗口位置大小。

mod render_data;
pub(crate) mod row;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IDC_ARROW, KillTimer, LoadCursorW, SW_HIDE,
    SW_SHOWNA, SetTimer, ShowWindow, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use cloudime_platform::ItemNumberStyle;
use cloudime_platform::protocol::Frame;
use cloudime_render::{HighlightAnimation, HighlightRect};

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

/// 高亮条从旧格子滑到新格子的时长（毫秒）。
const HIGHLIGHT_SLIDE_MS: u64 = 150;

/// 滑动期间的重画间隔（毫秒），约 60fps。
const SLIDE_FRAME_MS: u32 = 16;

/// 候选窗上高亮滑动动画的定时器编号（定时器按窗口区分）。
const SLIDE_TIMER_ID: usize = 1;

thread_local! {
    /// 本线程活着的候选窗口：HWND → 窗口。窗口过程按 HWND 查，查不到就忽略。
    static WINDOWS: RefCell<HashMap<isize, Rc<CandidateWindow>>> = RefCell::new(HashMap::new());
}

/// 候选窗口中正在跑的高亮滑动。
#[derive(Clone, Copy)]
struct Slide {
    /// 起点矩形（内容区坐标），由「上一帧的视觉位置」算得。
    from: HighlightRect,

    /// 起始时刻，用来算进度。
    started: Instant,
}

/// 本次 `set_content` 该怎么处理正在跑的高亮滑动。
enum SlidePlan {
    /// 保持现状：高亮没变，正在跑的动画继续朝同一目标走。
    Keep,

    /// 清掉动画：没有可动的高亮（单行 / 无高亮），或内容换了不续滑。
    Clear,

    /// 起一段新滑动，从当前视觉位置续滑。
    Start(Slide),
}

/// 候选窗口。内容经 `UpdateLayeredWindow` 一次贴上，窗口过程只额外处理滑动定时器。
pub(crate) struct CandidateWindow {
    hwnd: HWND,

    /// 绘制内容。
    data: RefCell<RenderData>,

    /// 正在跑的高亮滑动；`None` 没有动画。
    slide: RefCell<Option<Slide>>,

    /// 上一次重画时每一行 / 每一格的高亮矩形（内容区坐标）；连按时算当前视觉位置用。
    last_rects: RefCell<Vec<HighlightRect>>,

    /// 上次 `show` 的定位 anchor；滑动期间按它原地重画，不跟着光标跳。
    anchor: Cell<Option<RECT>>,

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
            slide: RefCell::new(None),
            last_rects: RefCell::new(Vec::new()),
            anchor: Cell::new(None),
            dpi: Cell::new(dpi),
            logged_dpi: Cell::new(None),
            painter,
            index_style: Cell::new(ItemNumberStyle::default()),
        })
    }

    /// 把窗口登记进本线程的表，窗口过程按 HWND 找回它。
    pub(crate) fn attach(self: &Rc<Self>) {
        WINDOWS.with(|map| map.borrow_mut().insert(self.hwnd.0 as isize, self.clone()));
    }

    /// 刷新内容（不定位、不显示）；`badges` 是每个候选的来源角标。
    ///
    /// - 高亮移动：起一段滑动动画；若上一段还在跑，从它插出来的当前视觉矩形续滑。
    /// - 高亮没变（行内容 / 拼音行可能变了，例如重排、只挪了窗口）：不打断正在跑的动画，
    ///   渲染器每帧按新内容重算目标矩形即可。
    /// - 翻页 / 新查询 / 重新弹出：内容换了不续滑，直接画新高亮。
    pub(crate) fn set_content(&self, frame: &Frame, badges: &[Option<char>]) {
        let mut next = self.data.borrow().clone();
        next.set(frame, badges, self.index_style.get());
        let (from, to) = (self.data.borrow().highlight, next.highlight);
        let plan = {
            let previous = self.data.borrow();
            let running = *self.slide.borrow();
            let rects = self.last_rects.borrow();
            plan_slide(
                &previous,
                &next,
                self.anchor.get().is_some(),
                running.as_ref(),
                &rects,
                Instant::now(),
            )
        };
        *self.data.borrow_mut() = next;
        match plan {
            // 高亮没变却送来新帧（重排、只挪了窗口）：不打断正在跑的滑动。
            SlidePlan::Keep => tracing::debug!(from, to, "高亮未变，保持高亮滑动"),
            SlidePlan::Clear => self.stop_slide(),
            SlidePlan::Start(slide) => {
                self.slide.replace(Some(slide));
            }
        }
    }

    /// 配置变了：换序号写法（三项字体与最小宽度在 painter 那边）。
    pub(crate) fn configure(&self, settings: &RenderSettings) {
        self.index_style.set(settings.item_number_style);
    }

    /// 按光标矩形定位并显示：贴光标下方（放不下放上方），四周留出阴影。
    pub(crate) fn show(&self, anchor: RECT) {
        self.anchor.set(Some(anchor));
        self.sync_dpi(anchor);
        self.redraw();
        self.sync_timer();
    }

    pub(crate) fn hide(&self) {
        // 忘掉 anchor 与上一帧的位置：下次重新弹出时即使内容碰巧一样，也不该跟上次的高亮位置做滑动。
        self.anchor.set(None);
        self.last_rects.borrow_mut().clear();
        self.stop_slide();
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    /// 定时器到点：按经过时间重画一帧，并决定还继不继续。
    fn on_slide_timer(&self) {
        if self.anchor.get().is_none() {
            self.stop_slide();
            return;
        }
        self.redraw();
        self.sync_timer();
    }

    /// 按当前内容与 `anchor`（上次记住的）重画并贴图；渲染不出来就隐藏。
    fn redraw(&self) {
        let Some(anchor) = self.anchor.get() else {
            return;
        };
        let rendered = {
            let animation = self.current_animation();
            let data = self.data.borrow();
            self.painter.borrow_mut().as_mut().and_then(|painter| {
                painter.render_frame(&data.render_frame(animation), data.layout, self.dpi.get())
            })
        };
        let Some(rendered) = rendered else {
            // 字体库加载失败或渲染出错（已记日志）：不贴图、不显示。
            self.hide();
            return;
        };
        *self.last_rects.borrow_mut() = rendered.highlight_rects.clone();
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

    /// 当前该用的高亮动画：已缓动的进度；没有动画或已走完为 `None`。
    fn current_animation(&self) -> Option<HighlightAnimation> {
        let slide = (*self.slide.borrow())?;
        let progress = eased_progress(slide.started.elapsed())?;
        Some(HighlightAnimation {
            from: slide.from,
            progress,
        })
    }

    /// 动画没走完就保证定时器在跑，走完或没有动画就杀掉并清掉状态。
    fn sync_timer(&self) {
        let running = self.slide.borrow().is_some_and(|slide| {
            slide.started.elapsed() < Duration::from_millis(HIGHLIGHT_SLIDE_MS)
        });
        if running {
            let _ = unsafe { SetTimer(Some(self.hwnd), SLIDE_TIMER_ID, SLIDE_FRAME_MS, None) };
        } else {
            self.stop_slide();
        }
    }

    fn stop_slide(&self) {
        self.slide.replace(None);
        let _ = unsafe { KillTimer(Some(self.hwnd), SLIDE_TIMER_ID) };
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

/// 本线程该 HWND 的候选窗口（窗口过程用）。
fn window_of(hwnd: HWND) -> Option<Rc<CandidateWindow>> {
    WINDOWS.with(|map| map.borrow().get(&(hwnd.0 as isize)).cloned())
}

/// cubic ease-out：起步快、收尾稳。
fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// 已缓动的进度：`None` 表示动画已走完（≥ [`HIGHLIGHT_SLIDE_MS`]）。
fn eased_progress(elapsed: Duration) -> Option<f32> {
    if elapsed >= Duration::from_millis(HIGHLIGHT_SLIDE_MS) {
        return None;
    }
    let linear = elapsed.as_secs_f32() / (HIGHLIGHT_SLIDE_MS as f32 / 1000.0);
    Some(ease_out_cubic(linear))
}

/// 某一时刻的视觉矩形：按已缓动进度在起点与目标之间插值；动画已走完就是目标矩形。
fn visual_rect(slide: &Slide, target: HighlightRect, now: Instant) -> HighlightRect {
    match eased_progress(now.saturating_duration_since(slide.started)) {
        Some(progress) => HighlightRect::lerp(slide.from, target, progress),
        None => target,
    }
}

/// 本次内容变化该怎么处理正在跑的高亮滑动。纯判定，便于单测。
///
/// - 窗口没显示、没有高亮行、只有一行：清掉动画。
/// - 高亮没变：正在跑的动画保持（行内容变了也让渲染器每帧重算目标矩形）。
/// - 高亮变了且内容（行 / 拼音行 / 页码等）没变：起新滑动；上一段还在跑就从它插出来的当前视觉
///   矩形续滑，否则从上一高亮行的矩形起。
/// - 高亮变了但内容换了（翻页 / 新查询 / 重新弹出）：不续滑，直接画。
fn plan_slide(
    previous: &RenderData,
    next: &RenderData,
    anchor_present: bool,
    running: Option<&Slide>,
    last_rects: &[HighlightRect],
    now: Instant,
) -> SlidePlan {
    if !anchor_present || next.highlight == usize::MAX || next.rows.len() <= 1 {
        return SlidePlan::Clear;
    }
    let content_same = previous.rows == next.rows
        && previous.preedit == next.preedit
        && previous.cursor == next.cursor
        && previous.footer == next.footer
        && previous.notice == next.notice
        && previous.layout == next.layout;
    if next.highlight == previous.highlight {
        return if running.is_some() {
            SlidePlan::Keep
        } else {
            SlidePlan::Clear
        };
    }
    if !content_same || previous.highlight == usize::MAX {
        return SlidePlan::Clear;
    }
    let Some(target) = last_rects.get(previous.highlight).copied() else {
        return SlidePlan::Clear;
    };
    let from = match running {
        Some(slide) => visual_rect(slide, target, now),
        None => target,
    };
    SlidePlan::Start(Slide { from, started: now })
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

/// 分层窗口无需 `WM_PAINT`；滑动动画的重画在 `WM_TIMER` 里做。
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TIMER && wparam.0 == SLIDE_TIMER_ID {
        if let Some(window) = window_of(hwnd) {
            window.on_slide_timer();
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use cloudime_platform::LayoutMode;
    use cloudime_render::{HighlightRect, Row};

    use super::{
        HIGHLIGHT_SLIDE_MS, RenderData, Slide, SlidePlan, eased_progress, plan_slide, visual_rect,
    };

    fn rect(top: f32) -> HighlightRect {
        HighlightRect::new(0.0, top, 100.0, top + 20.0)
    }

    /// 三行候选、高亮在第 `highlight` 行的内容。
    fn data(highlight: usize) -> RenderData {
        let mut data = RenderData::empty();
        data.rows = vec![
            Row::plain(0, "甲"),
            Row::plain(1, "乙"),
            Row::plain(2, "丙"),
        ];
        data.highlight = highlight;
        data.layout = LayoutMode::Vertical;
        data
    }

    fn slide(from: HighlightRect, started: Instant) -> Slide {
        Slide { from, started }
    }

    /// 高亮没变的新帧（重排、只挪窗口）不能打断正在跑的动画。
    #[test]
    fn highlight_unchanged_keeps_a_running_slide() {
        let prev = data(1);
        let mut next = data(1);
        next.rows = vec![
            Row::plain(0, "乙"),
            Row::plain(1, "甲"),
            Row::plain(2, "丙"),
        ];
        let running = slide(rect(0.0), Instant::now());
        let rects = [rect(0.0), rect(20.0), rect(40.0)];
        let plan = plan_slide(&prev, &next, true, Some(&running), &rects, Instant::now());
        assert!(matches!(plan, SlidePlan::Keep));
    }

    /// 只移动高亮、且没有在跑的动画：从上一高亮行的矩形起滑。
    #[test]
    fn moving_highlight_starts_from_the_previous_row_rect() {
        let prev = data(0);
        let next = data(1);
        let rects = [rect(0.0), rect(20.0), rect(40.0)];
        let now = Instant::now();
        match plan_slide(&prev, &next, true, None, &rects, now) {
            SlidePlan::Start(start) => {
                assert_eq!(start.from, rects[0]);
                assert_eq!(start.started, now);
            }
            _ => panic!("应起新滑动"),
        }
    }

    /// 连按：上一段 0 → 1 才跑一半，改按到 2 时起点应是当前视觉矩形，而不是第 0 行。
    #[test]
    fn continues_from_the_current_visual_position() {
        let prev = data(1);
        let next = data(2);
        let rects = [rect(0.0), rect(20.0), rect(40.0)];
        let now = Instant::now();
        let running = slide(
            rect(0.0),
            now - Duration::from_millis(HIGHLIGHT_SLIDE_MS / 2),
        );
        let SlidePlan::Start(start) = plan_slide(&prev, &next, true, Some(&running), &rects, now)
        else {
            panic!("应起新滑动");
        };
        assert!(start.from.top > rects[0].top && start.from.top < rects[1].top);
    }

    /// 翻页 / 新查询：内容换了，即使高亮也变了也不续滑。
    #[test]
    fn content_change_does_not_continue_the_slide() {
        let prev = data(0);
        let mut next = data(1);
        next.footer = Some("1/2".to_owned());
        let rects = [rect(0.0), rect(20.0), rect(40.0)];
        let plan = plan_slide(&prev, &next, true, None, &rects, Instant::now());
        assert!(matches!(plan, SlidePlan::Clear));
    }

    /// 窗口没显示、或只有一行候选：清掉动画。
    #[test]
    fn hidden_or_single_row_clears() {
        let prev = data(0);
        let next = data(1);
        let rects = [rect(0.0), rect(20.0)];
        assert!(matches!(
            plan_slide(&prev, &next, false, None, &rects, Instant::now()),
            SlidePlan::Clear
        ));
        let mut single = data(0);
        single.rows = vec![Row::plain(0, "甲")];
        assert!(matches!(
            plan_slide(&prev, &single, true, None, &rects, Instant::now()),
            SlidePlan::Clear
        ));
    }

    #[test]
    fn progress_is_zero_at_start_and_finishes_on_time() {
        assert_eq!(eased_progress(Duration::ZERO), Some(0.0));
        assert_eq!(
            eased_progress(Duration::from_millis(HIGHLIGHT_SLIDE_MS)),
            None
        );
    }

    #[test]
    fn visual_rect_is_the_target_once_finished() {
        let target = rect(40.0);
        let running = slide(
            rect(0.0),
            Instant::now() - Duration::from_millis(HIGHLIGHT_SLIDE_MS + 1),
        );
        assert_eq!(visual_rect(&running, target, Instant::now()), target);
    }
}
