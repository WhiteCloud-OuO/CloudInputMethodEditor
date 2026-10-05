//! 竖排：一行一个候选，序号 / 候选词 / 译文三列，页码在右下角。

use super::columns::Columns;
use super::{Metrics, Renderer, highlight_rect};
use crate::canvas::Canvas;
use crate::frame::{Frame, HighlightRect, Row};

impl Renderer {
    pub(super) fn vertical_size(&mut self, frame: &Frame, m: &Metrics) -> (f32, f32) {
        let columns = self.columns(&frame.rows, m);
        let mut width = columns.index_width + m.column_gap() + columns.text_width;
        if columns.annotation_width > 0.0 {
            width += m.column_gap() + columns.annotation_width;
        }
        if columns.badge_width > 0.0 {
            width += m.column_gap() + columns.badge_width;
        }
        let mut height = columns.row_height * frame.rows.len() as f32;
        if let Some(footer) = frame.footer.as_deref() {
            let footer_size = self.measure(footer, &m.footer_style());
            width = width.max(footer_size.width);
            height += footer_size.height + m.row_padding();
        }
        (width, height)
    }

    fn columns(&mut self, rows: &[Row], m: &Metrics) -> Columns {
        let mut columns = Columns {
            index_width: 0.0,
            text_width: 0.0,
            annotation_width: 0.0,
            badge_width: 0.0,
            row_height: 0.0,
        };
        let text_style = m.text_style();
        let index_style = m.index_style();
        let annotation_style = m.annotation_style(m.theme.colors.gloss);
        for row in rows {
            let index = self.measure(&row.index, &index_style);
            let text = self.measure(&row.text, &text_style);
            let annotation: f32 = row
                .annotation
                .iter()
                .map(|(s, _)| self.measure(s, &annotation_style).width)
                .sum();
            let badge = self.badge_width(row.badge.as_deref(), m);
            columns.index_width = columns.index_width.max(index.width);
            columns.text_width = columns.text_width.max(text.width);
            columns.annotation_width = columns.annotation_width.max(annotation);
            columns.badge_width = columns.badge_width.max(badge);
            columns.row_height = columns.row_height.max(text.height + m.row_padding() * 2.0);
        }
        columns
    }

    pub(super) fn draw_vertical(
        &mut self,
        canvas: &mut Canvas,
        frame: &Frame,
        m: &Metrics,
        left: f32,
        y: f32,
        content_width: f32,
    ) -> Vec<HighlightRect> {
        // 量尺寸时已整形过一遍，这里再整形一遍；等渲染器定型再把结果从 render 传下来。
        let columns = self.columns(&frame.rows, m);
        let text_x = left + m.padding() + columns.index_width + m.column_gap();
        let annotation_x = text_x + columns.text_width + m.column_gap();
        let badge_right = left + content_width - m.padding();
        let text_height = m.px(m.theme.text_font.line_height);
        // 先把各行的条子量好、画在文字之前：滑动中的条子可能盖到别的行，必须在它们下面。
        // 矩形用内容区坐标（`left` 是内容区左边，纵向同样从内容区顶边算），交给壳续滑用。
        let rects: Vec<HighlightRect> = (0..frame.rows.len())
            .map(|i| {
                let row_top = y - left + columns.row_height * i as f32;
                HighlightRect::new(
                    m.padding() / 2.0,
                    row_top,
                    content_width - m.padding() / 2.0,
                    row_top + columns.row_height,
                )
            })
            .collect();
        if let Some(rect) = highlight_rect(frame, &rects) {
            self.fill_highlight(canvas, m, rect, left);
        }
        for (i, row) in frame.rows.iter().enumerate() {
            let top = y + columns.row_height * i as f32 + m.row_padding();
            let small_offset = m.small_offset(text_height);
            self.draw_text(
                canvas,
                &row.index,
                &m.index_style(),
                left + m.padding(),
                top + m.index_offset(text_height),
            );
            self.draw_word(canvas, m, row, text_x, top);
            let mut x = annotation_x;
            for (segment, tone) in &row.annotation {
                let style = m.annotation_style(m.tone_color(*tone));
                x += self.draw_text(canvas, segment, &style, x, top + small_offset);
            }
            self.draw_badge(
                canvas,
                m,
                row.badge.as_deref(),
                badge_right,
                top,
                text_height,
            );
        }
        if let Some(footer) = frame.footer.as_deref() {
            let style = m.footer_style();
            let size = self.measure(footer, &style);
            self.draw_text(
                canvas,
                footer,
                &style,
                left + content_width - m.padding() - size.width,
                y + columns.row_height * frame.rows.len() as f32 + m.row_padding(),
            );
        }
        rects
    }
}
