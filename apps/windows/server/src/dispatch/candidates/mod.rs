//! 候选窗口输出：Router 只产出 [`Frame`] 与光标矩形，交给 [`CandidateSink`] 去画；帧没变就不重画。

mod event;
mod sink;

use cloudime_core::{CandidateKind, Learner};
use cloudime_platform::protocol::{CANDIDATE_CLICK_SINCE, Frame, ScreenRect, SessionId};

pub use self::event::CandidateEvent;
pub use self::sink::{CandidateSink, NoopSink, RenderSettings};
use super::Router;
use super::translate::TranslateAction;

impl Router {
    /// 候选窗上的鼠标操作（UI 线程发来）。鼠标不进按键那条路，所以高亮直接挪、上屏的文本先攒着，
    /// 等 DLL 下一拍 `Poll` 时用 `ServerMessage::Update::commit` 带回去落进文档。
    ///
    /// 收起态与展开态一样：悬停跟手、单击上屏。
    pub fn handle_candidate_event(&mut self, event: CandidateEvent) {
        // 多释义选择中：那一屏画的是释义不是候选，鼠标悬停挪高亮、单击选那一条上屏；
        // 右键（`Translate`）与移出不管。选择本身仍以数字键 / Esc 为主。
        if self.translate.choices().is_some() {
            match event {
                CandidateEvent::Hover(index) => {
                    if self.translate.set_choice_highlight(index) {
                        let frame = self.self_drawn_frame();
                        self.reconcile_candidates(&frame);
                    }
                }
                CandidateEvent::Commit(index) => {
                    // 单击那一条 = 按它的数字键：上屏文本同样等下一拍 `Poll` 带走
                    self.pending_commit = self.choose_sense(index);
                    self.recompose();
                    let frame = self.self_drawn_frame();
                    self.reconcile_candidates(&frame);
                }
                CandidateEvent::HoverLeft => {}
                // 释义选择里右键（点在哪儿都算）= Esc：取消选择
                CandidateEvent::Translate(_) => {
                    self.translate.end_choices();
                    let frame = self.self_drawn_frame();
                    self.reconcile_candidates(&frame);
                }
                // 中键 = `Shift + 反引号`：念高亮那条释义（不选、不退出，屏幕上没变化）
                CandidateEvent::Speak => self.speak(),
                // 释义选择那屏就几条，没有「翻页」这回事
                CandidateEvent::Page(_) => {}
            }
            return;
        }
        let page_size = self.page_size();
        let page_start = self.highlight / page_size * page_size;
        match event {
            CandidateEvent::Hover(index) => {
                // 鼠标指着哪一格记下来：再编辑拼音时不把高亮拉回页首（见 [`Router::recompose`]）
                self.hover_cell = Some(index);
                let target = page_start + index;
                if target < self.candidate_count() && target != self.highlight {
                    self.highlight = target;
                    self.navigated = true;
                    let frame = self.self_drawn_frame();
                    self.reconcile_candidates(&frame);
                }
            }
            CandidateEvent::HoverLeft => self.hover_cell = None,
            // 右键 = `Ctrl + 反引号`（只认点中格子的那一下；点空白处不放这里）；没开翻译 Tip 就不响应
            CandidateEvent::Translate(Some(index)) => {
                if !self.config.translate_enabled {
                    return;
                }
                match self.translate_action(page_start + index) {
                    // 上屏的文本和鼠标左键一样：等 DLL 下一拍 `Poll` 用 `Update::commit` 带走
                    TranslateAction::Commit(text) => {
                        self.pending_commit = text;
                        self.recompose();
                        let frame = self.self_drawn_frame();
                        self.reconcile_candidates(&frame);
                    }
                    TranslateAction::Choosing => {
                        let frame = self.self_drawn_frame();
                        self.reconcile_candidates(&frame);
                    }
                    TranslateAction::Nothing => {}
                }
            }
            // 不在释义选择界面时，右键点在空白处什么也不做
            CandidateEvent::Translate(None) => {}
            // 中键 = `Shift + 反引号`：念高亮候选的译文（没译文 / 没开翻译 Tip 就不念）
            CandidateEvent::Speak => self.speak(),
            // 滚轮（不带修饰键）= 翻页；展开「更多候选项」时 `scroll` 走的是一整屏。
            // 与键盘翻页的区别：高亮条**留在窗口同一格**，不回到页首。
            CandidateEvent::Page(step) => {
                self.scroll(step);
                let frame = self.self_drawn_frame();
                self.reconcile_candidates(&frame);
            }
            CandidateEvent::Commit(index) => {
                // 老 DLL 不认得 `Update` 里的 `commit`，点了只会把组句清掉、文本却进不了文档
                if !self.supports_candidate_click() {
                    tracing::debug!("这个应用的 DLL 还不支持鼠标点候选，忽略");
                    return;
                }
                let target = page_start + index;
                if target >= self.candidate_count() {
                    return;
                }
                if let Some(text) = self.commit_index(target) {
                    self.pending_commit = Some(text);
                }
                self.recompose();
                let frame = self.self_drawn_frame();
                self.reconcile_candidates(&frame);
            }
        }
    }

    /// 聚焦会话的 DLL 认不认得「鼠标点候选上屏」（v16 起）。
    fn supports_candidate_click(&self) -> bool {
        self.focused
            .and_then(|session| self.sessions.get(&session))
            .is_some_and(|info| info.protocol >= CANDIDATE_CLICK_SINCE)
    }

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
        self.hover_cell = None;
        self.candidates.hide();
    }

    pub(super) fn position_candidates(&mut self, session: SessionId, rect: ScreenRect) {
        if self.focused != Some(session) {
            return;
        }
        self.last_rect = Some(rect);
        // 组句结束后 `last_rect` 会清掉，但状态切换提示还要用它定位，所以另记一份留着。
        self.last_caret = Some(rect);
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
            display: None,
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
