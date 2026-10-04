//! 组句的展示状态：缓冲变化时重查候选并重建 [`Composed`]，高亮 / 翻页，按状态生成给 DLL 的帧。

mod state;

use cloudime_core::{Candidate, CandidateLayout, CandidateList, Query};
use cloudime_platform::protocol::{Frame, PreeditKind, PreeditSegment};

pub(super) use self::state::Composed;
use super::Router;

impl Router {
    /// 缓冲变化后：按 Engine 状态重建 [`Composed`]，归零高亮。
    pub(super) fn recompose(&mut self) {
        self.highlight = 0;
        self.navigated = false;
        if self.engine.composition().is_empty() {
            self.composed = None;
            self.stop_rescoring();
            return;
        }
        self.attach_loaded_model();
        let built = self.engine.query().ok().map(|query| {
            let (preedit, cursor) = marked_parts(&query);
            (query.candidates.items.clone(), preedit, cursor)
        });
        self.composed = Some(match built {
            Some((items, preedit, cursor)) => {
                let layout = CandidateLayout::new(items, self.config.page_size);
                Composed::Candidates {
                    preedit,
                    cursor,
                    layout,
                }
            }
            None => {
                let composition = self.engine.composition();
                let text = composition.text().to_owned();
                let cursor = text[..composition.cursor()].chars().count();
                Composed::Raw { text, cursor }
            }
        });
        self.schedule_rescoring();
    }

    /// 高亮移动 `delta`，夹在 `[0, 末尾]`，到页边自然换页。
    pub(super) fn move_highlight(&mut self, delta: isize) {
        let count = self.candidate_count();
        if count == 0 {
            self.highlight = 0;
            return;
        }
        let next = (self.highlight as isize + delta).clamp(0, count as isize - 1) as usize;
        self.navigated |= next != self.highlight;
        self.highlight = next;
    }

    /// 整页翻 `step`，高亮落到目标页第一个候选。
    pub(super) fn page(&mut self, step: isize) {
        let count = self.candidate_count();
        if count == 0 {
            self.highlight = 0;
            return;
        }
        let page_size = self.config.page_size;
        let page_count = count.div_ceil(page_size);
        let current = (self.highlight / page_size) as isize;
        let target = (current + step).clamp(0, page_count as isize - 1) as usize;
        if target != current as usize {
            self.navigated = true;
            self.engine.note_page_turn();
        }
        self.highlight = (target * page_size).min(count - 1);
    }

    pub(super) fn candidate_count(&self) -> usize {
        match &self.composed {
            Some(Composed::Candidates { layout, .. }) => layout.len(),
            _ => 0,
        }
    }

    /// 候选布局里第 `index` 个（跨页下标）。
    pub(super) fn layout_candidate(&self, index: usize) -> Option<Candidate> {
        match &self.composed {
            Some(Composed::Candidates { layout, .. }) => layout.candidate(index).cloned(),
            _ => None,
        }
    }

    pub(super) fn commit_index(&mut self, index: usize) -> Option<String> {
        let candidate = self.layout_candidate(index)?;
        Some(self.engine.commit(&candidate))
    }

    /// 按当前状态生成一帧：没在组句给空帧；否则给高亮所在的那一页。
    pub(super) fn current_frame(&self) -> Frame {
        self.raw_frame()
    }

    /// 自绘候选窗用的帧。
    pub(super) fn self_drawn_frame(&self) -> Frame {
        self.raw_frame()
    }

    fn raw_frame(&self) -> Frame {
        match &self.composed {
            None => Frame::default(),
            Some(Composed::Raw { text, cursor }) => Frame {
                preedit: vec![PreeditSegment {
                    text: text.clone(),
                    kind: PreeditKind::Typed,
                }],
                preedit_mode: self.config.preedit,
                cursor: *cursor,
                candidates: CandidateList { items: Vec::new() },
                highlight: usize::MAX,
                page: 0,
                page_count: 1,
                layout: self.config.layout,
                notice: self.notice.clone(),
            },
            Some(Composed::Candidates {
                preedit,
                cursor,
                layout,
                ..
            }) => {
                let page_size = self.config.page_size;
                let highlight = self.highlight.min(layout.len().saturating_sub(1));
                let page = highlight / page_size;
                let items: Vec<Candidate> = layout.page(page).to_vec();
                let candidates = CandidateList { items };
                Frame {
                    preedit: preedit.clone(),
                    preedit_mode: self.config.preedit,
                    cursor: *cursor,
                    candidates,
                    highlight: highlight - page * page_size,
                    page,
                    page_count: layout.pages().max(1),
                    layout: self.config.layout,
                    notice: self.notice.clone(),
                }
            }
        }
    }
}

/// 一次查询的拼音行分段与光标。
/// 光标用 Core 的映射：自动补的 `'` 会让显示串比敲的长。
pub(super) fn marked_parts(query: &Query) -> (Vec<PreeditSegment>, usize) {
    let preedit = query.marked_segments().iter().map(Into::into).collect();
    (preedit, query.segments_cursor())
}
