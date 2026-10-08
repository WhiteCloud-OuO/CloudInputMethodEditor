//! 悬浮状态条（Windows）：一排一样大的 SVG 图标按钮。
//! 每格一张 [`StatusCell::Icon`]，尺寸与间距按点写死（[`BUTTON_SIZE`] / [`BUTTON_GAP`]），按钮之间不再分隔线。

mod cell;
mod rendered;

pub use cell::StatusCell;
pub use rendered::RenderedStatus;

use super::{Metrics, Rendered, Renderer};
use crate::canvas::Canvas;
use crate::color::Color;
use crate::error::RenderError;
use crate::shadow::Shadow;
use crate::svg::draw_svg;
use crate::theme::Theme;

/// 图标按钮外接方框的边长（点）。
const BUTTON_SIZE: f32 = 20.0;

/// 按钮之间、按钮与状态条四周的间距（点）。
const BUTTON_GAP: f32 = 6.0;

impl Renderer {
    /// 画状态条：一排图标按钮。返回位图与各按钮右边界（内容坐标，供点击命中）。
    /// `background` / `icon` 由调用方按窗口给（悬浮工具栏、状态切换提示各一套），
    /// 图标按 alpha 染成 `icon`。
    pub fn render_status(
        &self,
        cells: &[StatusCell],
        theme: &Theme,
        scale: f32,
        shadow: Option<&Shadow>,
        background: Color,
        icon: Color,
    ) -> Result<RenderedStatus, RenderError> {
        let metrics = Metrics { theme, scale };
        let size = metrics.px(BUTTON_SIZE);
        let gap = metrics.px(BUTTON_GAP);
        // 宽 = n 个按钮 + （n − 1）个按钮间距 + 两侧各一个边距；高 = 一个按钮 + 上下边距
        let buttons = cells.len() as f32;
        let between = cells.len().saturating_sub(1) as f32;
        let content_width = (buttons * size + (between + 2.0) * gap).ceil();
        let content_height = (size + gap * 2.0).ceil();
        let margin = shadow.map_or(0.0, |s| metrics.px(s.margin()));
        let width = (content_width + margin * 2.0).ceil();
        let height = (content_height + margin * 2.0).ceil();
        let mut canvas = Canvas::new(width as u32, height as u32)?;
        let radius = metrics.corner_radius();
        if let Some(shadow) = shadow
            && let Some(content) =
                tiny_skia::Rect::from_xywh(margin, margin, content_width, content_height)
        {
            shadow.paint(&mut canvas, content, radius, scale);
        }
        canvas.fill_round_rect(
            margin,
            margin,
            content_width,
            content_height,
            radius,
            background,
        );
        let top = margin + gap;
        let mut left = margin + gap;
        let mut edges = Vec::with_capacity(cells.len());
        for cell in cells {
            draw_svg(&mut canvas, cell.svg(), left, top, size, Some(icon))?;
            left += size;
            edges.push(left - margin);
            left += gap;
        }
        Ok(RenderedStatus {
            rendered: Rendered {
                pixmap: canvas.into_pixmap(),
                content_x: margin as u32,
                content_y: margin as u32,
                content_width: content_width as u32,
                content_height: content_height as u32,
                scale,
                highlight_rects: Vec::new(),
            },
            cell_edges: edges,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{BUTTON_GAP, BUTTON_SIZE, StatusCell};
    use crate::fonts::FontLibrary;
    use crate::renderer::Renderer;
    use crate::shadow::Shadow;
    use crate::theme::Theme;

    /// 一整块红色方块。
    const RED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><rect width="20" height="20" fill="#ff0000"/></svg>"##;

    fn renderer() -> Option<Renderer> {
        // 没有系统字体的环境（CI 容器）跳过
        FontLibrary::system("zh-CN").ok().map(Renderer::new)
    }

    #[test]
    fn buttons_are_square_with_fixed_gaps() {
        let Some(renderer) = renderer() else {
            return;
        };
        let cells = [StatusCell::icon(RED), StatusCell::icon(RED)];
        let theme = Theme::new();
        let out = renderer
            .render_status(
                &cells,
                &theme,
                1.0,
                None,
                theme.colors.bar_background,
                theme.colors.bar_icon,
            )
            .unwrap();
        // 两个 20 pt 的方按钮 + 中间一个 6 pt 间距 + 两侧各 6 pt
        assert_eq!(
            out.rendered.content_width,
            (BUTTON_SIZE * 2.0 + BUTTON_GAP * 3.0) as u32
        );
        assert_eq!(
            out.rendered.content_height,
            (BUTTON_SIZE + BUTTON_GAP * 2.0) as u32
        );
        assert_eq!(out.cell_edges.len(), 2);
        assert_eq!(out.cell_edges[0], BUTTON_SIZE + BUTTON_GAP);
        assert_eq!(out.cell_edges[1], BUTTON_SIZE * 2.0 + BUTTON_GAP * 2.0);
    }

    #[test]
    fn icon_is_painted_inside_the_bar() {
        let Some(renderer) = renderer() else {
            return;
        };
        let theme = Theme::new();
        let out = renderer
            .render_status(
                &[StatusCell::icon(RED)],
                &theme,
                2.0,
                Some(&Shadow::panel()),
                theme.colors.bar_background,
                theme.colors.bar_icon,
            )
            .unwrap();
        // 图标按 alpha 染成主题色（深色），落在白底上就是非白像素；阴影也一起证明留了边
        let painted = out
            .rendered
            .pixmap
            .pixels()
            .iter()
            .any(|p| p.alpha() > 0 && p.red() < 200);
        assert!(painted, "SVG 图标没画上去");
        assert!(out.rendered.pixmap.width() > out.rendered.content_width);
        assert!(out.rendered.content_x > 0);
    }
}
