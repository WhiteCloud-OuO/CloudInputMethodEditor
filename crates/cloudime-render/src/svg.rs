//! SVG 图标栅格化：`usvg` 解析（resvg 自带）+ `resvg` 光栅化，出一张预乘 alpha 位图整张叠进画布。
//! resvg 关掉了默认特性：图标里没有文字，所以不引入系统字体扫描、svgz 解压与光栅图解码。

use resvg::usvg::{self, Transform};
use tiny_skia::Pixmap;

use crate::canvas::Canvas;
use crate::color::Color;
use crate::error::RenderError;

/// 把 `svg` 画进 `(x, y)` 为左上角、边长 `size` 像素的方块：按 SVG 自己的宽高比等比缩放，在方框里居中、不裁剪。
/// `color` 给 `Some` 时**按 alpha 把整张图标染成这个颜色**（SVG 里写的是什么色都不管，只取轮廓）。
pub(crate) fn draw_svg(
    canvas: &mut Canvas,
    svg: &str,
    x: f32,
    y: f32,
    size: f32,
    color: Option<Color>,
) -> Result<(), RenderError> {
    let side = size.ceil().max(1.0) as u32;
    let mut icon = rasterize(svg, side)?;
    if let Some(color) = color {
        tint(&mut icon, color);
    }
    canvas.blend_pixmap(x.round() as i32, y.round() as i32, &icon);
    Ok(())
}

/// 按每个像素的 alpha 把整张图标染成 `color`。
fn tint(icon: &mut Pixmap, color: Color) {
    for pixel in icon.pixels_mut() {
        *pixel = color.premultiplied(pixel.alpha());
    }
}

/// 解析 + 栅格化成 `side × side` 的位图，两边装不下就留白（`usvg::Size` 保证宽高为正，零尺寸在解析时就被拒）。
fn rasterize(svg: &str, side: u32) -> Result<Pixmap, RenderError> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).map_err(|error| {
        RenderError::InvalidSvg {
            message: error.to_string(),
        }
    })?;
    let size = tree.size();
    let extent = side as f32;
    let scale = (extent / size.width()).min(extent / size.height());
    let offset_x = (extent - size.width() * scale) / 2.0;
    let offset_y = (extent - size.height() * scale) / 2.0;
    let mut pixmap = Pixmap::new(side, side).ok_or(RenderError::InvalidSize {
        width: side,
        height: side,
    })?;
    // 行主序矩阵：先按原点缩放、再平移进方框正中
    let transform = Transform::from_row(scale, 0.0, 0.0, scale, offset_x, offset_y);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

#[cfg(test)]
mod tests {
    use super::rasterize;

    /// 宽是高的两倍：装进方框会左右满、上下留白。
    const RED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 1"><rect width="2" height="1" fill="#ff0000"/></svg>"##;

    #[test]
    fn fills_the_box_width_and_letterboxes_vertically() {
        let pixmap = rasterize(RED, 20).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (20, 20));
        let middle = pixmap.pixel(10, 10).unwrap();
        assert_eq!((middle.red(), middle.green(), middle.blue()), (255, 0, 0));
        assert_eq!(middle.alpha(), 255);
        assert_eq!(pixmap.pixel(10, 0).unwrap().alpha(), 0);
        assert_eq!(pixmap.pixel(10, 19).unwrap().alpha(), 0);
    }

    #[test]
    fn rejects_malformed_svg() {
        let error = rasterize("not an svg", 10).unwrap_err();
        assert!(error.to_string().contains("invalid SVG icon"));
    }
}
