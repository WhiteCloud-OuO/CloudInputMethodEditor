//! **图像**（`img`）元素：从**文件**或**文件句柄**读一张图（PNG / BMP / JPG），解码成预乘 RGBA 位图；
//! 画的时候按目标矩形缩放叠上去。
//!
//! 这是「能画的元素」里的 `img`（见 `docs/design/windows.md` §6）：
//! `text` 在 `text`、`svg` 在 [`crate::svg`]、`line` / `rect` / `path` 都走画布那几笔填充原语，
//! 图像这条以前只能叠**已经在内存里**的位图（`Canvas::blend_pixmap`），现在多了「读进来」这一步。
//!
//! 解码：PNG 优先走 tiny-skia（与编码、光栅化同一套）；BMP / JPG 交给 `image`（只开解码特性）。

use std::io::Read;
use std::path::Path;

use tiny_skia::{
    BlendMode, FillRule, FilterQuality, Mask, Pixmap, PixmapPaint, PremultipliedColorU8, Transform,
};

use crate::document::{PathCommand, build_path};
use crate::error::RenderError;

/// 从**文件**读一张图并解码（PNG / BMP / JPG）。
pub fn load_image(path: &Path) -> Result<Pixmap, RenderError> {
    let file = std::fs::File::open(path).map_err(|error| RenderError::InvalidImage {
        message: format!("打开 {} 失败：{error}", path.display()),
    })?;
    read_image(file)
}

/// 从**文件句柄**读一张图并解码 —— 任何 `Read` 都行（`File` / `Cursor` / 管道 / 内存里的字节流）。
pub fn read_image(reader: impl Read) -> Result<Pixmap, RenderError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_IMAGE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| RenderError::InvalidImage {
            message: format!("读图像失败：{error}"),
        })?;
    decode_image(&bytes)
}

/// 从字节解码成预乘 RGBA 位图：PNG（tiny-skia）优先，其余交给 `image`（BMP / JPG）。
pub fn decode_image(bytes: &[u8]) -> Result<Pixmap, RenderError> {
    if let Ok(pixmap) = Pixmap::decode_png(bytes) {
        return Ok(pixmap);
    }
    let decoded = image::load_from_memory(bytes).map_err(|error| RenderError::InvalidImage {
        message: error.to_string(),
    })?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut pixmap =
        Pixmap::new(width, height).ok_or(RenderError::InvalidSize { width, height })?;
    for (target, source) in pixmap.pixels_mut().iter_mut().zip(rgba.pixels()) {
        *target = PremultipliedColorU8::from_rgba(source[0], source[1], source[2], source[3])
            .unwrap_or(PremultipliedColorU8::TRANSPARENT);
    }
    Ok(pixmap)
}

/// **缩放**：按目标宽高重采样（双线性）；尺寸至少 1，像素数不能超过 [`MAX_IMAGE_PIXELS`]。
pub fn scale_image(image: &Pixmap, width: u32, height: u32) -> Result<Pixmap, RenderError> {
    let width = width.max(1);
    let height = height.max(1);
    if u64::from(width) * u64::from(height) > u64::from(MAX_IMAGE_PIXELS) {
        return Err(RenderError::InvalidSize { width, height });
    }
    let mut target =
        Pixmap::new(width, height).ok_or(RenderError::InvalidSize { width, height })?;
    draw_image(&mut target, image, 0.0, 0.0, width as f32, height as f32);
    Ok(target)
}

/// **几何旋转**：只支持 90 / 180 / 270 度（顺时针）。
pub fn rotate_image(image: &Pixmap, degrees: u16) -> Result<Pixmap, RenderError> {
    let (width, height) = (image.width(), image.height());
    let (new_width, new_height) = match degrees {
        90 | 270 => (height, width),
        180 => (width, height),
        other => {
            return Err(RenderError::InvalidImage {
                message: format!("旋转只支持 90 / 180 / 270 度，给的是 {other}"),
            });
        }
    };
    let mut target = Pixmap::new(new_width, new_height).ok_or(RenderError::InvalidSize {
        width: new_width,
        height: new_height,
    })?;
    for y in 0..new_height {
        for x in 0..new_width {
            let (sx, sy) = match degrees {
                90 => (y, height - 1 - x),
                180 => (width - 1 - x, height - 1 - y),
                _ => (width - 1 - y, x),
            };
            if let Some(pixel) = image.pixel(sx, sy) {
                target.pixels_mut()[(y * new_width + x) as usize] = pixel;
            }
        }
    }
    Ok(target)
}

/// **几何翻转**：水平 / 垂直。
pub fn flip_image(image: &Pixmap, horizontal: bool) -> Pixmap {
    let (width, height) = (image.width(), image.height());
    let mut target = Pixmap::new(width, height).expect("同尺寸位图");
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = if horizontal {
                (width - 1 - x, y)
            } else {
                (x, height - 1 - y)
            };
            if let Some(pixel) = image.pixel(sx, sy) {
                target.pixels_mut()[(y * width + x) as usize] = pixel;
            }
        }
    }
    target
}

/// **灰度化**：按 BT.601 权重（0.299 / 0.587 / 0.114）把 RGB 算成 0–255，alpha 不变。
pub fn gray_image(image: &Pixmap) -> Pixmap {
    let mut target = Pixmap::new(image.width(), image.height()).expect("同尺寸位图");
    for (source, destination) in image.pixels().iter().zip(target.pixels_mut()) {
        let gray = (299 * u32::from(source.red())
            + 587 * u32::from(source.green())
            + 114 * u32::from(source.blue()))
            / 1000;
        let gray = gray as u8;
        *destination = PremultipliedColorU8::from_rgba(gray, gray, gray, source.alpha())
            .unwrap_or(PremultipliedColorU8::TRANSPARENT);
    }
    target
}

/// **路径蒙版**：路径坐标按**图片像素**算，只留下路径内部（非零环绕）。
pub fn mask_image(image: &Pixmap, commands: &[PathCommand]) -> Result<Pixmap, RenderError> {
    let Some(path) = build_path(commands, 1.0, 0.0, 0.0) else {
        return Err(RenderError::InvalidImage {
            message: "路径蒙版画不出轮廓".to_owned(),
        });
    };
    let (width, height) = (image.width(), image.height());
    let mut mask = Mask::new(width, height).ok_or(RenderError::InvalidSize { width, height })?;
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
    let mut target = image.clone();
    target.apply_mask(&mask);
    Ok(target)
}

/// 把一张图按**目标矩形**（像素）缩放画到 `target` 上：左上角 `(x, y)`、宽 `width`、高 `height`。
/// 超出 `target` 的部分自然裁掉（与别的元素一样，「画得下多少算多少」）。
///
/// 缩放用双线性（`FilterQuality::Bilinear`）：界面里的图多是缩放显示的，最近邻会出锯齿。
pub fn draw_image(target: &mut Pixmap, image: &Pixmap, x: f32, y: f32, width: f32, height: f32) {
    let (source_width, source_height) = (image.width() as f32, image.height() as f32);
    if source_width <= 0.0 || source_height <= 0.0 || width <= 0.0 || height <= 0.0 {
        return;
    }
    let transform =
        Transform::from_translate(x, y).pre_scale(width / source_width, height / source_height);
    let paint = PixmapPaint {
        quality: FilterQuality::Bilinear,
        blend_mode: BlendMode::SourceOver,
        ..PixmapPaint::default()
    };
    target.draw_pixmap(0, 0, image.as_ref(), &paint, transform, None);
}

/// 读图的上限（防一份坏文件把内存吃光）。
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

/// 缩放后的像素数上限（与脚本窗口的图像上限一致）。
const MAX_IMAGE_PIXELS: u32 = 16 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    use tiny_skia::PremultipliedColorU8;

    /// 一张 `width`×`height` 的图，每个像素一个颜色（`colors` 按行优先给）。
    fn pixmap(width: u32, height: u32, colors: &[(u8, u8, u8, u8)]) -> Pixmap {
        let mut pixmap = Pixmap::new(width, height).expect("位图");
        for (pixel, &(r, g, b, a)) in pixmap.pixels_mut().iter_mut().zip(colors) {
            *pixel = PremultipliedColorU8::from_rgba(r, g, b, a).expect("颜色");
        }
        pixmap
    }

    /// 一张 2×2 的彩图：红、绿 / 蓝、白。
    fn quarters() -> Pixmap {
        pixmap(
            2,
            2,
            &[
                (255, 0, 0, 255),
                (0, 255, 0, 255),
                (0, 0, 255, 255),
                (255, 255, 255, 255),
            ],
        )
    }

    /// 编出来再解回去，像素不变（解码走的是 tiny-skia，与截图 / 导出用的是同一套 PNG）。
    #[test]
    fn decodes_what_it_encoded() {
        let source = quarters();
        let bytes = source.encode_png().expect("编码 PNG");
        let decoded = decode_image(&bytes).expect("解码 PNG");
        assert_eq!((decoded.width(), decoded.height()), (2, 2));
        assert_eq!(decoded.pixels(), source.pixels());
    }

    /// 文件句柄与文件两条路都能读。
    #[test]
    fn reads_from_a_handle_and_from_a_file() {
        let bytes = quarters().encode_png().expect("编码 PNG");

        let from_handle = read_image(std::io::Cursor::new(bytes.clone())).expect("从句柄解码");
        assert_eq!(from_handle.pixels(), quarters().pixels());

        let path =
            std::env::temp_dir().join(format!("cloudime-render-image-{}.png", std::process::id()));
        std::fs::write(&path, &bytes).expect("写临时文件");
        let from_file = load_image(&path).expect("从文件解码");
        let _ = std::fs::remove_file(&path);
        assert_eq!(from_file.pixels(), quarters().pixels());
    }

    /// 2×2、24 位的 BMP（底行红 / 绿，顶行蓝 / 白）：`image` 解码后颜色与方向都对。
    #[test]
    fn decodes_bmp_as_well_as_png() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&70u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&24u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // 底行（BMP 从下往上）：红、绿，再补两个对齐字节
        bytes.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
        // 顶行：蓝、白，再补两个对齐字节
        bytes.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]);

        let pixmap = decode_image(&bytes).expect("BMP 解得出来");
        assert_eq!((pixmap.width(), pixmap.height()), (2, 2));
        let top_left = pixmap.pixel(0, 0).unwrap();
        assert_eq!(
            (top_left.red(), top_left.green(), top_left.blue()),
            (0, 0, 255),
            "顶行是蓝色"
        );
        let bottom_left = pixmap.pixel(0, 1).unwrap();
        assert_eq!(
            (bottom_left.red(), bottom_left.green(), bottom_left.blue()),
            (255, 0, 0),
            "底行是红色"
        );
    }

    /// 缩放 / 旋转 / 翻转 / 灰度 / 路径蒙版：尺寸与颜色都按定义走。
    #[test]
    fn image_transformations_do_what_they_say() {
        let source = quarters();
        let scaled = scale_image(&source, 4, 4).expect("缩放");
        assert_eq!((scaled.width(), scaled.height()), (4, 4));

        let rotated = rotate_image(&source, 90).expect("旋转");
        assert_eq!((rotated.width(), rotated.height()), (2, 2));
        let top_left = rotated.pixel(0, 0).unwrap();
        assert_eq!(
            (top_left.red(), top_left.green(), top_left.blue()),
            (0, 0, 255),
            "顺时针 90 度后左上角是原图左下角（蓝）"
        );

        let flipped = flip_image(&source, true);
        let top_left = flipped.pixel(0, 0).unwrap();
        assert_eq!(
            (top_left.red(), top_left.green(), top_left.blue()),
            (0, 255, 0),
            "水平翻转后左上角是原图右上角（绿）"
        );

        let gray = gray_image(&source);
        let red = gray.pixel(0, 0).unwrap();
        assert_eq!((red.red(), red.green(), red.blue()), (76, 76, 76));
        assert_eq!(red.alpha(), 255);

        let masked = mask_image(
            &source,
            &[
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(1.0, 0.0),
                PathCommand::LineTo(1.0, 2.0),
                PathCommand::LineTo(0.0, 2.0),
                PathCommand::Close,
            ],
        )
        .expect("蒙版");
        assert_eq!(masked.pixel(0, 0).unwrap().alpha(), 255, "左半边保留");
        assert_eq!(masked.pixel(1, 0).unwrap().alpha(), 0, "右半边被蒙掉");
    }

    /// 不是 PNG 就报 `InvalidImage`（不 panic、不画半张）。
    #[test]
    fn rejects_garbage() {
        let error = decode_image(b"not a png").expect_err("坏数据该报错");
        assert!(matches!(error, RenderError::InvalidImage { .. }), "{error}");
    }

    /// 1:1 画到指定位置：像素原样；放大用双线性，纯色图放大后整块都是那个颜色（角上不会被混）。
    #[test]
    fn draws_an_image_into_the_target_rect() {
        let mut target = Pixmap::new(6, 6).expect("位图");
        draw_image(&mut target, &quarters(), 0.0, 0.0, 2.0, 2.0);
        assert_eq!(
            target.pixel(0, 0).map(|p| (p.red(), p.green())),
            Some((255, 0))
        );
        assert_eq!(
            target.pixel(1, 0).map(|p| (p.red(), p.green())),
            Some((0, 255))
        );
        assert_eq!(
            target.pixel(0, 1).map(|p| (p.red(), p.blue())),
            Some((0, 255))
        );
        assert_eq!(
            target.pixel(0, 2).map(|p| p.alpha()),
            Some(0),
            "画到 2×2 为止"
        );

        // 纯色 1×1 放大到 4×4：整块都该是它（只差舍入）
        let solid = pixmap(1, 1, &[(64, 128, 192, 255)]);
        let mut scaled = Pixmap::new(4, 4).expect("位图");
        draw_image(&mut scaled, &solid, 0.0, 0.0, 4.0, 4.0);
        for y in 0..4 {
            for x in 0..4 {
                let pixel = scaled.pixel(x, y).expect("像素");
                assert_eq!(
                    (pixel.red(), pixel.green(), pixel.blue()),
                    (64, 128, 192),
                    "({x}, {y})"
                );
            }
        }
    }
}
