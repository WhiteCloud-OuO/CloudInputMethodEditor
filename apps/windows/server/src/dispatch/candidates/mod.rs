//! 候选窗口输出：Router 只产出 [`Frame`] 与光标矩形，交给 [`CandidateSink`] 去画；帧没变就不重画。

mod sink;

use cloudime_core::{CandidateKind, Learner};
use cloudime_platform::protocol::{Frame, ScreenRect, SessionId};

pub use self::sink::{CandidateSink, NoopSink, RenderSettings};
use super::Router;

impl Router {
    /// 空帧收窗口；非空且已知光标矩形就重绘；还没收到矩形（组句刚起）先不显示，免得在旧位置闪一下。
    pub(super) fn reconcile_candidates(&mut self, frame: &Frame) {
        if frame.is_empty() {
            self.engine.note_displayed(std::iter::empty());
            self.hide_candidate_window();
        } else if let Some(rect) = self.last_rect {
            let badges = self.candidate_badges(frame);
            let unchanged = matches!(
                &self.last_shown,
                Some((f, b, r)) if f == frame && *b == badges && *r == rect
            );
            if !unchanged {
                // 词汇记录的「看到轮次」按真正显示的页算。
                self.engine.note_displayed(frame.candidates.items.iter());
                self.candidates.show(frame.clone(), badges.clone(), rect);
                self.last_shown = Some((frame.clone(), badges, rect));
            }
        }
    }

    /// 每个候选右侧的来源角标。
    fn candidate_badges(&self, frame: &Frame) -> Vec<Option<char>> {
        badges_of(frame, self.engine.learner())
    }

    pub(super) fn hide_candidate_window(&mut self) {
        self.last_rect = None;
        self.last_shown = None;
        self.candidates.hide();
    }

    pub(super) fn position_candidates(&mut self, session: SessionId, rect: ScreenRect) {
        if self.focused != Some(session) {
            return;
        }
        self.last_rect = Some(rect);
        // 自绘窗吃未降级的帧（降级只作用于发给 DLL 的那份）
        let frame = self.self_drawn_frame();
        self.reconcile_candidates(&frame);
    }
}

/// 每个候选右侧的来源角标：用户短语「短」、用户自造词「造」，其余没有。
fn badges_of(frame: &Frame, learner: &dyn Learner) -> Vec<Option<char>> {
    frame
        .candidates
        .items
        .iter()
        .map(|candidate| match candidate.kind {
            CandidateKind::Custom => Some('短'),
            CandidateKind::Sentence => Some('句'),
            _ if learner.is_user_word(&candidate.text) => Some('造'),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cloudime_core::{Candidate, CandidateKind, Learner};
    use cloudime_platform::protocol::Frame;

    use super::badges_of;

    /// 只认固定几个用户词的学习器。
    struct UserWords(Vec<&'static str>);

    impl Learner for UserWords {
        fn record(&mut self, _candidate: &Candidate) {}

        fn weight(&self, _text: &str) -> u32 {
            0
        }

        fn is_user_word(&self, text: &str) -> bool {
            self.0.contains(&text)
        }
    }

    fn candidate(text: &str, kind: CandidateKind) -> Candidate {
        Candidate {
            text: text.to_owned(),
            kind,
            syllables: Vec::new(),
            reading: None,
        }
    }

    #[test]
    fn badges_mark_phrases_sentences_and_user_words_only() {
        let mut frame = Frame::default();
        frame.candidates.items = vec![
            candidate("第1项", CandidateKind::Custom),
            candidate("想开发", CandidateKind::Sentence),
            candidate("青简", CandidateKind::Chinese),
            candidate("创造", CandidateKind::Chinese),
            candidate("hello", CandidateKind::English),
        ];
        let learner = UserWords(vec!["青简"]);
        assert_eq!(
            badges_of(&frame, &learner),
            [Some('短'), Some('句'), Some('造'), None, None]
        );
    }
}
