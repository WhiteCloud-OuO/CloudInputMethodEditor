//! **绘制**（painting）：直接把元素画在**窗口的 DC** 上 —— 内容**不持久**：窗口一更新 / 刷新就没了，
//! 下一帧得重画一遍（`docs/design/windows.md` §6）。
//!
//! 元素都经 [`crate::elements`] 的 traits 画。实现是 **GDI + GDI+ 混合**：
//!
//! - **需要抗锯齿的**（线、圆角矩形、圆、路径、文字、图像）走 **GDI+**：`GdipFillPath` / `GdipFillEllipse` /
//!   `GdipDrawLine` / `GdipDrawString` / `GdipDrawImageRect`，`SmoothingModeAntiAlias` +
//!   `TextRenderingHintAntiAlias`；颜色带 alpha（半透明主题色不用再压平）；
//! - **不需要抗锯齿的**（不透明的轴对齐矩形：面板底色、格子边框）仍走 **GDI** 的 `FillRect` —— 整像素、
//!   快（键位映射一屏七百多个矩形，GDI+ 会慢好几倍）。
//!
//! GDI+ 的 C API（windows-rs 只给了 flat 那套）在进程里**只启动一次**（[`gdiplus_started`]）；`Gdip*`
//! 对象按一次绘制现建、`Paint` 析构时统一删。与 [`crate::Render`] 的差别仍在：绘制画在窗口 DC 上，
//! **没有**逐像素 alpha，窗口边缘只能靠窗口区域切，柔和阴影得单独一块分层窗（`Renderer::render_shadow`）。

use std::cell::RefCell;
use std::ptr;
use std::sync::OnceLock;

use tiny_skia::{Path, PathSegment, Pixmap};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject, FillRect, GetObjectW, GetStockObject, HDC,
    HGDIOBJ, LOGFONTW,
};
use windows::Win32::Graphics::GdiPlus::{
    FillModeWinding, FontStyleItalic, FontStyleRegular, GdipAddPathArc, GdipAddPathBezier,
    GdipAddPathLine, GdipClosePathFigure, GdipCreateBitmapFromScan0, GdipCreateFont,
    GdipCreateFontFamilyFromName, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1,
    GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteFont, GdipDeleteFontFamily, GdipDeleteGraphics,
    GdipDeletePath, GdipDeletePen, GdipDeleteStringFormat, GdipDisposeImage, GdipDrawImageRect,
    GdipDrawLine, GdipDrawString, GdipFillEllipse, GdipFillPath, GdipFillRectangle,
    GdipMeasureString, GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdipSetStringFormatFlags,
    GdipSetTextRenderingHint, GdipStartPathFigure, GdipStringFormatGetGenericTypographic,
    GdiplusStartup, GdiplusStartupInput, GpBitmap, GpBrush, GpFont, GpFontFamily, GpGraphics,
    GpImage, GpPath, GpPen, GpSolidFill, GpStringFormat, PixelOffsetModeHalf, RectF,
    SmoothingModeAntiAlias, StringFormatFlagsNoClip, StringFormatFlagsNoWrap,
    TextRenderingHintAntiAlias, UnitPixel,
};
use windows::core::PCWSTR;

use crate::color::Color;
use crate::elements::{
    ImageElement, LineElement, PathElement, RectElement, SvgElement, TextElement,
};
use crate::error::RenderError;
use crate::svg;
use crate::text::{TextSize, TextStyle};

/// GDI+ 的进程级初始化（只做一次；`GdiplusShutdown` 故意不调 —— 退出阶段调它偶发卡住）。
fn gdiplus_started() -> bool {
    static STARTED: OnceLock<bool> = OnceLock::new();
    *STARTED.get_or_init(|| unsafe {
        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        GdiplusStartup(&mut token, &input, ptr::null_mut()).0 == 0
    })
}

/// 颜色 → GDI+ 的 ARGB。
fn argb(color: Color) -> u32 {
    u32::from(color.a) << 24
        | u32::from(color.r) << 16
        | u32::from(color.g) << 8
        | u32::from(color.b)
}

/// 颜色 → GDI 的 `COLORREF`（`0x00BBGGRR`）。
fn colorref(color: Color) -> windows::Win32::Foundation::COLORREF {
    windows::Win32::Foundation::COLORREF(
        (color.b as u32) << 16 | (color.g as u32) << 8 | color.r as u32,
    )
}

/// 主题没给字族时用系统界面字体（`DEFAULT_GUI_FONT` 的字族名）。
fn default_face() -> String {
    unsafe {
        let mut logfont = LOGFONTW::default();
        let size = std::mem::size_of::<LOGFONTW>() as i32;
        if GetObjectW(
            GetStockObject(DEFAULT_GUI_FONT),
            size,
            Some(&mut logfont as *mut _ as *mut core::ffi::c_void),
        ) > 0
        {
            let end = logfont
                .lfFaceName
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(0);
            let face = String::from_utf16_lossy(&logfont.lfFaceName[..end]);
            if !face.is_empty() {
                return face;
            }
        }
        "Microsoft YaHei".to_owned()
    }
}

/// 画在窗口 DC 上的那支笔。
pub struct Paint {
    hdc: HDC,

    /// GDI+ 的绘制上下文（抗锯齿靠它）；建不出来就退回纯 GDI（矩形与图像仍然能画）。
    graphics: *mut GpGraphics,

    /// 通用排版格式：不换行、不裁剪、按字形本身排版（不带 GDI+ 那圈默认边距）。
    format: *mut GpStringFormat,

    /// 这次绘制建过的 GDI+ 对象（同样的样式只建一次，析构时统一删）。
    brushes: RefCell<Vec<(u32, *mut GpSolidFill)>>,
    pens: RefCell<Vec<((u32, u32), *mut GpPen)>>,
    families: RefCell<Vec<(String, *mut GpFontFamily)>>,
    fonts: RefCell<Vec<(TextKey, *mut GpFont)>>,
}

/// 字体的「同不同样」看这几个字段。
#[derive(Debug, Clone, PartialEq, Eq)]
struct TextKey {
    face: String,

    /// 字号（像素，取整）。
    size: i32,

    italic: bool,
}

impl Paint {
    /// 直接在 `hdc` 上画 —— DC 由调用方拿（`GetDC` / `BeginPaint`）也由调用方还。
    pub fn new(hdc: HDC) -> Self {
        let mut graphics = ptr::null_mut();
        let mut format = ptr::null_mut();
        if gdiplus_started() {
            unsafe {
                if GdipCreateFromHDC(hdc, &mut graphics).0 != 0 {
                    graphics = ptr::null_mut();
                } else {
                    // 抗锯齿 + 像素对齐（默认以像素中心为原点，整像素线会发虚）
                    let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
                    let _ = GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAlias);
                    let _ = GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
                }
                if GdipStringFormatGetGenericTypographic(&mut format).0 != 0 {
                    format = ptr::null_mut();
                } else {
                    let _ = GdipSetStringFormatFlags(
                        format,
                        StringFormatFlagsNoWrap.0 | StringFormatFlagsNoClip.0,
                    );
                }
            }
        }
        if graphics.is_null() {
            tracing::warn!("GDI+ 起不来，绘制退回纯 GDI（没有抗锯齿）");
        }
        Self {
            hdc,
            graphics,
            format,
            brushes: RefCell::new(Vec::new()),
            pens: RefCell::new(Vec::new()),
            families: RefCell::new(Vec::new()),
            fonts: RefCell::new(Vec::new()),
        }
    }

    /// 一个 GDI+ 实心刷子；同样的颜色只建一次。
    fn brush(&self, color: Color) -> Option<*mut GpSolidFill> {
        if self.graphics.is_null() {
            return None;
        }
        let key = argb(color);
        if let Some((_, brush)) = self.brushes.borrow().iter().find(|(k, _)| *k == key) {
            return Some(*brush);
        }
        let mut brush = ptr::null_mut();
        unsafe {
            if GdipCreateSolidFill(key, &mut brush).0 != 0 {
                return None;
            }
        }
        self.brushes.borrow_mut().push((key, brush));
        Some(brush)
    }

    /// 一支 GDI+ 画笔（`width` 取到十分之一像素）；同样的颜色 / 粗细只建一次。
    fn pen(&self, color: Color, width: f32) -> Option<*mut GpPen> {
        if self.graphics.is_null() {
            return None;
        }
        let key = (argb(color), (width.max(0.1) * 10.0).round() as u32);
        if let Some((_, pen)) = self.pens.borrow().iter().find(|(k, _)| *k == key) {
            return Some(*pen);
        }
        let mut pen = ptr::null_mut();
        unsafe {
            if GdipCreatePen1(key.0, key.1 as f32 / 10.0, UnitPixel, &mut pen).0 != 0 {
                return None;
            }
        }
        self.pens.borrow_mut().push((key, pen));
        Some(pen)
    }

    /// 一个 GDI+ 字体；同样的样式只建一次。
    fn font(&self, style: &TextStyle) -> Option<*mut GpFont> {
        if self.graphics.is_null() {
            return None;
        }
        let key = TextKey {
            face: style.family.clone().unwrap_or_default(),
            size: style.size.round().max(1.0) as i32,
            italic: style.italic,
        };
        if let Some((_, font)) = self.fonts.borrow().iter().find(|(k, _)| *k == key) {
            return Some(*font);
        }
        let family = self.family(&key.face)?;
        let mut font = ptr::null_mut();
        let style_flag = if key.italic {
            FontStyleItalic.0
        } else {
            FontStyleRegular.0
        };
        unsafe {
            if GdipCreateFont(family, key.size as f32, style_flag, UnitPixel, &mut font).0 != 0 {
                return None;
            }
        }
        self.fonts.borrow_mut().push((key, font));
        Some(font)
    }

    /// 一个 GDI+ 字族（`face` 空 = 系统界面字体）。
    fn family(&self, face: &str) -> Option<*mut GpFontFamily> {
        let face = if face.is_empty() {
            default_face()
        } else {
            face.to_owned()
        };
        if let Some((_, family)) = self.families.borrow().iter().find(|(k, _)| *k == face) {
            return Some(*family);
        }
        let wide: Vec<u16> = face.encode_utf16().chain(std::iter::once(0)).collect();
        let mut family = ptr::null_mut();
        unsafe {
            if GdipCreateFontFamilyFromName(PCWSTR(wide.as_ptr()), ptr::null_mut(), &mut family).0
                != 0
            {
                return None;
            }
        }
        self.families.borrow_mut().push((face.to_owned(), family));
        Some(family)
    }

    /// 画一张预乘 RGBA 位图（GDI+ 的 `PARGB` 就是 BGRA 预乘）。
    fn draw_pixmap(&self, pixmap: &Pixmap, x: f32, y: f32, width: f32, height: f32) -> bool {
        if self.graphics.is_null() || width <= 0.0 || height <= 0.0 {
            return false;
        }
        let (pixmap_width, pixmap_height) = (pixmap.width() as i32, pixmap.height() as i32);
        if pixmap_width <= 0 || pixmap_height <= 0 {
            return false;
        }
        let mut bgra: Vec<u8> = Vec::with_capacity((pixmap_width * pixmap_height * 4) as usize);
        for pixel in pixmap.pixels() {
            bgra.extend_from_slice(&[pixel.blue(), pixel.green(), pixel.red(), pixel.alpha()]);
        }
        // `PixelFormat32bppPARGB`：预乘 ARGB，内存里就是 BGRA
        const PARGB: i32 = 0x000E_200B;
        let mut bitmap: *mut GpBitmap = ptr::null_mut();
        unsafe {
            if GdipCreateBitmapFromScan0(
                pixmap_width,
                pixmap_height,
                pixmap_width * 4,
                PARGB,
                Some(bgra.as_ptr()),
                &mut bitmap,
            )
            .0 != 0
            {
                return false;
            }
            let drawn =
                GdipDrawImageRect(self.graphics, bitmap as *mut GpImage, x, y, width, height).0
                    == 0;
            // 位图不拷贝像素（指向 `bgra`），用完立刻释放
            let _ = GdipDisposeImage(bitmap as *mut GpImage);
            drawn
        }
    }

    /// 一条折线 / 贝塞尔组成的填充路径。
    fn build_path(&self, path: &Path) -> Option<*mut GpPath> {
        let mut geometry: *mut GpPath = ptr::null_mut();
        unsafe {
            if GdipCreatePath(FillModeWinding, &mut geometry).0 != 0 {
                return None;
            }
            let mut current = (0.0_f32, 0.0_f32);
            let mut open = false;
            for segment in path.segments() {
                match segment {
                    PathSegment::MoveTo(point) => {
                        if open {
                            let _ = GdipClosePathFigure(geometry);
                        }
                        let _ = GdipStartPathFigure(geometry);
                        current = (point.x, point.y);
                        open = true;
                    }
                    PathSegment::LineTo(point) => {
                        let _ = GdipAddPathLine(geometry, current.0, current.1, point.x, point.y);
                        current = (point.x, point.y);
                    }
                    PathSegment::QuadTo(control, point) => {
                        // 二次贝塞尔升成三次（GDI+ 只有三次）
                        let third = 2.0 / 3.0;
                        let c1 = (
                            current.0 + third * (control.x - current.0),
                            current.1 + third * (control.y - current.1),
                        );
                        let c2 = (
                            point.x + third * (control.x - point.x),
                            point.y + third * (control.y - point.y),
                        );
                        let _ = GdipAddPathBezier(
                            geometry, current.0, current.1, c1.0, c1.1, c2.0, c2.1, point.x,
                            point.y,
                        );
                        current = (point.x, point.y);
                    }
                    PathSegment::CubicTo(control1, control2, point) => {
                        let _ = GdipAddPathBezier(
                            geometry, current.0, current.1, control1.x, control1.y, control2.x,
                            control2.y, point.x, point.y,
                        );
                        current = (point.x, point.y);
                    }
                    PathSegment::Close => {
                        if open {
                            let _ = GdipClosePathFigure(geometry);
                            open = false;
                        }
                    }
                }
            }
            if open {
                let _ = GdipClosePathFigure(geometry);
            }
        }
        Some(geometry)
    }
}

impl Drop for Paint {
    fn drop(&mut self) {
        unsafe {
            for (_, brush) in self.brushes.get_mut().drain(..) {
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
            for (_, pen) in self.pens.get_mut().drain(..) {
                let _ = GdipDeletePen(pen);
            }
            for (_, font) in self.fonts.get_mut().drain(..) {
                let _ = GdipDeleteFont(font);
            }
            for (_, family) in self.families.get_mut().drain(..) {
                let _ = GdipDeleteFontFamily(family);
            }
            if !self.format.is_null() {
                let _ = GdipDeleteStringFormat(self.format);
            }
            if !self.graphics.is_null() {
                let _ = GdipDeleteGraphics(self.graphics);
            }
        }
    }
}

impl TextElement for Paint {
    fn text(&mut self, text: &str, style: &TextStyle, x: f32, y: f32) -> f32 {
        let (Some(font), Some(brush)) = (self.font(style), self.brush(style.color)) else {
            return 0.0;
        };
        let wide: Vec<u16> = text.encode_utf16().collect();
        let layout = RectF {
            X: x,
            Y: y,
            Width: f32::MAX / 4.0,
            Height: style.line_height.max(1.0),
        };
        unsafe {
            let _ = GdipDrawString(
                self.graphics,
                PCWSTR(wide.as_ptr()),
                wide.len() as i32,
                font,
                &layout,
                self.format,
                brush as *const GpBrush,
            );
        }
        self.measure(text, style).width
    }

    fn measure(&self, text: &str, style: &TextStyle) -> TextSize {
        let Some(font) = self.font(style) else {
            return TextSize {
                width: 0.0,
                height: style.line_height,
            };
        };
        let wide: Vec<u16> = text.encode_utf16().collect();
        let layout = RectF {
            X: 0.0,
            Y: 0.0,
            Width: f32::MAX / 4.0,
            Height: f32::MAX / 4.0,
        };
        let mut bounds = RectF::default();
        unsafe {
            if GdipMeasureString(
                self.graphics,
                PCWSTR(wide.as_ptr()),
                wide.len() as i32,
                font,
                &layout,
                self.format,
                &mut bounds,
                ptr::null_mut(),
                ptr::null_mut(),
            )
            .0 != 0
            {
                return TextSize {
                    width: 0.0,
                    height: style.line_height,
                };
            }
        }
        TextSize {
            width: bounds.Width,
            height: style.line_height,
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
        self.draw_pixmap(&icon, x, y, side as f32, side as f32);
        Ok(())
    }
}

impl ImageElement for Paint {
    fn image(&mut self, image: &Pixmap, x: f32, y: f32, width: f32, height: f32) {
        self.draw_pixmap(image, x, y, width, height);
    }
}

impl LineElement for Paint {
    fn line(&mut self, from: (f32, f32), to: (f32, f32), thickness: f32, color: Color) {
        let Some(pen) = self.pen(color, thickness) else {
            return;
        };
        unsafe {
            let _ = GdipDrawLine(self.graphics, pen, from.0, from.1, to.0, to.1);
        }
    }
}

impl RectElement for Paint {
    fn rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        // 不透明的轴对齐矩形不需要抗锯齿：GDI 的 `FillRect` 整像素、快（格线 / 底色都走这条）
        if color.a == 255 && !self.graphics.is_null() {
            let rect = RECT {
                left: x.round() as i32,
                top: y.round() as i32,
                right: (x + width).round() as i32,
                bottom: (y + height).round() as i32,
            };
            unsafe {
                let brush = CreateSolidBrush(colorref(color));
                let _ = FillRect(self.hdc, &rect, brush);
                let _ = DeleteObject(HGDIOBJ(brush.0));
            }
            return;
        }
        let Some(brush) = self.brush(color) else {
            return;
        };
        unsafe {
            let _ = GdipFillRectangle(self.graphics, brush as *mut GpBrush, x, y, width, height);
        }
    }

    fn round_rect(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, color: Color) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let Some(brush) = self.brush(color) else {
            return;
        };
        let radius = radius.max(0.0).min(width.min(height) / 2.0);
        let mut geometry: *mut GpPath = ptr::null_mut();
        if unsafe { GdipCreatePath(FillModeWinding, &mut geometry) }.0 != 0 {
            return;
        }
        let (diameter, right, bottom) = (radius * 2.0, x + width, y + height);
        unsafe {
            // GDI+ 的角：0° = 三点钟，顺时针
            let _ = GdipAddPathArc(
                geometry,
                right - diameter,
                y,
                diameter,
                diameter,
                270.0,
                90.0,
            );
            let _ = GdipAddPathArc(
                geometry,
                right - diameter,
                bottom - diameter,
                diameter,
                diameter,
                0.0,
                90.0,
            );
            let _ = GdipAddPathArc(
                geometry,
                x,
                bottom - diameter,
                diameter,
                diameter,
                90.0,
                90.0,
            );
            let _ = GdipAddPathArc(geometry, x, y, diameter, diameter, 180.0, 90.0);
            let _ = GdipClosePathFigure(geometry);
            let _ = GdipFillPath(self.graphics, brush as *mut GpBrush, geometry);
            let _ = GdipDeletePath(geometry);
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        if radius <= 0.0 {
            return;
        }
        let Some(brush) = self.brush(color) else {
            return;
        };
        unsafe {
            let _ = GdipFillEllipse(
                self.graphics,
                brush as *mut GpBrush,
                cx - radius,
                cy - radius,
                radius * 2.0,
                radius * 2.0,
            );
        }
    }
}

impl PathElement for Paint {
    fn path(&mut self, path: &Path, color: Color) {
        let (Some(brush), Some(geometry)) = (self.brush(color), self.build_path(path)) else {
            return;
        };
        unsafe {
            let _ = GdipFillPath(self.graphics, brush as *mut GpBrush, geometry);
            let _ = GdipDeletePath(geometry);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;

    use super::*;

    /// 一张 40×40 的内存 DC + 32 位 DIB（自上而下）：绘制这条路不依赖窗口，照样能验。
    fn memory_dc(size: i32) -> (HDC, windows::Win32::Graphics::Gdi::HBITMAP, *mut c_void) {
        use windows::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, RGBQUAD, SelectObject,
        };

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
        let mut bits: *mut c_void = ptr::null_mut();
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
        use windows::Win32::Graphics::Gdi::{DeleteDC, DeleteObject};

        let (dc, bitmap, bits) = memory_dc(40);
        {
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
                *pixel =
                    tiny_skia::PremultipliedColorU8::from_rgba(255, 255, 255, 255).expect("颜色");
            }
            paint.image(&image, 2.0, 34.0, 4.0, 4.0);
        }
        unsafe {
            let red = pixel(bits, 40, 5, 5);
            assert_eq!((red.0, red.1, red.2), (0, 0, 255), "矩形是红的（BGRA）");
            let green = pixel(bits, 40, 15, 20);
            assert_eq!(green.1, 255, "线是绿的");
            let blue = pixel(bits, 40, 30, 8);
            assert_eq!(blue.0, 255, "圆是蓝的");
            // 三角形 (24,24)-(38,24)-(38,38) 的内部一点（斜边是 y = x，别取在边上）
            let magenta = pixel(bits, 40, 36, 30);
            assert!(magenta.0 > 200 && magenta.2 > 200, "路径是品红");
            let white = pixel(bits, 40, 3, 35);
            assert_eq!((white.0, white.1, white.2), (255, 255, 255), "图像贴上了");
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(dc);
        }
    }

    /// 文字走 GDI+：量出来的宽度与画上去的一致，并且真的落到像素上。
    #[test]
    fn text_measures_and_draws_with_the_same_toolkit() {
        use windows::Win32::Graphics::Gdi::{DeleteDC, DeleteObject};

        let (dc, bitmap, bits) = memory_dc(60);
        let style = TextStyle {
            size: 16.0,
            points: 16.0,
            line_height: 19.0,
            family: None,
            color: Color::rgb(0, 0, 0),
            strike: false,
            underline: false,
            italic: false,
            gamma: 1.0,
        };
        {
            let mut paint = Paint::new(dc);
            let measured = paint.measure("国国", &style);
            let drawn = paint.text("国国", &style, 2.0, 2.0);
            assert!(measured.width > 0.0, "量得出来");
            assert_eq!(measured.height, 19.0, "高度就是行高");
            assert!(
                (drawn - measured.width).abs() < 0.5,
                "画的宽度与量的一致（{drawn} vs {}）",
                measured.width
            );
        }
        unsafe {
            let mut dark = 0;
            for y in 2..24 {
                for x in 2..40 {
                    let (b, g, r, _) = pixel(bits, 60, x, y);
                    if (b as i32 + g as i32 + r as i32) / 3 < 128 {
                        dark += 1;
                    }
                }
            }
            assert!(dark > 5, "文字落到像素上（暗像素 {dark} 个）");
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(dc);
        }
    }
}
