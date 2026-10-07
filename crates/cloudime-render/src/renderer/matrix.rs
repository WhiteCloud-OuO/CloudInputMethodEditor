//! 矩阵：横排展开后的多行网格。一行 `frame.columns` 格，各列宽度由帧给（壳按整份候选估的，滚动时不变），
//! 超宽的候选截尾加「…」；网格下面固定留一行信息：高亮候选被截断时的完整文本、它的译文，页码在行尾。
//! 整个窗口的宽度只由列宽决定，信息行放不下的也截断——高亮怎么移、视口怎么滚，窗口都不跳。

use super::{
    BADGE_GAP, HIGHLIGHT_INSET, INDEX_GAP, Layout, Metrics, Renderer, VERTICAL_CELL_MIN_EMS,
    highlight_rect,
};
use crate::canvas::Canvas;
use crate::frame::{Frame, HighlightRect};
use crate::text::TextStyle;

/// 帧没给列宽时，一格里候选词最多多宽（按候选字号的倍数）。
const MAX_CELL_EMS: f32 = 4.0;

const ELLIPSIS: &str = "…";

/// 量好的网格：每格显示的文字（可能已截断）、各列的宽度与统一的行高（像素）。
struct Cells {
    texts: Vec<(String, bool)>,

    index_width: f32,

    /// 每列一格的宽度（序号 + 间距 + 候选词）；展开「更多候选项」时每列一样宽。
    column_widths: Vec<f32>,

    /// 实际用了几列：候选不够一屏时不留空列，窗口不会白宽一截。
    columns: usize,

    row_height: f32,
}

impl Cells {
    /// 第 `column` 列左边相对网格起点的偏移。
    fn offset(&self, column: usize, gap: f32) -> f32 {
        self.column_widths[..column].iter().sum::<f32>() + gap * column as f32
    }

    /// 网格总宽（不含两侧高亮留边）。
    fn width(&self, gap: f32) -> f32 {
        self.column_widths.iter().sum::<f32>()
            + gap * self.column_widths.len().saturating_sub(1) as f32
    }
}

impl Renderer {
    pub(super) fn matrix_size(&mut self, frame: &Frame, m: &Metrics, layout: Layout) -> (f32, f32) {
        if frame.rows.is_empty() {
            return (0.0, 0.0);
        }
        let cells = self.matrix_cells(frame, m, layout);
        let grid_rows = frame.rows.len().div_ceil(cells.columns);
        let info_height = self.bottom_line_height(frame, m);
        (
            cells.width(m.column_gap()) + m.px(HIGHLIGHT_INSET) * 2.0,
            cells.row_height * grid_rows as f32 + info_height,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_matrix(
        &mut self,
        canvas: &mut Canvas,
        frame: &Frame,
        m: &Metrics,
        layout: Layout,
        left: f32,
        y: f32,
        content_width: f32,
    ) -> Vec<HighlightRect> {
        if frame.rows.is_empty() {
            return Vec::new();
        }
        let cells = self.matrix_cells(frame, m, layout);
        let columns = cells.columns;
        let text_height = m.px(m.theme.text_font.line_height);
        let inset = m.px(HIGHLIGHT_INSET);
        let origin = left + m.padding() + inset;
        // 先把各格的条子量好、画在文字之前：滑动中的条子会盖到别的格。
        // 矩形用内容区坐标（`left` 是内容区左边，纵向同样从内容区顶边算），交给壳续滑用。
        let rects: Vec<HighlightRect> = (0..frame.rows.len())
            .map(|i| {
                let cell_width = cells.column_widths[i % columns];
                let x = origin - left + cells.offset(i % columns, m.column_gap());
                let row_y = y - left + cells.row_height * (i / columns) as f32;
                HighlightRect::new(
                    x - inset,
                    row_y,
                    x - inset + cell_width + inset * 2.0,
                    row_y + cells.row_height,
                )
            })
            .collect();
        if let Some(rect) = highlight_rect(frame, &rects) {
            self.fill_highlight(canvas, m, rect, left);
        }
        for (i, row) in frame.rows.iter().enumerate() {
            let (text, _) = &cells.texts[i];
            if text.is_empty() && row.index.is_empty() {
                continue;
            }
            let x = origin + cells.offset(i % columns, m.column_gap());
            let row_y = y + cells.row_height * (i / columns) as f32;
            let top = row_y + m.row_padding();
            let cell_width = cells.column_widths[i % columns];
            if !row.index.is_empty() {
                self.draw_text(
                    canvas,
                    &row.index,
                    &m.index_style(),
                    x,
                    top + m.index_offset(text_height),
                );
            }
            let mut shown = row.clone();
            shown.text.clone_from(text);
            let text_x = x + cells.index_width + m.px(INDEX_GAP);
            self.draw_word(canvas, m, &shown, text_x, top);
            // 展开态：角标贴在**格右边缘**（往里让 `BADGE_GAP`）——与收起横排的
            // 「文字后面跟 2 个字宽」不同，展开是格子对齐，角标各自靠右。
            let badge_width = self.badge_width(row.badge.as_deref(), m);
            self.draw_badge(
                canvas,
                m,
                row.badge.as_deref(),
                x + cell_width - m.px(BADGE_GAP) - badge_width,
                top,
            );
        }
        // 底部那一行：左侧翻译 Tip、右侧页码
        let grid_rows = frame.rows.len().div_ceil(columns);
        // 底部那一行的基准与竖排 / 横排保持一致（都是「最后一行的底边 + 一个行内留白」）：
        // 展开 / 收起切换时 Tip 不会上下跳。
        let info_top = y + cells.row_height * grid_rows as f32 + m.row_padding();
        self.draw_bottom_line(canvas, frame, m, left, info_top, content_width);
        rects
    }

    /// 每格的显示文字与各列宽度，以及这一格除候选词以外占多宽（展开「更多候选项」时**所有格子一样宽**）。
    ///
    /// 宽度基准：缺省取收起时那条高亮条的宽度（`min_cell_width`）——比它还长的候选截尾加「…」，
    /// 短的原样留白；还没这个宽度时（这次组句还没画过收起态）退回「一屏里最长的那条」，
    /// 单格封顶 [`MAX_CELL_EMS`]。
    /// **横排另有两档**（都是按候选字宽算的，见 `Renderer::char_width`）：候选词与角标之间的最小间隔
    /// 是 2 个字宽；每格最小宽度是 6 个字宽 + 角标宽度（角标不能把候选词挤没）。
    fn matrix_cells(&mut self, frame: &Frame, m: &Metrics, layout: Layout) -> Cells {
        let text_style = m.text_style();
        let columns = frame.columns.max(1);
        let em = m.px(m.theme.text_font.size);
        let horizontal = layout == Layout::Horizontal;
        // 展开「更多候选项」时不画序号，那一段宽度也就不用留
        let index_width = if frame.rows.iter().any(|row| !row.index.is_empty()) {
            self.measure("8", &m.index_style()).width
        } else {
            0.0
        };
        // 角标：各格一样宽，按最宽的那个角标留位置。展开态角标贴在**格右边缘**（往里让 `BADGE_GAP`），
        // 所以这段位置要算进 `chrome`，候选词的截断上限跟着让出来，两者不会叠在一起。
        let badge_width = frame
            .rows
            .iter()
            .map(|row| self.badge_width(row.badge.as_deref(), m))
            .fold(0.0_f32, f32::max);
        let badge_space = if badge_width > 0.0 {
            m.px(BADGE_GAP) + badge_width
        } else {
            0.0
        };
        // 格子里除候选词以外的宽度（展开态就只剩间距与角标）
        let chrome = index_width + m.px(INDEX_GAP) + badge_space;
        // 下限：收起时那条高亮条的宽度；展开态另有两档按候选字宽算的下限——
        // 横排是「6 个字宽 + 角标宽度」，竖排是「这一屏最长的候选 + 2 个字宽 + 一个角标字宽」
        // （竖排这个**不管有没有角标都把角标字宽算进去**）。
        let min_width = if horizontal {
            frame
                .min_cell_width
                .max(self.horizontal_min_cell_width(m, badge_width))
        } else {
            let longest = frame
                .rows
                .iter()
                .map(|row| self.measure(&row.text, &text_style).width)
                .fold(0.0_f32, f32::max);
            let vertical =
                longest + VERTICAL_CELL_MIN_EMS * self.char_width(m) + self.badge_char_width(m);
            frame.min_cell_width.max(vertical)
        };
        let limit = if min_width > chrome {
            min_width - chrome
        } else {
            let widest = frame
                .rows
                .iter()
                .map(|row| self.measure(&row.text, &text_style).width)
                .fold(0.0_f32, f32::max);
            widest.min(em * MAX_CELL_EMS)
        };
        let texts = frame
            .rows
            .iter()
            .map(|row| self.truncate(&row.text, &text_style, limit))
            .collect();
        let row_height = self.measure("国", &text_style).height + m.row_padding() * 2.0;
        let cell_width = (chrome + limit).max(min_width);
        let used = columns.min(frame.rows.len().max(1)).max(1);
        Cells {
            texts,
            index_width,
            column_widths: vec![cell_width; used],
            columns: used,
            row_height,
        }
    }
}

impl Renderer {
    /// `text` 宽度超过 `max_width` 就从末尾去字、补上「…」直到放得下；返回显示文字与是否截断过。
    pub(super) fn truncate(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: f32,
    ) -> (String, bool) {
        if text.is_empty() || self.measure(text, style).width <= max_width {
            return (text.to_owned(), false);
        }
        let mut kept: Vec<char> = text.chars().collect();
        while kept.pop().is_some() {
            let mut candidate: String = kept.iter().collect();
            candidate.push_str(ELLIPSIS);
            if kept.is_empty() || self.measure(&candidate, style).width <= max_width {
                return (candidate, true);
            }
        }
        (ELLIPSIS.to_owned(), true)
    }
}
