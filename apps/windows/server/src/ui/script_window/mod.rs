//! 脚本窗口：Lua 用 `cloudime.window.create` / `create_auxiliary` 建出来的两类窗口。
//!
//! 规矩与别的窗口一样（`docs/design/windows.md`）：
//!
//! - **独立窗口**：自己一块分层窗、自己的生命周期（脚本 `close` 或进程退出时销毁）；
//! - **辅助窗口**：挂在某块**脚本独立窗口**名下的一件**资产**，走 [`AuxiliaryWindowTemplate`]，
//!   贴宿主摆位、默认不接鼠标；
//! - 内容都是脚本给的声明式 [`Document`]（点坐标），这里经 [`Painter::render_document`] 出图、
//!   [`layered::present`] 整块贴上 —— 与候选窗同一个渲染器、同一层阴影。
//!
//! **线程**：本模块全部跑在 UI 线程上；脚本在工人线程产出 [`WindowRequest`]，经命令通道过来。
//! 鼠标动作反过来经 [`ScriptWindowEvents`]（`Send` 闭包）回到工人线程，再派给脚本的 `window` 事件。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, HTCLIENT, HTTRANSPARENT, IDC_ARROW, LoadCursorW, MA_NOACTIVATE, SW_HIDE,
    SW_SHOWNA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, ShowWindow,
    WM_CAPTURECHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCHITTEST, WM_RBUTTONUP,
};
use windows::core::{PCWSTR, Result, w};

use cloudime_platform::LayoutMode;
use cloudime_render::{Document, HitShape};
use cloudime_script::{
    AuxiliarySpec, ScriptWindowEvent, WindowAction, WindowId, WindowRequest, WindowSpec,
};

use super::auxiliary::{Asset, Bridge, DesignatedAssetToGetFocus, Focus, Host};
use super::auxiliary_window_template::{AuxiliaryWindowSpec, AuxiliaryWindowTemplate, PanelAction};
use super::independent_window_template::IndependentWindowTemplate;
use super::layered;
use super::monitor;
use super::painter::SharedPainter;
use super::window_class::WindowClass;

/// 脚本独立窗口的窗口类（所有脚本窗口共用）。
const CLASS_NAME: PCWSTR = w!("CloudIMEScriptWindow");

static CLASS: WindowClass = WindowClass::new();

/// 脚本窗口鼠标事件的出口：UI 线程调它，把事件送进工人线程的 `Work` 通道。
///
/// `Arc`（而不是 `Rc`）：它要随 UI 线程的启动闭包跨线程移动；窗口之间共享同一份。
pub(super) type ScriptWindowEvents = Arc<dyn Fn(ScriptWindowEvent) + Send + Sync>;

thread_local! {
    /// 本线程建出来的脚本独立窗口（HWND → 窗口）：窗口过程收鼠标 / 命中测试时按它找回状态。
    static REGISTRY: RefCell<HashMap<isize, Weak<ScriptWindow>>> = RefCell::new(HashMap::new());
}

/// 脚本建出来的全部窗口（独立窗口 + 名下的辅助窗口），按编号索引。
pub(super) struct ScriptWindows {
    windows: HashMap<WindowId, Rc<ScriptWindow>>,
    auxiliaries: HashMap<WindowId, Rc<ScriptAuxiliary>>,
    painter: SharedPainter,
    events: ScriptWindowEvents,
}

impl ScriptWindows {
    /// 一块窗口都还没有：`painter` 与别的窗口共用（同一份字形缓存）。
    pub(super) fn new(painter: SharedPainter, events: ScriptWindowEvents) -> Self {
        Self {
            windows: HashMap::new(),
            auxiliaries: HashMap::new(),
            painter,
            events,
        }
    }

    /// 脚本的一次窗口操作（命令通道里来的，按顺序执行）。
    pub(super) fn apply(&mut self, request: WindowRequest) {
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

/// 拖动状态：按下时的光标屏幕坐标 + 窗口左上角（物理像素）+ 有没有超过阈值。
#[derive(Debug, Clone, Copy)]
struct Drag {
    start_cursor: (i32, i32),
    start_window: (i32, i32),
    moved: bool,
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

    /// 鼠标正停在哪个元素上（1 起）。
    hover: Cell<Option<usize>>,

    /// 拖动中：按下时的光标屏幕坐标 + 窗口左上角 + 有没有超过阈值。
    drag: Cell<Option<Drag>>,

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
            hover: Cell::new(None),
            drag: Cell::new(None),
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
        *self.document.borrow_mut() = document;
        self.revision.set(self.revision.get() + 1);
        if self.shown.get()
            && let Some(position) = self.position.get()
        {
            self.render_and_present(position);
            self.sync_assets();
        }
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
        if !self.shown.replace(true) {
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
            let Some(rendered) =
                painter.render_document(&self.document.borrow(), dpi, self.spec.shadow)
            else {
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
    fn hit_at(&self, client_x: i32, client_y: i32) -> (f32, f32, Option<usize>) {
        let scale = self.scale.get().max(0.01);
        let margin = self.margin.get();
        let x = (client_x + margin) as f32 / scale;
        let y = (client_y + margin) as f32 / scale;
        (x, y, element_at(&self.hits.borrow(), x, y))
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
        let (x, y, element) = self.hit_at(client_x, client_y);
        (self.events)(ScriptWindowEvent {
            window: self.id,
            name: self.name.clone(),
            action,
            x,
            y,
            element,
            delta,
        });
    }

    fn emit_move(&self, client_x: i32, client_y: i32) {
        self.last.set((client_x, client_y));
        let (_, _, element) = self.hit_at(client_x, client_y);
        if self.hover.replace(element) == element {
            return;
        }
        if element.is_some() {
            self.emit(WindowAction::Hover, client_x, client_y);
        }
    }

    fn emit_leave(&self) {
        self.hover.set(None);
        let (x, y) = self.last.get();
        self.emit(WindowAction::Leave, x, y);
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
    fn begin_drag(&self) {
        let mut point = POINT::default();
        let _ = unsafe { GetCursorPos(&mut point) };
        let (x, y) = self.position.get().unwrap_or((0, 0));
        let margin = self.margin.get();
        self.drag.set(Some(Drag {
            start_cursor: (point.x, point.y),
            start_window: (x - margin, y - margin),
            moved: false,
        }));
        let _ = unsafe { SetCapture(self.hwnd()) };
        // 点一下就算拿到键盘焦点（脚本可以据此处理按键）
        self.notify_focus();
    }

    /// 左键按着移动：超过阈值就把窗口挪过去（名下资产跟着重新摆）。
    fn drag_to(&self) {
        let Some(mut drag) = self.drag.get() else {
            return;
        };
        let mut point = POINT::default();
        let _ = unsafe { GetCursorPos(&mut point) };
        let dx = point.x - drag.start_cursor.0;
        let dy = point.y - drag.start_cursor.1;
        if !drag.moved && dx.abs().max(dy.abs()) < DRAG_THRESHOLD {
            return;
        }
        drag.moved = true;
        self.drag.set(Some(drag));
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
        self.sync_assets();
    }

    /// 左键抬起：返回「刚才是不是拖过」（拖过就不算点击）。
    fn end_drag(&self) -> bool {
        let _ = unsafe { ReleaseCapture() };
        self.drag.take().map(|drag| drag.moved).unwrap_or(false)
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
                delta: panel.delta,
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

/// 点 → 物理像素。
fn points_to_pixels(x: f32, y: f32, dpi: u32) -> (i32, i32) {
    let scale = dpi.max(1) as f32 / 96.0;
    ((x * scale).round() as i32, (y * scale).round() as i32)
}

/// 一个长度：点 → 物理像素。
fn points(value: f32, dpi: u32) -> i32 {
    (value * dpi.max(1) as f32 / 96.0).round() as i32
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
            let transparent = super::mouse_transparent(hwnd);
            LRESULT(if transparent {
                HTTRANSPARENT as isize
            } else {
                HTCLIENT as isize
            })
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // 左键按下：开始记拖动起点；同时算这块窗口拿到键盘焦点。
        WM_LBUTTONDOWN => {
            with_window(hwnd, ScriptWindow::begin_drag);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            track_leave(hwnd);
            with_window(hwnd, |window| {
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
        // 拖过就不是点击（阈值在 `drag_to` 里判）。
        WM_LBUTTONUP => {
            with_window(hwnd, |window| {
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
            with_window(hwnd, |window| window.drag.set(None));
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
}
