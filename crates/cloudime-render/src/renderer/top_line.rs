//! 顶部拼音行：各段按样式画、自己画光标、右侧临时状态。

use super::{CARET_WIDTH, Metrics, Renderer, STATUS_GAP};
use crate::canvas::Canvas;
use crate::frame::{Frame, Preedit, PreeditStyle};

impl Renderer {
    /// 顶部拼音行（含右侧状态）需要的宽高；没有这一行时都是 0。
    pub(super) fn top_line_size(&mut self, frame: &Frame, m: &Metrics) -> (f32, f32) {
        if !frame.has_top_line() {
            return (0.0, 0.0);
        }
        let style = m.pinyin_style(m.theme.colors.pinyin);
        let line_height = style.line_height;
        let mut width = 0.0;
        if let Some(preedit) = &frame.preedit {
            width += self.measure(&preedit.text(), &style).width + m.px(CARET_WIDTH);
        }
        if let Some(status) = &frame.status {
            if frame.preedit.is_some() {
                width += m.px(STATUS_GAP);
            }
            width += self.measure(status, &style).width;
        }
        (width, line_height + m.row_padding() * 2.0)
    }

    /// 返回占用高度。`left` 是内容区左边。
    pub(super) fn draw_top_line(
        &mut self,
        canvas: &mut Canvas,
        frame: &Frame,
        m: &Metrics,
        left: f32,
        y: f32,
    ) -> f32 {
        if !frame.has_top_line() {
            return 0.0;
        }
        let line_height = m.px(m.theme.pinyin_font.line_height);
        let top = y + m.row_padding();
        let mut x = left + m.padding();
        if let Some(preedit) = &frame.preedit {
            x += self.draw_preedit(canvas, m, preedit, x, top, line_height);
            if frame.status.is_some() {
                x += m.px(STATUS_GAP);
            }
        }
        // 临时状态：灰字。
        if let Some(status) = &frame.status {
            let style = m.annotation_style(m.theme.colors.gloss);
            self.draw_text(canvas, status, &style, x, top);
        }
        line_height + m.row_padding() * 2.0
    }

    /// 画拼音行的各段与光标，返回占用宽度（含光标）。
    fn draw_preedit(
        &mut self,
        canvas: &mut Canvas,
        m: &Metrics,
        preedit: &Preedit,
        x: f32,
        top: f32,
        line_height: f32,
    ) -> f32 {
        let mut cursor_x = x;
        for segment in &preedit.segments {
            let style = match segment.style {
                PreeditStyle::Typed | PreeditStyle::Rest => m.pinyin_style(m.theme.colors.pinyin),
                PreeditStyle::Struck => m.pinyin_style(m.theme.colors.pinyin).struck(),
            };
            cursor_x += self.draw_text(canvas, &segment.text, &style, cursor_x, top);
        }
        let measure_style = m.pinyin_style(m.theme.colors.pinyin);
        let caret_x = x + self.measure(&preedit.before_cursor(), &measure_style).width;
        canvas.fill_rect(
            caret_x,
            top,
            m.px(CARET_WIDTH),
            line_height,
            m.theme.colors.text,
        );
        cursor_x - x + m.px(CARET_WIDTH)
    }
}
