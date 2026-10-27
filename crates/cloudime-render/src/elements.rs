//! **能画的元素**的 traits：`text` / `svg` / `img` / `line` / `rect`（含圆角与圆）/ `path`。
//!
//! 两个实现，对应 `docs/design/windows.md` §6 的两种画法：
//!
//! 实现是 [`crate::Render`] —— **渲染**：先画进一块**内存区域**，画完**整块**交给窗口，内容**持久**。
//! （**绘制**那条路 —— 直接画在窗口 DC 上、内容不持久 —— 2026-10-17 试过一版，后来删了；那版在 git 历史里。）
//!
//! 规矩只有一句：**窗口里的元素只准经这几条 trait 画** —— 谁要新元素，就加一条 trait、两边各实现一次；
//! 别在窗口 / 模板里另抄一遍原语（重复造轮子）。
//!
//! 坐标一律是**像素**、左上角原点；`text` 的 `(x, y)` 是行框左上角（`y` 不是基线）。

use tiny_skia::{Path, Pixmap, Rect};

use crate::color::Color;
use crate::error::RenderError;
use crate::text::{TextSize, TextStyle};

/// **文本**：画一段、量一段 —— 两条路（渲染 / 绘制）各用自己那套字形管线。
pub trait TextElement {
    /// 按 `style` 在 `(x, y)`（行框左上角）画一段，返回这段的宽度。
    fn text(&mut self, text: &str, style: &TextStyle, x: f32, y: f32) -> f32;

    /// 只量不画（排版要先知道宽度）；宽度与 [`Self::text`] 返回的那个一致。
    fn measure(&self, text: &str, style: &TextStyle) -> TextSize;

    /// 这个样式（字族）能不能显示 `ch`。默认当作能，只有真正走字形管线的实现才需要覆盖。
    fn supports_char(&mut self, _ch: char, _style: &TextStyle) -> bool {
        true
    }
}

/// **SVG 图形**：按 SVG 自己的宽高比等比缩放，塞进 `(x, y)` 起 `size` 见方的框里（居中、不裁剪）。
/// `tint` 给 `Some` 时**按 alpha 把整张图标染成这个颜色**（只取轮廓，SVG 里写的颜色不算）。
pub trait SvgElement {
    fn svg(
        &mut self,
        svg: &str,
        x: f32,
        y: f32,
        size: f32,
        tint: Option<Color>,
    ) -> Result<(), RenderError>;
}

/// **图像**：把 `image` 按目标矩形（左上角 `(x, y)`、宽 `width`、高 `height`）缩放画上，越界裁掉。
pub trait ImageElement {
    fn image(&mut self, image: &Pixmap, x: f32, y: f32, width: f32, height: f32);
}

/// **线条**：`from` → `to`，粗 `thickness`。
pub trait LineElement {
    fn line(&mut self, from: (f32, f32), to: (f32, f32), thickness: f32, color: Color);
}

/// **矩形**（含圆角矩形与圆形）。
pub trait RectElement {
    /// 直角矩形。
    fn rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color);

    /// 圆角矩形；`radius` 超过半边长按半边长算。
    fn round_rect(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, color: Color);

    /// 圆：圆心 + 半径。
    fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color);
}

/// **路径**：tiny-skia 的 [`Path`]（直线 / 二次 / 三次贝塞尔 / 闭合），按**非零环绕**填充。
pub trait PathElement {
    fn path(&mut self, path: &Path, color: Color);
}

/// **描边**：给圆角矩形画一圈线（面板缺省的边框用它）。
pub trait StrokeElement {
    fn stroke_round_rect(&mut self, rect: Rect, radius: f32, thickness: f32, color: Color);
}

/// **单件阴影**：在控件自己的矩形下面垫一层柔光（文本按钮用）。
///
/// 默认空实现：只有真正走位图的后端需要覆盖；绘制后端没有离屏层时忽略即可。
pub trait ShadowElement {
    fn shadow(&mut self, _shadow: &crate::Shadow, _content: Rect, _radius: f32, _scale: f32) {}
}

/// 控件绘制用的一整套原语：能画控件的地方必须同时实现上面这些 trait。
///
/// `Render` 全实现了，所以控件在内存渲染器上直接可用；以后加别的后端（比如直接画 DC）
/// 只要也实现这几个 trait，控件代码不用改。
pub trait ControlRender:
    TextElement
    + SvgElement
    + ImageElement
    + LineElement
    + RectElement
    + PathElement
    + StrokeElement
    + ShadowElement
{
}

impl<T> ControlRender for T where
    T: TextElement
        + SvgElement
        + ImageElement
        + LineElement
        + RectElement
        + PathElement
        + StrokeElement
        + ShadowElement
{
}
