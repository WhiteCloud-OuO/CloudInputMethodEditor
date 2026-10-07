//! 候选窗口一次绘制要用的全部内容，由帧换算而来；渲染器要的帧由 [`RenderData::render_frame`] 再换一次。

use cloudime_platform::ItemNumberStyle;
use cloudime_platform::LayoutMode;
use cloudime_platform::protocol::{Frame, PreeditKind};
use cloudime_render::{
    HighlightAnimation, Preedit, PreeditSegment, PreeditStyle, Row, TipSegment, Tone,
};
use cloudime_translate::Sense;

use super::row;

/// 一次绘制要用的全部内容。
#[derive(Clone)]
pub(crate) struct RenderData {
    /// 顶部拼音行的各段。
    pub(super) preedit: Vec<(String, PreeditKind)>,

    /// 光标在拼音行里的字符位置。
    pub(super) cursor: usize,

    /// 候选行。
    pub(super) rows: Vec<Row>,

    /// 高亮行下标（页内）。
    pub(super) highlight: usize,

    /// 页码，只有多页时有。
    pub(super) footer: Option<String>,

    /// 屏幕提示（当前没有来源写入），画在拼音行下方。
    pub(super) notice: Option<String>,

    /// 候选排布。
    pub(super) layout: LayoutMode,

    /// 展开「更多候选项」时一行几格（矩阵）；`0` = 没展开。
    pub(super) columns: usize,

    /// 序号的写法。
    pub(super) index_style: ItemNumberStyle,

    /// 底部那一行左侧的翻译 Tip（词性斜体、释义常规），没有译文时为空。
    pub(super) tip: Vec<TipSegment>,
}

impl RenderData {
    pub(super) fn empty() -> Self {
        Self {
            preedit: Vec::new(),
            cursor: 0,
            rows: Vec::new(),
            highlight: usize::MAX,
            footer: None,
            notice: None,
            layout: LayoutMode::default(),
            columns: 0,
            index_style: ItemNumberStyle::default(),
            tip: Vec::new(),
        }
    }

    pub(super) fn set(
        &mut self,
        frame: &Frame,
        badges: &[Option<char>],
        index_style: ItemNumberStyle,
    ) {
        self.layout = frame.layout;
        self.columns = frame.columns;
        self.index_style = index_style;
        self.tip = tip_segments(frame);
        // 多释义选择（Ctrl + 反引号之后）：顶部那一行换成被翻译的词条、候选行换成各条释义
        match &frame.tip_choices {
            Some(choices) => {
                self.preedit = vec![(choices.word.clone(), PreeditKind::Typed)];
                self.cursor = choices.word.chars().count();
                self.rows = choices
                    .senses
                    .iter()
                    .enumerate()
                    .map(|(index, sense)| chooser_row(index, sense, index_style))
                    .collect();
                // 高亮第几条释义：鼠标悬停挪它，圆角矩形跟着滑（动画走的是候选窗那一套）
                self.highlight = frame.highlight;
            }
            None => {
                self.preedit = window_preedit(frame);
                self.cursor = frame.cursor;
                self.rows = frame
                    .candidates
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, candidate)| {
                        row::from_candidate(
                            i,
                            candidate,
                            badges.get(i).copied().flatten(),
                            index_style,
                        )
                    })
                    .collect();
                self.highlight = frame.highlight;
            }
        }
        // 展开「更多候选项」时不画序号：候选密排成格子，序号既挤又用不上
        // （数字键那时是跳页，不再选词）。
        if self.columns > 0 {
            for row in &mut self.rows {
                row.index.clear();
            }
        }
        // 页码一直显示（只有一页也显示 `1/1`）：底部那一行（翻译 Tip 在左）的位置总在。
        // 释义选择那一屏不是候选页，没有页码（也就没有底部那一行）。
        self.footer = frame
            .tip_choices
            .is_none()
            .then(|| format!("{}/{}", frame.page + 1, frame.page_count));
        self.notice = frame.notice.clone();
    }

    /// 渲染器要的帧。提示（删了什么词）在渲染器里画在拼音行右侧；`highlight_animation` 是纯展示的
    /// 高亮条滑动信息，`None` 直接画在高亮行；`min_cell_width` 是展开成网格时每格的最小宽度，见
    /// [`cloudime_render::Frame::min_cell_width`]。
    pub(super) fn render_frame(
        &self,
        highlight_animation: Option<HighlightAnimation>,
        min_cell_width: f32,
    ) -> cloudime_render::Frame {
        let preedit = (!self.preedit.is_empty()).then(|| Preedit {
            segments: self
                .preedit
                .iter()
                .map(|(text, kind)| PreeditSegment {
                    text: text.clone(),
                    style: match kind {
                        PreeditKind::Typed => PreeditStyle::Typed,
                        PreeditKind::Rest => PreeditStyle::Rest,
                        PreeditKind::Corrected => PreeditStyle::Struck,
                    },
                })
                .collect(),
            cursor: self.cursor,
        });
        cloudime_render::Frame {
            preedit,
            rows: self.rows.clone(),
            // 协议里 usize::MAX 表示不高亮。
            highlighted: (self.highlight != usize::MAX).then_some(self.highlight),
            highlight_animation,
            columns: self.columns,
            min_cell_width,
            footer: self.footer.clone(),
            tip: (!self.tip.is_empty()).then(|| self.tip.clone()),
            status: self.notice.clone(),
        }
    }
}

/// 多释义选择里的一行：序号 + 译文 + `(词性)` 注解（没有词性就只有译文）。
fn chooser_row(index: usize, sense: &Sense, style: ItemNumberStyle) -> Row {
    Row {
        index: style.format(index + 1),
        text: sense.text.clone(),
        annotation: sense
            .pos
            .as_ref()
            .map(|pos| vec![(format!("({pos})"), Tone::Faint)])
            .unwrap_or_default(),
        badge: None,
    }
}

/// 底部那一行左侧的翻译 Tip 的段落：词性斜体、释义常规（颜色按词条学没学会）、词性与分隔符最浅。
fn tip_segments(frame: &Frame) -> Vec<TipSegment> {
    let Some(tip) = frame.tip.as_ref() else {
        return Vec::new();
    };
    let tone = if tip.learned {
        Tone::TranslateLearned
    } else {
        Tone::TranslateFresh
    };
    let mut segments = Vec::new();
    for (index, sense) in tip.senses.iter().enumerate() {
        if index > 0 {
            segments.push(TipSegment::new("; ", Tone::TranslateMeta, false));
        }
        if let Some(pos) = &sense.pos {
            segments.push(TipSegment::new(
                format!("{pos} "),
                Tone::TranslateMeta,
                true,
            ));
        }
        segments.push(TipSegment::new(sense.text.clone(), tone, false));
        if let Some(reading) = &sense.reading {
            segments.push(TipSegment::new(
                format!("({reading})"),
                Tone::TranslateMeta,
                false,
            ));
        }
    }
    segments
}

/// 窗口顶部要画的拼音行：`[general] preedit` 配成「只在行内」时为空（拼音已经在应用里）；
/// 整句补全仍画在这一行上，所以空行时窗口仍可能留出这条线。
fn window_preedit(frame: &Frame) -> Vec<(String, PreeditKind)> {
    if !frame.preedit_mode.in_window() {
        return Vec::new();
    }
    frame
        .preedit
        .iter()
        .map(|segment| (segment.text.clone(), segment.kind))
        .collect()
}

#[cfg(test)]
mod tests {
    use cloudime_platform::PreeditMode;
    use cloudime_platform::protocol::{Frame, PreeditKind, PreeditSegment};

    use super::window_preedit;

    fn frame(mode: PreeditMode) -> Frame {
        Frame {
            preedit: vec![PreeditSegment {
                text: "ni'hao".to_owned(),
                kind: PreeditKind::Typed,
            }],
            preedit_mode: mode,
            ..Frame::default()
        }
    }

    /// 只有「只在行内」不给窗口拼音行；另两档窗口都得画。
    #[test]
    fn window_keeps_the_pinyin_row_unless_inline_only() {
        assert_eq!(window_preedit(&frame(PreeditMode::Both)).len(), 1);
        assert_eq!(window_preedit(&frame(PreeditMode::Window)).len(), 1);
        assert!(window_preedit(&frame(PreeditMode::Inline)).is_empty());
    }
}
