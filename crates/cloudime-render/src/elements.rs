//! **能画的元素**的 traits：`text` / `svg` / `img` / `line` / `rect`（含圆角与圆）/ `path`。
//!
//! 两个实现，对应 `docs/design/windows.md` §6 的两种画法：
//!
//! - [`crate::Render`] —— **渲染**：先画进一块**内存区域**，画完**整块**交给窗口，内容**持久**；
//! - [`crate::Paint`] —— **绘制**：直接画在**窗口的 DC** 上，内容**不持久**（窗口一刷新就没了）。
//!
//! 规矩只有一句：**窗口里的元素只准经这几条 trait 画** —— 谁要新元素，就加一条 trait、两边各实现一次；
//! 别在窗口 / 模板里另抄一遍原语（重复造轮子）。
//!
//! 坐标一律是**像素**、左上角原点；`text` 的 `(x, y)` 是行框左上角（`y` 不是基线）。

use tiny_skia::{Path, Pixmap};

use crate::color::Color;
use crate::error::RenderError;
use crate::text::TextStyle;

/// **文本**：按 `style` 在 `(x, y)`（行框左上角）画一段，返回这段的宽度。
pub trait TextElement {
    fn text(&mut self, text: &str, style: &TextStyle, x: f32, y: f32) -> f32;
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
