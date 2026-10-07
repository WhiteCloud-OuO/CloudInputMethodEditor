//! 组句的展示状态：缓冲变化时重查候选并重建 [`Composed`]，高亮 / 翻页，按状态生成给 DLL 的帧。

mod state;

use cloudime_core::{Candidate, CandidateLayout, CandidateList, Query};
use cloudime_platform::LayoutMode;
use cloudime_platform::protocol::{Frame, PreeditKind, PreeditSegment, TipChoices};

pub(super) use self::state::Composed;
use super::Router;

/// 展开「更多候选项」时一屏几列（竖排）；横排用的列数就是候选项个数（也就是一屏 5 行）。
const EXPAND_COLUMNS: usize = 5;

impl Router {
    /// 一页几个候选：没展开就是设置里的候选项个数；展开「更多候选项」时是一整屏
    /// （竖排 5 列、横排 5 行，都是 5 × 候选项个数）。
    pub(super) fn page_size(&self) -> usize {
        if self.show_more {
            self.config.page_size * EXPAND_COLUMNS
        } else {
            self.config.page_size
        }
    }

    /// 展开「更多候选项」时一行排几格；`0` = 没展开，渲染器按竖排 / 横排画。
    /// 竖排展开成 5 列（行数就是候选项个数），横排展开成 5 行（列数就是候选项个数）。
    pub(super) fn grid_columns(&self) -> usize {
        if !self.show_more {
            return 0;
        }
        match self.config.layout {
            LayoutMode::Vertical => EXPAND_COLUMNS,
            LayoutMode::Horizontal => self.config.page_size,
        }
    }

    /// 缓冲变化后：按 Engine 状态重建 [`Composed`]，归零高亮。
    pub(super) fn recompose(&mut self) {
        // 鼠标正指着高亮那一格时不归零：用户是拿鼠标点着那一格在打字，拉回页首之后高亮条还会补一段
        // 从页首滑到鼠标那儿的动画，看着像它自己跑过去。保持不动就是「钉在鼠标上」（收起态同理）。
        let page_size = self.page_size();
        let pinned = self
            .hover_cell
            .is_some_and(|cell| self.highlight % page_size == cell);
        if !pinned {
            self.highlight = 0;
        }
        self.navigated = false;
        if self.engine.composition().is_empty() {
            self.composed = None;
            self.stop_rescoring();
            // 这次组句完了：下一段拼音的候选窗从收起来的样子出现（展开只活在一次组句里）
            self.show_more = false;
            self.hover_cell = None;
            return;
        }
        self.attach_loaded_model();
        let built = self.engine.query().ok().map(|query| {
            let (preedit, cursor) = marked_parts(&query);
            (query.candidates.items.clone(), preedit, cursor)
        });
        self.composed = Some(match built {
            Some((items, preedit, cursor)) => {
                let layout = CandidateLayout::new(items, page_size);
                Composed::Candidates {
                    preedit,
                    cursor,
                    layout,
                }
            }
            None => {
                // 查询失败也带上已选文本（`云朵shurufa` 里拼音段切不动时仍在组句）。
                let raw = self.engine.raw_preedit();
                let cursor = raw.text[..raw.cursor_bytes].chars().count();
                Composed::Raw {
                    text: raw.text,
                    cursor,
                }
            }
        });
        if pinned {
            // 新候选可能比刚才少，钉住的下标夹回去
            let count = self.candidate_count();
            self.highlight = self.highlight.min(count.saturating_sub(1));
        }
        self.schedule_rescoring();
    }

    /// 高亮移动 `delta`，夹在 `[0, 末尾]`，到页边自然换页。收起态的方向键用它。
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

    /// 展开态的方向键：高亮在网格里上下移 `rows` 行 / 左右移 `columns` 列（本行内），
    /// 走到这一屏的边上就不动。返回是否真动了。
    pub(super) fn move_highlight_in_grid(&mut self, rows: isize, columns: isize) -> bool {
        let grid = self.grid_columns();
        let page_size = self.page_size();
        if grid == 0 || page_size == 0 {
            return false;
        }
        let count = self.candidate_count();
        let page_start = self.highlight / page_size * page_size;
        // 一屏里实际有几个：最后一屏可能不满
        let page_len = (count - page_start).min(page_size);
        if page_len == 0 {
            return false;
        }
        let index = self.highlight - page_start;
        let target = if rows != 0 {
            let row = index / grid;
            let target_row = row as isize + rows;
            if target_row < 0 || target_row as usize >= page_len.div_ceil(grid) {
                return false;
            }
            target_row as usize * grid + index % grid
        } else {
            let target = index as isize + columns;
            // 左右不越过本行：末行不满时走到这一行最后一个就停
            if target < 0 || target as usize / grid != index / grid {
                return false;
            }
            target as usize
        };
        if target >= page_len || target == index {
            return false;
        }
        self.navigated = true;
        self.highlight = page_start + target;
        true
    }

    /// 展开 / 收起「更多候选项」（组句里的 Tab）。没有候选（查询失败）时什么也不做。
    pub(super) fn toggle_show_more(&mut self) {
        if !matches!(self.composed, Some(Composed::Candidates { .. })) {
            return;
        }
        self.show_more = !self.show_more;
        self.navigated = true;
        // 一页的格数跟着变（展开后一屏 = 5 × 候选项个数），布局按新的页大小重排；
        // 高亮是跨页下标，还指着同一个候选，页码跟着重算。
        let page_size = self.page_size();
        if let Some(Composed::Candidates { layout, .. }) = self.composed.as_mut() {
            layout.set_page_size(page_size);
        }
    }

    /// 滚轮翻页：只换**这一页的内容**，高亮条**留在窗口的同一格**（原来在第几格，翻完还在第几格），
    /// 与「编辑拼音不把高亮拉回页首」是同一个意思。键盘翻页仍走 [`Self::page`]（落到新页第一个）。
    pub(super) fn scroll(&mut self, step: isize) {
        let page_size = self.page_size().max(1);
        let count = self.candidate_count();
        if count == 0 {
            return;
        }
        let current = self.highlight / page_size;
        let cell = self.highlight % page_size;
        let last = (count - 1) / page_size;
        let target = (current as isize + step).clamp(0, last as isize) as usize;
        if target == current {
            return;
        }
        // 目标页可能不满（最后一页）：格号夹到这一页的最后一个
        let page_len = (count - target * page_size).min(page_size);
        self.highlight = target * page_size + cell.min(page_len - 1);
        self.navigated = true;
        self.engine.note_page_turn();
    }

    /// 翻 `step` 页，高亮落到目标页第一个候选。
    pub(super) fn page(&mut self, step: isize) {
        let current = self.highlight / self.page_size().max(1);
        let target = (current as isize + step).max(0) as usize;
        self.goto_page(target);
    }

    /// 跳到第 `page` 页（从 0 起），高亮落到页首；越界夹到最后一页。
    pub(super) fn goto_page(&mut self, page: usize) {
        let count = self.candidate_count();
        let page_size = self.page_size();
        if count == 0 || page_size == 0 {
            self.highlight = 0;
            return;
        }
        let target = page.min((count - 1) / page_size);
        if target != self.highlight / page_size {
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

    /// 选中第 `index` 个候选：候选把整段转换完时返回要上屏的文本，只吃一部分时返回 `None`
    /// （这一选择并进组句，壳重画预编辑即可）。没有这一格也返回 `None`。
    pub(super) fn commit_index(&mut self, index: usize) -> Option<String> {
        let candidate = self.layout_candidate(index)?;
        self.engine.commit(&candidate)
    }

    /// 按当前状态生成一帧：没在组句给空帧；否则给高亮所在的那一页。
    pub(super) fn current_frame(&self) -> Frame {
        self.raw_frame()
    }

    /// 自绘候选窗用的帧：多释义选择时整屏换成释义，否则在底部那一行补上翻译 Tip。
    /// 发给 DLL 的那份（[`Self::current_frame`]）不带这两样东西——DLL 只是渲染端，用不上。
    pub(super) fn self_drawn_frame(&self) -> Frame {
        let mut frame = self.raw_frame();
        if let Some(choices) = self.translate.choices() {
            frame.tip = None;
            // 释义选择不是候选页那一屏：高亮是「第几条释义」，页码由壳那边按 `tip_choices` 去掉
            frame.highlight = choices.highlight;
            // 展开态（Tab）下临时收回单列 / 单行：这一屏是释义列表，不该按候选网格铺
            frame.columns = 0;
            frame.tip_choices = Some(TipChoices {
                word: choices.word.clone(),
                senses: choices.senses.clone(),
            });
            return frame;
        }
        if self.config.translate_enabled {
            frame.tip = self.translate.tip(&self.highlighted_text());
        }
        frame
    }

    /// 当前高亮候选的文本：本地词典按它查词条。
    pub(super) fn highlighted_text(&self) -> String {
        self.layout_candidate(self.highlight)
            .map(|candidate| candidate.text)
            .unwrap_or_default()
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
                columns: 0,
                tip: None,
                tip_choices: None,
                notice: self.notice.clone(),
            },
            Some(Composed::Candidates {
                preedit,
                cursor,
                layout,
                ..
            }) => {
                let page_size = self.page_size();
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
                    columns: self.grid_columns(),
                    tip: None,
                    tip_choices: None,
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
