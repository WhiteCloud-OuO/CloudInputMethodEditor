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
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    DeleteObject, FF_DONTCARE, FW_NORMAL, HFONT, OUT_DEFAULT_PRECIS,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE,
    TRACKMOUSEEVENT, TrackMouseEvent, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, ES_AUTOHSCROLL, ES_LEFT, GetCursorPos, GetSystemMetrics,
    HTCLIENT, HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, LoadCursorW, MA_NOACTIVATE, SM_CXDOUBLECLK,
    SM_CYDOUBLECLK, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow, WM_CAPTURECHANGED,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST,
    WM_RBUTTONUP, WM_SETFONT, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use cloudime_platform::LayoutMode;
use cloudime_render::{ButtonState, Document, Element, HitShape, Region};
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

/// 一个 `text_edit` 覆盖在脚本窗口上的系统 EDIT。
struct TextEditWindow {
    /// 元素下标（`document.elements`，0 起）。
    element: usize,
    hwnd: HWND,
    font: HFONT,
}

/// 拖动状态：按下时的光标屏幕坐标 + 窗口左上角（物理像素）。
#[derive(Debug, Clone, Copy)]
struct Drag {
    start_cursor: (i32, i32),
    start_window: (i32, i32),
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

    /// 各 `text_edit` 的系统 EDIT 覆盖窗。
    text_edits: RefCell<Vec<TextEditWindow>>,

    /// 拖动中：按下时的光标屏幕坐标 + 窗口左上角。
    drag: Cell<Option<Drag>>,

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

    /// 按当前内容重建系统 EDIT 覆盖窗（旧的全部销毁）。
    fn rebuild_text_edits(&self) {
        self.destroy_text_edits();
        let mut edits = self.text_edits.borrow_mut();
        let document = self.document.borrow();
        for (index, element) in document.elements.iter().enumerate() {
            let Element::TextEdit(edit) = element else {
                continue;
            };
            let style = windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                WS_POPUP.0 | ES_AUTOHSCROLL as u32 | ES_LEFT as u32,
            );
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                    w!("EDIT"),
                    w!(""),
                    style,
                    0,
                    0,
                    0,
                    0,
                    Some(self.hwnd()),
                    None,
                    None,
                    None,
                )
            };
            let Ok(hwnd) = hwnd else {
                tracing::warn!(id = self.id, index, "text_edit：建系统 EDIT 失败");
                continue;
            };
            let text = HSTRING::from(edit.text.as_str());
            let _ = unsafe { SetWindowTextW(hwnd, &text) };
            let (family, theme_size) = self
                .painter
                .borrow()
                .as_ref()
                .map(|painter| painter.text_font(edit.font))
                .unwrap_or((None, 14.0));
            let size = edit.size.unwrap_or(theme_size);
            let height = -((size * self.dpi.get() as f32 / 96.0).round() as i32).max(1);
            let family = family
                .map(|family| HSTRING::from(family.as_str()))
                .unwrap_or_default();
            let font = unsafe {
                CreateFontW(
                    height,
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    (DEFAULT_PITCH.0 as u32) | (FF_DONTCARE.0 as u32),
                    &family,
                )
            };
            let _ = unsafe {
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                )
            };
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            edits.push(TextEditWindow {
                element: index,
                hwnd,
                font,
            });
        }
    }

    /// 把系统 EDIT 摆到背景形状的文本区；隐藏时一起隐藏。
    fn sync_text_edits(&self) {
        let Some(position) = self.position.get() else {
            return;
        };
        let scale = self.scale.get().max(0.01);
        let shown = self.shown.get();
        let document = self.document.borrow();
        for edit in self.text_edits.borrow().iter() {
            let Some(Element::TextEdit(control)) = document.elements.get(edit.element) else {
                continue;
            };
            let (x, y, width, height) = control.edit_rect();
            let x = position.0 + ((control.x + x) * scale).round() as i32;
            let y = position.1 + ((control.y + y) * scale).round() as i32;
            let width = (width * scale).round().max(1.0) as i32;
            let height = (height * scale).round().max(1.0) as i32;
            let _ = unsafe {
                SetWindowPos(
                    edit.hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    width,
                    height,
                    SWP_NOACTIVATE,
                )
            };
            let _ = unsafe { ShowWindow(edit.hwnd, if shown { SW_SHOWNA } else { SW_HIDE }) };
        }
    }

    fn destroy_text_edits(&self) {
        for edit in self.text_edits.borrow_mut().drain(..) {
            let _ = unsafe { DestroyWindow(edit.hwnd) };
            let _ = unsafe { DeleteObject(edit.font.into()) };
        }
    }

    /// 点到文本编辑框的区域（背景胶囊的边距）时，把焦点交给系统 EDIT。
    fn focus_text_edit(&self, element: Option<usize>) {
        let Some(index) = element.and_then(|element| element.checked_sub(1)) else {
            return;
        };
        let edits = self.text_edits.borrow();
        let Some(edit) = edits.iter().find(|edit| edit.element == index) else {
            return;
        };
        let _ = unsafe { SetForegroundWindow(edit.hwnd) };
        let _ = unsafe { SetFocus(Some(edit.hwnd)) };
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
        self.reset_button_states();
        for edit in self.text_edits.borrow().iter() {
            let _ = unsafe { ShowWindow(edit.hwnd, SW_HIDE) };
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
        let region = super::region::hit(&self.regions.borrow(), x, y);
        (x, y, element, region)
    }

    /// 区域下标 → 区域名（事件里给人看的那个）。
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
        let region = self.region_name(region);
        let (ctrl, alt, shift) = modifiers();
        (self.events)(ScriptWindowEvent {
            window: self.id,
            name: self.name.clone(),
            action,
            x,
            y,
            element,
            region,
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
            with_window(hwnd, |window| {
                let (_, _, element, _) = window.hit_at(client_x(lparam), client_y(lparam));
                window.focus_text_edit(element);
                window.begin_button_press(client_x(lparam), client_y(lparam));
                window.begin_drag(client_x(lparam), client_y(lparam));
            });
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
}
