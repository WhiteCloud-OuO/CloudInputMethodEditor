//! 横排：候选排成一行，高亮那个下面单独一行译文，页码在行尾。

use super::item::Item;
use super::{HIGHLIGHT_INSET, INDEX_GAP, Metrics, Renderer, highlight_rect};
use crate::canvas::Canvas;
use crate::frame::{Frame, HighlightRect, Row};

/// 一项的内容宽度：序号 + 候选词 +（有角标时）间隔 + 角标。
/// `badge_gap` 是候选词与角标之间的间隔（横排用两个候选字宽，见 `Renderer::horizontal_badge_gap`）。
fn item_width(item: &Item, m: &Metrics, badge_gap: f32) -> f32 {
    let badge = if item.badge_width > 0.0 {
        badge_gap + item.badge_width
    } else {
        0.0
    };
    item.index_width + m.px(INDEX_GAP) + item.text_width + badge
}

impl Renderer {
    pub(super) fn horizontal_size(&mut self, frame: &Frame, m: &Metrics) -> (f32, f32) {
        if frame.rows.is_empty() {
            return (0.0, 0.0);
        }
        let badge_gap = self.horizontal_badge_gap(m);
        let (items, row_height) = self.items(&frame.rows, m);
        let mut width: f32 = items
            .iter()
            .map(|item| item_width(item, m, badge_gap))
            .sum::<f32>()
            + m.column_gap() * items.len().saturating_sub(1) as f32
            + m.px(HIGHLIGHT_INSET) * 2.0;
        if let Some(footer) = frame.footer.as_deref() {
            width = width.max(m.column_gap() + self.measure(footer, &m.footer_style()).width);
        }
        let mut height = row_height + self.info_height(frame, m);
        if let Some((annotation_width, annotation_height)) =
            self.highlighted_annotation_size(frame, m)
        {
            width = width.max(annotation_width);
            height += annotation_height;
        }
        (width, height)
    }

    /// 横排时高亮候选的译文行尺寸；高亮候选没有译文时为 `None`。
    fn highlighted_annotation_size(&mut self, frame: &Frame, m: &Metrics) -> Option<(f32, f32)> {
        let row = frame.rows.get(frame.highlighted?)?;
        if row.annotation.is_empty() {
            return None;
        }
        let style = m.annotation_style(m.theme.colors.gloss);
        let width: f32 = row
            .annotation
            .iter()
            .map(|(s, _)| self.measure(s, &style).width)
            .sum();
        Some((width, style.line_height + m.row_padding()))
    }

    /// 横排各项的尺寸与统一行高。
    fn items(&mut self, rows: &[Row], m: &Metrics) -> (Vec<Item>, f32) {
        let mut row_height: f32 = 0.0;
        let text_style = m.text_style();
        let index_style = m.index_style();
        let items = rows
            .iter()
            .map(|row| {
                let index = self.measure(&row.index, &index_style);
                let text = self.measure(&row.text, &text_style);
                let badge = self.badge_width(row.badge.as_deref(), m);
                row_height = row_height.max(text.height + m.row_padding() * 2.0);
                Item {
                    index_width: index.width,
                    text_width: text.width,
                    badge_width: badge,
                }
            })
            .collect();
        (items, row_height)
    }

    pub(super) fn draw_horizontal(
        &mut self,
        canvas: &mut Canvas,
        frame: &Frame,
        m: &Metrics,
        left: f32,
        y: f32,
        content_width: f32,
    ) -> Vec<HighlightRect> {
        if frame.rows.is_empty() {
            return Vec::new();
        }
        // 量尺寸时已整形过一遍，这里再整形一遍；等渲染器定型再把结果从 render 传下来。
        let (items, row_height) = self.items(&frame.rows, m);
        let badge_gap = self.horizontal_badge_gap(m);
        let top = y + m.row_padding();
        let text_height = m.px(m.theme.text_font.line_height);
        let inset = m.px(HIGHLIGHT_INSET);
        // 先把各项的条子量好、画在文字之前：滑动中的条子会盖到别的项。
        // 矩形用内容区坐标（`left` 是内容区左边，纵向同样从内容区顶边算），交给壳续滑用。
        let mut x = m.padding() + inset;
        let rects: Vec<HighlightRect> = items
            .iter()
            .map(|item| {
                let width = item_width(item, m, badge_gap);
                let rect = HighlightRect::new(
                    x - inset,
                    y - left,
                    x - inset + width + inset * 2.0,
                    y - left + row_height,
                );
                x += width + m.column_gap();
                rect
            })
            .collect();
        if let Some(rect) = highlight_rect(frame, &rects) {
            self.fill_highlight(canvas, m, rect, left);
        }
        let mut x = left + m.padding() + inset;
        for (row, item) in frame.rows.iter().zip(&items) {
            let width = item_width(item, m, badge_gap);
            self.draw_text(
                canvas,
                &row.index,
                &m.index_style(),
                x,
                top + m.index_offset(text_height),
            );
            self.draw_word(canvas, m, row, x + item.index_width + m.px(INDEX_GAP), top);
            // 角标紧跟在候选文字后面，中间留 `badge_gap`（两个候选字宽）。
            // 它离格右边缘的距离 = 上面 `item_width` 里那一段减去角标宽度，画在这里正好对上。
            self.draw_badge(
                canvas,
                m,
                row.badge.as_deref(),
                x + width - item.badge_width,
                top,
            );
            x += width + m.column_gap();
        }
        // 高亮候选的译文（横排时它在候选下面单独一行）、以及它下面的底部信息区。
        // 两者的纵向次序必须与 `horizontal_size` 里预留的「候选行 + 译文 + 信息区」一致：
        // 以前信息区画在 `top + row_height`、译文画在 `y + row_height + row_padding()/2`，
        // 两段落在同一处，译文长一点就和 Tip / 页码叠在一起。
        let annotation_height = self
            .highlighted_annotation_size(frame, m)
            .map_or(0.0, |(_, height)| height);
        if let Some(row) = frame.highlighted.and_then(|i| frame.rows.get(i)) {
            let mut x = left + m.padding() + inset;
            let annotation_top = y + row_height + m.row_padding() / 2.0;
            for (segment, tone) in &row.annotation {
                let style = m.annotation_style(m.tone_color(*tone));
                x += self.draw_text(canvas, segment, &style, x, annotation_top);
            }
        }
        // 没有译文时保持原来的基准（`top` 已经含一个行内留白，与竖排 / 矩阵一致，
        // 展开 / 收起切换时 Tip 不会上下跳）；有译文就排在它下面。
        let info_top = if annotation_height > 0.0 {
            y + row_height + annotation_height
        } else {
            top + row_height
        };
        self.draw_info(canvas, frame, m, left, info_top, content_width);
        rects
    }
}
