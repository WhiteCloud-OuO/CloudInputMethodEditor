//! 候选窗口一次绘制要用的全部内容，由帧换算而来；渲染器要的帧由 [`RenderData::render_frame`] 再换一次。

use cloudime_platform::ItemNumberStyle;
use cloudime_platform::LayoutMode;
use cloudime_platform::protocol::{Frame, PreeditKind};
use cloudime_render::{HighlightAnimation, Preedit, PreeditSegment, PreeditStyle, Row};

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

    /// 序号的写法。
    pub(super) index_style: ItemNumberStyle,
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
            index_style: ItemNumberStyle::default(),
        }
    }

    pub(super) fn set(
        &mut self,
        frame: &Frame,
        badges: &[Option<char>],
        index_style: ItemNumberStyle,
    ) {
        self.layout = frame.layout;
        self.index_style = index_style;
        self.preedit = window_preedit(frame);
        self.cursor = frame.cursor;
        self.rows = frame
            .candidates
            .items
            .iter()
            .enumerate()
            .map(|(i, candidate)| {
                row::from_candidate(i, candidate, badges.get(i).copied().flatten(), index_style)
            })
            .collect();
        self.highlight = frame.highlight;
        self.footer =
            (frame.page_count > 1).then(|| format!("{}/{}", frame.page + 1, frame.page_count));
        self.notice = frame.notice.clone();
    }

    /// 渲染器要的帧。提示（删了什么词）在渲染器里画在拼音行右侧；`highlight_animation` 是纯展示的
    /// 高亮条滑动信息，`None` 直接画在高亮行。
    pub(super) fn render_frame(
        &self,
        highlight_animation: Option<HighlightAnimation>,
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
            columns: 0,
            column_ems: Vec::new(),
            footer: self.footer.clone(),
            status: self.notice.clone(),
        }
    }
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
