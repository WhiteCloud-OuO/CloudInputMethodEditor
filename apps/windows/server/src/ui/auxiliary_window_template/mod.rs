//! **辅助窗口模板**（`auxiliary_window_template`）：给「辅助窗口」（独立窗口的**资产**，
//! 见 [`super::auxiliary`]）共用的那块壳子 —— 一块自绘的分层面板。
//!
//! 画的是**一块面板**：主题背景色 + 圆角，圆角与候选窗口同源（都是 `theme.corner_radius`），
//! 出图用云朵自带的渲染器，`UpdateLayeredWindow` 一次贴上（与独立窗口同一条路）。
//! `set_lines` 可以往里填几行小字（键位映射参考这类纯文本）。
//!
//! 三个属性：**宽 / 高 / 间隙**（[`AuxiliaryWindowSpec`]，物理像素）。没设过就按内容与宿主的形状来：
//!
//! - **有内容**（眼下只有「键位映射参考」）：格子边长按**目标高度**反推 —— 竖排用宿主（候选窗口）
//!   **现在的高度**（两个窗口一样高；候选窗底下多一行译文 / 多一页页码时它也跟着高），但**不小于**
//!   「候选窗口装 9 个候选项」那档（候选只有一两项时格子会小到字看不清，宁可让它比候选窗高一点）；
//!   横排宿主又扁又宽、按不了，直接按那档算；
//! - **没内容**（版面还没生成）：竖排高度对齐「候选窗口装 9 个候选项」的高度，宽度取它的 **√2 倍**（向下取整）；
//!   横排宽度对齐「候选窗口装 6 个候选项（每个两个字）」的宽度，高度取它的 **√5 倍**（向下取整）；
//! - 间隙固定 16 逻辑像素（按 DPI 折成物理像素）。
//!
//! 摆位（[`AuxiliaryWindowTemplate::show`] 给宿主的内容区矩形与排布方向）：按**候选窗口矩形中心点**
//! 所在的半屏往另一边摆 —— 竖排时中心在屏幕右半边就贴**左侧**（否则贴右侧），横排时中心在下半边就贴
//! **上方**（否则贴下方）；最后整体夹进所在显示器的工作区。
//!
//! **只管画与摆**：什么时候显示 / 收起由用它的那件资产自己定（拿 [`super::auxiliary::Asset`] 实现它）。

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HTTRANSPARENT, MA_NOACTIVATE, SW_HIDE,
    SW_SHOWNA, ShowWindow, WM_MOUSEACTIVATE, WM_NCHITTEST, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use cloudime_platform::LayoutMode;
use cloudime_platform::input_schemes;
use cloudime_render::{Layout, key_map_cell, key_map_size};

use super::layered;
use super::monitor;
use super::painter::SharedPainter;
use super::window_class::WindowClass;

const CLASS_NAME: PCWSTR = w!("CloudIMEAuxiliaryWindow");
static CLASS: WindowClass = WindowClass::new();

/// 与候选窗口之间的默认间隙（**逻辑像素**，按 DPI 折成物理像素）。
pub(super) const DEFAULT_GAP: i32 = 16;

/// 竖排的默认高度按「候选窗口装 9 个候选项」量。
const VERTICAL_ROWS: usize = 9;

/// 横排的默认宽度按「候选窗口装 6 个候选项」量（每个候选假定两个字）。
const HORIZONTAL_ROWS: usize = 6;

/// 渲染器还没起来时的兜底尺寸（物理像素）。
const FALLBACK_SIZE: (i32, i32) = (240, 240);

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

/// 窗口模板创建的那个窗口；不显示时只是藏在层里。
pub(super) struct AuxiliaryWindowTemplate {
    hwnd: HWND,

    /// 云朵渲染器；`None` 不绘制（字体库加载失败）。
    painter: SharedPainter,

    /// 上次用的 DPI。
    dpi: Cell<u32>,

    /// 用户设过的三个属性；`None` 的项现算默认。
    spec: Cell<AuxiliaryWindowSpec>,

    /// 面板里要画的键位映射参考版面（平台那边按当前方案准备）；空版面 = 一块空面板。
    key_map: RefCell<input_schemes::KeyMapLayout>,

    /// 这次显示摆在哪（内容左上角 + 内容尺寸）；`None` = 没显示。
    placed: Cell<Option<(i32, i32, i32, i32)>>,
}

impl AuxiliaryWindowTemplate {
    /// 建一个隐藏的模板窗口。失败返回 `Err`，调用方降级为不显示。
    pub(super) fn new(painter: SharedPainter) -> Result<Self> {
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
            painter,
            dpi: Cell::new(unsafe { GetDpiForSystem() }.max(96)),
            spec: Cell::new(AuxiliaryWindowSpec::default()),
            key_map: RefCell::new(input_schemes::KeyMapLayout::default()),
            placed: Cell::new(None),
        })
    }

    /// 设三个属性（物理像素）；`None` 的项回到默认算法。
    ///
    /// 现在还没有谁来调它 —— 这是模板留给下一步的口子（将来由配置 / 脚本喂）。
    #[allow(dead_code)]
    pub(super) fn set_spec(&self, spec: AuxiliaryWindowSpec) {
        self.spec.set(spec);
    }

    /// 换面板里要画的键位映射参考版面（当前方案变了一整份重送）；空版面就是一块空面板。
    pub(super) fn set_key_map(&self, layout: input_schemes::KeyMapLayout) {
        *self.key_map.borrow_mut() = layout;
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
        // 内容尺寸：有键位映射参考就按它算（格子边长由目标高度反推，见 `key_map_target`），
        // 没有内容才按宿主（候选窗口）的形状算默认。
        let render_layout = to_render_layout(&self.key_map.borrow());
        let target = key_map_target(candidate, layout, dpi);
        let cell = self
            .painter
            .borrow_mut()
            .as_mut()
            .filter(|_| !render_layout.rows.is_empty())
            .map(|painter| key_map_cell_for(target, painter.key_map_cell(dpi)));
        let (default_width, default_height) = match cell {
            Some(cell) => {
                let (width, height) = key_map_size(&render_layout, cell);
                (points(width, dpi), points(height, dpi))
            }
            None => self.default_size(layout, dpi),
        };
        let content = (
            spec.width.unwrap_or(default_width).max(1),
            spec.height.unwrap_or(default_height).max(1),
        );
        let gap = spec.gap.unwrap_or((DEFAULT_GAP * dpi as i32) / 96);
        let rendered = self
            .painter
            .borrow_mut()
            .as_mut()
            .and_then(|painter| match cell {
                Some(cell) => painter.render_key_map(&render_layout, cell, dpi),
                None => painter.render_panel(content, dpi),
            });
        let Some(rendered) = rendered else {
            tracing::warn!("窗口模板：渲染器没起来 / 画不出来，不显示");
            self.hide();
            return;
        };
        // 位图四周还有阴影留白，摆的是内容左上角，贴图位置要把留白减掉。
        let margin = rendered.content_x as i32;
        let (x, y) = place(candidate, content, gap, layout);
        let updated = layered::present(self.hwnd, &rendered.pixmap, (x - margin, y - margin), 255);
        if let Err(error) = updated {
            tracing::warn!(%error, "窗口模板：贴图失败，不显示");
            self.hide();
            return;
        }
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
        let _ = unsafe { ShowWindow(self.hwnd, SW_SHOWNA) };
        self.placed.set(Some((x, y, content.0, content.1)));
    }

    /// 收起（没显示过也无妨）。
    pub(super) fn hide(&self) {
        if self.placed.replace(None).is_some() {
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            tracing::info!("窗口模板已收起");
        }
    }

    /// 按候选窗口的形状算默认大小（物理像素）。
    fn default_size(&self, layout: LayoutMode, dpi: u32) -> (i32, i32) {
        let measured = self
            .painter
            .borrow_mut()
            .as_mut()
            .map(|painter| match layout {
                LayoutMode::Vertical => {
                    painter.candidate_size(Layout::Vertical, VERTICAL_ROWS, dpi)
                }
                LayoutMode::Horizontal => {
                    painter.candidate_size(Layout::Horizontal, HORIZONTAL_ROWS, dpi)
                }
            });
        let Some((width, height)) = measured else {
            return FALLBACK_SIZE;
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
}

impl Drop for AuxiliaryWindowTemplate {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// 点 → 物理像素（窗口模板里只有这一处按 DPI 折尺寸）。
fn points(value: f32, dpi: u32) -> i32 {
    ((value * dpi as f32) / 96.0).round().max(1.0) as i32
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

/// 键位映射参考的格子边长（点）：有目标（竖排：宿主现在的高度）就按它反推，但**不小于**
/// 「装 9 个候选项」那档（`nine_rows`）—— 候选只有一两项时格子会小到字看不清，宁可让它比候选窗
/// 高一点。没目标（横排 / 宿主没显示）就用那档。
fn key_map_cell_for(target: Option<f32>, nine_rows: f32) -> f32 {
    match target {
        Some(points) => key_map_cell(points).max(nine_rows),
        None => nine_rows,
    }
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

/// 分层窗口无需 `WM_PAINT`；内容由 `ui::mod` 那边推。
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // 现在还只是一块空面板：鼠标一律穿透到下面的窗口。
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::{key_map_cell_for, key_map_target, place_in};
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

    /// 格子边长：跟宿主走，但**不小于「装 9 个候选项」那档** —— 候选只有一两项时别缩到字看不清。
    #[test]
    fn the_key_map_cell_never_goes_below_the_nine_row_size() {
        let nine = 79.6_f32; // 「装 9 个候选项」量出来的格子（96 dpi、16pt 候选）
        // 宿主比那档高（9 行 + 页码行 = 313px）→ 跟宿主
        let taller = key_map_cell_for(Some(313.0), nine);
        assert!(taller > nine, "{taller}");
        // 宿主很矮（1 行 = 69px）→ 停在 9 行那档，不再往下缩
        assert_eq!(key_map_cell_for(Some(69.0), nine), nine);
        // 横排 / 宿主没显示 → 就是那档
        assert_eq!(key_map_cell_for(None, nine), nine);
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
