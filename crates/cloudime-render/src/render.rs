//! **渲染**（rendering）：先画进一块**内存区域**，画完整块交给窗口 —— 内容**持久**，窗口怎么刷新都不变
//! （`docs/design/windows.md` §6）。
//!
//! 元素都经 [`crate::elements`] 的 traits 画，这里只是「渲染」那一侧的实现：内存区域 = tiny-skia 的
//! `Pixmap`（预乘 RGBA），文字 = [`TextPainter`]（cosmic-text + swash，句柄共享 —— 画的时候还要量字）。
//!
//!
//! 用法（窗口侧）：`Render::new(w, h, text.clone())` → 经 traits 画完 → `into_pixmap()` 交给
//! `ui::layered::present` 一次贴上。

use std::cell::RefCell;
use std::rc::Rc;

use tiny_skia::{BlendMode, Path, Pixmap};

use crate::canvas::Canvas;
use crate::color::Color;
use crate::elements::{
    ImageElement, LineElement, PathElement, RectElement, StrokeElement, SvgElement, TextElement,
};
use crate::error::RenderError;
use crate::image;
use crate::svg;
use crate::text::{TextPainter, TextSize, TextStyle};

/// 一块**渲染**用的内存区域 + 字形管线的句柄。
pub struct Render {
    canvas: Canvas,
    text: Rc<RefCell<TextPainter>>,
}

impl Render {
    /// 开一块 `width`×`height` 的内存区域（预乘 RGBA）；文字用 `text` 那套字形管线。
    pub fn new(
        width: u32,
        height: u32,
        text: Rc<RefCell<TextPainter>>,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            canvas: Canvas::new(width, height)?,
            text,
        })
    }

    /// 收工：把这块内存区域交出去（窗口拿去整块贴上）。
    pub fn into_pixmap(self) -> Pixmap {
        self.canvas.into_pixmap()
    }

    /// 内存区域宽（像素）。
    pub fn width(&self) -> u32 {
        self.canvas.width()
    }

    /// 内存区域高（像素）。
    pub fn height(&self) -> u32 {
        self.canvas.height()
    }

    /// 量一段文字（宽高，像素）：画之前排版用，与 [`TextElement::text`] 同一套字形管线。
    pub fn measure(&self, text: &str, style: &TextStyle) -> TextSize {
        self.text.borrow_mut().measure(text, style)
    }

    /// 把一整块位图**直接拷**过来（调用方保证目标这块还是空的；要 source-over 用 [`ImageElement::image`]）。
    pub(crate) fn blit(&mut self, layer: &Pixmap) {
        let target = self.canvas.pixmap_mut();
        if target.width() == layer.width() && target.height() == layer.height() {
            target.data_mut().copy_from_slice(layer.data());
        }
    }
}

impl TextElement for Render {
    fn text(&mut self, text: &str, style: &TextStyle, x: f32, y: f32) -> f32 {
        self.text
            .borrow_mut()
            .draw(&mut self.canvas, text, style, x, y)
    }

    fn measure(&self, text: &str, style: &TextStyle) -> TextSize {
        Render::measure(self, text, style)
    }

    fn supports_char(&mut self, ch: char, style: &TextStyle) -> bool {
        self.text.borrow_mut().supports_char(ch, style)
    }
}

impl SvgElement for Render {
    fn svg(
        &mut self,
        svg: &str,
        x: f32,
        y: f32,
        size: f32,
        tint: Option<Color>,
    ) -> Result<(), RenderError> {
        svg::draw_svg(&mut self.canvas, svg, x, y, size, tint)
    }
}

impl ImageElement for Render {
    fn image(&mut self, image: &Pixmap, x: f32, y: f32, width: f32, height: f32) {
        image::draw_image(self.canvas.pixmap_mut(), image, x, y, width, height);
    }
}

impl LineElement for Render {
    fn line(&mut self, from: (f32, f32), to: (f32, f32), thickness: f32, color: Color) {
        self.canvas.line(from, to, thickness, color);
    }
}

impl RectElement for Render {
    fn rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        self.canvas.fill_rect(x, y, width, height, color);
    }

    fn round_rect(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, color: Color) {
        self.canvas
            .fill_round_rect(x, y, width, height, radius, color);
    }

    fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        self.canvas.circle(cx, cy, radius, color);
    }
}

impl PathElement for Render {
    fn path(&mut self, path: &Path, color: Color) {
        self.canvas.fill_path(path, color, BlendMode::SourceOver);
    }
}

impl StrokeElement for Render {
    fn stroke_round_rect(
        &mut self,
        rect: tiny_skia::Rect,
        radius: f32,
        thickness: f32,
        color: Color,
    ) {
        self.canvas
            .stroke_round_rect(rect, radius, thickness, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fonts::FontLibrary;
    use crate::text::TextPainter;

    /// 元素 trait 画出来的东西落在正确的像素上：矩形、圆角、圆、线、路径、图像。
    #[test]
    fn elements_land_on_the_memory_region() {
        let painter = Rc::new(RefCell::new(TextPainter::new(
            FontLibrary::system("zh-CN").expect("字体库"),
        )));
        let mut render = Render::new(40, 40, painter).expect("内存区域");

        render.rect(0.0, 0.0, 10.0, 10.0, Color::rgb(255, 0, 0));
        render.round_rect(0.0, 12.0, 10.0, 10.0, 3.0, Color::rgb(0, 255, 0));
        render.circle(30.0, 8.0, 6.0, Color::rgb(0, 0, 255));
        render.line((0.0, 30.0), (20.0, 30.0), 4.0, Color::rgb(255, 255, 0));

        let mut builder = tiny_skia::PathBuilder::new();
        builder.move_to(24.0, 24.0);
        builder.line_to(38.0, 24.0);
        builder.line_to(38.0, 38.0);
        builder.close();
        render.path(&builder.finish().expect("路径"), Color::rgb(255, 0, 255));

        let mut image = Pixmap::new(2, 2).expect("图像");
        for pixel in image.pixels_mut() {
            *pixel = tiny_skia::PremultipliedColorU8::from_rgba(0, 0, 0, 255).expect("颜色");
        }
        render.image(&image, 30.0, 30.0, 4.0, 4.0);

        let pixmap = render.into_pixmap();
        let pixel = |x: u32, y: u32| pixmap.pixel(x, y).expect("像素");
        assert_eq!(pixel(5, 5).red(), 255, "矩形中心是红");
        assert_eq!(pixel(5, 17).green(), 255, "圆角矩形中心是绿");
        assert_eq!(pixel(30, 8).blue(), 255, "圆心是蓝");
        assert!(pixel(10, 30).red() > 200, "线条在 y=30 那一行");
        assert!(
            pixel(36, 30).red() > 200 && pixel(36, 30).blue() > 200,
            "路径是品红"
        );
        assert_eq!(pixel(37, 36).alpha(), 255, "图像画上了");
        assert_eq!(pixel(0, 39).alpha(), 0, "没画的地方还透明");
    }

    /// 同一块内存区域上能量字（`Render::measure` 与画字共用一套管线）。
    #[test]
    fn measuring_shares_the_text_pipeline() {
        let painter = Rc::new(RefCell::new(TextPainter::new(
            FontLibrary::system("zh-CN").expect("字体库"),
        )));
        let render = Render::new(40, 40, painter).expect("内存区域");
        let style = TextStyle::new(
            crate::theme::FontSpec::with_size(16.0),
            16.0,
            Color::rgb(0, 0, 0),
            1.0,
        );
        let size = render.measure("云朵", &style);
        assert!(size.width > 0.0, "量得出来");
        assert_eq!(size.height, style.line_height);
    }
}
