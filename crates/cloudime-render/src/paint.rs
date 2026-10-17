//! **绘制**（painting）：直接把元素画在**窗口的 DC** 上 —— 内容**不持久**：窗口一更新 / 刷新就没了，
//! 下一帧得重画一遍（`docs/design/windows.md` §6）。
//!
//! 元素都经 [`crate::elements`] 的 traits 画，这里是「绘制」那一侧的实现：GDI（`FillRect` / `RoundRect` /
//! `Ellipse` / `LineTo` / `FillPath` / `TextOut` / `AlphaBlend`）。
//!
//! ⚠️ 与 [`crate::Render`] 不共享字形管线：文字只有 GDI 那套整形（没有字距微调 / 连字 / 彩色 emoji），
//! 抗锯齿也只有 GDI 那档。所以窗口里**默认不用它**（见 §6「一律用渲染」），留给「临时涂两笔、下一帧
//! 自己消失」的场合。GDI 对象一律**现建现删**，不缓存。

use std::ffi::c_void;

use tiny_skia::{Path, PathSegment, Pixmap};
use windows::Win32::Foundation::{COLORREF, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    BeginPath, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CloseFigure, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, Ellipse, EndPath, FF_DONTCARE, FillPath, FillRect,
    GetTextExtentPoint32W, HDC, HGDIOBJ, LineTo, MoveToEx, OUT_DEFAULT_PRECIS, PS_NULL, PS_SOLID,
    PolyBezierTo, RGBQUAD, RoundRect, SelectObject, SetBkMode, SetPolyFillMode, SetTextColor,
    TRANSPARENT, TextOutW, WINDING,
};
use windows::core::PCWSTR;

use crate::color::Color;
use crate::elements::{
    ImageElement, LineElement, PathElement, RectElement, SvgElement, TextElement,
};
use crate::error::RenderError;
use crate::svg;
use crate::text::TextStyle;

/// 画在窗口 DC 上的那支笔。
pub struct Paint {
    hdc: HDC,
}

impl Paint {
    /// 直接在 `hdc` 上画 —— DC 由调用方拿（`GetDC` / `BeginPaint`）也由调用方还。
    pub fn new(hdc: HDC) -> Self {
        Self { hdc }
    }

    /// 把一张预乘 RGBA 位图按目标矩形贴上去（`AlphaBlend`，逐像素 alpha）。
    fn blit(&self, pixmap: &Pixmap, x: f32, y: f32, width: f32, height: f32) {
        let (source_width, source_height) = (pixmap.width() as i32, pixmap.height() as i32);
        if source_width <= 0 || source_height <= 0 || width <= 0.0 || height <= 0.0 {
            return;
        }
        // DIB 要 BGRA（仍然是预乘）
        let mut bgra: Vec<u8> = Vec::with_capacity((source_width * source_height * 4) as usize);
        for pixel in pixmap.pixels() {
            bgra.extend_from_slice(&[pixel.blue(), pixel.green(), pixel.red(), pixel.alpha()]);
        }
        let header = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: source_width,
            // 负数 = 自上而下，跟位图的行序一致
            biHeight: -source_height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO {
            bmiHeader: header,
            bmiColors: [RGBQUAD::default()],
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        unsafe {
            let Ok(bitmap) = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                return;
            };
            if bits.is_null() {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                return;
            }
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), bgra.len());
            let dc = CreateCompatibleDC(None);
            let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = AlphaBlend(
                self.hdc,
                x.round() as i32,
                y.round() as i32,
                width.round() as i32,
                height.round() as i32,
                dc,
                0,
                0,
                source_width,
                source_height,
                blend,
            );
            SelectObject(dc, previous);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(dc);
        }
    }
}

/// 颜色 → `COLORREF`（`0x00BBGGRR`）。
fn colorref(color: Color) -> COLORREF {
    COLORREF((color.b as u32) << 16 | (color.g as u32) << 8 | color.r as u32)
}

/// 选一个 GDI 对象进来、用完换回旧的并删掉（绘制这条路不缓存 GDI 对象）。
fn with_selected<R>(hdc: HDC, object: HGDIOBJ, draw: impl FnOnce() -> R) -> R {
    unsafe {
        let previous = SelectObject(hdc, object);
        let out = draw();
        SelectObject(hdc, previous);
        let _ = DeleteObject(object);
        out
    }
}

impl TextElement for Paint {
    fn text(&mut self, text: &str, style: &TextStyle, x: f32, y: f32) -> f32 {
        let face: Vec<u16> = style
            .family
            .as_deref()
            .unwrap_or_default()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let font = CreateFontW(
                -(style.size.round() as i32).max(1),
                0,
                0,
                0,
                400,
                style.italic.into(),
                style.underline.into(),
                style.strike.into(),
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                PCWSTR(face.as_ptr()),
            );
            let mut size = SIZE::default();
            with_selected(self.hdc, HGDIOBJ(font.0), || {
                let _ = SetBkMode(self.hdc, TRANSPARENT);
                SetTextColor(self.hdc, colorref(style.color));
                let _ = GetTextExtentPoint32W(self.hdc, &wide, &mut size);
                let _ = TextOutW(self.hdc, x.round() as i32, y.round() as i32, &wide);
            });
            size.cx as f32
        }
    }
}

impl SvgElement for Paint {
    fn svg(
        &mut self,
        svg: &str,
        x: f32,
        y: f32,
        size: f32,
        tint: Option<Color>,
    ) -> Result<(), RenderError> {
        let side = size.ceil().max(1.0) as u32;
        let mut icon = svg::rasterize(svg, side)?;
        if let Some(tint) = tint {
            for pixel in icon.pixels_mut() {
                *pixel = tint.premultiplied(pixel.alpha());
            }
        }
        self.blit(&icon, x, y, side as f32, side as f32);
        Ok(())
    }
}

impl ImageElement for Paint {
    fn image(&mut self, image: &Pixmap, x: f32, y: f32, width: f32, height: f32) {
        self.blit(image, x, y, width, height);
    }
}

impl LineElement for Paint {
    fn line(&mut self, from: (f32, f32), to: (f32, f32), thickness: f32, color: Color) {
        let pen =
            unsafe { CreatePen(PS_SOLID, thickness.round().max(1.0) as i32, colorref(color)) };
        with_selected(self.hdc, HGDIOBJ(pen.0), || unsafe {
            let _ = MoveToEx(self.hdc, from.0.round() as i32, from.1.round() as i32, None);
            let _ = LineTo(self.hdc, to.0.round() as i32, to.1.round() as i32);
        });
    }
}

impl RectElement for Paint {
    fn rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        let brush = unsafe { CreateSolidBrush(colorref(color)) };
        let rect = RECT {
            left: x.round() as i32,
            top: y.round() as i32,
            right: (x + width).round() as i32,
            bottom: (y + height).round() as i32,
        };
        with_selected(self.hdc, HGDIOBJ(brush.0), || unsafe {
            let _ = FillRect(self.hdc, &rect, brush);
        });
    }

    fn round_rect(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, color: Color) {
        let brush = unsafe { CreateSolidBrush(colorref(color)) };
        // 无边框的画笔：`RoundRect` 会同时描边 + 填充
        let pen = unsafe { CreatePen(PS_NULL, 0, COLORREF(0)) };
        let ellipse = (radius.max(0.0) * 2.0).round() as i32;
        let (left, top) = (x.round() as i32, y.round() as i32);
        let (right, bottom) = ((x + width).round() as i32, (y + height).round() as i32);
        unsafe {
            let previous_pen = SelectObject(self.hdc, HGDIOBJ(pen.0));
            with_selected(self.hdc, HGDIOBJ(brush.0), || {
                let _ = RoundRect(self.hdc, left, top, right, bottom, ellipse, ellipse);
            });
            SelectObject(self.hdc, previous_pen);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        let brush = unsafe { CreateSolidBrush(colorref(color)) };
        let pen = unsafe { CreatePen(PS_NULL, 0, COLORREF(0)) };
        let (left, top) = ((cx - radius).round() as i32, (cy - radius).round() as i32);
        let (right, bottom) = ((cx + radius).round() as i32, (cy + radius).round() as i32);
        unsafe {
            let previous_pen = SelectObject(self.hdc, HGDIOBJ(pen.0));
            with_selected(self.hdc, HGDIOBJ(brush.0), || {
                let _ = Ellipse(self.hdc, left, top, right, bottom);
            });
            SelectObject(self.hdc, previous_pen);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
    }
}

impl PathElement for Paint {
    fn path(&mut self, path: &Path, color: Color) {
        unsafe {
            let _ = BeginPath(self.hdc);
            let mut current = (0.0_f32, 0.0_f32);
            for segment in path.segments() {
                match segment {
                    PathSegment::MoveTo(point) => {
                        let _ = MoveToEx(
                            self.hdc,
                            point.x.round() as i32,
                            point.y.round() as i32,
                            None,
                        );
                        current = (point.x, point.y);
                    }
                    PathSegment::LineTo(point) => {
                        let _ = LineTo(self.hdc, point.x.round() as i32, point.y.round() as i32);
                        current = (point.x, point.y);
                    }
                    PathSegment::QuadTo(control, point) => {
                        // GDI 没有二次贝塞尔：按 2/3 系数升成三次
                        let third = 2.0 / 3.0;
                        let c1 = (
                            current.0 + third * (control.x - current.0),
                            current.1 + third * (control.y - current.1),
                        );
                        let c2 = (
                            point.x + third * (control.x - point.x),
                            point.y + third * (control.y - point.y),
                        );
                        let _ = PolyBezierTo(
                            self.hdc,
                            &[point_of(c1), point_of(c2), point_of((point.x, point.y))],
                        );
                        current = (point.x, point.y);
                    }
                    PathSegment::CubicTo(c1, c2, point) => {
                        let _ = PolyBezierTo(
                            self.hdc,
                            &[
                                point_of((c1.x, c1.y)),
                                point_of((c2.x, c2.y)),
                                point_of((point.x, point.y)),
                            ],
                        );
                        current = (point.x, point.y);
                    }
                    PathSegment::Close => {
                        let _ = CloseFigure(self.hdc);
                    }
                }
            }
            let _ = EndPath(self.hdc);
            let brush = CreateSolidBrush(colorref(color));
            with_selected(self.hdc, HGDIOBJ(brush.0), || {
                let _ = SetPolyFillMode(self.hdc, WINDING);
                let _ = FillPath(self.hdc);
            });
        }
    }
}

fn point_of((x, y): (f32, f32)) -> POINT {
    POINT {
        x: x.round() as i32,
        y: y.round() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一张 40×40 的内存 DC + 32 位 DIB（自上而下）：绘制这条路不依赖窗口，照样能验。
    fn memory_dc(size: i32) -> (HDC, windows::Win32::Graphics::Gdi::HBITMAP, *mut c_void) {
        let header = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO {
            bmiHeader: header,
            bmiColors: [RGBQUAD::default()],
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bitmap =
                CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).expect("DIB");
            SelectObject(dc, HGDIOBJ(bitmap.0));
            (dc, bitmap, bits)
        }
    }

    /// 读回一个像素（DIB 是 BGRA）。
    unsafe fn pixel(bits: *mut c_void, size: i32, x: i32, y: i32) -> (u8, u8, u8, u8) {
        unsafe {
            let at = bits.cast::<u8>().offset(((y * size + x) * 4) as isize);
            (*at, *at.offset(1), *at.offset(2), *at.offset(3))
        }
    }

    /// 矩形、线、圆、路径、图像都真的落到窗口 DC 的像素上。
    #[test]
    fn elements_land_on_the_window_dc() {
        let (dc, bitmap, bits) = memory_dc(40);
        let mut paint = Paint::new(dc);

        paint.rect(0.0, 0.0, 10.0, 10.0, Color::rgb(255, 0, 0));
        paint.line((0.0, 20.0), (30.0, 20.0), 4.0, Color::rgb(0, 255, 0));
        paint.circle(30.0, 8.0, 6.0, Color::rgb(0, 0, 255));

        let mut builder = tiny_skia::PathBuilder::new();
        builder.move_to(24.0, 24.0);
        builder.line_to(38.0, 24.0);
        builder.line_to(38.0, 38.0);
        builder.close();
        paint.path(&builder.finish().expect("路径"), Color::rgb(255, 0, 255));

        let mut image = Pixmap::new(2, 2).expect("图像");
        for pixel in image.pixels_mut() {
            *pixel = tiny_skia::PremultipliedColorU8::from_rgba(255, 255, 255, 255).expect("颜色");
        }
        paint.image(&image, 2.0, 34.0, 4.0, 4.0);

        unsafe {
            // GDI 往 32 位 DIB 里写的是 0x00BBGGRR：alpha 位不算数（真窗口上也忽略它）
            let red = pixel(bits, 40, 5, 5);
            assert_eq!((red.0, red.1, red.2), (0, 0, 255), "矩形是红的（BGRA）");
            let green = pixel(bits, 40, 15, 20);
            assert_eq!(green.1, 255, "线是绿的");
            let blue = pixel(bits, 40, 30, 8);
            assert_eq!(blue.0, 255, "圆是蓝的");
            let magenta = pixel(bits, 40, 36, 36);
            assert!(magenta.0 > 200 && magenta.2 > 200, "路径是品红");
            let white = pixel(bits, 40, 3, 35);
            assert_eq!((white.0, white.1, white.2), (255, 255, 255), "图像贴上了");
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(dc);
        }
    }
}
