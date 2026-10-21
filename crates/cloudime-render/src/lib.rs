//! 自绘渲染器：把候选窗的一帧（拼音行、候选行、页脚）按主题画成一张位图，壳只负责把位图贴到窗口上。
//!
//! 栅格用 tiny-skia（纯 CPU），文字用 cosmic-text（fontdb 选字体 + 整形 + swash 栅格，带回退链与彩色 emoji）。
//! 字体不扫系统目录，按清单只加载界面字体、中文、日文、emoji 几个文件（[`FontLibrary`]）。
//! 所有尺寸以「点」为单位写在 [`Theme`] 里，[`Renderer`] 按缩放倍数换成像素；输出是预乘 alpha 的 RGBA 位图。
//!
//! **能画的元素**（`docs/design/windows.md` §6）：`text`（[`FontLibrary`] + 字形管线）、`svg`（`svg` 模块）、
//! `img`（[`decode_image`] / [`load_image`] / [`read_image`] / [`draw_image`]）、`line` / `rect`（含圆角与圆）/
//! `path`（画布那几笔填充原语，tiny-skia 的路径）。
//!
//! 元素各是一条 trait（`elements`）；唯一的实现是 `Render`（渲染：离屏位图，整块贴给窗口，内容持久）。
//! 窗口里的每一笔都经这些 trait 画。
//!
//! 设计与验收见 `docs/design/rendering.md`。

mod canvas;
mod color;
mod control;
mod document;
mod elements;
mod error;
mod fonts;
mod frame;
mod image;
mod layout;
mod render;
mod renderer;
mod shadow;
mod svg;
mod text;
mod theme;

#[cfg(target_os = "windows")]
pub use color::Color;
pub use control::{
    ButtonColors, ButtonShape, ButtonState, FlatSvgButton, Label, LabelRun, Panel, ScrollAxis,
    ScrollBar, StaticImage, StaticShape, SvgAlign, TextBlock, TextBox, TextBoxScroll, TextEdit,
};
/// 脚本窗口 / 面板的内容：**声明式**元素表（display list），由 [`Renderer::render_document`] 一次出图。
pub use document::{
    ControlName, Document, Element, FontSlot, HitRect, HitShape, InstanceInfo, PathCommand, Region,
    StaticTextContent, TextEditPlacement,
};
/// **能画的元素**的 traits：`text` / `svg` / `img` / `line` / `rect`（含圆角与圆）/ `path`；
/// 以及控件绘制用的一整套 [`ControlRender`]。
pub use elements::{
    ControlRender, ImageElement, LineElement, PathElement, RectElement, StrokeElement, SvgElement,
    TextElement,
};
pub use error::RenderError;
/// Windows 的字体登记：按字族名找文件、列字族名（设置页用）。
#[cfg(target_os = "windows")]
pub use fonts::directwrite as system_fonts;
pub use fonts::{FontLibrary, UiFont};
pub use frame::{
    Auxiliary, Frame, HighlightAnimation, HighlightRect, Preedit, PreeditSegment, PreeditStyle,
    Row, TipSegment, Tone,
};
pub use image::{
    decode_image, draw_image, flip_image, gray_image, load_image, mask_image, read_image,
    rotate_image, scale_image,
};
pub use layout::Layout;
/// **渲染**：先画进一块内存区域，画完整块交给窗口，内容持久。
pub use render::Render;
pub use renderer::{
    KeyCap, KeyMapLayout, Rendered, RenderedDocument, RenderedStatus, Renderer, StatusCell,
    draw_key_map, key_map_cell, key_map_pixel_size, key_map_size,
};
pub use shadow::Shadow;
pub use svg::svg_size;
pub use text::{TextPainter, TextSize, TextStyle};
pub use theme::{FontSpec, Palette, Theme};

/// 让 `tiny_skia::Pixmap` 的使用方不用再单独依赖 tiny-skia。
pub use tiny_skia::Pixmap;

/// 按左上角对齐把位图裁到 `width`×`height`：多出来的部分丢掉，不足的部分补透明。
///
/// 候选窗**展开**（Tab）的过渡用它：位图仍然按最终尺寸画，贴图时只贴出「长到哪儿」的那一块——
/// 内容不缩放，看上去像是框在长、内容逐步露出来。
pub fn clip_pixmap(pixmap: &Pixmap, width: u32, height: u32) -> Option<Pixmap> {
    use tiny_skia::{PixmapPaint, Transform};

    let mut clipped = Pixmap::new(width, height)?;
    let mut canvas = clipped.as_mut();
    canvas.draw_pixmap(
        0,
        0,
        pixmap.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    Some(clipped)
}

#[cfg(test)]
mod tests {
    use super::{Pixmap, clip_pixmap};

    /// 造一张 `width`×`height` 的位图，第 `(x, y)` 个像素标成 `(x, y, 0, 255)` 好认。
    fn sample(width: u32, height: u32) -> Pixmap {
        let mut pixmap = Pixmap::new(width, height).unwrap();
        for y in 0..height {
            for x in 0..width {
                pixmap.pixels_mut()[(y * width + x) as usize] =
                    tiny_skia::ColorU8::from_rgba(x as u8, y as u8, 0, 255).premultiply();
            }
        }
        pixmap
    }

    #[test]
    fn clipping_keeps_the_top_left_corner() {
        let source = sample(4, 4);
        let clipped = clip_pixmap(&source, 2, 3).expect("裁得出来");
        assert_eq!((clipped.width(), clipped.height()), (2, 3));
        // 左上角对齐：留下的就是原来左上 2×3 的那块
        assert_eq!(clipped.pixels()[0], source.pixels()[0]);
        // 输出第 (1, 2) 个像素 = 源（4 宽）第 9 个
        assert_eq!(clipped.pixels()[5], source.pixels()[9]);
    }

    #[test]
    fn clipping_larger_pads_with_transparent() {
        let source = sample(2, 2);
        let clipped = clip_pixmap(&source, 4, 3).expect("裁得出来");
        assert_eq!((clipped.width(), clipped.height()), (4, 3));
        assert_eq!(clipped.pixels()[0], source.pixels()[0]);
        // 放大出来的部分全透明：输出第 (3, 0) 与 (3, 1) 个像素
        assert_eq!(clipped.pixels()[3].alpha(), 0);
        assert_eq!(clipped.pixels()[7].alpha(), 0);
    }
}
