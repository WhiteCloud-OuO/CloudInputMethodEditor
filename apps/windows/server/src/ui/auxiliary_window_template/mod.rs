//! **辅助窗口模板**（`auxiliary_window_template`）：给「辅助窗口」（独立窗口的**资产**，
//! 见 [`super::auxiliary`]）共用的那块壳子 —— 一块自绘的**分层面板**。
//!
//! 画的是**一块面板**：主题背景色 + 圆角 + 阴影，圆角与候选窗口同源（都是 `theme.corner_radius`）。
//! 走**渲染**（`docs/design/windows.md` §6）：云朵渲染器把整块（阴影 + 圆角 + 内容）出成预乘 RGBA 位图，
//! `UpdateLayeredWindow` 一次贴上 —— 贴上去之后窗口刷新不会把它擦掉。
//!
//! //! 三个属性：**宽 / 高 / 间隙**（[`AuxiliaryWindowSpec`]，物理像素）。没设过就按内容与宿主的形状来：
//!
//! - **有内容**（眼下只有「键位映射参考」）：格子边长按**目标高度**反推 —— 竖排用宿主（候选窗口）
//!   **现在的高度**（两个窗口一样高；候选窗底下多一行译文 / 多一页页码时它也跟着高），
//!   横排宿主又扁又宽、按不了，直接按「候选窗口装 9 个候选项」那档；两种排布都再夹一道
//!   [`MIN_KEY_MAP_HEIGHT`]（**316 物理像素**）—— 谁都不比它矮，格子里的字才撑得住；
//! - **没内容**（版面还没生成）：竖排高度对齐「候选窗口装 9 个候选项」的高度，宽度取它的 **√2 倍**（向下取整）；
//!   横排宽度对齐「候选窗口装 6 个候选项（每个两个字）」的宽度，高度取它的 **√5 倍**（向下取整）；
//! - 间隙固定 16 逻辑像素（按 DPI 折成物理像素）。
//!
//! 摆位（[`AuxiliaryWindowTemplate::show`] 给宿主的内容区矩形与排布方向）：按**候选窗口矩形中心点**
//! 所在的半屏往另一边摆 —— 竖排时中心在屏幕右半边就贴**左侧**（否则贴右侧），横排时中心在下半边就贴
//! **上方**（否则贴下方）；最后整体夹进所在显示器的工作区。
//!
//! **只管画与摆**：什么时候显示 / 收起由用它的那件资产自己定（拿 [`super::auxiliary::Asset`] 实现它）。
//!
//! **焦点**：这块面板**默认不具备焦点** —— 鼠标一律穿透到下面的窗口，键也轮不到它（窗口建出来就带
//! `WS_EX_TRANSPARENT`，命中测试也回 `HTTRANSPARENT`）。创建时声明了允许获取焦点
//! （[`Self::new`] 的 `able_to_get_focus`）也只是「能」：真拿到要所有者**指定**（`ui::auxiliary`
//! 那套 [`super::auxiliary::Focus`]）；拿到之后才接鼠标（[`Self::set_focused`]），
//! 同时所有者那侧让位（见 [`super::independent_window_template`]）。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HTCLIENT, HTTRANSPARENT, MA_NOACTIVATE,
    SW_HIDE, SW_SHOWNA, ShowWindow, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCHITTEST, WM_RBUTTONUP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use cloudime_platform::LayoutMode;
use cloudime_platform::input_schemes;
use cloudime_render::{Color, ControlName, Layout, StaticTextContent, key_map_cell};

use super::layered;
use super::monitor;
use super::painter::{Painter, SharedPainter};
use super::window_class::WindowClass;

const CLASS_NAME: PCWSTR = w!("CloudIMEAuxiliaryWindow");
static CLASS: WindowClass = WindowClass::new();

thread_local! {
    /// 本线程建出来的辅助窗口面板（HWND → 模板）：窗口过程收鼠标事件时按它找回 Rust 状态。
    /// 存 `Weak`：不替模板续命，`Drop` 之后自然没了。
    static REGISTRY: RefCell<HashMap<isize, Weak<AuxiliaryWindowTemplate>>> =
        RefCell::new(HashMap::new());
}

/// 与候选窗口之间的默认间隙（**逻辑像素**，按 DPI 折成物理像素）。
pub(super) const DEFAULT_GAP: i32 = 16;

/// 竖排的默认高度按「候选窗口装 9 个候选项」量。
const VERTICAL_ROWS: usize = 9;

/// 横排的默认宽度按「候选窗口装 6 个候选项」量（每个候选假定两个字）。
const HORIZONTAL_ROWS: usize = 6;

/// 窗口模板的三个属性（**物理像素**）；`None` = 用按候选窗口形状算出来的默认值。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct AuxiliaryWindowSpec {
    /// 宽。
    pub width: Option<i32>,

    /// 高。
    pub height: Option<i32>,

    /// 与候选窗口之间的间隙。
    pub gap: Option<i32>,
}

/// 面板里画什么。
enum Content {
    /// 空面板：主题背景 + 圆角 + 阴影，尺寸按宿主形状 / 用户设的属性算。
    Empty,

    /// 键位映射参考（候选窗口的资产）：格子边长按目标高度反推。
    KeyMap(input_schemes::KeyMapLayout),

    /// 脚本给的一份内容（`cloudime.window.create_auxiliary` + `set_content`）。
    Document(cloudime_render::Document),
}

/// 面板上的鼠标动作（键位映射参考不接鼠标；脚本辅助窗口接）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PanelAction {
    Click,
    RightClick,
    Hover,
    Leave,
    Wheel,
}

impl PanelAction {
    /// 与脚本窗口事件同名，布局的 `events` 表用它查处理器。
    fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::RightClick => "right_click",
            Self::Hover => "hover",
            Self::Leave => "leave",
            Self::Wheel => "wheel",
        }
    }
}

/// 一次面板鼠标事件：坐标是**内容区点**；`element` 是命中的元素（1 起，`None` = 没命中），
/// `region` 是命中的区域名（`Document.regions` 里的 `name`）。
#[derive(Debug, Clone)]
pub(super) struct PanelEvent {
    pub action: PanelAction,
    pub x: f32,
    pub y: f32,
    pub element: Option<usize>,
    pub region: Option<String>,
    pub control: Option<String>,
    pub handler: Option<String>,
    pub delta: Option<i32>,
}

/// 面板鼠标事件的出口：脚本辅助窗口用它把点击 / 悬停送回工人线程。
pub(super) type PanelEvents = Box<dyn Fn(PanelEvent) + Send>;

/// 窗口模板创建的那个窗口；不显示时只是藏在层里。
pub(super) struct AuxiliaryWindowTemplate {
    hwnd: HWND,

    /// 创建时声明的「允许获取焦点」（`able_to_get_focus`）：`false`（缺省）时**永远**不接输入。
    focusable: bool,

    /// 这会儿焦点在不在它身上（所有者经 [`Self::set_focused`] 指定）。
    focused: Cell<bool>,

    /// 云朵渲染器；`None` 不绘制（字体库加载失败）。
    painter: SharedPainter,

    /// 上次用的 DPI。
    dpi: Cell<u32>,

    /// 用户设过的三个属性；`None` 的项现算默认。
    spec: Cell<AuxiliaryWindowSpec>,

    /// 面板里画什么。
    content: RefCell<Content>,

    /// 鼠标事件出口；`None` = 不接鼠标（键位映射参考）。
    events: RefCell<Option<PanelEvents>>,

    /// 最近一帧每个元素的命中区域（内容区坐标，点）：脚本辅助窗口的鼠标事件用。
    hits: RefCell<Vec<Option<cloudime_render::HitShape>>>,

    /// 脚本内容里声明的区域（点击 / 拖拽范围）。
    regions: RefCell<Vec<cloudime_render::Region>>,

    /// 脚本内容里的控件名（布局文件）：事件里给 `control`。
    control_names: RefCell<Vec<ControlName>>,

    /// 鼠标正停在哪个元素上（1 起；`None` = 没停在任何元素上）：悬停事件只在换元素时派。
    /// 与区域一起组成「这一拍停在哪」，换元素 / 换区域都算换。
    hover: Cell<Option<(Option<usize>, Option<usize>)>>,

    /// 最后一次收到的客户区坐标：离开窗口时用它报位置。
    last: Cell<(i32, i32)>,

    /// 最近一帧的渲染倍数（点 → 像素）。
    scale: Cell<f32>,

    /// 最近一帧位图四周的阴影留白（像素）。
    margin: Cell<i32>,

    /// 要不要阴影（脚本可以关掉；键位映射参考 / 空面板按缺省开）。
    shadow: Cell<bool>,

    /// 这次显示摆在哪（内容左上角 + 内容尺寸；物理像素）；`None` = 没显示。
    placed: Cell<Option<(i32, i32, i32, i32)>>,

    /// 版面版本：`set_key_map` / `set_document` 每换一次 +1（判断要不要重画）。
    revision: Cell<u64>,

    /// 上次真画出来的是哪一幅（位置 / 尺寸 / 版面版本 / 渲染器代号）：一样就不再重画。
    painted: Cell<Option<Painted>>,
}

/// 上次画出来的那一幅：位置 `(x, y)` + 内容尺寸 + 版面版本 + 渲染器代号。
type Painted = (i32, i32, i32, i32, u64, u64);

impl AuxiliaryWindowTemplate {
    /// 建一个隐藏的模板窗口。`able_to_get_focus` = 这件资产创建时**声明**自己允不允许获取焦点
    /// （见 [`super::auxiliary::Asset::able_to_get_focus`]）；`false`（缺省）时永远不接输入。
    /// 声明了也只是「能」—— 创建之后的焦点仍在所有者手上，直到所有者**指定**给它。失败返回 `Err`，
    /// 调用方降级为不显示。
    pub(super) fn new(painter: SharedPainter, able_to_get_focus: bool) -> Result<Self> {
        CLASS.ensure(|| WNDCLASSEXW {
            lpfnWndProc: Some(wndproc),
            hInstance: super::module_handle(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        })?;
        // TRANSPARENT：现在还只是一块空面板，鼠标点它等于点在底下的应用上。NOACTIVATE：不抢焦点。
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TRANSPARENT,
                CLASS_NAME,
                w!("云朵辅助窗口"),
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
            focusable: able_to_get_focus,
            focused: Cell::new(false),
            painter,
            dpi: Cell::new(unsafe { GetDpiForSystem() }.max(96)),
            spec: Cell::new(AuxiliaryWindowSpec::default()),
            content: RefCell::new(Content::Empty),
            events: RefCell::new(None),
            hits: RefCell::new(Vec::new()),
            regions: RefCell::new(Vec::new()),
            control_names: RefCell::new(Vec::new()),
            hover: Cell::new(None),
            last: Cell::new((0, 0)),
            scale: Cell::new(1.0),
            margin: Cell::new(0),
            shadow: Cell::new(true),
            placed: Cell::new(None),
            revision: Cell::new(0),
            painted: Cell::new(None),
        })
    }

    /// 把自己登记进本线程的表，窗口过程按 HWND 找回它（`Weak`：不续命，`Drop` 之后自然没了）。
    pub(super) fn attach(self: &Rc<Self>) {
        REGISTRY.with(|map| {
            map.borrow_mut()
                .insert(self.hwnd.0 as isize, Rc::downgrade(self));
        });
    }

    /// 设三个属性（物理像素）；`None` 的项回到默认算法。
    pub(super) fn set_spec(&self, spec: AuxiliaryWindowSpec) {
        self.spec.set(spec);
    }

    /// 换面板里要画的键位映射参考版面（当前方案变了一整份重送）；空版面就是一块空面板。
    pub(super) fn set_key_map(&self, layout: input_schemes::KeyMapLayout) {
        *self.content.borrow_mut() = Content::KeyMap(layout);
        // 版面变了：下次 `show` 一定重画
        self.revision.set(self.revision.get() + 1);
    }

    /// 换面板里要画的脚本内容（`cloudime.window.create_auxiliary` + `set_content`）。
    pub(super) fn set_document(&self, document: cloudime_render::Document) {
        *self.control_names.borrow_mut() = document.control_names.clone();
        *self.content.borrow_mut() = Content::Document(document);
        self.revision.set(self.revision.get() + 1);
    }

    /// 更新静态 `label` / `text_block` 的文字；只影响内容，不重建窗口。
    pub(super) fn update_static_content(&self, name: &str, content: StaticTextContent) -> bool {
        let mut document = self.content.borrow_mut();
        let Content::Document(document) = &mut *document else {
            tracing::warn!("辅助窗口：只有脚本内容才能更新静态文本");
            return false;
        };
        if !document.update_static_content(name, content) {
            tracing::warn!(control = name, "辅助窗口：名字或内容类型不匹配");
            return false;
        }
        self.revision.set(self.revision.get() + 1);
        true
    }

    /// 改一个扁平 SVG 按钮的背景色；只影响内容，不重建窗口。
    pub(super) fn set_button_background(&self, name: &str, color: Color) -> bool {
        let mut document = self.content.borrow_mut();
        let Content::Document(document) = &mut *document else {
            tracing::warn!("辅助窗口：只有脚本内容才能改按钮颜色");
            return false;
        };
        if !document.set_button_background(name, color) {
            tracing::warn!(control = name, "辅助窗口：按钮名字或类型不匹配");
            return false;
        }
        self.revision.set(self.revision.get() + 1);
        true
    }

    /// 动态改一个区域的命中范围（合并 / 剪除）：只影响鼠标命中，不重画。
    pub(super) fn edit_region(&self, name: &str, shape: cloudime_render::HitShape, combine: bool) {
        let mut content = self.content.borrow_mut();
        let Content::Document(document) = &mut *content else {
            tracing::warn!("窗口模板：只有脚本内容才有区域");
            return;
        };
        let Some(region) = document
            .regions
            .iter_mut()
            .find(|region| region.name == name)
        else {
            tracing::warn!(region = name, "窗口模板：改区域时找不到它");
            return;
        };
        if combine {
            region.combine(shape);
        } else {
            region.prune(shape);
        }
        // 鼠标命中用的是模板自己那份，跟着内容走
        *self.regions.borrow_mut() = document.regions.clone();
    }

    /// 要不要阴影（脚本 `shadow = false` 时关掉）。
    pub(super) fn set_shadow(&self, shadow: bool) {
        self.shadow.set(shadow);
        self.revision.set(self.revision.get() + 1);
    }

    /// 装鼠标事件出口（脚本辅助窗口用；键位映射参考不装，鼠标一律穿透）。
    pub(super) fn set_events(&self, events: PanelEvents) {
        *self.events.borrow_mut() = Some(events);
    }

    /// 客户区坐标 → 内容区坐标（点）+ 命中的元素（1 起）+ 区域下标（0 起）。
    fn hit_at(&self, client_x: i32, client_y: i32) -> (f32, f32, Option<usize>, Option<usize>) {
        let scale = self.scale.get().max(0.01);
        let margin = self.margin.get();
        // 窗口左上角 = 内容左上角 - 留白；客户区 0 点在位图 0 点，所以内容像素 = 客户区 - 留白
        let x = (client_x - margin) as f32 / scale;
        let y = (client_y - margin) as f32 / scale;
        let element = self
            .hits
            .borrow()
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, hit)| {
                hit.as_ref()
                    .filter(|hit| hit.contains(x, y))
                    .map(|_| index + 1)
            });
        let region = super::region::hit(&self.regions.borrow(), x, y);
        (x, y, element, region)
    }

    /// 把一次鼠标动作交给出口（没装出口就什么也不做）。
    fn emit(&self, action: PanelAction, client_x: i32, client_y: i32, delta: Option<i32>) {
        let (x, y, element, region) = self.hit_at(client_x, client_y);
        let (region, handler) = region
            .and_then(|index| {
                let regions = self.regions.borrow();
                let region = regions.get(index)?;
                Some((
                    Some(region.name.clone()),
                    region.handler(action.as_str()).map(str::to_owned),
                ))
            })
            .unwrap_or((None, None));
        let control = element.and_then(|element| {
            let index = element.checked_sub(1)?;
            self.control_names
                .borrow()
                .iter()
                .find(|control| control.path.len() == 1 && control.path[0] == index)
                .map(|control| control.name.clone())
        });
        if let Some(events) = self.events.borrow().as_ref() {
            events(PanelEvent {
                action,
                x,
                y,
                element,
                region,
                control,
                handler,
                delta,
            });
        }
    }

    /// 鼠标移动：只有停到**另一个元素 / 区域**上才派一次 `hover`（同一元素里挪不重复派）。
    fn emit_move(&self, client_x: i32, client_y: i32) {
        self.last.set((client_x, client_y));
        let (_, _, element, region) = self.hit_at(client_x, client_y);
        let key = (element, region);
        if self.hover.replace(Some(key)) == Some(key) {
            return;
        }
        if element.is_some() || region.is_some() {
            self.emit(PanelAction::Hover, client_x, client_y, None);
        }
    }

    /// 鼠标离开窗口：派一次 `leave`（带着离开前的位置）。
    fn emit_leave(&self) {
        self.hover.set(None);
        let (x, y) = self.last.get();
        self.emit(PanelAction::Leave, x, y, None);
    }

    /// 创建时声明的 `able_to_get_focus`。
    pub(super) fn focusable(&self) -> bool {
        self.focusable
    }

    /// 焦点**指定**给它 / 从它手上收回（由所有者驱动，见 [`super::auxiliary::Focus`]）：
    /// 有焦点才接鼠标（那会儿去掉鼠标穿透），没有就一律穿透到下面的窗口。
    ///
    /// 没声明 `able_to_get_focus` 的面板不会被打开输入（只记一条日志 —— 那说明调用方写错了，
    /// 或者所有者没照 [`super::auxiliary::Assets::designate_focus`] 的规矩来）。
    pub(super) fn set_focused(&self, focused: bool) {
        if focused && !self.focusable {
            tracing::warn!("窗口模板：这件资产创建时没声明 able_to_get_focus，不接输入");
            return;
        }
        if self.focused.replace(focused) == focused {
            return;
        }
        // 鼠标穿透就靠那一个位：窗口过程那边的命中测试也读它（两边同一个来源）
        super::set_mouse_transparent(self.hwnd, !focused);
        tracing::debug!(focused, "窗口模板：鼠标输入开关");
    }

    /// 贴在候选窗口的内容矩形旁边显示：`layout` 决定贴右侧（竖排）还是下方（横排）。
    pub(super) fn show(&self, candidate: RECT, layout: LayoutMode) {
        let point = POINT {
            x: candidate.left,
            y: candidate.top,
        };
        let dpi = monitor::dpi_near(point)
            .or_else(|| {
                let dpi = unsafe { GetDpiForWindow(self.hwnd) };
                (dpi != 0).then_some(dpi)
            })
            .unwrap_or(self.dpi.get())
            .max(1);
        self.dpi.set(dpi);
        let spec = self.spec.get();
        let target = key_map_target(candidate, layout, dpi);
        // 出图：键位映射参考 / 脚本内容 / 空面板，三选一
        let (rendered, hits, regions, generation) = {
            let content = self.content.borrow();
            let mut painter = self.painter.borrow_mut();
            let Some(painter) = painter.as_mut() else {
                tracing::warn!("窗口模板：渲染器没起来，不显示");
                self.hide();
                return;
            };
            let generation = painter.generation();
            match &*content {
                Content::KeyMap(layout_map) if !layout_map.rows.is_empty() => {
                    let render_layout = to_render_layout(layout_map);
                    let nine_rows = painter.key_map_height_nine(dpi);
                    let cell = key_map_cell(key_map_height(target, nine_rows, dpi));
                    (
                        painter.render_key_map(&render_layout, cell, dpi),
                        Vec::new(),
                        Vec::new(),
                        generation,
                    )
                }
                Content::Document(document) => {
                    let rendered = painter.render_document(document, dpi, self.shadow.get(), true);
                    let hits = rendered
                        .as_ref()
                        .map(|rendered| rendered.hits.clone())
                        .unwrap_or_default();
                    let regions = document.regions.clone();
                    (
                        rendered.map(|rendered| rendered.rendered),
                        hits,
                        regions,
                        generation,
                    )
                }
                _ => {
                    let (default_width, default_height) = default_size(painter, layout, dpi);
                    let content = (
                        spec.width.unwrap_or(default_width).max(1),
                        spec.height.unwrap_or(default_height).max(1),
                    );
                    (
                        painter.render_panel(content, dpi),
                        Vec::new(),
                        Vec::new(),
                        generation,
                    )
                }
            }
        };
        let Some(rendered) = rendered else {
            tracing::warn!("窗口模板：渲染器画不出来，不显示");
            self.hide();
            return;
        };
        let content = (
            rendered.content_width as i32,
            rendered.content_height as i32,
        );
        let gap = spec.gap.unwrap_or((DEFAULT_GAP * dpi as i32) / 96);
        let (x, y) = place(candidate, content, gap, layout);
        // 位置 / 尺寸 / 版面 / 渲染器都没变：窗口里已经是这一幅，不重贴
        let revision = self.revision.get();
        let signature = (x, y, content.0, content.1, revision, generation);
        if self.painted.get() == Some(signature) {
            return;
        }
        // 位图四周还有阴影留白，摆的是内容左上角，贴图位置要把留白减掉。
        let margin = rendered.content_x as i32;
        if let Err(error) =
            layered::present(self.hwnd, &rendered.pixmap, (x - margin, y - margin), 255)
        {
            tracing::warn!(%error, "窗口模板：贴图失败，不显示");
            self.hide();
            return;
        }
        *self.hits.borrow_mut() = hits;
        *self.regions.borrow_mut() = regions;
        self.scale.set(rendered.scale.max(0.01));
        self.margin.set(margin);
        let _ = unsafe { ShowWindow(self.hwnd, SW_SHOWNA) };
        self.painted.set(Some(signature));
        // 从「没显示」到「显示」记一条：排查「按了键没看见」时看它就知道摆到哪去了。
        if self.placed.get().is_none() {
            tracing::info!(
                x,
                y,
                width = content.0,
                height = content.1,
                gap,
                dpi,
                layout = ?layout,
                "窗口模板已摆出"
            );
        }
        self.placed.set(Some((x, y, content.0, content.1)));
    }

    /// 收起（没显示过也无妨）。
    pub(super) fn hide(&self) {
        // 收起来之后窗口内容就不作数了：下次显示重画
        self.painted.set(None);
        self.hits.borrow_mut().clear();
        self.regions.borrow_mut().clear();
        if self.placed.replace(None).is_some() {
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            tracing::info!("窗口模板已收起");
        }
    }
}

/// 按候选窗口的形状算默认大小（物理像素）。
fn default_size(painter: &mut Painter, layout: LayoutMode, dpi: u32) -> (i32, i32) {
    let (width, height) = match layout {
        LayoutMode::Vertical => painter.candidate_size(Layout::Vertical, VERTICAL_ROWS, dpi),
        LayoutMode::Horizontal => painter.candidate_size(Layout::Horizontal, HORIZONTAL_ROWS, dpi),
    };
    match layout {
        // 竖排：高度对齐「9 个候选项」，宽度取它的 √2 倍（向下取整）
        LayoutMode::Vertical => {
            let height = height.max(1);
            ((height as f32 * std::f32::consts::SQRT_2) as i32, height)
        }
        // 横排：宽度对齐「6 个两字候选」，高度取它的 √5 倍（向下取整）
        LayoutMode::Horizontal => {
            let width = width.max(1);
            (width, (width as f32 * 5.0_f32.sqrt()) as i32)
        }
    }
}

impl Drop for AuxiliaryWindowTemplate {
    fn drop(&mut self) {
        REGISTRY.with(|map| {
            map.borrow_mut().remove(&(self.hwnd.0 as isize));
        });
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// 「键位映射参考」画多高（**点**）：竖排取**宿主（候选窗口）现在的内容高度** —— 两个窗口一样高，
/// 候选窗底下多一行译文 / 多一页页码时它也就跟着高（那两行的有无要看译文与页数，量不出来）。
/// 横排宿主又扁又宽、按不了，返回 `None`（调用方按「装 9 个候选项」量一个缺省高度）；
/// 宿主没显示（高度不是正的）也返回 `None`。
fn key_map_target(host: RECT, layout: LayoutMode, dpi: u32) -> Option<f32> {
    match layout {
        LayoutMode::Horizontal => None,
        LayoutMode::Vertical => {
            let points = (host.bottom - host.top) as f32 / (dpi.max(1) as f32 / 96.0);
            (points > 0.0).then_some(points)
        }
    }
}

/// 「键位映射参考」窗口的**最低高度**（**物理像素**）：无论竖排还是横排都不比它矮。
///
/// 别按逻辑像素理解 —— 这是「看得清」那条线（格子边长由这个高度反推，太矮了格子里那些字号撑不住）。
const MIN_KEY_MAP_HEIGHT: f32 = 316.0;

/// 「键位映射参考」窗口画多高（**点**）：**竖排**跟宿主（候选窗口）现在的高度一样高，
/// 候选窗底下多一行译文 / 多一页页码时它也跟着高（那两行的有无要看译文与页数，量不出来）；
/// **横排**宿主又扁又宽、按不了，用「装 9 个候选项」那档（`nine_rows`）。
/// 两种排布都再夹一道 [`MIN_KEY_MAP_HEIGHT`]（物理像素折成点）—— 谁都不比它矮。
fn key_map_height(host: Option<f32>, nine_rows: f32, dpi: u32) -> f32 {
    let floor = MIN_KEY_MAP_HEIGHT / (dpi.max(1) as f32 / 96.0);
    host.unwrap_or(nine_rows).max(nine_rows).max(floor)
}

/// 平台那边的版面（Server 按当前方案准备）→ 渲染器要的版面（画什么）。
///
/// 两侧的 `KeyMapLayout` / `KeyCap` 各有一份：平台那份是「方案里有什么」，渲染器那份是「画什么」，
/// 渲染器不依赖平台（那是另一层的类型），转换就落在这里。
fn to_render_layout(layout: &input_schemes::KeyMapLayout) -> cloudime_render::KeyMapLayout {
    let cap = |cap: &input_schemes::KeyCap| cloudime_render::KeyCap {
        key: cap.key.clone(),
        initial: cap.initial.clone(),
        mapping: cap.mapping.clone(),
    };
    cloudime_render::KeyMapLayout {
        rows: layout
            .rows
            .iter()
            .map(|row| row.iter().map(cap).collect())
            .collect(),
        zero_initials: layout.zero_initials.iter().map(cap).collect(),
    }
}

/// 内容左上角：按**候选窗口矩形中心点**所在的半屏往另一边摆 ——
///
/// - 竖排：中心在屏幕（工作区）**右半边** → 贴候选窗**左侧**；在**左半边** → 贴**右侧**；
/// - 横排：中心在**下半边** → 贴**上方**；在**上半边** → 贴**下方**。
///
/// 这样两个窗口总是往屏幕里侧靠，不会贴到边上被挤掉。贴出去的位置最后统一夹进工作区。
fn place(candidate: RECT, content: (i32, i32), gap: i32, layout: LayoutMode) -> (i32, i32) {
    let work = monitor::work_area_near(POINT {
        x: candidate.left,
        y: candidate.top,
    });
    place_in(work, candidate, content, gap, layout)
}

/// [`place`] 的纯函数部分（工作区给死，便于单测）。
fn place_in(
    work: RECT,
    candidate: RECT,
    content: (i32, i32),
    gap: i32,
    layout: LayoutMode,
) -> (i32, i32) {
    let clamp_x = |x: i32| x.clamp(work.left, (work.right - content.0).max(work.left));
    let clamp_y = |y: i32| y.clamp(work.top, (work.bottom - content.1).max(work.top));
    let center_x = (candidate.left + candidate.right) / 2;
    let center_y = (candidate.top + candidate.bottom) / 2;
    match layout {
        LayoutMode::Vertical => {
            let x = if center_x > work.left + (work.right - work.left) / 2 {
                candidate.left - gap - content.0
            } else {
                candidate.right + gap
            };
            (clamp_x(x), clamp_y(candidate.top))
        }
        LayoutMode::Horizontal => {
            let y = if center_y > work.top + (work.bottom - work.top) / 2 {
                candidate.top - gap - content.1
            } else {
                candidate.bottom + gap
            };
            (clamp_x(candidate.left), clamp_y(y))
        }
    }
}

/// 分层窗口无需 `WM_PAINT`；内容由 [`AuxiliaryWindowTemplate::show`] 出图后整块贴上。
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // 没焦点（缺省）时窗口带着 `WS_EX_TRANSPARENT`：鼠标一律穿透到下面的窗口。
        // 命中测试照那个位回答 —— 与 `set_focused` 同一个来源，两边不会打架。
        WM_NCHITTEST => {
            let transparent = super::mouse_transparent(hwnd);
            LRESULT(if transparent {
                HTTRANSPARENT as isize
            } else {
                HTCLIENT as isize
            })
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // 装了事件出口（脚本辅助窗口）才把鼠标动作送出去；键位映射参考没装，这些消息就当没看见。
        WM_MOUSEMOVE => {
            track_leave(hwnd);
            with_template(hwnd, |template| {
                template.emit_move(client_x(lparam), client_y(lparam));
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_template(hwnd, AuxiliaryWindowTemplate::emit_leave);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            with_template(hwnd, |template| {
                template.emit(PanelAction::Click, client_x(lparam), client_y(lparam), None);
            });
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            with_template(hwnd, |template| {
                template.emit(
                    PanelAction::RightClick,
                    client_x(lparam),
                    client_y(lparam),
                    None,
                );
            });
            LRESULT(0)
        }
        // 滚轮的坐标在 `lparam` 里是**屏幕**坐标（与鼠标移动不同），先换算成客户区。
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            let mut point = POINT {
                x: client_x(lparam),
                y: client_y(lparam),
            };
            let _ = unsafe { ScreenToClient(hwnd, &mut point) };
            with_template(hwnd, |template| {
                template.emit(PanelAction::Wheel, point.x, point.y, Some(delta));
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 客户区坐标：`lparam` 低 16 位是 x、高 16 位是 y（带符号）。
fn client_x(lparam: LPARAM) -> i32 {
    (lparam.0 & 0xFFFF) as u16 as i16 as i32
}

fn client_y(lparam: LPARAM) -> i32 {
    ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32
}

/// 让窗口在鼠标离开时收到一条 `WM_MOUSELEAVE`（每次移动调一次，系统自己只报一次）。
fn track_leave(hwnd: HWND) {
    let mut event = TRACKMOUSEEVENT {
        cbSize: core::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    let _ = unsafe { TrackMouseEvent(&mut event) };
}

/// 按 HWND 找回本线程的模板（`Weak` 升级不上来就什么也不做）。
fn with_template(hwnd: HWND, act: impl FnOnce(&AuxiliaryWindowTemplate)) {
    let template =
        REGISTRY.with(|map| map.borrow().get(&(hwnd.0 as isize)).and_then(Weak::upgrade));
    if let Some(template) = template {
        act(&template);
    }
}

#[cfg(test)]
mod tests {
    use super::{MIN_KEY_MAP_HEIGHT, key_map_height, key_map_target, place_in};
    use cloudime_platform::LayoutMode;
    use windows::Win32::Foundation::RECT;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    /// 「键位映射参考」的目标高度：竖排就是宿主现在的（内容）高度，按 DPI 折回点；横排不按宿主，
    /// 宿主没显示（高度不是正的）也交给缺省。
    #[test]
    fn the_key_map_target_follows_the_host_height_only_when_vertical() {
        let host = rect(0, 0, 812, 302);
        assert_eq!(key_map_target(host, LayoutMode::Vertical, 96), Some(302.0));
        let scaled = key_map_target(host, LayoutMode::Vertical, 144).expect("竖排有目标");
        assert!((scaled - 302.0 / 1.5).abs() < 0.01, "{scaled}");
        assert_eq!(key_map_target(host, LayoutMode::Horizontal, 96), None);
        assert_eq!(
            key_map_target(rect(0, 0, 812, 0), LayoutMode::Vertical, 96),
            None,
            "宿主没显示（高度 0）→ 用「装 9 个候选项」的缺省"
        );
    }

    /// 高度：竖排跟宿主、横排用「装 9 个候选项」那档，但两种排布都不低于 [`MIN_KEY_MAP_HEIGHT`]
    /// （物理像素折成点）—— 候选很少、或者屏幕缩放 150% 时都按这条线来。
    #[test]
    fn the_key_map_window_never_goes_below_the_minimum_height() {
        let nine = 313.0_f32; // 「装 9 个候选项」量出来的高度（96 dpi、16pt 候选）
        // 宿主比那档高 → 跟宿主
        assert_eq!(key_map_height(Some(469.0), nine, 96), 469.0);
        // 宿主比那档矮（1 行 = 69px）、或者横排（没有宿主）→ 停在最低高度
        assert_eq!(key_map_height(Some(69.0), nine, 96), MIN_KEY_MAP_HEIGHT);
        assert_eq!(key_map_height(None, nine, 96), MIN_KEY_MAP_HEIGHT);
        // 150% 缩放：最低高度按物理像素算，折成点就是 316 / 1.5；「装 9 个候选项」那档在 144 dpi
        // 是 469 点（比这条线高），所以只有目标比它矮时才看得出下限
        assert!((key_map_height(None, 100.0, 144) - MIN_KEY_MAP_HEIGHT / 1.5).abs() < 0.01);
        assert_eq!(key_map_height(None, 469.0, 144), 469.0);
        assert_eq!(key_map_height(Some(500.0), 469.0, 144), 500.0);
    }

    /// 竖排：候选窗中心在屏幕**左半边**贴右侧、在**右半边**贴左侧 —— 总往屏幕里侧靠。
    #[test]
    fn vertical_goes_to_the_other_side_of_the_screen_half() {
        let work = rect(0, 0, 1920, 1080);
        // 中心 x = 250（左半边）→ 贴右侧（400 + 16）
        assert_eq!(
            place_in(
                work,
                rect(100, 100, 400, 400),
                (200, 300),
                16,
                LayoutMode::Vertical
            ),
            (416, 100),
            "左半边 → 贴右侧"
        );
        // 中心 x = 1750（右半边）→ 贴左侧（1600 − 16 − 200）
        assert_eq!(
            place_in(
                work,
                rect(1600, 100, 1900, 400),
                (200, 300),
                16,
                LayoutMode::Vertical
            ),
            (1384, 100),
            "右半边 → 贴左侧"
        );
    }

    /// 横排：候选窗中心在**上半边**贴下方、在**下半边**贴上方。
    #[test]
    fn horizontal_goes_to_the_other_side_of_the_screen_half() {
        let work = rect(0, 0, 1920, 1080);
        // 中心 y = 150（上半边）→ 贴下方（200 + 16）
        assert_eq!(
            place_in(
                work,
                rect(100, 100, 400, 200),
                (300, 200),
                16,
                LayoutMode::Horizontal
            ),
            (100, 216),
            "上半边 → 贴下方"
        );
        // 中心 y = 950（下半边）→ 贴上方（900 − 16 − 200）
        assert_eq!(
            place_in(
                work,
                rect(100, 900, 400, 1000),
                (300, 200),
                16,
                LayoutMode::Horizontal
            ),
            (100, 684),
            "下半边 → 贴上方"
        );
    }

    /// 贴出去的内容始终夹在工作区里（候选窗贴着屏幕边时也一样）。
    #[test]
    fn the_window_stays_inside_the_work_area() {
        let work = rect(0, 0, 800, 600);
        let (x, y) = place_in(
            work,
            rect(700, 560, 800, 610),
            (400, 300),
            16,
            LayoutMode::Vertical,
        );
        assert!(x >= work.left && x + 400 <= work.right, "x={x}");
        assert!(y >= work.top && y + 300 <= work.bottom, "y={y}");
    }
}
