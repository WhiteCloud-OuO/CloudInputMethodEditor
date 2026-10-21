//! 脚本窗口：Lua 用 `cloudime.window.create` / `create_auxiliary` 建出来的两类窗口。
//!
//! 规矩与别的窗口一样（`docs/design/windows.md`）：
//!
//! - **独立窗口**：自己一块分层窗、自己的生命周期（脚本 `close` 或进程退出时销毁）；
//! - **辅助窗口**：挂在某块**脚本独立窗口**名下的一件**资产**，走 [`AuxiliaryWindowTemplate`]，
//!   贴宿主摆位、默认不接鼠标；
//! - 内容都是脚本给的声明式 [`Document`]（点坐标），这里经 [`crate::painter::Painter::render_document`] 出图、
//!   [`layered::present`] 整块贴上 —— 与候选窗同一个渲染器、同一层阴影。
//!
//! **线程**：本模块全部跑在 UI 线程上；脚本在工人线程产出 [`WindowRequest`]，经命令通道过来。
//! 鼠标动作反过来经 [`ScriptWindowEvents`]（`Send` 闭包）回到工人线程，再派给脚本的 `window` 事件。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetCursorPos, GetSystemMetrics, HTCLIENT, HTTRANSPARENT, IDC_ARROW, KillTimer,
    LoadCursorW, MA_NOACTIVATE, SM_CXDOUBLECLK, SM_CYDOUBLECLK, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SetTimer, SetWindowPos, ShowWindow, WM_CAPTURECHANGED,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST,
    WM_RBUTTONUP, WM_TIMER,
};
use windows::core::{PCWSTR, Result, w};

use crate::auxiliary::{Asset, Bridge, DesignatedAssetToGetFocus, Focus, Host};
use crate::auxiliary_window_template::{AuxiliaryWindowSpec, AuxiliaryWindowTemplate, PanelAction};
use crate::independent_window_template::IndependentWindowTemplate;
use crate::layered;
use crate::monitor;
use crate::painter::SharedPainter;
use crate::window::mouse_transparent;
use crate::window_class::WindowClass;
use cloudime_platform::LayoutMode;
use cloudime_render::{
    ButtonState, Color, Document, Element, FontSlot, HitShape, Region, ScrollAxis,
    StaticTextContent, TextBoxScroll,
};
use cloudime_script::{
    AuxiliarySpec, ScriptWindowEvent, WindowAction, WindowId, WindowRequest, WindowSpec,
};
use text_host::{
    ScrollInfo, TextHostWindow, WM_TEXT_CHANGED, WM_TEXT_FOCUS, WM_TEXT_KEY, WM_TEXT_SCROLL,
    WM_TEXT_WHEEL,
};

mod text_host;
mod text_store;
mod tsf;

/// 脚本独立窗口的窗口类（所有脚本窗口共用）。
const CLASS_NAME: PCWSTR = w!("CloudIMEScriptWindow");

static CLASS: WindowClass = WindowClass::new();

/// 脚本窗口鼠标事件的出口：UI 线程调它，把事件送进工人线程的 `Work` 通道。
///
/// `Arc`（而不是 `Rc`）：它要随 UI 线程的启动闭包跨线程移动；窗口之间共享同一份。
pub type ScriptWindowEvents = Arc<dyn Fn(ScriptWindowEvent) + Send + Sync>;

thread_local! {
    /// 本线程建出来的脚本独立窗口（HWND → 窗口）：窗口过程收鼠标 / 命中测试时按它找回状态。
    static REGISTRY: RefCell<HashMap<isize, Weak<ScriptWindow>>> = RefCell::new(HashMap::new());
}

/// 脚本建出来的全部窗口（独立窗口 + 名下的辅助窗口），按编号索引。
pub struct ScriptWindows {
    windows: HashMap<WindowId, Rc<ScriptWindow>>,
    auxiliaries: HashMap<WindowId, Rc<ScriptAuxiliary>>,
    painter: SharedPainter,
    events: ScriptWindowEvents,
}

impl ScriptWindows {
    /// 一块窗口都还没有：`painter` 与别的窗口共用（同一份字形缓存）。
    pub fn new(painter: SharedPainter, events: ScriptWindowEvents) -> Self {
        Self {
            windows: HashMap::new(),
            auxiliaries: HashMap::new(),
            painter,
            events,
        }
    }

    /// 脚本的一次窗口操作（命令通道里来的，按顺序执行）。
    pub fn apply(&mut self, request: WindowRequest) {
        match request {
            WindowRequest::Create(id, spec) => self.create(id, spec),
            WindowRequest::CreateAuxiliary(id, spec) => self.create_auxiliary(id, spec),
            WindowRequest::SetContent(id, document) => {
                if let Some(window) = self.windows.get(&id) {
                    window.set_content(document);
                } else if let Some(aux) = self.auxiliaries.get(&id).cloned() {
                    aux.set_content(document);
                    self.sync_owner(&aux);
                } else {
                    tracing::warn!(id, "脚本窗口：换内容时找不到这块窗口");
                }
            }
            WindowRequest::Show(id, position) => {
                if let Some(window) = self.windows.get(&id) {
                    window.show(position);
                } else if let Some(aux) = self.auxiliaries.get(&id).cloned() {
                    aux.show();
                    self.sync_owner(&aux);
                } else {
                    tracing::warn!(id, "脚本窗口：显示时找不到这块窗口");
                }
            }
            WindowRequest::Hide(id) => {
                if let Some(window) = self.windows.get(&id) {
                    window.hide();
                } else if let Some(aux) = self.auxiliaries.get(&id).cloned() {
                    if let Some(owner) = self.windows.get(&aux.owner) {
                        // §4 规则 7：焦点先归还，再收起这件资产
                        owner.release_focus(&aux.name);
                    }
                    aux.hide();
                    self.sync_owner(&aux);
                } else {
                    tracing::warn!(id, "脚本窗口：收起时找不到这块窗口");
                }
            }
            WindowRequest::Close(id) => self.close(id),
            WindowRequest::Focus(id, focus) => self.focus(id, focus),
            WindowRequest::FocusText { id, name } => {
                if let Some(window) = self.windows.get(&id) {
                    window.focus_text(&name);
                } else {
                    tracing::warn!(id, control = name, "脚本窗口：focus_text 时找不到这块窗口");
                }
            }
            WindowRequest::EditRegion {
                id,
                name,
                shape,
                combine,
            } => {
                if let Some(window) = self.windows.get(&id) {
                    window.edit_region(&name, shape, combine);
                } else if let Some(aux) = self.auxiliaries.get(&id) {
                    aux.edit_region(&name, shape, combine);
                } else {
                    tracing::warn!(id, "脚本窗口：改区域时找不到这块窗口");
                }
            }
            WindowRequest::SetText { id, name, text } => {
                if let Some(window) = self.windows.get(&id) {
                    window.set_text(&name, &text);
                } else {
                    tracing::warn!(id, control = name, "脚本窗口：set_text 时找不到这块窗口");
                }
            }
            WindowRequest::UpdateContent { id, name, content } => {
                if let Some(window) = self.windows.get(&id) {
                    window.update_static_content(&name, content);
                } else if let Some(aux) = self.auxiliaries.get(&id).cloned() {
                    aux.update_static_content(&name, content);
                    self.sync_owner(&aux);
                } else {
                    tracing::warn!(
                        id,
                        control = name,
                        "脚本窗口：update_content 时找不到这块窗口"
                    );
                }
            }
            WindowRequest::SetButtonBackground { id, name, color } => {
                if let Some(window) = self.windows.get(&id) {
                    window.set_button_background(&name, color);
                } else if let Some(aux) = self.auxiliaries.get(&id).cloned() {
                    aux.set_button_background(&name, color);
                    self.sync_owner(&aux);
                } else {
                    tracing::warn!(
                        id,
                        control = name,
                        "脚本窗口：set_button_background 时找不到这块窗口"
                    );
                }
            }
            WindowRequest::SetSelectionBackground { id, name, color } => {
                if let Some(window) = self.windows.get(&id) {
                    window.set_selection_background(&name, color);
                } else {
                    tracing::warn!(
                        id,
                        control = name,
                        "脚本窗口：set_selection_background 时找不到这块窗口"
                    );
                }
            }
            WindowRequest::SelectAll { id, name, select } => {
                if let Some(window) = self.windows.get(&id) {
                    window.select_all(&name, select);
                } else {
                    tracing::warn!(id, control = name, "脚本窗口：select_all 时找不到这块窗口");
                }
            }
            WindowRequest::ReplaceText {
                id,
                name,
                find,
                text,
            } => {
                if let Some(window) = self.windows.get(&id) {
                    window.replace_text(&name, &find, &text);
                } else {
                    tracing::warn!(id, control = name, "脚本窗口：replace 时找不到这块窗口");
                }
            }
            WindowRequest::ClearText { id, name } => {
                if let Some(window) = self.windows.get(&id) {
                    window.clear_text(&name);
                } else {
                    tracing::warn!(id, control = name, "脚本窗口：clear 时找不到这块窗口");
                }
            }
        }
    }

    fn create(&mut self, id: WindowId, spec: WindowSpec) {
        if self.windows.contains_key(&id) {
            tracing::warn!(id, "脚本窗口：编号重复，忽略这次创建");
            return;
        }
        match ScriptWindow::new(id, spec.clone(), self.painter.clone(), self.events.clone()) {
            Ok(window) => {
                tracing::info!(id, name = %spec.name, width = spec.width, height = spec.height,
                    interactive = spec.interactive, "脚本独立窗口已建");
                self.windows.insert(id, window);
            }
            Err(error) => tracing::warn!(%error, id, "建脚本独立窗口失败"),
        }
    }

    fn create_auxiliary(&mut self, id: WindowId, spec: AuxiliarySpec) {
        if self.auxiliaries.contains_key(&id) || self.windows.contains_key(&id) {
            tracing::warn!(id, "脚本窗口：编号重复，忽略这次创建");
            return;
        }
        let Some(owner) = self.windows.get(&spec.owner).cloned() else {
            tracing::warn!(
                id,
                owner = spec.owner,
                "脚本辅助窗口：宿主不存在（独立窗口要先建）"
            );
            return;
        };
        match ScriptAuxiliary::new(id, spec.clone(), self.painter.clone(), self.events.clone()) {
            Ok(aux) => {
                let aux = Rc::new(aux);
                tracing::info!(id, name = %spec.name, owner = spec.owner, interactive = spec.interactive,
                    "脚本辅助窗口已建");
                owner.add_asset(Box::new(AssetHandle(aux.clone())));
                self.auxiliaries.insert(id, aux);
            }
            Err(error) => tracing::warn!(%error, id, "建脚本辅助窗口失败"),
        }
    }

    fn close(&mut self, id: WindowId) {
        if let Some(window) = self.windows.remove(&id) {
            // 关掉的窗口如果正拿着键盘焦点：先告诉工人线程（`key` 载荷不别再带它）
            window.notify_blur();
            tracing::info!(id, name = %window.name, "脚本独立窗口已关（名下资产一起放掉）");
            // 名下资产是它自己的资产表里放着的：`ScriptWindow` 一 `Drop`，模板先 `retire` 再销毁。
            self.auxiliaries.retain(|_, aux| aux.owner != id);
            drop(window);
            return;
        }
        if let Some(aux) = self.auxiliaries.remove(&id) {
            if let Some(owner) = self.windows.get(&aux.owner) {
                owner.retire_asset(&aux.name);
            }
            tracing::info!(id, name = %aux.name, owner = aux.owner, "脚本辅助窗口已关");
        }
    }

    /// 键盘焦点（独立窗口）或逻辑焦点（辅助窗口，经宿主指定）。
    fn focus(&self, id: WindowId, focus: bool) {
        if let Some(window) = self.windows.get(&id) {
            if focus {
                window.notify_focus();
            } else {
                window.notify_blur();
            }
            return;
        }
        let Some(aux) = self.auxiliaries.get(&id) else {
            tracing::warn!(id, "脚本辅助窗口：focus 时找不到这块窗口");
            return;
        };
        let Some(owner) = self.windows.get(&aux.owner) else {
            return;
        };
        let moved = if focus {
            DesignatedAssetToGetFocus::designate_focus(owner.as_ref(), &aux.name)
        } else {
            DesignatedAssetToGetFocus::return_focus_to_owner(owner.as_ref())
        };
        if !moved {
            tracing::warn!(
                id,
                focus,
                name = %aux.name,
                "脚本辅助窗口：焦点没动（没声明 interactive / 本来就在它手上）"
            );
        }
    }

    /// 宿主收起 / 显示 / 换内容之后，把它名下的资产驱动一遍。
    fn sync_owner(&self, aux: &Rc<ScriptAuxiliary>) {
        if let Some(owner) = self.windows.get(&aux.owner) {
            owner.sync_assets();
        }
    }
}

/// 脚本窗口上一次贴上去的那一幅：位置 + 内容尺寸 + 内容版本 + DPI + 渲染器代号。
type Painted = ((i32, i32), (i32, i32), u64, u64, u64);

/// 上一次按钮单击：时间、按钮下标、客户区坐标（判双击用）。
type LastClick = (Instant, usize, (i32, i32));

/// 一个 `text_edit` 覆盖在脚本窗口上的 windowless RichEdit 宿主。
struct TextEditWindow {
    /// 元素路径（`document.elements` 里的根 / 面板 children 下标）。
    path: Vec<usize>,

    /// 自动区域名 / 控件名，`get_text` / `set_text` / 焦点都按它找。
    name: String,

    /// 是 `text_box`（多行）还是 `text_edit`；同名但类型变了不能复用宿主。
    multiline: bool,

    /// 控件左上角在内容区的绝对位置（点，已经累加父面板偏移）。
    origin: (f32, f32),

    /// 上一次派发给脚本的文本，重复的 `EN_CHANGE` / 输入消息不再重复派事件。
    last_text: RefCell<String>,

    host: Rc<TextHostWindow>,
}

/// 拖动状态：按下时的光标屏幕坐标 + 窗口左上角（物理像素）。
#[derive(Debug, Clone, Copy)]
struct Drag {
    start_cursor: (i32, i32),
    start_window: (i32, i32),
}

/// 滚动条拖动状态：元素下标 + 按下点相对滑块起点的偏移（点）。
#[derive(Debug, Clone, Copy)]
struct ScrollDrag {
    index: usize,
    offset: f32,
}

/// `text_box` 内部滚动条拖动：元素路径 + 轴向 + 按下点相对滑块起点的偏移（点）。
struct TextScrollDrag {
    path: Vec<usize>,
    axis: ScrollAxis,
    offset: f32,
}

/// `text_box` 内部滚动条的命中结果：轴向、滚动状态、轨道矩形、控件内坐标。
type TextBarHit = (ScrollAxis, TextBoxScroll, (f32, f32, f32, f32), (f32, f32));

/// 建 RichEdit 宿主前的统一参数（单行 / 多行共用）。
struct TextHostJob {
    path: Vec<usize>,
    name: String,
    origin: (f32, f32),
    text: String,
    font: FontSlot,
    font_name: Option<String>,
    size: Option<f32>,
    fill: Option<Color>,
    text_color: Option<Color>,
    selection_background: Option<Color>,
    selection_color: Option<Color>,
    caret_color: Option<Color>,
    font_flag: u8,
    read_only: bool,
    passkey: bool,
    multiline: bool,
    auto_wrap: bool,
    line_spacing: u16,
}

/// 滚动条非拖拽改值（滚轮 / 点轨道）的过渡动画。
const SCROLL_ANIM_MS: u64 = 140;
const SCROLL_ANIM_FRAME_MS: u32 = 16;

struct ScrollAnimation {
    from: u64,
    to: u64,
    start: Instant,
}

/// `text_box` 内部滚动条的非拖拽动画（像素）。
struct TextScrollAnimation {
    from: i32,
    to: i32,
    start: Instant,
}

/// 一次拖动要移动超过多少像素才算「拖」而不是「点」。
const DRAG_THRESHOLD: i32 = 4;

/// 一块脚本独立窗口。
pub(super) struct ScriptWindow {
    id: WindowId,
    name: String,
    spec: WindowSpec,

    /// 独立窗口模板（自己那块分层窗 + 名下的资产 + 这一组的焦点）。
    template: RefCell<IndependentWindowTemplate>,

    /// 要画的内容（`set_content` 整份替换）。
    document: RefCell<Document>,

    painter: SharedPainter,
    events: ScriptWindowEvents,

    /// 显示着没有。
    shown: Cell<bool>,

    /// 内容区左上角的屏幕坐标（物理像素）。
    position: Cell<Option<(i32, i32)>>,

    /// 最近一帧的内容区尺寸（物理像素）。
    content_size: Cell<(i32, i32)>,

    /// 内容版本：`set_content` / 换阴影时 +1。
    revision: Cell<u64>,

    /// 上次真贴上去的是哪一幅（位置 / 尺寸 / 版本 / 渲染器代号）。
    painted: Cell<Option<Painted>>,

    /// 最近一帧每个元素的命中区域（内容区坐标，点）。
    hits: RefCell<Vec<Option<HitShape>>>,

    /// 内容里声明的区域（点击 / 拖拽的命中范围；与 `document.regions` 同步）。
    regions: RefCell<Vec<Region>>,

    /// 鼠标正停在哪（元素下标、区域下标；都是 0 起，`None` = 没停在任何东西上）：
    /// 悬停事件只在「换元素 / 换区域」时派一次。
    hover: Cell<Option<(Option<usize>, Option<usize>)>>,

    /// 当前鼠标所在的扁平按钮元素下标。
    hover_button: Cell<Option<usize>>,

    /// 当前按下的扁平按钮元素下标（按下态）。
    pressed_button: Cell<Option<usize>>,

    /// 上一次按钮单击：时间、按钮下标、客户区坐标（判双击用）。
    last_click: Cell<Option<LastClick>>,

    /// 各 `text_edit` 的 windowless RichEdit 宿主窗。
    text_edits: RefCell<Vec<TextEditWindow>>,

    /// 拖动中：按下时的光标屏幕坐标 + 窗口左上角。
    drag: Cell<Option<Drag>>,

    /// 滚动条拖动：正在拖哪个元素 + 按下点相对滑块起点的偏移。
    scrollbar_drag: Cell<Option<ScrollDrag>>,

    /// `text_box` 内部滚动条拖动。
    text_scroll_drag: RefCell<Option<TextScrollDrag>>,

    /// 滚动条过渡动画（元素下标 → 起止值 / 开始时间）。
    scroll_anims: RefCell<HashMap<usize, ScrollAnimation>>,

    /// `text_box` 内部滚动条过渡动画（元素路径 + 轴向）。
    text_scroll_anims: RefCell<HashMap<(Vec<usize>, ScrollAxis), TextScrollAnimation>>,

    /// 动画定时器（0 = 没启动）。
    scroll_anim_timer: Cell<usize>,

    /// 这一次按下之后有没有真拖动过（系统可能在 `WM_LBUTTONUP` 之前就撤掉捕获，
    /// 所以「拖过就不算点击」得单独记）。
    moved_since_press: Cell<bool>,

    /// 最后一次收到的客户区坐标。
    last: Cell<(i32, i32)>,

    /// 最近一帧的渲染倍数与阴影留白。
    scale: Cell<f32>,
    margin: Cell<i32>,

    /// 最近一次渲染用的 DPI。
    dpi: Cell<u32>,
}

impl ScriptWindow {
    fn new(
        id: WindowId,
        spec: WindowSpec,
        painter: SharedPainter,
        events: ScriptWindowEvents,
    ) -> Result<Rc<Self>> {
        let cursor = spec
            .interactive
            .then(|| unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default());
        let template = IndependentWindowTemplate::new(
            &CLASS,
            CLASS_NAME,
            w!("云朵脚本窗口"),
            wndproc,
            !spec.interactive,
            cursor,
        )?;
        let window = Rc::new(Self {
            id,
            name: spec.name.clone(),
            document: RefCell::new(Document::empty(spec.width, spec.height)),
            template: RefCell::new(template),
            painter,
            events,
            spec,
            shown: Cell::new(false),
            position: Cell::new(None),
            content_size: Cell::new((0, 0)),
            revision: Cell::new(0),
            painted: Cell::new(None),
            hits: RefCell::new(Vec::new()),
            regions: RefCell::new(Vec::new()),
            hover: Cell::new(None),
            hover_button: Cell::new(None),
            pressed_button: Cell::new(None),
            last_click: Cell::new(None),
            text_edits: RefCell::new(Vec::new()),
            drag: Cell::new(None),
            scrollbar_drag: Cell::new(None),
            text_scroll_drag: RefCell::new(None),
            scroll_anims: RefCell::new(HashMap::new()),
            text_scroll_anims: RefCell::new(HashMap::new()),
            scroll_anim_timer: Cell::new(0),
            moved_since_press: Cell::new(false),
            last: Cell::new((0, 0)),
            scale: Cell::new(1.0),
            margin: Cell::new(0),
            dpi: Cell::new(system_dpi()),
        });
        REGISTRY.with(|map| {
            map.borrow_mut()
                .insert(window.hwnd().0 as isize, Rc::downgrade(&window));
        });
        Ok(window)
    }

    fn hwnd(&self) -> HWND {
        self.template.borrow().hwnd()
    }

    /// 换一整份内容：显示着就立刻重画（名下的资产跟着重新摆）。
    fn set_content(&self, document: Document) {
        *self.regions.borrow_mut() = document.regions.clone();
        *self.document.borrow_mut() = document;
        // 新内容里的按钮从 normal 开始；鼠标下次移动再按位置切状态。
        self.hover.set(None);
        self.hover_button.set(None);
        self.pressed_button.set(None);
        self.scrollbar_drag.set(None);
        *self.text_scroll_drag.borrow_mut() = None;
        self.scroll_anims.borrow_mut().clear();
        self.text_scroll_anims.borrow_mut().clear();
        self.stop_scroll_anim_timer();
        self.last_click.set(None);
        self.rebuild_text_edits();
        self.revision.set(self.revision.get() + 1);
        if self.shown.get()
            && let Some(position) = self.position.get()
        {
            self.render_and_present(position);
            self.sync_assets();
        }
    }

    /// 按当前内容同步 windowless RichEdit 宿主窗。
    ///
    /// 同名、同类型的编辑框保留原宿主（keyed diff）：列表重排 / 只改其它行时，
    /// 焦点、选区、正在输入的文本都不会被打断；新出现的才建，消失的随 `old` 一起析构。
    fn rebuild_text_edits(&self) {
        let mut old: HashMap<String, TextEditWindow> = self
            .text_edits
            .borrow_mut()
            .drain(..)
            .map(|edit| (edit.name.clone(), edit))
            .collect();
        let mut jobs = Vec::new();
        {
            let document = self.document.borrow();
            for placement in document.text_edits() {
                let Some(Element::TextEdit(edit)) = document.element_at_path(&placement.path)
                else {
                    continue;
                };
                if edit.region_name.is_empty() {
                    continue;
                }
                jobs.push(TextHostJob {
                    path: placement.path,
                    name: edit.region_name.clone(),
                    origin: (placement.x, placement.y),
                    text: edit.text.clone(),
                    font: edit.font,
                    font_name: edit.font_name.clone(),
                    size: edit.size,
                    fill: edit.fill,
                    text_color: edit.text_color,
                    selection_background: edit.selection_background,
                    selection_color: edit.selection_color,
                    caret_color: edit.caret_color,
                    font_flag: edit.font_flag,
                    read_only: edit.read_only,
                    passkey: edit.passkey,
                    multiline: false,
                    auto_wrap: false,
                    line_spacing: 0,
                });
            }
            for placement in document.text_boxes() {
                let Some(Element::TextBox(edit)) = document.element_at_path(&placement.path) else {
                    continue;
                };
                if edit.region_name.is_empty() {
                    continue;
                }
                jobs.push(TextHostJob {
                    path: placement.path,
                    name: edit.region_name.clone(),
                    origin: (placement.x, placement.y),
                    text: edit.text.clone(),
                    font: edit.font,
                    font_name: edit.font_name.clone(),
                    size: edit.size,
                    fill: edit.fill,
                    text_color: edit.text_color,
                    selection_background: edit.selection_background,
                    selection_color: edit.selection_color,
                    caret_color: edit.caret_color,
                    font_flag: edit.font_flag,
                    read_only: edit.read_only,
                    passkey: false,
                    multiline: true,
                    auto_wrap: edit.auto_line_wrap,
                    line_spacing: edit.line_spacing,
                });
            }
        }

        let (
            theme_family,
            theme_size,
            theme_background,
            theme_text,
            theme_selection_background,
            theme_selection_text,
        ) = match self.painter.borrow().as_ref() {
            Some(painter) => {
                let (family, size) = painter.text_font(FontSlot::Candidate);
                let (background, text) = painter.text_edit_colors();
                let (selection_background, selection_text) = painter.text_edit_selection_colors();
                (
                    family,
                    size,
                    background,
                    text,
                    selection_background,
                    selection_text,
                )
            }
            None => (
                None,
                14.0,
                Color::rgba(255, 255, 255, 255),
                Color::rgba(34, 34, 34, 255),
                Color::rgba(204, 232, 255, 255),
                Color::rgba(34, 34, 34, 255),
            ),
        };

        let mut edits = self.text_edits.borrow_mut();
        for job in jobs {
            if let Some(mut edit) = old.remove(&job.name)
                && edit.multiline == job.multiline
            {
                edit.path = job.path;
                edit.origin = job.origin;
                if edit.host.text_string() != job.text {
                    edit.host.set_text(&job.text);
                }
                *edit.last_text.borrow_mut() = job.text;
                edits.push(edit);
                continue;
            }
            let (family, size) = match self.painter.borrow().as_ref() {
                Some(painter) => painter.text_font(job.font),
                None => (theme_family.clone(), theme_size),
            };
            let size = job.size.unwrap_or(size);
            let family = job.font_name.clone().or(family);
            let text_color = job.text_color.unwrap_or(theme_text);
            let background = job
                .fill
                .filter(|color| color.a == 255)
                .unwrap_or(theme_background);
            let selection_background = job
                .selection_background
                .unwrap_or(theme_selection_background);
            let selection_text = job.selection_color.unwrap_or(theme_selection_text);
            let caret_color = job.caret_color.unwrap_or(text_color);
            let host = TextHostWindow::create(
                self.hwnd(),
                &job.text,
                family.as_deref(),
                size,
                colorref(background),
                colorref(text_color),
                colorref(selection_background),
                colorref(selection_text),
                colorref(caret_color),
                job.font_flag,
                job.read_only,
                job.passkey,
                job.multiline,
                job.auto_wrap,
                job.line_spacing,
                self.dpi.get(),
            );
            match host {
                Ok(host) => {
                    if job.multiline {
                        // 先用「没有滚动条」的文本区让 RichEdit 排一次，再把真实范围写回控件。
                        let initial =
                            self.document
                                .borrow()
                                .element_at_path(&job.path)
                                .and_then(|element| match element {
                                    Element::TextBox(edit) => Some(edit.edit_rect()),
                                    _ => None,
                                });
                        if let Some((_, _, width, height)) = initial {
                            let scale = self.dpi.get() as f32 / 96.0;
                            host.set_bounds(
                                0,
                                0,
                                (width * scale).round().max(1.0) as i32,
                                (height * scale).round().max(1.0) as i32,
                            );
                        }
                        host.ensure_layout();
                        let (horizontal, vertical) = host.scroll_info();
                        self.apply_text_box_scroll(&job.path, horizontal, vertical);
                    }
                    edits.push(TextEditWindow {
                        path: job.path,
                        name: job.name,
                        multiline: job.multiline,
                        origin: job.origin,
                        last_text: RefCell::new(job.text),
                        host,
                    });
                }
                Err(error) => {
                    tracing::warn!(id = self.id, path = ?job.path, %error, "text：建 windowless RichEdit 失败");
                }
            }
        }
    }

    /// 把 RichEdit 的滚动范围写回 `text_box`，返回滚动条显隐是否变化。
    fn apply_text_box_scroll(
        &self,
        path: &[usize],
        horizontal: ScrollInfo,
        vertical: ScrollInfo,
    ) -> bool {
        let mut document = self.document.borrow_mut();
        let Some(Element::TextBox(edit)) = document.element_at_path_mut(path) else {
            return false;
        };
        let horizontal = if edit.auto_line_wrap {
            TextBoxScroll::default()
        } else {
            scroll_state(horizontal)
        };
        let vertical = scroll_state(vertical);
        let changed = edit.h_scroll != horizontal || edit.v_scroll != vertical;
        edit.h_scroll = horizontal;
        edit.v_scroll = vertical;
        changed
    }

    fn refresh_text_box_scroll(&self, path: &[usize], host: &TextHostWindow) -> bool {
        let (horizontal, vertical) = host.scroll_info();
        self.apply_text_box_scroll(path, horizontal, vertical)
    }

    /// 把 RichEdit 宿主摆到背景形状的文本区；隐藏时一起隐藏。
    fn sync_text_edits(&self) {
        let Some(position) = self.position.get() else {
            return;
        };
        let scale = self.scale.get().max(0.01);
        let shown = self.shown.get();
        let document = self.document.borrow();
        for edit in self.text_edits.borrow().iter() {
            let Some(element) = document.element_at_path(&edit.path) else {
                continue;
            };
            let (x, y, width, height) = match element {
                Element::TextEdit(control) => control.edit_rect(),
                Element::TextBox(control) => control.edit_rect(),
                _ => continue,
            };
            let x = position.0 + ((edit.origin.0 + x) * scale).round() as i32;
            let y = position.1 + ((edit.origin.1 + y) * scale).round() as i32;
            let width = (width * scale).round().max(1.0) as i32;
            let height = (height * scale).round().max(1.0) as i32;
            edit.host.set_bounds(x, y, width, height);
            edit.host.show(shown);
        }
    }

    fn destroy_text_edits(&self) {
        self.text_edits.borrow_mut().clear();
    }

    /// 点到文本编辑框（顶层或面板里）的区域时，把焦点交给对应的 RichEdit 宿主。
    fn focus_text_edit(&self, element: Option<usize>, region: Option<String>) {
        let edits = self.text_edits.borrow();
        let target = edits.iter().find(|edit| {
            region.as_deref() == Some(edit.name.as_str())
                || element
                    .is_some_and(|element| edit.path.len() == 1 && edit.path[0] + 1 == element)
        });
        if let Some(edit) = target {
            edit.host.focus();
        }
    }

    /// 按控件名把键盘焦点交给 `text_edit` / `text_box` 的宿主。
    fn focus_text(&self, name: &str) {
        if let Some(host) = self.text_edit_host(name) {
            host.focus();
        } else {
            tracing::warn!(
                id = self.id,
                control = name,
                "text_edit：focus_text 找不到控件"
            );
        }
    }

    /// 按控件名找编辑框宿主（`set_text` 用）。
    fn text_edit_host(&self, name: &str) -> Option<Rc<TextHostWindow>> {
        self.text_edits
            .borrow()
            .iter()
            .find(|edit| edit.name == name)
            .map(|edit| edit.host.clone())
    }

    /// 把某个编辑框的文本同步到 UI 宿主（脚本 `set_text` 用）。
    fn set_text(&self, name: &str, text: &str) {
        if let Some(host) = self.text_edit_host(name) {
            host.set_text(text);
        } else {
            tracing::warn!(
                id = self.id,
                control = name,
                "text_edit：set_text 找不到控件"
            );
        }
    }

    /// 改一个扁平 SVG 按钮的背景色；只重画，不重建控件。
    fn set_button_background(&self, name: &str, color: Color) -> bool {
        {
            let mut document = self.document.borrow_mut();
            if !document.set_button_background(name, color) {
                tracing::warn!(
                    id = self.id,
                    control = name,
                    "flat_svg_button：set_button_background 名字或类型不匹配"
                );
                return false;
            }
        }
        self.revision.set(self.revision.get() + 1);
        if self.shown.get()
            && let Some(position) = self.position.get()
        {
            self.render_and_present(position);
        }
        true
    }

    /// 改编辑框的选区背景色（RichEdit 宿主里画选区）。
    fn set_selection_background(&self, name: &str, color: Color) -> bool {
        if let Some(host) = self.text_edit_host(name) {
            host.set_selection_background(colorref(color));
            true
        } else {
            tracing::warn!(
                id = self.id,
                control = name,
                "text_edit：set_selection_background 找不到控件"
            );
            false
        }
    }

    /// 更新静态 `label` / `text_block` 的文字；只重画，不重建编辑框。
    fn update_static_content(&self, name: &str, content: StaticTextContent) -> bool {
        {
            let mut document = self.document.borrow_mut();
            if !document.update_static_content(name, content) {
                tracing::warn!(
                    id = self.id,
                    control = name,
                    "静态文本：名字或内容类型不匹配"
                );
                return false;
            }
        }
        self.revision.set(self.revision.get() + 1);
        if self.shown.get()
            && let Some(position) = self.position.get()
        {
            self.render_and_present(position);
        }
        true
    }

    /// 全选 / 取消选择。
    fn select_all(&self, name: &str, select: bool) {
        if let Some(host) = self.text_edit_host(name) {
            host.select_all(select);
        } else {
            tracing::warn!(
                id = self.id,
                control = name,
                "text_edit：select_all 找不到控件"
            );
        }
    }

    /// 把文本里的 `find` 全部替换成 `text`。
    fn replace_text(&self, name: &str, find: &str, text: &str) {
        if let Some(host) = self.text_edit_host(name) {
            host.replace_text(find, text);
        } else {
            tracing::warn!(
                id = self.id,
                control = name,
                "text_edit：replace 找不到控件"
            );
        }
    }

    /// 清空文本。
    fn clear_text(&self, name: &str) {
        if let Some(host) = self.text_edit_host(name) {
            host.set_text("");
        } else {
            tracing::warn!(id = self.id, control = name, "text_edit：clear 找不到控件");
        }
    }

    /// RichEdit 报告 `EN_CHANGE`：把新文本作为 `text_changed` 事件送到工人线程。
    fn text_changed(&self, host_hwnd: HWND) {
        let edits = self.text_edits.borrow();
        let Some(edit) = edits.iter().find(|edit| edit.host.hwnd() == host_hwnd) else {
            return;
        };
        let text = edit.host.text_string();
        if *edit.last_text.borrow() == text {
            return;
        }
        *edit.last_text.borrow_mut() = text.clone();
        self.refresh_text_box_scroll(&edit.path, &edit.host);
        let document = self.document.borrow();
        let Some(element) = document.element_at_path(&edit.path) else {
            return;
        };
        let region_name = match element {
            Element::TextEdit(control) => &control.region_name,
            Element::TextBox(control) => &control.region_name,
            _ => return,
        };
        let handler = self.region_handler(region_name, WindowAction::TextChanged);
        let control_name = document
            .control_name_at_path(&edit.path)
            .map(str::to_owned)
            .unwrap_or_else(|| region_name.clone());
        let region = Some(region_name.clone());
        let element = (edit.path.len() == 1).then_some(edit.path[0] + 1);
        let instance = document.instances.get(&control_name).cloned();
        let control = instance
            .as_ref()
            .map(|instance| instance.local_name.clone())
            .unwrap_or(control_name);
        let (x, y) = self.last.get();
        let (ctrl, alt, shift) = modifiers();
        (self.events)(ScriptWindowEvent {
            window: self.id,
            name: self.name.clone(),
            action: WindowAction::TextChanged,
            x: x as f32,
            y: y as f32,
            element,
            region,
            control: Some(control),
            instance,
            handler,
            text: Some(text),
            key: None,
            modifier: None,
            value: None,
            delta: None,
            ctrl,
            alt,
            shift,
        });
    }

    /// 编辑框按键按下（`key_pressed`）或焦点变化（`focused` / `unfocused`）。
    fn emit_text_event(
        &self,
        host_hwnd: HWND,
        action: WindowAction,
        key: Option<u32>,
        modifier: Option<u8>,
    ) {
        let edits = self.text_edits.borrow();
        let Some(edit) = edits.iter().find(|edit| edit.host.hwnd() == host_hwnd) else {
            return;
        };
        let document = self.document.borrow();
        let Some(element) = document.element_at_path(&edit.path) else {
            return;
        };
        let region_name = match element {
            Element::TextEdit(control) => &control.region_name,
            Element::TextBox(control) => &control.region_name,
            _ => return,
        };
        let handler = self.region_handler(region_name, action);
        let control_name = document
            .control_name_at_path(&edit.path)
            .map(str::to_owned)
            .unwrap_or_else(|| region_name.clone());
        let region = Some(region_name.clone());
        let element = (edit.path.len() == 1).then_some(edit.path[0] + 1);
        let instance = document.instances.get(&control_name).cloned();
        let control = instance
            .as_ref()
            .map(|instance| instance.local_name.clone())
            .unwrap_or(control_name);
        let (x, y) = self.last.get();
        let (ctrl, alt, shift) = modifiers();
        (self.events)(ScriptWindowEvent {
            window: self.id,
            name: self.name.clone(),
            action,
            x: x as f32,
            y: y as f32,
            element,
            region,
            control: Some(control),
            instance,
            handler,
            text: None,
            key,
            modifier,
            value: None,
            delta: None,
            ctrl,
            alt,
            shift,
        });
    }

    fn text_key_pressed(&self, host_hwnd: HWND, key: u32, modifier: u8) {
        self.emit_text_event(
            host_hwnd,
            WindowAction::KeyPressed,
            Some(key),
            Some(modifier),
        );
    }

    fn text_focus_changed(&self, host_hwnd: HWND, focused: bool) {
        self.emit_text_event(
            host_hwnd,
            if focused {
                WindowAction::Focused
            } else {
                WindowAction::Unfocused
            },
            None,
            None,
        );
    }

    /// RichEdit 滚动位置变了：刷新内部滚动条并重画，让滑块跟上。
    fn text_scrolled(&self, host_hwnd: HWND) {
        let edits = self.text_edits.borrow();
        let Some(edit) = edits.iter().find(|edit| edit.host.hwnd() == host_hwnd) else {
            return;
        };
        let path = edit.path.clone();
        let host = edit.host.clone();
        drop(edits);
        if self.refresh_text_box_scroll(&path, &host) && self.shown.get() {
            self.repaint();
        }
    }

    /// 多行编辑框文本区里的滚轮：交给内部滚动条动画，而不是直接跳。
    fn text_wheel(&self, host_hwnd: HWND, delta: i32) {
        let path = {
            let edits = self.text_edits.borrow();
            let Some(edit) = edits.iter().find(|edit| edit.host.hwnd() == host_hwnd) else {
                return;
            };
            edit.path.clone()
        };
        let target = {
            let document = self.document.borrow();
            let Some(Element::TextBox(edit)) = document.element_at_path(&path) else {
                return;
            };
            let axis = if edit.v_scroll.visible {
                ScrollAxis::Vertical
            } else if edit.h_scroll.visible && !edit.auto_line_wrap {
                ScrollAxis::Horizontal
            } else {
                return;
            };
            let scroll = edit.scroll(axis);
            let step = (scroll.page.max(16) / 4).max(8);
            let target = if delta > 0 {
                scroll.value.saturating_sub(step).max(scroll.min)
            } else {
                scroll.value.saturating_add(step).min(scroll.max)
            };
            (axis, target)
        };
        self.animate_text_box_scroll(&path, target.0, target.1);
    }

    /// 命中下标若是扁平按钮，返回它。
    fn button_index(&self, element: Option<usize>) -> Option<usize> {
        // `element_at` / 事件载荷里的 `element` 是 **1 起** 的；`hits` / `document.elements` 是 0 起。
        let index = element?.checked_sub(1)?;
        matches!(
            self.document.borrow().elements.get(index),
            Some(Element::FlatSvgButton(_))
        )
        .then_some(index)
    }

    fn scrollbar_index(&self, element: Option<usize>) -> Option<usize> {
        let index = element?.checked_sub(1)?;
        matches!(
            self.document.borrow().elements.get(index),
            Some(Element::ScrollBar(_))
        )
        .then_some(index)
    }

    fn text_box_index(&self, element: Option<usize>) -> Option<usize> {
        let index = element?.checked_sub(1)?;
        matches!(
            self.document.borrow().elements.get(index),
            Some(Element::TextBox(_))
        )
        .then_some(index)
    }

    /// 命中 `text_box` 的内部滚动条；返回轴向、滚动状态、轨道矩形和控件内坐标。
    fn text_box_bar_at(&self, index: usize, x: f32, y: f32) -> Option<TextBarHit> {
        let document = self.document.borrow();
        let Some(Element::TextBox(edit)) = document.elements.get(index) else {
            return None;
        };
        let local = (x - edit.x, y - edit.y);
        for axis in [ScrollAxis::Horizontal, ScrollAxis::Vertical] {
            let scroll = edit.scroll(axis);
            if !scroll.visible {
                continue;
            }
            let (bx, by, bw, bh) = edit.bar_rect(axis);
            if local.0 >= bx && local.0 <= bx + bw && local.1 >= by && local.1 <= by + bh {
                return Some((axis, scroll, (bx, by, bw, bh), local));
            }
        }
        None
    }

    fn text_box_scroll_value(
        &self,
        index: usize,
        axis: ScrollAxis,
        point: (f32, f32),
        offset: f32,
    ) -> Option<u64> {
        let document = self.document.borrow();
        let Some(Element::TextBox(edit)) = document.elements.get(index) else {
            return None;
        };
        let scroll = edit.scroll(axis);
        if !scroll.visible || scroll.max <= scroll.min {
            return None;
        }
        let (bx, by, bw, bh) = edit.bar_rect(axis);
        let (thumb_x, thumb_y, thumb_w, thumb_h) = edit.thumb_rect(axis);
        let (start, track, thumb) = match axis {
            ScrollAxis::Horizontal => (bx, bw, thumb_w),
            ScrollAxis::Vertical => (by, bh, thumb_h),
        };
        let position = match axis {
            ScrollAxis::Horizontal => point.0 - edit.x,
            ScrollAxis::Vertical => point.1 - edit.y,
        };
        let travel = (track - thumb).max(1.0);
        let ratio = ((position - start - offset) / travel).clamp(0.0, 1.0);
        let _ = (thumb_x, thumb_y);
        Some(scroll.min + (ratio as f64 * (scroll.max - scroll.min) as f64).round() as u64)
    }

    fn text_box_host(&self, path: &[usize]) -> Option<Rc<TextHostWindow>> {
        self.text_edits
            .borrow()
            .iter()
            .find(|edit| edit.path == path)
            .map(|edit| edit.host.clone())
    }

    /// 立即把文本滚动到指定位置（拖拽和动画收尾用）。
    fn set_text_box_scroll(&self, path: &[usize], axis: ScrollAxis, value: u64) {
        self.cancel_text_box_animation(path, axis);
        self.set_text_box_scroll_immediate(path, axis, value);
    }

    fn set_text_box_scroll_immediate(&self, path: &[usize], axis: ScrollAxis, value: u64) {
        let Some(host) = self.text_box_host(path) else {
            return;
        };
        let (h, v) = host.scroll_info();
        let (x, y) = match axis {
            ScrollAxis::Horizontal => (value as i32, v.pos),
            ScrollAxis::Vertical => (h.pos, value as i32),
        };
        host.set_scroll(x, y);
        self.refresh_text_box_scroll(path, &host);
        self.repaint();
    }

    fn text_box_scroll_current(&self, path: &[usize], axis: ScrollAxis) -> Option<i32> {
        let document = self.document.borrow();
        let Some(Element::TextBox(edit)) = document.element_at_path(path) else {
            return None;
        };
        Some(edit.scroll(axis).value.min(i32::MAX as u64) as i32)
    }

    /// 非拖拽改值：和独立滚动条一样用 140ms ease-out 过渡。
    fn animate_text_box_scroll(&self, path: &[usize], axis: ScrollAxis, target: u64) {
        let Some(current) = self.text_box_scroll_current(path, axis) else {
            return;
        };
        let target = target.min(i32::MAX as u64) as i32;
        let key = (path.to_vec(), axis);
        let logical = self
            .text_scroll_anims
            .borrow()
            .get(&key)
            .map(|anim| anim.to)
            .unwrap_or(current);
        if logical == target {
            return;
        }
        if current == target {
            self.cancel_text_box_animation(path, axis);
            self.set_text_box_scroll_immediate(path, axis, target as u64);
            return;
        }
        self.text_scroll_anims.borrow_mut().insert(
            key,
            TextScrollAnimation {
                from: current,
                to: target,
                start: Instant::now(),
            },
        );
        self.ensure_scroll_anim_timer();
    }

    fn cancel_text_box_animation(&self, path: &[usize], axis: ScrollAxis) {
        let key = (path.to_vec(), axis);
        self.text_scroll_anims.borrow_mut().remove(&key);
        if self.scroll_anims.borrow().is_empty() && self.text_scroll_anims.borrow().is_empty() {
            self.stop_scroll_anim_timer();
        }
    }

    fn begin_text_box_press(&self, client_x: i32, client_y: i32) -> bool {
        let (x, y, element, _) = self.hit_at(client_x, client_y);
        let Some(index) = self.text_box_index(element) else {
            return false;
        };
        let Some((axis, scroll, _, local)) = self.text_box_bar_at(index, x, y) else {
            return false;
        };
        let document = self.document.borrow();
        let Some(Element::TextBox(edit)) = document.elements.get(index) else {
            return false;
        };
        let (tx, ty, tw, th) = edit.thumb_rect(axis);
        let in_thumb = local.0 >= tx && local.0 <= tx + tw && local.1 >= ty && local.1 <= ty + th;
        let path = vec![index];
        if in_thumb {
            let offset = match axis {
                ScrollAxis::Horizontal => local.0 - tx,
                ScrollAxis::Vertical => local.1 - ty,
            };
            *self.text_scroll_drag.borrow_mut() = Some(TextScrollDrag { path, axis, offset });
            let _ = unsafe { SetCapture(self.hwnd()) };
        } else {
            // 点轨道：翻一页。
            let page = scroll.page.max(1) as i32;
            let target = match axis {
                ScrollAxis::Horizontal => {
                    if local.0 < tx {
                        scroll.value.saturating_sub(page as u64)
                    } else {
                        scroll.value.saturating_add(page as u64).min(scroll.max)
                    }
                }
                ScrollAxis::Vertical => {
                    if local.1 < ty {
                        scroll.value.saturating_sub(page as u64)
                    } else {
                        scroll.value.saturating_add(page as u64).min(scroll.max)
                    }
                }
            };
            drop(document);
            self.animate_text_box_scroll(&path, axis, target);
        }
        true
    }

    fn update_text_box_drag(&self, client_x: i32, client_y: i32) -> bool {
        let (path, axis, offset) = {
            let drag = self.text_scroll_drag.borrow();
            let Some(drag) = drag.as_ref() else {
                return false;
            };
            (drag.path.clone(), drag.axis, drag.offset)
        };
        let (x, y, _, _) = self.hit_at(client_x, client_y);
        let Some(index) = path.last().copied() else {
            return false;
        };
        let Some(value) = self.text_box_scroll_value(index, axis, (x, y), offset) else {
            return false;
        };
        self.set_text_box_scroll(&path, axis, value);
        true
    }

    fn end_text_box_drag(&self) -> bool {
        let drag = self.text_scroll_drag.borrow_mut().take();
        if drag.is_some() {
            let _ = unsafe { ReleaseCapture() };
        }
        drag.is_some()
    }

    fn scrollbar(&self, index: usize) -> Option<cloudime_render::ScrollBar> {
        match self.document.borrow().elements.get(index) {
            Some(Element::ScrollBar(bar)) => Some(bar.clone()),
            _ => None,
        }
    }

    fn set_scrollbar_value(&self, index: usize, value: u64) -> bool {
        let mut document = self.document.borrow_mut();
        let Some(Element::ScrollBar(bar)) = document.elements.get_mut(index) else {
            return false;
        };
        if bar.value == value {
            return false;
        }
        bar.value = value;
        true
    }

    /// 滚动条当前逻辑目标（动画中是 `to`，否则是显示值）。
    fn scrollbar_target(&self, index: usize) -> Option<u64> {
        if let Some(anim) = self.scroll_anims.borrow().get(&index) {
            return Some(anim.to);
        }
        self.scrollbar(index).map(|bar| bar.value)
    }

    fn ensure_scroll_anim_timer(&self) {
        if self.scroll_anim_timer.get() != 0 {
            return;
        }
        let timer = unsafe { SetTimer(Some(self.hwnd()), 0, SCROLL_ANIM_FRAME_MS, None) };
        self.scroll_anim_timer.set(timer);
    }

    fn stop_scroll_anim_timer(&self) {
        let timer = self.scroll_anim_timer.replace(0);
        if timer != 0 {
            let _ = unsafe { KillTimer(Some(self.hwnd()), timer) };
        }
    }

    fn cancel_scroll_animation(&self, index: usize) {
        let mut anims = self.scroll_anims.borrow_mut();
        anims.remove(&index);
        drop(anims);
        if self.scroll_anims.borrow().is_empty() && self.text_scroll_anims.borrow().is_empty() {
            self.stop_scroll_anim_timer();
        }
    }

    /// 非拖拽改值：从当前显示值平滑过渡到目标，并立刻派一次目标值事件。
    fn animate_scrollbar_value(&self, index: usize, target: u64, x: f32, y: f32) {
        let Some(bar) = self.scrollbar(index) else {
            return;
        };
        let current = bar.value;
        let logical = self
            .scroll_anims
            .borrow()
            .get(&index)
            .map(|anim| anim.to)
            .unwrap_or(current);
        if logical == target {
            return;
        }
        self.emit_scrollbar_value(index, target, x, y);
        if current == target {
            self.cancel_scroll_animation(index);
            return;
        }
        self.scroll_anims.borrow_mut().insert(
            index,
            ScrollAnimation {
                from: current,
                to: target,
                start: Instant::now(),
            },
        );
        self.ensure_scroll_anim_timer();
    }

    fn step_scroll_animations(&self) {
        let now = Instant::now();
        let mut finished = Vec::new();
        let mut changed = false;
        {
            let mut document = self.document.borrow_mut();
            let mut anims = self.scroll_anims.borrow_mut();
            for (index, anim) in anims.iter() {
                let elapsed = now.duration_since(anim.start).as_millis() as u64;
                let progress = (elapsed as f64 / SCROLL_ANIM_MS as f64).min(1.0);
                let eased = 1.0 - (1.0 - progress).powi(3);
                let delta = anim.to as i128 - anim.from as i128;
                let value = (anim.from as i128 + (delta as f64 * eased).round() as i128)
                    .clamp(0, u64::MAX as i128) as u64;
                if let Some(Element::ScrollBar(bar)) = document.elements.get_mut(*index)
                    && bar.value != value
                {
                    bar.value = value;
                    changed = true;
                }
                if progress >= 1.0 {
                    finished.push(*index);
                }
            }
            for index in finished {
                anims.remove(&index);
            }
        }
        let text_changed = self.step_text_scroll_animations(now);
        if changed || text_changed {
            self.repaint();
        }
        if self.scroll_anims.borrow().is_empty() && self.text_scroll_anims.borrow().is_empty() {
            self.stop_scroll_anim_timer();
        }
    }

    /// 推进 `text_box` 内部滚动条动画；返回这一帧是否改了显示值。
    fn step_text_scroll_animations(&self, now: Instant) -> bool {
        let mut finished = Vec::new();
        let mut changed = false;
        {
            let mut anims = self.text_scroll_anims.borrow_mut();
            for ((path, axis), anim) in anims.iter() {
                let elapsed = now.duration_since(anim.start).as_millis() as u64;
                let progress = (elapsed as f64 / SCROLL_ANIM_MS as f64).min(1.0);
                let eased = 1.0 - (1.0 - progress).powi(3);
                let delta = anim.to as i128 - anim.from as i128;
                let value = (anim.from as i128 + (delta as f64 * eased).round() as i128)
                    .clamp(0, i32::MAX as i128) as i32;
                let Some(host) = self.text_box_host(path) else {
                    finished.push((path.clone(), *axis));
                    continue;
                };
                let (h, v) = host.scroll_info();
                let (x, y) = match axis {
                    ScrollAxis::Horizontal => (value, v.pos),
                    ScrollAxis::Vertical => (h.pos, value),
                };
                host.set_scroll(x, y);
                let mut document = self.document.borrow_mut();
                if let Some(Element::TextBox(edit)) = document.element_at_path_mut(path) {
                    let scroll = match axis {
                        ScrollAxis::Horizontal => &mut edit.h_scroll,
                        ScrollAxis::Vertical => &mut edit.v_scroll,
                    };
                    if scroll.value != value as u64 {
                        scroll.value = value as u64;
                        changed = true;
                    }
                }
                if progress >= 1.0 {
                    finished.push((path.clone(), *axis));
                }
            }
            for key in finished {
                anims.remove(&key);
            }
        }
        changed
    }

    fn emit_scrollbar_value(&self, index: usize, value: u64, x: f32, y: f32) {
        let document = self.document.borrow();
        let Some(Element::ScrollBar(bar)) = document.elements.get(index) else {
            return;
        };
        let handler = self.region_handler(&bar.region_name, WindowAction::ValueChanged);
        let control = document
            .control_name(index + 1)
            .map(str::to_owned)
            .unwrap_or_else(|| bar.region_name.clone());
        let instance = document.instances.get(&control).cloned();
        let control = instance
            .as_ref()
            .map(|instance| instance.local_name.clone())
            .unwrap_or(control);
        let region = Some(bar.region_name.clone());
        let (ctrl, alt, shift) = modifiers();
        (self.events)(ScriptWindowEvent {
            window: self.id,
            name: self.name.clone(),
            action: WindowAction::ValueChanged,
            x,
            y,
            element: Some(index + 1),
            region,
            control: Some(control),
            instance,
            handler,
            text: None,
            key: None,
            modifier: None,
            value: Some(value),
            delta: None,
            ctrl,
            alt,
            shift,
        });
    }

    /// 左键按在滚动条上：点滑块开始拖，点轨道先跳过去再拖。返回是否吃掉了这次按下。
    fn begin_scrollbar_press(&self, client_x: i32, client_y: i32) -> bool {
        let (x, y, element, _) = self.hit_at(client_x, client_y);
        let Some(index) = self.scrollbar_index(element) else {
            return false;
        };
        let Some(bar) = self.scrollbar(index) else {
            return false;
        };
        let (tx, ty, tw, th) = bar.thumb_rect();
        let inside_thumb = x >= tx && x <= tx + tw && y >= ty && y <= ty + th;
        if !inside_thumb {
            let before = match bar.axis {
                cloudime_render::ScrollAxis::Horizontal => x < tx,
                cloudime_render::ScrollAxis::Vertical => y < ty,
            };
            let value = if before {
                let current = self.scrollbar_target(index).unwrap_or(bar.value);
                current.saturating_sub(bar.step).max(bar.min)
            } else {
                let current = self.scrollbar_target(index).unwrap_or(bar.value);
                current.saturating_add(bar.step).min(bar.max)
            };
            self.animate_scrollbar_value(index, value, x, y);
            return true;
        }
        self.cancel_scroll_animation(index);
        let offset = match bar.axis {
            cloudime_render::ScrollAxis::Horizontal => x - tx,
            cloudime_render::ScrollAxis::Vertical => y - ty,
        };
        self.scrollbar_drag.set(Some(ScrollDrag { index, offset }));
        let _ = unsafe { SetCapture(self.hwnd()) };
        true
    }

    fn update_scrollbar_drag(&self, client_x: i32, client_y: i32) -> bool {
        let Some(drag) = self.scrollbar_drag.get() else {
            return false;
        };
        let (x, y, _, _) = self.hit_at(client_x, client_y);
        let Some(bar) = self.scrollbar(drag.index) else {
            self.scrollbar_drag.set(None);
            return true;
        };
        self.cancel_scroll_animation(drag.index);
        let value = bar.value_at((x, y), drag.offset);
        if self.set_scrollbar_value(drag.index, value) {
            self.repaint();
            self.emit_scrollbar_value(drag.index, value, x, y);
        }
        true
    }

    fn end_scrollbar_drag(&self) -> bool {
        if self.scrollbar_drag.replace(None).is_some() {
            let _ = unsafe { ReleaseCapture() };
            true
        } else {
            false
        }
    }

    /// 滚轮落在滚动条上：按 ±1 步进值；返回是否吃掉了滚轮。
    fn scrollbar_wheel(&self, client_x: i32, client_y: i32, delta: i32) -> bool {
        let (x, y, element, _) = self.hit_at(client_x, client_y);
        if self.text_box_wheel_at(element, x, y, delta) {
            return true;
        }
        let Some(index) = self.scrollbar_index(element) else {
            return false;
        };
        let Some(bar) = self.scrollbar(index) else {
            return false;
        };
        let value = if delta > 0 {
            let current = self.scrollbar_target(index).unwrap_or(bar.value);
            current.saturating_sub(bar.step).max(bar.min)
        } else {
            let current = self.scrollbar_target(index).unwrap_or(bar.value);
            current.saturating_add(bar.step).min(bar.max)
        };
        self.animate_scrollbar_value(index, value, x, y);
        true
    }

    /// 滚轮落在 `text_box` 上：滚动其内部文本；落在文本区时宿主自己也会处理滚轮。
    fn text_box_wheel_at(&self, element: Option<usize>, x: f32, y: f32, delta: i32) -> bool {
        let Some(index) = self.text_box_index(element) else {
            return false;
        };
        let (path, axis, target) = {
            let document = self.document.borrow();
            let Some(Element::TextBox(edit)) = document.elements.get(index) else {
                return false;
            };
            if x < edit.x || x > edit.x + edit.width || y < edit.y || y > edit.y + edit.height {
                return false;
            }
            let axis = if edit.v_scroll.visible {
                ScrollAxis::Vertical
            } else if edit.h_scroll.visible && !edit.auto_line_wrap {
                ScrollAxis::Horizontal
            } else {
                return false;
            };
            let scroll = edit.scroll(axis);
            let step = (scroll.page.max(16) / 4).max(8);
            let target = if delta > 0 {
                scroll.value.saturating_sub(step).max(scroll.min)
            } else {
                scroll.value.saturating_add(step).min(scroll.max)
            };
            (vec![index], axis, target)
        };
        self.animate_text_box_scroll(&path, axis, target);
        true
    }

    /// 改按钮状态；状态真的变了返回 `true`。
    fn set_button_state(&self, index: usize, state: ButtonState) -> bool {
        let mut document = self.document.borrow_mut();
        let Some(Element::FlatSvgButton(button)) = document.elements.get_mut(index) else {
            return false;
        };
        if button.state == state {
            return false;
        }
        button.state = state;
        true
    }

    /// 按当前窗口位置重画（不换内容）。
    fn repaint(&self) {
        if let Some(position) = self.position.get() {
            // 内容没换、位置也没换，靠 revision 让 `render_and_present` 别走「签名相同」的短路。
            self.revision.set(self.revision.get() + 1);
            self.render_and_present(position);
        }
    }

    /// 鼠标换到另一个元素：旧按钮回 normal、新按钮进 hovered（按下时保持 pressed）。
    fn update_hover_button(&self, hovered: Option<usize>) {
        let hovered = self.button_index(hovered);
        let previous = self.hover_button.replace(hovered);
        if previous == hovered {
            return;
        }
        let pressed = self.pressed_button.get();
        let mut changed = false;
        if let Some(old) = previous
            && Some(old) != pressed
        {
            changed |= self.set_button_state(old, ButtonState::Normal);
        }
        if let Some(new) = hovered {
            let state = if Some(new) == pressed {
                ButtonState::Pressed
            } else {
                ButtonState::Hovered
            };
            changed |= self.set_button_state(new, state);
        }
        if changed {
            self.repaint();
        }
        let (x, y) = self.last.get();
        if let Some(old) = previous {
            self.emit_button(WindowAction::MouseLeave, old, x, y);
        }
        if let Some(new) = hovered {
            self.emit_button(WindowAction::MouseEnter, new, x, y);
        }
    }

    /// 左键在按钮上按下：进入 pressed。
    fn begin_button_press(&self, client_x: i32, client_y: i32) {
        let (_, _, element, _) = self.hit_at(client_x, client_y);
        let Some(index) = self.button_index(element) else {
            return;
        };
        self.pressed_button.set(Some(index));
        if self.set_button_state(index, ButtonState::Pressed) {
            self.repaint();
        }
        self.emit_button(WindowAction::MousePressed, index, client_x, client_y);
    }

    /// 左键抬起：pressed 结束；鼠标还在按钮上就回到 hovered，否则 normal。
    fn end_button_press(&self, client_x: i32, client_y: i32) {
        let Some(index) = self.pressed_button.replace(None) else {
            // 不是按钮上的点击：双击链断开。
            self.last_click.set(None);
            return;
        };
        let clicked = !self.moved_since_press.get();
        let state = if self.hover_button.get() == Some(index) {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        };
        if self.set_button_state(index, state) {
            self.repaint();
        }
        self.emit_button(WindowAction::MouseReleased, index, client_x, client_y);
        if !clicked {
            self.last_click.set(None);
            return;
        }
        let (ctrl, alt, shift) = modifiers();
        if ctrl || alt || shift {
            // 单击 / 双击都禁用修饰键。
            self.last_click.set(None);
            return;
        }
        self.emit_button(WindowAction::MouseClicked, index, client_x, client_y);
        let now = Instant::now();
        let double_ms = unsafe { GetDoubleClickTime() };
        let double_dx = unsafe { GetSystemMetrics(SM_CXDOUBLECLK) }.max(1);
        let double_dy = unsafe { GetSystemMetrics(SM_CYDOUBLECLK) }.max(1);
        let double = self
            .last_click
            .get()
            .is_some_and(|(then, last_index, (last_x, last_y))| {
                last_index == index
                    && now.duration_since(then).as_millis() <= u128::from(double_ms)
                    && (client_x - last_x).abs() <= double_dx
                    && (client_y - last_y).abs() <= double_dy
            });
        if double {
            self.emit_button(WindowAction::MouseDoubleClicked, index, client_x, client_y);
        }
        self.last_click
            .set(Some((now, index, (client_x, client_y))));
    }

    /// 捕获丢失 / 窗口收起：按下态直接退回 normal。
    fn cancel_button_press(&self) {
        let Some(index) = self.pressed_button.replace(None) else {
            return;
        };
        if self.set_button_state(index, ButtonState::Normal) {
            self.repaint();
        }
    }

    /// 收起 / 换内容：所有扁平按钮回 normal。
    fn reset_button_states(&self) {
        self.hover_button.set(None);
        self.pressed_button.set(None);
        self.last_click.set(None);
        let mut document = self.document.borrow_mut();
        for element in &mut document.elements {
            if let Element::FlatSvgButton(button) = element {
                button.state = ButtonState::Normal;
            }
        }
    }

    /// 动态改一个区域的命中范围（合并 / 剪除）：只影响鼠标命中，不重画。
    fn edit_region(&self, name: &str, shape: HitShape, combine: bool) {
        let mut document = self.document.borrow_mut();
        let Some(region) = document
            .regions
            .iter_mut()
            .find(|region| region.name == name)
        else {
            tracing::warn!(id = self.id, region = name, "脚本窗口：改区域时找不到它");
            return;
        };
        if combine {
            region.combine(shape);
        } else {
            region.prune(shape);
        }
        // 命中用的是 `self.regions`，跟着内容里的那份走
        *self.regions.borrow_mut() = document.regions.clone();
    }

    /// 显示：`position` 是内容区左上角的屏幕坐标（点）；`None` = 上次的位置 / 光标旁。
    fn show(&self, position: Option<(f32, f32)>) {
        let (position, dpi) = match position {
            Some((x, y)) => resolve_point(x, y, self.dpi.get()),
            None => match self.position.get() {
                // 上次的位置是物理像素：直接问那个显示器现在多少 DPI（用户可能把显示器挪了）
                Some(position) => (
                    position,
                    monitor::dpi_near(POINT {
                        x: position.0,
                        y: position.1,
                    })
                    .unwrap_or(self.dpi.get())
                    .max(1),
                ),
                None => default_position(),
            },
        };
        self.dpi.set(dpi.max(1));
        self.render_and_present(position);
        let first = !self.shown.replace(true);
        self.sync_text_edits();
        if first {
            tracing::info!(id = self.id, name = %self.name, x = position.0, y = position.1, dpi,
                "脚本独立窗口已显示");
        }
        self.sync_assets();
    }

    /// 内容换了 / 刚显示：按 `self.dpi` 出图并贴上。
    fn render_and_present(&self, position: (i32, i32)) {
        let dpi = self.dpi.get().max(1);
        let rendered = {
            let mut painter = self.painter.borrow_mut();
            let Some(painter) = painter.as_mut() else {
                tracing::warn!(id = self.id, "脚本窗口：渲染器没起来，不显示");
                self.hide();
                return;
            };
            let generation = painter.generation();
            let Some(rendered) = painter.render_document(
                &self.document.borrow(),
                dpi,
                self.spec.shadow,
                self.spec.background,
            ) else {
                tracing::warn!(id = self.id, "脚本窗口：画不出来，不显示");
                self.hide();
                return;
            };
            (rendered, generation)
        };
        let (rendered, generation) = rendered;
        let content = (
            rendered.rendered.content_width as i32,
            rendered.rendered.content_height as i32,
        );
        let signature = (
            position,
            content,
            self.revision.get(),
            dpi as u64,
            generation,
        );
        if self.painted.get() == Some(signature) && self.shown.get() {
            return;
        }
        let margin = rendered.rendered.content_x as i32;
        if let Err(error) = layered::present(
            self.hwnd(),
            &rendered.rendered.pixmap,
            (position.0 - margin, position.1 - margin),
            255,
        ) {
            tracing::warn!(%error, id = self.id, "脚本窗口：贴图失败，不显示");
            self.hide();
            return;
        }
        *self.hits.borrow_mut() = rendered.hits;
        self.scale.set(rendered.rendered.scale.max(0.01));
        self.margin.set(margin);
        self.content_size.set(content);
        self.position.set(Some(position));
        self.painted.set(Some(signature));
        let _ = unsafe { ShowWindow(self.hwnd(), SW_SHOWNA) };
        self.sync_text_edits();
    }

    /// 拖到另一块显示器之后：DPI 变了就按新 DPI 重画（点 → 像素的换算跟着换）。
    fn refresh_after_move(&self) {
        let Some(position) = self.position.get() else {
            return;
        };
        let dpi = monitor::dpi_near(POINT {
            x: position.0,
            y: position.1,
        })
        .unwrap_or(self.dpi.get())
        .max(1);
        if dpi == self.dpi.get() {
            return;
        }
        self.dpi.set(dpi);
        self.render_and_present(position);
        self.sync_assets();
    }

    fn hide(&self) {
        self.painted.set(None);
        self.hits.borrow_mut().clear();
        self.hover.set(None);
        self.scrollbar_drag.set(None);
        *self.text_scroll_drag.borrow_mut() = None;
        self.scroll_anims.borrow_mut().clear();
        self.text_scroll_anims.borrow_mut().clear();
        self.stop_scroll_anim_timer();
        self.reset_button_states();
        for edit in self.text_edits.borrow().iter() {
            edit.host.show(false);
        }
        if self.shown.replace(false) {
            let _ = unsafe { ShowWindow(self.hwnd(), SW_HIDE) };
            tracing::info!(id = self.id, name = %self.name, "脚本独立窗口已收起");
        }
        // 收起时键盘焦点也放掉（焦点不留在看不见的窗口上）
        self.notify_blur();
        // 收起时焦点先回到自己身上（§4 规则 7：焦点不留在看不见的资产上），再收起名下的资产
        self.template.borrow().return_focus_to_owner();
        self.sync_assets();
    }

    fn add_asset(&self, asset: Box<dyn Asset>) {
        self.template.borrow_mut().assets_mut().add(asset);
    }

    fn retire_asset(&self, name: &str) {
        self.template.borrow_mut().assets_mut().retire_named(name);
        // 焦点可能刚回到自己身上（关掉的那件正拿着焦点）：鼠标开关跟着对齐
        self.template.borrow().focusing();
        self.sync_assets();
    }

    /// 焦点在叫 `name` 的那件资产身上就交还给自己（脚本收起它时先走这一下）。
    fn release_focus(&self, name: &str) {
        let template = self.template.borrow();
        if template.assets().focused_name_is(name) {
            template.return_focus_to_owner();
        }
    }

    fn sync_assets(&self) {
        self.template.borrow().assets().sync(self);
    }

    /// 客户区坐标 → 内容区坐标（点）+ 命中的元素（1 起）。
    /// 客户区坐标 → 内容坐标 + 命中的元素下标 + 区域下标（都是 0 起）。
    fn hit_at(&self, client_x: i32, client_y: i32) -> (f32, f32, Option<usize>, Option<usize>) {
        let (x, y) = content_point(client_x, client_y, self.margin.get(), self.scale.get());
        let element = element_at(&self.hits.borrow(), x, y);
        let region = crate::region::hit(&self.regions.borrow(), x, y);
        (x, y, element, region)
    }

    /// 命中的区域名 + 这个事件声明的处理器名。
    fn region_details(
        &self,
        index: Option<usize>,
        action: WindowAction,
    ) -> (Option<String>, Option<String>) {
        let Some(index) = index else {
            return (None, None);
        };
        let regions = self.regions.borrow();
        let Some(region) = regions.get(index) else {
            return (None, None);
        };
        (
            Some(region.name.clone()),
            region.handler(action.as_str()).map(str::to_owned),
        )
    }

    /// 按区域名找处理器名（扁平按钮事件不经过当前命中区域，单独查一次）。
    fn region_handler(&self, name: &str, action: WindowAction) -> Option<String> {
        self.regions
            .borrow()
            .iter()
            .find(|region| region.name == name)
            .and_then(|region| region.handler(action.as_str()))
            .map(str::to_owned)
    }

    /// 命中的根元素对应的布局控件名。
    fn control_name(&self, element: Option<usize>) -> Option<String> {
        element.and_then(|element| {
            self.document
                .borrow()
                .control_name(element)
                .map(str::to_owned)
        })
    }

    /// 区域名对应的控件名（嵌套在面板里的动态控件没有根元素命中，只能按区域名反查）。
    fn control_name_for_region(&self, region: &str) -> Option<String> {
        self.document
            .borrow()
            .control_names
            .iter()
            .find(|control| control.name == region)
            .map(|control| control.name.clone())
    }

    /// 区域下标 → 区域名。
    fn region_name(&self, index: Option<usize>) -> Option<String> {
        index.and_then(|index| {
            self.regions
                .borrow()
                .get(index)
                .map(|region| region.name.clone())
        })
    }

    fn emit(&self, action: WindowAction, client_x: i32, client_y: i32) {
        self.emit_with_delta(action, client_x, client_y, None);
    }

    fn emit_with_delta(
        &self,
        action: WindowAction,
        client_x: i32,
        client_y: i32,
        delta: Option<i32>,
    ) {
        let (x, y, element, region) = self.hit_at(client_x, client_y);
        let (region, handler) = self.region_details(region, action);
        let control = self.control_name(element).or_else(|| {
            region
                .as_deref()
                .and_then(|name| self.control_name_for_region(name))
        });
        let instance = control
            .as_deref()
            .and_then(|name| self.document.borrow().instances.get(name).cloned());
        let control = instance
            .as_ref()
            .map(|instance| instance.local_name.clone())
            .or(control);
        let (ctrl, alt, shift) = modifiers();
        (self.events)(ScriptWindowEvent {
            window: self.id,
            name: self.name.clone(),
            action,
            x,
            y,
            element,
            region,
            control,
            instance,
            handler,
            text: None,
            key: None,
            modifier: None,
            value: None,
            delta,
            ctrl,
            alt,
            shift,
        });
    }

    fn emit_move(&self, client_x: i32, client_y: i32) {
        self.last.set((client_x, client_y));
        let (_, _, element, region) = self.hit_at(client_x, client_y);
        self.update_hover_button(element);
        let key = (element, region);
        if self.hover.replace(Some(key)) == Some(key) {
            return;
        }
        if element.is_some() || region.is_some() {
            self.emit(WindowAction::Hover, client_x, client_y);
        }
    }

    fn emit_leave(&self) {
        self.hover.set(None);
        self.update_hover_button(None);
        let (x, y) = self.last.get();
        self.emit(WindowAction::Leave, x, y);
    }

    /// 某个扁平按钮的自动区域名（事件载荷用）。
    fn button_region(&self, index: usize) -> Option<String> {
        match self.document.borrow().elements.get(index) {
            Some(Element::FlatSvgButton(button)) => Some(button.region_name.clone()),
            _ => None,
        }
    }

    /// 给某个扁平按钮发事件（`element` / `region` 固定成它，不被当前命中覆盖）。
    fn emit_button(&self, action: WindowAction, index: usize, client_x: i32, client_y: i32) {
        let Some(region) = self.button_region(index) else {
            return;
        };
        let handler = self.region_handler(&region, action);
        let control = self.control_name(Some(index + 1));
        let instance = control
            .as_deref()
            .and_then(|name| self.document.borrow().instances.get(name).cloned());
        let control = instance
            .as_ref()
            .map(|instance| instance.local_name.clone())
            .or(control);
        let (x, y, _, _) = self.hit_at(client_x, client_y);
        let (ctrl, alt, shift) = modifiers();
        (self.events)(ScriptWindowEvent {
            window: self.id,
            name: self.name.clone(),
            action,
            x,
            y,
            element: Some(index + 1),
            region: Some(region),
            control,
            instance,
            handler,
            text: None,
            key: None,
            modifier: None,
            value: None,
            delta: None,
            ctrl,
            alt,
            shift,
        });
    }

    /// 这块窗口拿到了键盘焦点（点它 / `win:focus()`）：告诉工人线程，`key` 载荷据此带上 `window`。
    fn notify_focus(&self) {
        let (x, y) = self.last.get();
        self.emit(WindowAction::Focus, x, y);
    }

    /// 键盘焦点离开（点了别的窗口 / 收起 / 销毁 / `win:focus(false)`）。
    fn notify_blur(&self) {
        let (x, y) = self.last.get();
        self.emit(WindowAction::Blur, x, y);
    }

    /// 左键按下：记住起点，等移动超过阈值再算「拖」。
    ///
    /// 按在 `drag = false` 的区域上不拖窗口（点击照常）—— 留给以后区域自己处理拖动（滑块之类）。
    fn begin_drag(&self, client_x: i32, client_y: i32) {
        // 每次按下都从「这一按还没拖动」起算。上一次拖拽结束时如果走的是
        // 「`drag = false` 区域」（比如关闭按钮）这条提前返回的路，`end_drag` 不会被调到，
        // 不重置就会把下一次正常点击也当成拖拽（表现：悬停有反应，点击没反应）。
        self.moved_since_press.set(false);
        let (_, _, _, region) = self.hit_at(client_x, client_y);
        if let Some(index) = region
            && !self
                .regions
                .borrow()
                .get(index)
                .is_some_and(|region| region.draggable)
        {
            self.notify_focus();
            return;
        }
        let mut point = POINT::default();
        let _ = unsafe { GetCursorPos(&mut point) };
        let (x, y) = self.position.get().unwrap_or((0, 0));
        let margin = self.margin.get();
        self.drag.set(Some(Drag {
            start_cursor: (point.x, point.y),
            start_window: (x - margin, y - margin),
        }));
        let _ = unsafe { SetCapture(self.hwnd()) };
        // 点一下就算拿到键盘焦点（脚本可以据此处理按键）
        self.notify_focus();
    }

    /// 左键按着移动：超过阈值就把窗口挪过去（名下资产跟着重新摆）。
    fn drag_to(&self) {
        let Some(drag) = self.drag.get() else {
            return;
        };
        let mut point = POINT::default();
        let _ = unsafe { GetCursorPos(&mut point) };
        let dx = point.x - drag.start_cursor.0;
        let dy = point.y - drag.start_cursor.1;
        if !self.moved_since_press.get() && dx.abs().max(dy.abs()) < DRAG_THRESHOLD {
            return;
        }
        self.moved_since_press.set(true);
        let (x, y) = (drag.start_window.0 + dx, drag.start_window.1 + dy);
        let _ = unsafe {
            SetWindowPos(
                self.hwnd(),
                None,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        let margin = self.margin.get();
        self.position.set(Some((x + margin, y + margin)));
        self.sync_text_edits();
        self.sync_assets();
    }

    /// 左键抬起：返回「刚才是不是拖过」（拖过就不算点击）。
    ///
    /// 系统有时会在 `WM_LBUTTONUP` **之前**就撤掉捕获并送来 `WM_CAPTURECHANGED`（那时 `drag` 已经清了），
    /// 所以「拖过没有」记在 [`Self::moved_since_press`] 里，不跟着 `drag` 走。
    fn end_drag(&self) -> bool {
        self.drag.set(None);
        let _ = unsafe { ReleaseCapture() };
        let moved = self.moved_since_press.get();
        // 下一次按下会重新起算；这里顺手清掉，别让退出路径不同的那次拖拽漏到后面的点击上。
        self.moved_since_press.set(false);
        moved
    }
}

impl Bridge for ScriptWindow {
    fn host(&self) -> Host {
        let rect = self.position.get().map(|(x, y)| {
            let (width, height) = self.content_size.get();
            RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
            }
        });
        Host {
            rect: rect.filter(|_| self.shown.get()),
            layout: LayoutMode::Vertical,
            expanded: false,
        }
    }

    fn asset_on(&self) -> bool {
        // 脚本辅助窗口各管各的开关（`shown` 在 `ScriptAuxiliary` 里），这里不按整组过滤。
        true
    }

    fn designate_focus(&self, name: &str) -> bool {
        self.template.borrow().designate_focus(name)
    }

    fn return_focus_to_owner(&self) {
        self.template.borrow().return_focus_to_owner();
    }
}

impl Drop for ScriptWindow {
    fn drop(&mut self) {
        self.destroy_text_edits();
        self.stop_scroll_anim_timer();
        REGISTRY.with(|map| {
            map.borrow_mut().remove(&(self.hwnd().0 as isize));
        });
    }
}

/// 脚本独立窗口也是「所有者」：能把焦点指定给名下声明过 `interactive` 的辅助窗口，或者收回来。
impl DesignatedAssetToGetFocus for ScriptWindow {
    fn designate_focus(&self, name: &str) -> bool {
        self.template.borrow().designate_focus(name)
    }

    fn return_focus_to_owner(&self) -> bool {
        self.template.borrow().return_focus_to_owner()
    }

    fn focus(&self) -> Focus {
        self.template.borrow().focus()
    }

    fn focusing(&self) {
        self.template.borrow().focusing();
    }

    fn unfocusing(&self) {
        self.template.borrow().unfocusing();
    }
}

/// 一件脚本辅助窗口（宿主名下的一件资产）。
pub(super) struct ScriptAuxiliary {
    id: WindowId,
    name: String,
    owner: WindowId,
    spec: AuxiliarySpec,
    template: Rc<AuxiliaryWindowTemplate>,

    /// 脚本让它显示着没有（`aux:show()` / `aux:hide()`）。
    shown: Cell<bool>,
}

impl ScriptAuxiliary {
    fn new(
        id: WindowId,
        spec: AuxiliarySpec,
        painter: SharedPainter,
        events: ScriptWindowEvents,
    ) -> Result<Self> {
        let name = format!("script-{id}");
        let template = Rc::new(AuxiliaryWindowTemplate::new(painter, spec.interactive)?);
        template.attach();
        template.set_shadow(spec.shadow);
        if let Some(gap) = spec.gap {
            // 间隙按点给；具体像素在 `sync` 里按宿主所在显示器的 DPI 算（这里先记一个缺省）
            let dpi = unsafe { GetDpiForSystem() }.max(96);
            template.set_spec(AuxiliaryWindowSpec {
                width: None,
                height: None,
                gap: Some(points(gap, dpi)),
            });
        }
        let id_for_events = id;
        let name_for_events = spec.name.clone();
        let events_for_mouse = events.clone();
        template.set_events(Box::new(move |panel| {
            events_for_mouse(ScriptWindowEvent {
                window: id_for_events,
                name: name_for_events.clone(),
                action: match panel.action {
                    PanelAction::Click => WindowAction::Click,
                    PanelAction::RightClick => WindowAction::RightClick,
                    PanelAction::Hover => WindowAction::Hover,
                    PanelAction::Leave => WindowAction::Leave,
                    PanelAction::Wheel => WindowAction::Wheel,
                },
                x: panel.x,
                y: panel.y,
                element: panel.element,
                region: panel.region.clone(),
                control: panel.control.clone(),
                instance: None,
                handler: panel.handler.clone(),
                text: None,
                key: None,
                modifier: None,
                value: None,
                delta: panel.delta,
                ctrl: false,
                alt: false,
                shift: false,
            });
        }));
        Ok(Self {
            id,
            name,
            owner: spec.owner,
            spec,
            template,
            shown: Cell::new(false),
        })
    }

    fn set_content(&self, document: Document) {
        self.template.set_document(document);
    }

    fn update_static_content(&self, name: &str, content: StaticTextContent) -> bool {
        self.template.update_static_content(name, content)
    }

    fn set_button_background(&self, name: &str, color: Color) -> bool {
        self.template.set_button_background(name, color)
    }

    /// 动态改一个区域的命中范围（合并 / 剪除）。
    fn edit_region(&self, name: &str, shape: cloudime_render::HitShape, combine: bool) {
        self.template.edit_region(name, shape, combine);
    }

    fn show(&self) {
        if !self.shown.replace(true) {
            tracing::info!(id = self.id, name = %self.name, owner = self.owner, "脚本辅助窗口已显示");
        }
    }

    fn hide(&self) {
        if self.shown.replace(false) {
            self.template.hide();
            tracing::info!(id = self.id, name = %self.name, "脚本辅助窗口已收起");
        }
    }

    /// 焦点指定给它了：接鼠标（模板自己知道有没有声明 `interactive`）。
    fn set_focused(&self, focused: bool) {
        self.template.set_focused(focused);
    }

    fn sync(&self, owner: &dyn Bridge) {
        if !self.shown.get() {
            self.template.hide();
            return;
        }
        let host = owner.host();
        let Some(rect) = host.rect else {
            self.template.hide();
            return;
        };
        // 间隙（点）按宿主所在显示器的 DPI 折成像素
        if let Some(gap) = self.spec.gap {
            let dpi = monitor::dpi_near(POINT {
                x: rect.left,
                y: rect.top,
            })
            .unwrap_or(96)
            .max(1);
            self.template.set_spec(AuxiliaryWindowSpec {
                width: None,
                height: None,
                gap: Some(points(gap, dpi)),
            });
        }
        self.template.show(rect, host.layout);
    }
}

/// 把 [`ScriptAuxiliary`] 放进宿主的资产表：表里持有它，管理器也持有它（同一个 `Rc`）。
struct AssetHandle(Rc<ScriptAuxiliary>);

impl Asset for AssetHandle {
    fn name(&self) -> &str {
        &self.0.name
    }

    fn sync(&self, owner: &dyn Bridge) {
        self.0.sync(owner);
    }

    fn hide(&self) {
        self.0.hide();
    }

    fn able_to_get_focus(&self) -> bool {
        self.0.template.focusable()
    }

    fn focusing(&self) {
        self.0.set_focused(true);
    }

    fn unfocusing(&self) {
        self.0.set_focused(false);
    }
}

/// 命中测试：从**最后画的**（最上面）往下找，第一个装下这个点的元素就是它。
fn element_at(hits: &[Option<HitShape>], x: f32, y: f32) -> Option<usize> {
    hits.iter().enumerate().rev().find_map(|(index, hit)| {
        hit.as_ref()
            .filter(|hit| hit.contains(x, y))
            .map(|_| index + 1)
    })
}

/// 客户区坐标 → 内容区坐标（点）。
///
/// 窗口左上角 = **内容左上角 − 阴影留白**；客户区 0 点对应位图 0 点，所以内容像素 = 客户区 − 留白
/// （别加反了：加反会差出两倍留白，真机上表现为「点哪儿都偏一截」）。
fn content_point(client_x: i32, client_y: i32, margin: i32, scale: f32) -> (f32, f32) {
    let scale = scale.max(0.01);
    (
        (client_x - margin) as f32 / scale,
        (client_y - margin) as f32 / scale,
    )
}

/// 点 → 物理像素。
fn points_to_pixels(x: f32, y: f32, dpi: u32) -> (i32, i32) {
    let scale = dpi.max(1) as f32 / 96.0;
    ((x * scale).round() as i32, (y * scale).round() as i32)
}

/// 一个长度：点 → 物理像素。
fn points(value: f32, dpi: u32) -> i32 {
    (value * dpi.max(1) as f32 / 96.0).round() as i32
}

/// 渲染器的 `Color` → Win32 `COLORREF`（0x00BBGGRR）。
fn scroll_state(info: ScrollInfo) -> TextBoxScroll {
    let min = info.min.max(0) as u64;
    let page = info.page.max(0) as u64;
    // RichEdit 的 `TxGetVScroll` / `TxGetHScroll` 返回的 `max` 是内容总高度 / 宽度，
    // 不是可滚到的最大位置；真正能滚到的最后位置要再减掉一页。
    // 不减这一页会让滑块只走到约 95%，并且拖动到底时位置被 RichEdit 夹回。
    let extent = info.max.max(info.min) as u64;
    let max = extent.saturating_sub(page).max(min);
    let value = (info.pos.max(info.min) as u64).clamp(min, max);
    let visible = max > min;
    TextBoxScroll {
        min,
        max,
        value,
        page,
        visible,
    }
}

fn colorref(color: cloudime_render::Color) -> windows::Win32::Foundation::COLORREF {
    windows::Win32::Foundation::COLORREF(
        color.r as u32 | ((color.g as u32) << 8) | ((color.b as u32) << 16),
    )
}

/// 系统 DPI（拿不到就按 100%）。
fn system_dpi() -> u32 {
    unsafe { GetDpiForSystem() }.max(96)
}

/// 脚本给的屏幕坐标（点）→ 物理像素 + 那块显示器的 DPI。
///
/// 混合 DPI 下「点」没有全局意义：先按当前 DPI 猜一次落点，问那个显示器多少 DPI，再按它重算；
/// 落点跨到另一块显示器时最多再迭代两次（两块不同 DPI 的屏交界处）。
fn resolve_point(x: f32, y: f32, initial_dpi: u32) -> ((i32, i32), u32) {
    let mut dpi = initial_dpi.max(1);
    let mut position = points_to_pixels(x, y, dpi);
    for _ in 0..2 {
        let Some(next) = monitor::dpi_near(POINT {
            x: position.0,
            y: position.1,
        }) else {
            break;
        };
        let next = next.max(1);
        if next == dpi {
            break;
        }
        dpi = next;
        position = points_to_pixels(x, y, dpi);
    }
    (position, dpi)
}

/// `show()` 没给位置、也没有上次的位置：摆在光标右下 16 点（按光标所在显示器的 DPI）。
fn default_position() -> ((i32, i32), u32) {
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_ok() {
        let dpi = monitor::dpi_near(point).unwrap_or_else(system_dpi).max(1);
        let offset = (16 * dpi as i32) / 96;
        ((point.x + offset, point.y + offset), dpi)
    } else {
        ((100, 100), system_dpi())
    }
}

/// 客户区坐标：`lparam` 低 16 位是 x、高 16 位是 y（带符号）。
/// 当前键盘修饰键状态（鼠标事件载荷用）。
fn modifiers() -> (bool, bool, bool) {
    let pressed = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe {
        GetKeyState(key.0 as i32) < 0
    };
    (pressed(VK_CONTROL), pressed(VK_MENU), pressed(VK_SHIFT))
}

fn client_x(lparam: LPARAM) -> i32 {
    (lparam.0 & 0xFFFF) as u16 as i16 as i32
}

fn client_y(lparam: LPARAM) -> i32 {
    ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32
}

/// 让窗口在鼠标离开时收到一条 `WM_MOUSELEAVE`。
fn track_leave(hwnd: HWND) {
    let mut event = TRACKMOUSEEVENT {
        cbSize: core::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    let _ = unsafe { TrackMouseEvent(&mut event) };
}

/// 按 HWND 找回本线程的脚本窗口。
fn with_window(hwnd: HWND, act: impl FnOnce(&ScriptWindow)) {
    let window = REGISTRY.with(|map| map.borrow().get(&(hwnd.0 as isize)).and_then(Weak::upgrade));
    if let Some(window) = window {
        act(&window);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // 不接鼠标的脚本窗口带 `WS_EX_TRANSPARENT`：命中测试照那个位回答（与模板同一个来源）。
        WM_NCHITTEST => {
            let transparent = mouse_transparent(hwnd);
            LRESULT(if transparent {
                HTTRANSPARENT as isize
            } else {
                HTCLIENT as isize
            })
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // 左键按下：开始记拖动起点；同时算这块窗口拿到键盘焦点。
        WM_LBUTTONDOWN => {
            with_window(hwnd, |window| {
                if window.begin_scrollbar_press(client_x(lparam), client_y(lparam)) {
                    return;
                }
                if window.begin_text_box_press(client_x(lparam), client_y(lparam)) {
                    return;
                }
                let (_, _, element, region) = window.hit_at(client_x(lparam), client_y(lparam));
                let region_name = window.region_name(region);
                window.focus_text_edit(element, region_name);
                window.begin_button_press(client_x(lparam), client_y(lparam));
                window.begin_drag(client_x(lparam), client_y(lparam));
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            track_leave(hwnd);
            with_window(hwnd, |window| {
                if window.update_scrollbar_drag(client_x(lparam), client_y(lparam)) {
                    return;
                }
                if window.update_text_box_drag(client_x(lparam), client_y(lparam)) {
                    return;
                }
                if window.drag.get().is_some() {
                    window.drag_to();
                } else {
                    window.emit_move(client_x(lparam), client_y(lparam));
                }
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_window(hwnd, ScriptWindow::emit_leave);
            LRESULT(0)
        }
        WM_TIMER => {
            let active = Cell::new(false);
            with_window(hwnd, |window| {
                if window.scroll_anim_timer.get() != 0 {
                    active.set(true);
                    window.step_scroll_animations();
                }
            });
            if active.get() {
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_TEXT_CHANGED => {
            let host = HWND(wparam.0 as *mut _);
            with_window(hwnd, |window| window.text_changed(host));
            LRESULT(0)
        }
        WM_TEXT_KEY => {
            let host = HWND(wparam.0 as *mut _);
            let key = (lparam.0 & 0xFFFF) as u32;
            let modifier = ((lparam.0 >> 16) & 0xFF) as u8;
            with_window(hwnd, |window| window.text_key_pressed(host, key, modifier));
            LRESULT(0)
        }
        WM_TEXT_FOCUS => {
            let host = HWND(wparam.0 as *mut _);
            with_window(hwnd, |window| {
                window.text_focus_changed(host, lparam.0 != 0)
            });
            LRESULT(0)
        }
        WM_TEXT_SCROLL => {
            let host = HWND(wparam.0 as *mut _);
            with_window(hwnd, |window| window.text_scrolled(host));
            LRESULT(0)
        }
        WM_TEXT_WHEEL => {
            let host = HWND(wparam.0 as *mut _);
            let delta = lparam.0 as i32;
            with_window(hwnd, |window| window.text_wheel(host, delta));
            LRESULT(0)
        }
        // 拖过就不是点击（阈值在 `drag_to` 里判）。
        WM_LBUTTONUP => {
            with_window(hwnd, |window| {
                if window.end_scrollbar_drag() {
                    return;
                }
                if window.end_text_box_drag() {
                    return;
                }
                window.end_button_press(client_x(lparam), client_y(lparam));
                if window.end_drag() {
                    // 拖到另一块显示器：DPI 变了就按新 DPI 重画
                    window.refresh_after_move();
                } else {
                    window.emit(WindowAction::Click, client_x(lparam), client_y(lparam));
                }
            });
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            with_window(hwnd, |window| {
                window.emit(WindowAction::RightClick, client_x(lparam), client_y(lparam));
            });
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            with_window(hwnd, |window| {
                window.cancel_button_press();
                window.drag.set(None);
                window.scrollbar_drag.set(None);
                *window.text_scroll_drag.borrow_mut() = None;
            });
            LRESULT(0)
        }
        // 滚轮的坐标在 `lparam` 里是屏幕坐标，先换算成客户区；`delta` 在 `wparam` 高 16 位。
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            let mut point = POINT {
                x: client_x(lparam),
                y: client_y(lparam),
            };
            let _ = unsafe { ScreenToClient(hwnd, &mut point) };
            with_window(hwnd, |window| {
                if window.scrollbar_wheel(point.x, point.y, delta) {
                    return;
                }
                window.emit_with_delta(WindowAction::Wheel, point.x, point.y, Some(delta));
            });
            LRESULT(0)
        }
        _ => unsafe {
            windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloudime_render::HitRect;

    /// 命中测试从最上面那个元素往下找：后画的盖住先画的。
    #[test]
    fn the_topmost_element_wins_the_hit_test() {
        let hits = vec![
            Some(HitShape::Rect(HitRect::new(0.0, 0.0, 100.0, 100.0))),
            Some(HitShape::Rect(HitRect::new(10.0, 10.0, 20.0, 20.0))),
            None,
        ];
        assert_eq!(element_at(&hits, 50.0, 50.0), Some(1));
        assert_eq!(element_at(&hits, 15.0, 15.0), Some(2), "后画的在上面");
        assert_eq!(element_at(&hits, 200.0, 200.0), None);
    }

    /// 点 → 像素按 DPI 折：100% 一点一像素，200% 翻倍。
    #[test]
    fn points_scale_with_the_dpi() {
        assert_eq!(points_to_pixels(10.0, 20.0, 96), (10, 20));
        assert_eq!(points_to_pixels(10.0, 20.0, 192), (20, 40));
        assert_eq!(points_to_pixels(10.5, 20.5, 96), (11, 21));
    }

    /// 客户区 → 内容区要**减掉**阴影留白：客户区 `(留白, 留白)` 才是内容 `(0, 0)`。
    #[test]
    fn client_coordinates_subtract_the_shadow_margin() {
        assert_eq!(content_point(30, 30, 30, 1.0), (0.0, 0.0));
        assert_eq!(content_point(90, 100, 30, 1.0), (60.0, 70.0));
        assert_eq!(
            content_point(60, 60, 30, 2.0),
            (15.0, 15.0),
            "高 DPI 再除倍数"
        );
    }

    /// RichEdit 的 max 是内容总高，不是可滚到的最大位置；要减一页，否则滑块到不了底。
    #[test]
    fn scroll_state_subtracts_the_page_from_the_content_extent() {
        let info = ScrollInfo {
            min: 0,
            max: 2310,
            pos: 2194,
            page: 116,
        };
        let state = scroll_state(info);
        assert_eq!(state.max, 2194, "110 行 × 21 像素，减掉 116 像素视口");
        assert_eq!(state.value, 2194);
        assert_eq!(state.page, 116);
        assert!(state.visible);
    }

    /// 位置超出可滚范围时夹到最大位置；内容不足一页时不显示滚动条。
    #[test]
    fn scroll_state_clamps_and_hides_when_content_fits() {
        let state = scroll_state(ScrollInfo {
            min: 0,
            max: 2310,
            pos: 2300,
            page: 116,
        });
        assert_eq!(state.max, 2194);
        assert_eq!(state.value, 2194);

        let state = scroll_state(ScrollInfo {
            min: 0,
            max: 100,
            pos: 0,
            page: 150,
        });
        assert_eq!(state.max, 0);
        assert_eq!(state.value, 0);
        assert!(!state.visible);
    }
}
