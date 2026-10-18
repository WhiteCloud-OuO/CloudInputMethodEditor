//! 位图画布：tiny-skia `Pixmap` 之上的几个填充原语，加字形位图的逐像素 source-over 混合。坐标一律是像素、左上角原点。

use tiny_skia::{
    BlendMode, FillRule, Paint, Path, PathBuilder, Pixmap, PremultipliedColorU8, Rect, Transform,
};

use crate::color::{Color, mul_u8, premultiply};
use crate::error::RenderError;

pub(crate) struct Canvas {
    /// 预乘 RGBA。
    pixmap: Pixmap,
}

impl Canvas {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, RenderError> {
        Pixmap::new(width, height)
            .map(|pixmap| Self { pixmap })
            .ok_or(RenderError::InvalidSize { width, height })
    }

    pub(crate) fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub(crate) fn height(&self) -> u32 {
        self.pixmap.height()
    }

    pub(crate) fn into_pixmap(self) -> Pixmap {
        self.pixmap
    }

    pub(crate) fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        let Some(rect) = Rect::from_xywh(x, y, width, height) else {
            return;
        };
        self.pixmap.fill_rect(
            rect,
            &paint(color, BlendMode::SourceOver),
            Transform::identity(),
            None,
        );
    }

    pub(crate) fn fill_round_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        color: Color,
    ) {
        if let Some(path) = round_rect(x, y, width, height, radius) {
            self.fill_path(&path, color, BlendMode::SourceOver);
        }
    }

    /// 圆角矩形描边（线宽 `thickness` 像素，居中在路径上）。
    pub(crate) fn stroke_round_rect(
        &mut self,
        rect: Rect,
        radius: f32,
        thickness: f32,
        color: Color,
    ) {
        let Some(path) = round_rect(rect.x(), rect.y(), rect.width(), rect.height(), radius) else {
            return;
        };
        let stroke = tiny_skia::Stroke {
            width: thickness.max(0.0),
            ..tiny_skia::Stroke::default()
        };
        self.pixmap.stroke_path(
            &path,
            &paint(color, BlendMode::SourceOver),
            &stroke,
            Transform::identity(),
            None,
        );
    }

    pub(crate) fn fill_path(&mut self, path: &Path, color: Color, blend: BlendMode) {
        self.pixmap.fill_path(
            path,
            &paint(color, blend),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    /// 一条粗 `thickness` 的线（任意角度）：拿一个细长的四边形填出来，带抗锯齿。
    pub(crate) fn line(&mut self, from: (f32, f32), to: (f32, f32), thickness: f32, color: Color) {
        let (x1, y1) = from;
        let (x2, y2) = to;
        let half = (thickness / 2.0).max(0.0);
        let (dx, dy) = (x2 - x1, y2 - y1);
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f32::EPSILON || half <= 0.0 {
            return;
        }
        let (nx, ny) = (-dy / length * half, dx / length * half);
        let mut builder = PathBuilder::new();
        builder.move_to(x1 + nx, y1 + ny);
        builder.line_to(x2 + nx, y2 + ny);
        builder.line_to(x2 - nx, y2 - ny);
        builder.line_to(x1 - nx, y1 - ny);
        builder.close();
        if let Some(path) = builder.finish() {
            self.fill_path(&path, color, BlendMode::SourceOver);
        }
    }

    /// 圆：四段三次贝塞尔近似（控制点系数与圆角矩形同一套）。
    pub(crate) fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        if radius <= 0.0 {
            return;
        }
        const KAPPA: f32 = 0.552_284_8;
        let k = radius * KAPPA;
        let mut builder = PathBuilder::new();
        builder.move_to(cx + radius, cy);
        builder.cubic_to(cx + radius, cy + k, cx + k, cy + radius, cx, cy + radius);
        builder.cubic_to(cx - k, cy + radius, cx - radius, cy + k, cx - radius, cy);
        builder.cubic_to(cx - radius, cy - k, cx - k, cy - radius, cx, cy - radius);
        builder.cubic_to(cx + k, cy - radius, cx + radius, cy - k, cx + radius, cy);
        builder.close();
        if let Some(path) = builder.finish() {
            self.fill_path(&path, color, BlendMode::SourceOver);
        }
    }

    /// 底下的位图（图像元素要往上画）。
    pub(crate) fn pixmap_mut(&mut self) -> &mut Pixmap {
        &mut self.pixmap
    }

    /// 把另一张位图整张叠上来（左上角对齐到 `(x, y)`）。
    pub(crate) fn blend_pixmap(&mut self, x: i32, y: i32, other: &Pixmap) {
        let width = other.width();
        for (i, pixel) in other.pixels().iter().enumerate() {
            if pixel.alpha() == 0 {
                continue;
            }
            let px = x + (i as u32 % width) as i32;
            let py = y + (i as u32 / width) as i32;
            self.blend_pixel(px, py, *pixel);
        }
    }

    /// 8 位覆盖率遮罩（普通字形）按颜色叠上来。
    ///
    /// 字形合成是文字那条路的热点：先按覆盖率把颜色**预乘成一张 256 项的表**（每像素少 4 次乘法），
    /// 再把矩形裁到画布内、按行切片逐像素叠 —— 不再每像素判一次边界。
    pub(crate) fn blend_mask(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        data: &[u8],
        color: Color,
    ) {
        if width == 0 || height == 0 || data.len() < (width * height) as usize {
            return;
        }
        let mut coverage_lut = [PremultipliedColorU8::TRANSPARENT; 256];
        for (coverage, slot) in coverage_lut.iter_mut().enumerate().skip(1) {
            *slot = color.premultiplied(coverage as u8);
        }
        let canvas_width = self.pixmap.width() as i32;
        let canvas_height = self.pixmap.height() as i32;
        let left = x.max(0);
        let top = y.max(0);
        let right = (x + width as i32).min(canvas_width);
        let bottom = (y + height as i32).min(canvas_height);
        if right <= left || bottom <= top {
            return;
        }
        let (visible_width, visible_height) = ((right - left) as usize, (bottom - top) as usize);
        let mask_left = (left - x) as usize;
        let first_mask_row = (top - y) as usize;
        for row in 0..visible_height {
            let mask_start = (first_mask_row + row) * width as usize + mask_left;
            let mask_row = &data[mask_start..mask_start + visible_width];
            let pixel_start = (top as usize + row) * canvas_width as usize + left as usize;
            let pixels = &mut self.pixmap.pixels_mut()[pixel_start..pixel_start + visible_width];
            for (dst, &coverage) in pixels.iter_mut().zip(mask_row) {
                if coverage == 0 {
                    continue;
                }
                blend_over(dst, coverage_lut[coverage as usize]);
            }
        }
    }

    /// 直通 RGBA 位图（彩色 emoji）叠上来。
    pub(crate) fn blend_rgba(&mut self, x: i32, y: i32, width: u32, height: u32, data: &[u8]) {
        for (i, px) in data.chunks_exact(4).enumerate() {
            if px[3] == 0 {
                continue;
            }
            let col = (i as u32 % width) as i32;
            let row = (i as u32 / width) as i32;
            if row >= height as i32 {
                return;
            }
            self.blend_pixel(x + col, y + row, premultiply(px[0], px[1], px[2], px[3]));
        }
    }

    /// source-over：`dst = src + dst × (1 − src.a)`。
    fn blend_pixel(&mut self, x: i32, y: i32, src: PremultipliedColorU8) {
        if x < 0 || y < 0 || x >= self.pixmap.width() as i32 || y >= self.pixmap.height() as i32 {
            return;
        }
        let index = y as usize * self.pixmap.width() as usize + x as usize;
        blend_over(&mut self.pixmap.pixels_mut()[index], src);
    }
}

/// source-over 的单像素合成：`dst = src + dst × (1 − src.a)`（预乘 RGBA）。
fn blend_over(dst: &mut PremultipliedColorU8, src: PremultipliedColorU8) {
    let inverse = 255 - src.alpha();
    let alpha = src.alpha().saturating_add(mul_u8(dst.alpha(), inverse));
    let channel = |source: u8, back: u8| source.saturating_add(mul_u8(back, inverse)).min(alpha);
    if let Some(out) = PremultipliedColorU8::from_rgba(
        channel(src.red(), dst.red()),
        channel(src.green(), dst.green()),
        channel(src.blue(), dst.blue()),
        alpha,
    ) {
        *dst = out;
    }
}

fn paint(color: Color, blend: BlendMode) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.to_skia());
    paint.anti_alias = true;
    paint.blend_mode = blend;
    paint
}

/// 圆角矩形路径；圆角用三次贝塞尔近似四分之一圆。
pub(crate) fn round_rect(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<Path> {
    let r = radius.min(width / 2.0).min(height / 2.0).max(0.0);
    if r <= 0.0 {
        return Rect::from_xywh(x, y, width, height).map(PathBuilder::from_rect);
    }
    // 四分之一圆的贝塞尔控制点系数
    const KAPPA: f32 = 0.552_284_8;
    let k = r * KAPPA;
    let (right, bottom) = (x + width, y + height);
    let mut path = PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(right - r, y);
    path.cubic_to(right - r + k, y, right, y + r - k, right, y + r);
    path.line_to(right, bottom - r);
    path.cubic_to(
        right,
        bottom - r + k,
        right - r + k,
        bottom,
        right - r,
        bottom,
    );
    path.line_to(x + r, bottom);
    path.cubic_to(x + r - k, bottom, x, bottom - r + k, x, bottom - r);
    path.line_to(x, y + r);
    path.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    path.close();
    path.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blends_source_over_and_clips_to_bounds() {
        let mut canvas = Canvas::new(2, 1).unwrap();
        canvas.fill_rect(0.0, 0.0, 2.0, 1.0, Color::rgb(0, 0, 255));
        canvas.blend_mask(0, 0, 1, 1, &[255], Color::rgb(255, 0, 0));
        canvas.blend_mask(5, 5, 1, 1, &[255], Color::rgb(255, 0, 0));
        let pixmap = canvas.into_pixmap();
        let red = pixmap.pixel(0, 0).unwrap();
        assert_eq!((red.red(), red.blue(), red.alpha()), (255, 0, 255));
        let blue = pixmap.pixel(1, 0).unwrap();
        assert_eq!((blue.red(), blue.blue()), (0, 255));
    }

    #[test]
    fn half_alpha_over_white_lightens() {
        let mut canvas = Canvas::new(1, 1).unwrap();
        canvas.fill_rect(0.0, 0.0, 1.0, 1.0, Color::rgb(255, 255, 255));
        canvas.blend_mask(0, 0, 1, 1, &[255], Color::gray(0, 128));
        let px = canvas.into_pixmap().pixel(0, 0).unwrap();
        assert_eq!(px.alpha(), 255);
        assert!((px.red() as i32 - 127).abs() <= 1, "got {}", px.red());
    }
}
