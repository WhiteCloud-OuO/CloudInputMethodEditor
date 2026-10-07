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
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_CONTROL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IDC_ARROW, KillTimer, LoadCursorW,
    MA_NOACTIVATE, SW_HIDE, SW_SHOWNA, SetTimer, ShowWindow, WM_CONTEXTMENU, WM_LBUTTONDOWN,
    WM_MBUTTONDOWN, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use cloudime_platform::ItemNumberStyle;
use cloudime_platform::protocol::Frame;
use cloudime_render::{HighlightAnimation, HighlightRect, Pixmap, clip_pixmap};

pub(crate) use self::render_data::RenderData;
use super::CandidateEvents;
use super::layered;
use super::monitor;
use super::painter::SharedPainter;
use super::window_class::WindowClass;
use crate::dispatch::{CandidateEvent, RenderSettings};

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

/// Ctrl + 滚轮一格：候选窗（含文字、留白、阴影）缩放的比例。
const ZOOM_STEP_RATIO: f32 = 1.2;

/// 缩放的级数范围：`1.2^-3` ≈ 58%，`1.2^6` ≈ 299%。
const ZOOM_MIN_STEP: i32 = -3;
const ZOOM_MAX_STEP: i32 = 6;

/// 缩放级数 → 倍数。按 1.2 的整数次幂算（不是反复乘浮点），所以放大再缩回来能精确回到 100%。
fn zoom_factor(step: i32) -> f32 {
    ZOOM_STEP_RATIO.powi(step)
}

/// 滚一格之后的缩放级数（夹在上下限里）。
fn next_zoom_step(current: i32, delta: i16) -> i32 {
    (current + if delta > 0 { 1 } else { -1 }).clamp(ZOOM_MIN_STEP, ZOOM_MAX_STEP)
}

/// 展开 / 收起「更多候选项」（Tab）的过渡时长（毫秒）。
const TRANSITION_MS: u64 = 300;

/// 过渡开始时整张位图的不透明度（之后淡入到 255）。
const TRANSITION_ALPHA_FROM: f32 = 170.0;

/// 过渡到 `progress`（0..=1）时窗口该多大、多不透明。
///
/// 尺寸在「切换前的大小 → 切换后的大小」之间插值，位图本身不缩放——贴的时候只贴出这么大一块，
/// 看上去就是框在长 / 缩、内容逐步露出来（或收回去）。
fn transition_frame(from: (u32, u32), to: (u32, u32), progress: f32) -> ((u32, u32), u8) {
    let lerp = |a: u32, b: u32| {
        (a as f32 + (b as f32 - a as f32) * progress)
            .round()
            .max(1.0) as u32
    };
    let alpha = TRANSITION_ALPHA_FROM + (255.0 - TRANSITION_ALPHA_FROM) * progress;
    (
        (lerp(from.0, to.0), lerp(from.1, to.1)),
        alpha.round().clamp(0.0, 255.0) as u8,
    )
}

/// 本次内容变化要不要起**展开**过渡；要的话给出起点尺寸（上一帧真正贴出去的大小）。
///
/// 条件是 `columns` 由 0 变成非 0——**只有展开方向**（Tab 换的就是它；释义列表临时收起也走这条）。
/// **收起方向不做过渡**：试过交叉淡化 + 裁剪旧屏 + 固定画布，真机上始终看不出变化——
/// 分层窗口「缩小时不一定立刻重画」这件事没法稳定绕开，按决定只保留展开。
/// 与 [`plan_slide`] 各走各的：Tab 那一下 `plan_slide` 会判 `Clear`（内容换了不续滑）。
fn transition_plan(
    previous: &RenderData,
    next: &RenderData,
    shown: bool,
    last_content: (u32, u32),
) -> Option<(u32, u32)> {
    let expanding = previous.columns == 0 && next.columns > 0;
    (expanding && shown && last_content.0 > 0 && last_content.1 > 0).then_some(last_content)
}

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

/// 展开过渡里某一帧怎么贴：尺寸在旧新之间插值，整张位图淡入。
#[derive(Clone, Copy)]
struct TransitionFrame {
    /// 这一帧要贴出去的内容尺寸（物理像素）。
    shown: (u32, u32),

    /// 整张位图的额外不透明度（过渡开始时半透明，淡入到 255）。
    alpha: u8,
}

/// 展开（Tab）的过渡：框从切换前的大小长到新大小。
#[derive(Clone, Copy)]
struct Transition {
    /// 切换前贴出去的内容尺寸（物理像素）。
    from: (u32, u32),

    /// 起始时刻，用来算进度。
    started: Instant,
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

    /// 上次绘制时内容区左上角在位图里的位置（阴影留白）：客户区坐标换内容区坐标用。
    margin: Cell<(i32, i32)>,

    /// 鼠标当前停在哪一格上：同一格不重复上报给 Router。
    hover: Cell<Option<usize>>,

    /// 上次**收起态**画出来的候选高亮条有多宽：展开成网格时它当每格的最小宽度
    /// （见 [`cloudime_render::Frame::min_cell_width`]），免得单字候选挤成一小团。
    collapsed_width: Cell<f32>,

    /// 上一次真正贴出去的内容尺寸（物理像素）。展开过渡的起点按它算，
    /// 所以连按 Tab 也是从眼前这一帧接着长，不会跳。
    last_content: Cell<(u32, u32)>,

    /// 正在跑的**展开**过渡；`None` 没有过渡。
    transition: RefCell<Option<Transition>>,

    /// 这次过渡已经画了几帧（`frame_with_transition` 每走过一帧 +1）：只用来在结束那行日志里
    /// 说明「中间帧到底画出来了没有」——帧数≈时长 / 16 ms 才算正常。
    transition_frames: Cell<u32>,

    /// 候选窗上的鼠标操作（悬停 / 单击）回给 Router。
    events: CandidateEvents,

    /// 滚轮缩放的第几级（Ctrl + 滚轮）：倍数 = `1.2^zoom_step`，只活在内存里（重启回 100%）。
    zoom_step: Cell<i32>,
}

impl CandidateWindow {
    /// 建一个隐藏的候选窗口。
    pub(crate) fn new(painter: SharedPainter, events: CandidateEvents) -> Result<Self> {
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
            margin: Cell::new((0, 0)),
            hover: Cell::new(None),
            collapsed_width: Cell::new(0.0),
            last_content: Cell::new((0, 0)),
            transition: RefCell::new(None),
            transition_frames: Cell::new(0),
            events,
            zoom_step: Cell::new(0),
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
        let (plan, transition_from, rect_count, row_count) = {
            let previous = self.data.borrow();
            let running = *self.slide.borrow();
            let rects = self.last_rects.borrow();
            let shown = self.anchor.get().is_some();
            let plan = plan_slide(
                &previous,
                &next,
                shown,
                running.as_ref(),
                &rects,
                Instant::now(),
            );
            let transition_from = transition_plan(&previous, &next, shown, self.last_content.get());
            (plan, transition_from, rects.len(), next.rows.len())
        };
        *self.data.borrow_mut() = next;
        // 内容换了：鼠标停在哪一格要重新算
        self.hover.set(None);
        let anchor_present = self.anchor.get().is_some();
        match plan {
            // 高亮没变却送来新帧（重排、只挪了窗口）：不打断正在跑的滑动。
            SlidePlan::Keep => tracing::debug!(from, to, "高亮未变，保持高亮滑动"),
            // **只停滑动**：Tab 换内容时也会走到这里（`plan_slide` 判「内容换了不续滑」），
            // 顺手清掉过渡就正好把下面那条也一起清了——那正是「Tab 没有动画」的原因。
            SlidePlan::Clear => {
                // 高亮没动时的普通帧也走这里，`debug` 下才看得见。
                tracing::debug!(
                    prev = from,
                    next = to,
                    rows = row_count,
                    rects = rect_count,
                    anchor = anchor_present,
                    "高亮滑动计划：Clear"
                );
                self.stop_slide();
            }
            SlidePlan::Start(slide) => {
                tracing::debug!(
                    prev = from,
                    next = to,
                    rects = rect_count,
                    from = ?slide.from,
                    "高亮滑动计划：Start"
                );
                self.slide.replace(Some(slide));
            }
        }
        // 展开（Tab）：起一段「框从上一帧的大小长到新大小」的过渡（下一帧 `redraw` 里量到新尺寸）。
        // 与 `plan_slide` 无关，各走各的。日志用 `info`：按 Tab 是用户动作，留个可查的痕迹。
        if let Some(from) = transition_from {
            self.transition_frames.set(0);
            self.transition.replace(Some(Transition {
                from,
                started: Instant::now(),
            }));
            tracing::info!(?from, "展开过渡：起动画");
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
        // 收起态的宽度也跟着作废：换了个应用（窗口宽度不同）不该拿上一段组句的宽度当最小宽度。
        self.collapsed_width.set(0.0);
        // 滚轮缩放是临时的：窗口一关就回到 100%，下次弹出是正常大小。
        self.zoom_step.set(0);
        self.hover.set(None);
        self.stop_animation();
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    /// 定时器到点：按经过时间重画一帧，并决定还继不继续。
    fn on_slide_timer(&self) {
        if self.anchor.get().is_none() {
            self.stop_animation();
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
            let frame = data.render_frame(animation, self.collapsed_width.get());
            self.painter
                .borrow_mut()
                .as_mut()
                .and_then(|painter| painter.render_frame(&frame, data.layout, self.effective_dpi()))
        };
        let Some(rendered) = rendered else {
            // 字体库加载失败或渲染出错（已记日志）：不贴图、不显示。
            self.hide();
            return;
        };
        *self.last_rects.borrow_mut() = rendered.highlight_rects.clone();
        // 收起态：把高亮那条的宽度记下来，展开成网格时当每格的最小宽度。
        {
            let data = self.data.borrow();
            if data.columns == 0
                && let Some(rect) = rendered
                    .highlight_rects
                    .get(data.highlight)
                    .or_else(|| rendered.highlight_rects.first())
            {
                self.collapsed_width.set(rect.width());
            }
        }
        self.margin
            .set((rendered.content_x as i32, rendered.content_y as i32));
        let content = (rendered.content_width, rendered.content_height);
        if content.0 == 0 || content.1 == 0 {
            self.hide();
            return;
        }
        // 展开 / 收起过渡：位图**内容**在缩 / 长（不缩放），并前后互淡；画布尺寸另有讲究，见下。
        // 位置按**最终**尺寸算，免得过渡中边长边缩时上下位置翻来覆去。
        let transition = self.frame_with_transition(content);
        // 位图四周还有阴影留白，裁的是位图，得按留白换算回内容尺寸
        let margin = rendered.content_x.max(rendered.content_y);
        let size = |content: (u32, u32)| (content.0 + margin * 2, content.1 + margin * 2);
        // 展开过渡：位图内容不缩放，只贴出「长到哪儿」的那一块并淡入（收起方向不做过渡，
        // 原因见 `transition_plan` 的注释）。
        let composed = transition.and_then(|frame| {
            clip_pixmap(&rendered.pixmap, size(frame.shown).0, size(frame.shown).1)
        });
        let alpha = transition.map_or(255, |frame| frame.alpha);
        let pixmap: &Pixmap = composed.as_ref().unwrap_or(&rendered.pixmap);
        self.last_content.set(match transition {
            Some(frame) => frame.shown,
            None => content,
        });
        let (content_x, content_y) = place(anchor, (content.0 as i32, content.1 as i32));
        let updated = layered::present(
            self.hwnd,
            pixmap,
            (
                content_x - rendered.content_x as i32,
                content_y - rendered.content_y as i32,
            ),
            alpha,
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
        let progress = eased_progress(slide.started.elapsed(), HIGHLIGHT_SLIDE_MS)?;
        Some(HighlightAnimation {
            from: slide.from,
            progress,
        })
    }

    /// 这一帧该贴多大、多不透明：展开过渡期间在「上一帧的大小 → 这一帧的大小」之间插值，
    /// 同时淡入；没有过渡（或已走完）就是这一帧本身的尺寸、完全不透明。
    fn frame_with_transition(&self, to: (u32, u32)) -> Option<TransitionFrame> {
        let transition = (*self.transition.borrow())?;
        let Some(progress) = eased_progress(transition.started.elapsed(), TRANSITION_MS) else {
            tracing::info!(
                from = ?transition.from,
                to = ?to,
                elapsed_ms = transition.started.elapsed().as_millis() as u64,
                frames = self.transition_frames.get(),
                "展开过渡：结束"
            );
            self.transition.replace(None);
            return None;
        };
        self.transition_frames.set(self.transition_frames.get() + 1);
        let (shown, alpha) = transition_frame(transition.from, to, progress);
        tracing::debug!(?shown, progress, "展开过渡：这一帧");
        Some(TransitionFrame { shown, alpha })
    }

    /// 有动画（高亮滑动 / 展开收起过渡）没走完就保证定时器在跑，都走完就杀掉并清掉状态。
    fn sync_timer(&self) {
        let sliding = self.slide.borrow().is_some_and(|slide| {
            slide.started.elapsed() < Duration::from_millis(HIGHLIGHT_SLIDE_MS)
        });
        let transitioning = self.transition.borrow().is_some_and(|transition| {
            transition.started.elapsed() < Duration::from_millis(TRANSITION_MS)
        });
        if sliding || transitioning {
            let _ = unsafe { SetTimer(Some(self.hwnd), SLIDE_TIMER_ID, SLIDE_FRAME_MS, None) };
        } else {
            self.stop_animation();
        }
    }

    /// 停掉高亮滑动（不动展开 / 收起过渡）。
    fn stop_slide(&self) {
        self.slide.replace(None);
        if self.transition.borrow().is_none() {
            let _ = unsafe { KillTimer(Some(self.hwnd), SLIDE_TIMER_ID) };
        }
    }

    /// 停掉高亮滑动与展开 / 收起过渡，并杀掉共用那一个定时器。
    fn stop_animation(&self) {
        self.slide.replace(None);
        self.transition.replace(None);
        let _ = unsafe { KillTimer(Some(self.hwnd), SLIDE_TIMER_ID) };
    }

    /// 客户区坐标落在第几格候选上。收起态也有各行的矩形（竖排一行一个、横排一个一行），
    /// 所以单击在两种状态下都认；悬停另有一道闸（见 [`Self::on_mouse_move`]）。
    fn cell_at(&self, client: (i32, i32)) -> Option<usize> {
        let (margin_x, margin_y) = self.margin.get();
        let x = (client.0 - margin_x) as f32;
        let y = (client.1 - margin_y) as f32;
        self.last_rects
            .borrow()
            .iter()
            .position(|rect| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
    }

    /// 鼠标移过一格：让 Router 把高亮挪过去（同一格只报一次，鼠标划过去不然会刷屏）。
    /// 收起态与展开态一样跟手。
    fn on_mouse_move(&self, client: (i32, i32)) {
        let Some(index) = self.cell_at(client) else {
            return;
        };
        if self.hover.replace(Some(index)) == Some(index) {
            return;
        }
        (self.events)(CandidateEvent::Hover(index));
    }

    /// 单击一格：上屏（Server 侧把文本交给 DLL，见 `Router::handle_candidate_event`）。
    fn on_mouse_down(&self, client: (i32, i32)) {
        if let Some(index) = self.cell_at(client) {
            (self.events)(CandidateEvent::Commit(index));
        }
    }

    /// 鼠标移出候选窗：不再指着任何一格（Router 那边据此不再把高亮钉在鼠标上）。
    fn on_mouse_leave(&self) {
        if self.hover.replace(None).is_some() {
            (self.events)(CandidateEvent::HoverLeft);
        }
    }

    /// 右键单击一格：不在释义选择界面时等于按 `Ctrl + 反引号`（上屏这一格的译文 / 进多释义选择），
    /// 在释义选择界面里等于 `Esc`。「启用翻译 Tip」关着时 Router 那边什么也不做。
    /// 点没点在格子上都上报（`None` 表示空白处），由 Router 按当前状态决定。
    fn on_mouse_right_down(&self, client: (i32, i32)) {
        (self.events)(CandidateEvent::Translate(self.cell_at(client)));
    }

    /// 中键单击候选窗：等于按 `Shift + 反引号`（念高亮候选的译文）。点在哪一格都一样，
    /// 念的是高亮那个；在释义选择界面里念高亮那条释义。
    fn on_mouse_middle_down(&self) {
        (self.events)(CandidateEvent::Speak);
    }

    /// 滚轮：不带修饰键翻页（下滚下一页），按着 Ctrl 缩放整个候选窗。
    fn on_mouse_wheel(&self, delta: i16) {
        // 修饰键看实时状态：滚轮消息常常是「非活动窗口的悬停滚动」发来的，wParam 低位不保证准
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        if ctrl {
            self.zoom(delta);
            return;
        }
        // 滚轮往前推（delta > 0）是上一页，往后拉是下一页
        (self.events)(CandidateEvent::Page(if delta > 0 { -1 } else { 1 }));
    }

    /// Ctrl + 滚轮一格：整个候选窗（含文字、留白、阴影）按 1.2 的**整数次幂**缩放，下滚放大。
    /// 倍数按级数算，所以放大再缩回来能精确回到 100%；只在内存里，Server 重启就回 100%。
    fn zoom(&self, delta: i16) {
        let current = self.zoom_step.get();
        let step = next_zoom_step(current, delta);
        if step == current {
            return;
        }
        // 收起态记下的那条宽度是**旧倍数**下的像素：先换算过来，展开成的网格才不会按旧宽度算格子
        let ratio = zoom_factor(step) / zoom_factor(current);
        self.collapsed_width.set(self.collapsed_width.get() * ratio);
        self.zoom_step.set(step);
        self.redraw();
    }

    /// 实际用来画的 DPI：显示器 DPI × 滚轮缩放倍数（渲染器只认「点 → 像素」一个倍数）。
    fn effective_dpi(&self) -> u32 {
        let dpi = self.dpi.get().max(96) as f32;
        (dpi * zoom_factor(self.zoom_step.get())).round().max(1.0) as u32
    }

    /// 让窗口在鼠标移出时收到一条 `WM_MOUSELEAVE`（系统只报一次，每次 `WM_MOUSEMOVE` 都要重新登记）。
    fn track_mouse_leave(&self) {
        let mut track = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.hwnd,
            dwHoverTime: 0,
        };
        let _ = unsafe { TrackMouseEvent(&mut track) };
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

/// 已缓动的进度：`None` 表示动画已走完（≥ `total_ms`）。
fn eased_progress(elapsed: Duration, total_ms: u64) -> Option<f32> {
    let total = Duration::from_millis(total_ms);
    if elapsed >= total {
        return None;
    }
    let linear = elapsed.as_secs_f32() / total.as_secs_f32();
    Some(ease_out_cubic(linear))
}

/// 某一时刻的视觉矩形：按已缓动进度在起点与目标之间插值；动画已走完就是目标矩形。
fn visual_rect(slide: &Slide, target: HighlightRect, now: Instant) -> HighlightRect {
    match eased_progress(
        now.saturating_duration_since(slide.started),
        HIGHLIGHT_SLIDE_MS,
    ) {
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
        && previous.layout == next.layout
        && previous.columns == next.columns;
    // 注意**不比 `tip`**：翻译 Tip 显示的就是高亮候选的译文（壳每帧跟着高亮重算），拿它判断
    // 「内容换了没」会让「挪到有译文的候选」统统被当成内容变化、滑动动画被清掉
    // （真机表现：横排 / 竖排里一部分候选之间有动画、一部分没有）。
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
    // 滚轮：不带修饰键翻页、按着 Ctrl 缩放（见 `on_mouse_wheel`）。位置用不上，只取方向。
    if msg == WM_MOUSEWHEEL {
        if let Some(window) = window_of(hwnd) {
            window.on_mouse_wheel((wparam.0 >> 16) as i16);
        }
        return LRESULT(0);
    }
    // 点候选窗不抢应用焦点（与状态条一样）：焦点留在应用里，鼠标点选才能接着往那儿上屏。
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    // 右键：按下那一下做翻译动作（等于 `Ctrl + 反引号`）；抬起与随之而来的 `WM_CONTEXTMENU` 一并吃掉，
    // 不弹任何菜单。中键：按下那一下发音（等于 `Shift + 反引号`）。左键、移动、移出照旧。
    if msg == WM_MOUSEMOVE
        || msg == WM_LBUTTONDOWN
        || msg == WM_RBUTTONDOWN
        || msg == WM_MBUTTONDOWN
        || msg == WM_RBUTTONUP
        || msg == WM_CONTEXTMENU
        || msg == WM_MOUSELEAVE
    {
        if let Some(window) = window_of(hwnd) {
            // lparam 低 16 位是 x、高 16 位是 y（客户区坐标，各有符号）
            let client = (
                (lparam.0 & 0xFFFF) as i16 as i32,
                ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            );
            if msg == WM_MOUSEMOVE {
                window.track_mouse_leave();
                window.on_mouse_move(client);
            } else if msg == WM_LBUTTONDOWN {
                window.on_mouse_down(client);
            } else if msg == WM_RBUTTONDOWN {
                window.on_mouse_right_down(client);
            } else if msg == WM_MBUTTONDOWN {
                window.on_mouse_middle_down();
            } else if msg == WM_MOUSELEAVE {
                window.on_mouse_leave();
            }
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use cloudime_platform::LayoutMode;
    use cloudime_render::{HighlightRect, Row, TipSegment, Tone};

    use super::{
        HIGHLIGHT_SLIDE_MS, RenderData, Slide, SlidePlan, TRANSITION_MS, ZOOM_MAX_STEP,
        ZOOM_MIN_STEP, eased_progress, next_zoom_step, plan_slide, transition_frame,
        transition_plan, visual_rect, zoom_factor,
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

    /// **Tip 不算内容变化**：翻译 Tip 跟着高亮走（壳每帧按高亮候选重算），拿它判断「内容换了没」
    /// 会让「挪到有译文的候选」统统丢掉滑动动画——真机就是「一部分候选之间有动画、一部分没有」。
    #[test]
    fn a_tip_that_follows_the_highlight_does_not_clear_the_slide() {
        let mut prev = data(0);
        prev.tip = vec![TipSegment::new("n. hello", Tone::TranslateFresh, false)];
        let next = data(1); // 高亮挪到第二行，Tip 没了
        let rects = [rect(0.0), rect(20.0), rect(40.0)];
        let plan = plan_slide(&prev, &next, true, None, &rects, Instant::now());
        assert!(matches!(plan, SlidePlan::Start(_)), "Tip 变了不该清掉滑动");
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
        assert_eq!(
            eased_progress(Duration::ZERO, HIGHLIGHT_SLIDE_MS),
            Some(0.0)
        );
        assert_eq!(
            eased_progress(
                Duration::from_millis(HIGHLIGHT_SLIDE_MS),
                HIGHLIGHT_SLIDE_MS
            ),
            None
        );
    }

    /// 只有**展开**方向起过渡（收起方向按决定不做）；起手条件还要求窗口已显示、知道上一帧尺寸。
    /// 它和 `SlidePlan` 各走各的——Tab 那一下 `plan_slide` 判的是 `Clear`（内容换了不续滑）。
    #[test]
    fn only_expanding_starts_a_transition() {
        let mut collapsed = data(0);
        collapsed.columns = 0;
        let mut expanded = data(0);
        expanded.columns = 5;
        let rects = [rect(0.0), rect(20.0), rect(40.0)];
        let now = Instant::now();

        // 收起 → 展开：滑动清掉，但过渡要从上一帧的尺寸起（曾经因为在 `Clear` 里顺手清掉过渡而完全没动画）
        assert!(matches!(
            plan_slide(&collapsed, &expanded, true, None, &rects, now),
            SlidePlan::Clear
        ));
        assert_eq!(
            transition_plan(&collapsed, &expanded, true, (320, 240)),
            Some((320, 240))
        );
        // 展开 → 收起：不起过渡
        assert_eq!(
            transition_plan(&expanded, &collapsed, true, (980, 260)),
            None
        );
        // 没在显示、或还不知道上一帧尺寸：不起过渡
        assert_eq!(
            transition_plan(&collapsed, &expanded, false, (320, 240)),
            None
        );
        assert_eq!(transition_plan(&collapsed, &expanded, true, (0, 0)), None);
        // `columns` 没变（翻页、高亮移动、编辑拼音）：不起过渡
        assert_eq!(
            transition_plan(&collapsed, &collapsed, true, (320, 240)),
            None
        );
        assert_eq!(
            transition_plan(&expanded, &expanded, true, (980, 260)),
            None
        );
    }

    /// 展开 / 收起过渡：尺寸在旧新之间插值、位图不缩放，不透明度从半透明淡到全不透明；
    /// 进度到 1 就是终点、完全不透明。
    #[test]
    fn transition_frames_grow_the_box_and_fade_in() {
        let (from, to) = ((300, 100), (900, 200));
        assert_eq!(transition_frame(from, to, 0.0), ((300, 100), 170));
        assert_eq!(transition_frame(from, to, 0.5), ((600, 150), 213));
        assert_eq!(transition_frame(from, to, 1.0), ((900, 200), 255));
        // 收起方向一样：从大往小插值
        assert_eq!(transition_frame(to, from, 0.5).0, (600, 150));
        // 过渡时长与高亮滑动分开
        assert_eq!(TRANSITION_MS, 300);
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

    /// Ctrl + 滚轮的缩放：级数按 1.2 的整数次幂算，所以上去再下来能精确回到 100%，到头就停。
    #[test]
    fn zoom_steps_are_powers_of_the_ratio_and_stop_at_the_limits() {
        assert_eq!(zoom_factor(0), 1.0);
        assert!((zoom_factor(1) - 1.2).abs() < 1e-6);
        assert!((zoom_factor(2) - 1.44).abs() < 1e-6);
        assert!((zoom_factor(-1) - 1.0 / 1.2).abs() < 1e-6);
        assert!(zoom_factor(ZOOM_MAX_STEP) > 2.9);
        assert!(zoom_factor(ZOOM_MIN_STEP) < 0.59);
        // 上下限之外再滚也不动
        assert_eq!(next_zoom_step(ZOOM_MAX_STEP, 120), ZOOM_MAX_STEP);
        assert_eq!(next_zoom_step(ZOOM_MIN_STEP, -120), ZOOM_MIN_STEP);
        // 放大再缩回来：级数回到恰好 0 = 100%
        assert_eq!(next_zoom_step(next_zoom_step(0, 120), -120), 0);
        assert_eq!(
            zoom_factor(next_zoom_step(next_zoom_step(0, 120), -120)),
            1.0
        );
    }
}
