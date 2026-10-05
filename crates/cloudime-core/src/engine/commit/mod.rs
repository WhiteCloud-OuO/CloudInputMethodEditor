//! 上屏：按候选消耗缓冲区、对齐音节、学习与撤销、自动造词。

use super::alignment::Alignment;
use super::input_log::{InputLogEntry, InputLogger, InputSource};
use super::learning::Learner;
use super::query::EnglishTail;
use super::{
    AUTO_WORD_MAX_CHARS, AUTO_WORD_THRESHOLD, AUTO_WORD_THRESHOLD_SAME_BUFFER,
    EXPLICIT_TRANSITION_WEIGHT, Engine, choice_key, segment_longest_prefix,
};
use crate::candidate::{Candidate, CandidateKind};
use crate::correction::typo;
use crate::{parser, sentence};
use cloudime_dictionary::Dictionary;

mod chain;
mod last;
mod transition;

pub(super) use chain::CommitChain;
pub use last::LastCommit;
pub use transition::Transition;

impl Engine {
    /// 选中一个候选：记入学习，从缓冲区消耗掉它对应的拼音，并把这一选择并进组句。
    ///
    /// 返回 `None` 表示还留在组句里（这次选的文本并进已选段，后面还有未选拼音，壳应接着 [`Self::query`]）；
    /// 返回 `Some(text)` 表示整段转换完了，`text` 是「已选文本 + 本次」拼起来的整体上屏文本，组句已清空。
    ///
    /// 候选比输入短时（`kaifazhe` 选了 开发），剩余拼音留在缓冲区；候选的最后一个音节比输入长时
    /// （`kaif` 选了 开发），把输入吃完。选中不立刻落进文档，等整段转换完或回车 / 标点时再一次性交给应用。
    pub fn commit(&mut self, candidate: &Candidate) -> Option<String> {
        self.commit_with(candidate, InputSource::from(candidate.kind))
    }

    /// [`Self::commit`] 的内部形式：`source` 写进输入日志（来源不同时的记录字段），
    /// 便于测试或壳在特殊来源下直接调用。
    pub(super) fn commit_with(
        &mut self,
        candidate: &Candidate,
        source: InputSource,
    ) -> Option<String> {
        let traditional_text = candidate.text.clone();
        let mut candidate_owned = candidate.clone();
        if self.traditional
            && let Some(simp) = self.traditional_map.borrow().get(&candidate_owned.text)
        {
            candidate_owned.text = simp.clone();
        }
        let candidate = &candidate_owned;
        // 整句不是一个词，不记词频；按路径上的词逐条记转移（喂个人 n-gram），路径要在拼音消耗前重算
        let sentence_words = (candidate.kind == CandidateKind::Sentence)
            .then(|| self.sentence_words(candidate))
            .flatten();
        // 下面每条路都可能改学习数据，格子候选的排序跟着变
        self.forget_span_cache();
        // 一段拼音里的第一个词：记下整段的学习键，整段分几次选完时合起来看（见 [`Self::finish_buffer`]）；
        // `split` 表示这次上屏接在同一段拼音里前一次上屏之后
        let split = self.chain.same_buffer();
        if !split {
            let (_, key) = self.whole_scope();
            self.chain.begin_buffer(key);
        }
        let mut typos = Vec::new();
        let (consumed, input) = match candidate.kind {
            CandidateKind::Chinese => {
                self.learner.record(candidate);
                let (consumed, input) = self.consumed_by(candidate);
                self.learner.record_choice(&input, &candidate.text);
                typos = self.accepted_typos(candidate);
                (consumed, input)
            }
            CandidateKind::Sentence => {
                typos = self.accepted_typos(candidate);
                self.consumed_by(candidate)
            }
            // 英文词与快捷候选对应整段作用域；选中的英文词记次数并进个人英文词表，下次同样的前缀它靠前
            CandidateKind::English | CandidateKind::Shortcut | CandidateKind::Custom => {
                if candidate.kind == CandidateKind::English {
                    self.learner.record(candidate);
                    self.learner.learn_english(&candidate.text);
                }
                self.whole_scope()
            }
        };
        self.apply_retraction(&input, &candidate.text);
        self.recording.clear();
        for (typed, intended) in &typos {
            tracing::debug!(typed, intended, "记录敲错");
            self.learner.record_typo(typed, intended);
        }
        let keys =
            self.composition.scope()[..consumed.min(self.composition.scope().len())].to_owned();
        let log_id = self.log_commit(&keys, &candidate.text, source);
        self.meter_commit(&candidate.text, source, false);
        // 上屏的中文词记进词汇记录（英文候选、整句等不算「一个词」）
        if !self.private && matches!(candidate.kind, CandidateKind::Chinese) {
            self.vocabulary.record_commit(&candidate.text);
        }
        // 选中：拼音移出缓冲区、并进已选段；`buffer_left` 看的是还没转换的未选拼音。
        self.composition.select_prefix(&traditional_text, consumed);
        let buffer_left = !self.composition.text().is_empty();
        match candidate.kind {
            CandidateKind::Chinese => {
                self.record_word(
                    &candidate.text,
                    &candidate.syllables,
                    EXPLICIT_TRANSITION_WEIGHT,
                    true,
                    buffer_left,
                );
            }
            CandidateKind::Sentence => match sentence_words {
                Some(words) => {
                    // 句末的英文词（我想学好rust 的 rust）记进个人英文词表，和英文候选上屏一样
                    if let Some(word) = words.last().filter(|w| is_english_word(w)) {
                        self.learner.learn_english(&word.text);
                    }
                    // 紧接着同一段拼音里自选的词（`jidiaole` 选了 挤，剩下的 掉了 走整句）：接缝是用户自己定的，
                    // 第一个词的转移按自选记双份。不参与两词造词：我 + 的… 这种接缝太常见、转移计数早就够了，
                    // 会把 我的 一类造成用户词；整段合成词由 [`Self::finish_buffer`] 管
                    let junction = self.chain.same_buffer() && self.chain.previous().is_some();
                    let last = words.len() - 1;
                    for (index, word) in words.iter().enumerate() {
                        let times = if index == 0 && junction {
                            EXPLICIT_TRANSITION_WEIGHT
                        } else {
                            1
                        };
                        self.record_word(
                            &word.text,
                            &word.syllables,
                            times,
                            false,
                            buffer_left || index < last,
                        );
                    }
                }
                None => self.chain.reset(),
            },
            CandidateKind::English | CandidateKind::Shortcut | CandidateKind::Custom => {
                self.chain.reset()
            }
        }
        // 一次整句上屏里的几个词不算分段选，只有这段拼音经过至少两次上屏才合起来看
        let phrase = if split && !buffer_left {
            self.finish_buffer()
        } else {
            None
        };
        self.punctuation.note_committed(&candidate.text);
        self.history.record(&candidate.text);
        let learned = matches!(
            candidate.kind,
            CandidateKind::Chinese | CandidateKind::Sentence
        );
        let commit = if learned {
            LastCommit {
                text: candidate.text.clone(),
                chars: traditional_text.chars().count(),
                input,
                chosen: matches!(candidate.kind, CandidateKind::Chinese)
                    .then(|| candidate.text.clone()),
                transitions: std::mem::take(&mut self.recording),
                typos,
                erased: 0,
                log_id,
                phrase,
            }
        } else {
            let mut plain = LastCommit::plain(&candidate.text);
            plain.chars = traditional_text.chars().count();
            plain
        };
        self.remember_commit(commit);
        if buffer_left {
            // 还有未选拼音：这次选择只是并进组句，等后面选完或回车 / 标点再整体交给应用。
            None
        } else {
            // 整段转换完：已选文本（这次选择的文本已并进去）+ 本次一起交给应用，组句清空。
            let selected = self.composition.selected_text();
            self.composition.clear();
            Some(selected)
        }
    }

    /// 一段拼音分几次选完了（`jidiaole` 先选 挤、剩下的走整句 掉了）：这几个词合起来就是用户对这段拼音的答案。
    /// 记一次「整段拼音 → 合成词」的选择；选到 [`AUTO_WORD_THRESHOLD_SAME_BUFFER`] 次、词库里没有、
    /// 不超过 [`AUTO_WORD_MAX_CHARS`] 字就造成用户词，下次整段打出来它直接排第一。返回记下的选择，撤销时退回。
    pub(super) fn finish_buffer(&mut self) -> Option<(String, String)> {
        let words = self.chain.buffer_words();
        if words.len() < 2 {
            return None;
        }
        let text: String = words.iter().map(|(t, _)| t.as_str()).collect();
        let syllables: Vec<String> = words.iter().flat_map(|(_, s)| s.iter().cloned()).collect();
        let chars = text.chars().count();
        let key = self.chain.buffer_key().to_owned();
        if key.is_empty() || chars > AUTO_WORD_MAX_CHARS || chars != syllables.len() {
            return None;
        }
        self.learner.record_choice(&key, &text);
        let candidate = Candidate {
            text,
            display: None,
            kind: CandidateKind::Chinese,
            syllables,
            reading: None,
        };
        if !self.knows_word(&candidate)
            && self.learner.choice_weight(&key, &candidate.text) >= AUTO_WORD_THRESHOLD_SAME_BUFFER
        {
            tracing::debug!(text = %candidate.text, "整段拼音分次选完，自动造词");
            let weight = self.initial_user_weight(&candidate.text, &candidate.syllables);
            self.learner
                .learn_word(&candidate.text, &candidate.syllables, weight);
        }
        Some((key, candidate.text))
    }

    /// Ctrl + 数字 上屏整句候选时调：把「整段输入 → 这个整句」记一次选择，并（≤[`AUTO_WORD_MAX_CHARS`] 字、
    /// 词库里没有、记够 [`AUTO_WORD_THRESHOLD_SAME_BUFFER`] 次时）收进用户自造词库。
    /// 要在 [`Self::commit`] **之前**调：整段输入键还没被消耗，记选择用的键才对得上。返回这次是否刚好收进词库。
    pub fn remember_sentence(&mut self, candidate: &Candidate) -> bool {
        if candidate.kind != CandidateKind::Sentence {
            return false;
        }
        let chars = candidate.text.chars().count();
        if chars == 0 || chars > AUTO_WORD_MAX_CHARS || chars != candidate.syllables.len() {
            return false;
        }
        let (_, key) = self.whole_scope();
        if key.is_empty() {
            return false;
        }
        self.learner.record_choice(&key, &candidate.text);
        if self.knows_word(candidate)
            || self.learner.choice_weight(&key, &candidate.text) < AUTO_WORD_THRESHOLD_SAME_BUFFER
        {
            return false;
        }
        tracing::debug!(text = %candidate.text, "整句候选记够两次，收进用户词库");
        let weight = self.initial_user_weight(&candidate.text, &candidate.syllables);
        self.learner
            .learn_word(&candidate.text, &candidate.syllables, weight);
        true
    }

    /// 最近删掉的上屏里有一次是同一段拼音（或它的前缀）、这次却选了别的词：把那次记的学习退回去。
    /// 删掉几个词再从头重打是常事（「沃德 书」删掉重打成「我的 书」），所以往前找最近一次删干净的同段拼音；
    /// 重打后选的还是同一个词就不算选错，只把那条记录丢掉。这次的拼音与删掉的哪次都对不上时，删掉的那些当作在改别处，全忘掉。
    pub(super) fn apply_retraction(&mut self, input: &str, text: &str) {
        let Some(index) = self
            .recent_commits
            .iter()
            .rposition(|c| c.is_erased() && c.same_input(input))
        else {
            // 删掉的那次拼音与这次相近但不同（wode 删了重打 wodi）：重打了键，记进输入日志给敲错纠正当样本；学习不动
            if let Some(erased) = self
                .recent_commits
                .iter()
                .rev()
                .find(|c| c.is_erased() && is_retyped(&c.input, input))
            {
                self.logger.record(InputLogEntry::Retype {
                    before: erased.input.clone(),
                    after: input.to_owned(),
                    of: erased.log_id,
                });
            }
            self.recent_commits.retain(|c| !c.is_erased());
            return;
        };
        let last = self.recent_commits.remove(index);
        if last.text == text {
            return;
        }
        tracing::debug!(retracted = %last.text, chosen = %text, "上次选错了，撤销它的学习");
        self.logger.record(InputLogEntry::Retract {
            of: last.log_id,
            text: last.text.clone(),
            chosen: text.to_owned(),
        });
        if let Some(chosen) = &last.chosen {
            self.learner.unrecord(chosen);
            self.learner.unrecord_choice(&last.input, chosen);
        }
        for transition in &last.transitions {
            self.learner.unrecord_transition(
                transition.context(),
                &transition.word,
                transition.times,
            );
        }
        for (typed, intended) in &last.typos {
            self.learner.unrecord_typo(typed, intended);
        }
        if let Some((key, phrase)) = &last.phrase {
            self.learner.unrecord_choice(key, phrase);
        }
    }

    /// 这个候选上屏算接受了哪些音节级敲错：纠错读法按那处编辑换算，普通读法靠对齐时对上的敲错变体。
    pub(super) fn accepted_typos(&self, candidate: &Candidate) -> Vec<(String, String)> {
        let keys = self.composition.scope();
        if let Some(correction) = self.correction_reading_for(keys, &candidate.syllables) {
            let alignment = self.align(&correction.corrected, &candidate.syllables);
            let mut typos = alignment.typos;
            typos.extend(correction.typo_pair(alignment.consumed));
            return typos;
        }
        self.align(keys, &candidate.syllables).typos
    }

    /// 整句候选对应的词序列：重算一次整句转换，文本对得上才算（对不上说明候选来自别处，不记）。
    /// 末尾是英文词的整句（我想学好rust 的 rust）先按头段试，英文词作为最后一个词（音节就是敲的字母）。
    pub(super) fn sentence_words(
        &self,
        candidate: &Candidate,
    ) -> Option<Vec<sentence::SentenceWord>> {
        let scope = self.composition.scope();
        if let Some(tail) = self.split_english_tail(scope)
            && let Some(words) = self.mixed_words(scope, &tail, &candidate.text)
        {
            return Some(words);
        }
        let conversion = {
            let (segmentations, _) = segment_longest_prefix(scope, self.use_jian_pin).ok()?;
            self.convert_sentence(&segmentations.first()?.patterns())?
        };
        (conversion.text == candidate.text).then_some(conversion.words)
    }

    /// 头段拼音的转换加上英文尾段，与 `text` 对得上时的词序列。
    fn mixed_words(
        &self,
        scope: &str,
        tail: &EnglishTail,
        text: &str,
    ) -> Option<Vec<sentence::SentenceWord>> {
        let segmentations =
            parser::segment_with(&scope[..tail.head_len], self.use_jian_pin).ok()?;
        let mut conversion = self.convert_sentence(&segmentations.first()?.patterns())?;
        conversion.text.push_str(&tail.word);
        if conversion.text != text {
            return None;
        }
        conversion.words.push(sentence::SentenceWord {
            text: tail.word.clone(),
            syllables: vec![scope[tail.head_len..].to_owned()],
            placeholder: false,
        });
        Some(conversion.words)
    }

    /// 候选消耗多少作用域字节，以及按输入串记学习用的键（候选覆盖的那段全拼字母）。
    /// 纠错读法按纠正后的拼音算，再按那处编辑换算回原串。
    pub(super) fn consumed_by(&self, candidate: &Candidate) -> (usize, String) {
        let keys = self.composition.scope();
        let consumed = match self.correction_reading_for(keys, &candidate.syllables) {
            Some(correction) => correction
                .edit
                .to_original(
                    self.align(&correction.corrected, &candidate.syllables)
                        .consumed,
                )
                .min(keys.len()),
            None => self.align(keys, &candidate.syllables).consumed,
        };
        (consumed, choice_key(keys, consumed))
    }

    /// 候选的音节逐个对到输入上：每个音节按原样 / 规范写法、模糊音或敲错变体（长的在前）、没打完的前缀消耗输入，
    /// 先找能把**每个音节都对上**的对齐（带回溯：`pingyin` 对 拼音 时 `pin` 原样只吃三个字母会剩下 `gyin`，
    /// 退回来按敲错变体 `ping` → `pin` 吃四个），找不到才退回逐个贪心对、对不上的地方停下。
    pub(super) fn align(&self, input: &str, syllables: &[String]) -> Alignment {
        let mut typos = Vec::new();
        self.align_full(input, 0, syllables, &mut typos)
            .unwrap_or_else(|| self.align_greedy(input, syllables))
    }

    /// 从 `pos` 起把剩下的音节全对上的第一种对齐（按每步的优先级深度优先）；对不上返回 `None`。
    fn align_full(
        &self,
        input: &str,
        pos: usize,
        syllables: &[String],
        typos: &mut Vec<(String, String)>,
    ) -> Option<Alignment> {
        let Some((syllable, remaining)) = syllables.split_first() else {
            return Some(Alignment {
                consumed: pos,
                typos: typos.clone(),
            });
        };
        let (rest, start) = self.rest_at(input, pos);
        for (len, typo) in self.syllable_steps(rest, syllable) {
            // 消耗完输入后还有音节没对：不算全对上（候选比敲的长）
            if start + len == input.len() && !remaining.is_empty() {
                continue;
            }
            if typo {
                typos.push((rest[..len].to_owned(), syllable.clone()));
            }
            let found = self.align_full(input, start + len, remaining, typos);
            if typo {
                typos.pop();
            }
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// 逐个贪心对：每个音节取第一种能对上的消耗，都对不上就取公共前缀，公共前缀也没有就停。
    fn align_greedy(&self, input: &str, syllables: &[String]) -> Alignment {
        let mut alignment = Alignment::default();
        let mut pos = 0;
        for syllable in syllables {
            let (rest, start) = self.rest_at(input, pos);
            if let Some((len, typo)) = self.syllable_steps(rest, syllable).into_iter().next() {
                if typo {
                    alignment
                        .typos
                        .push((rest[..len].to_owned(), syllable.clone()));
                }
                pos = start + len;
                continue;
            }
            let common = syllable
                .bytes()
                .zip(rest.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            if common == 0 {
                break;
            }
            pos = start + common;
        }
        alignment.consumed = pos;
        alignment
    }

    /// `pos` 处这个音节能看到的输入段（到下一个 `'` 为止）与它的起点（跳过开头的 `'`）。
    fn rest_at<'a>(&self, input: &'a str, pos: usize) -> (&'a str, usize) {
        let start = if pos > 0 && input[pos..].starts_with('\'') {
            pos + 1
        } else {
            pos
        };
        let rest = &input[start..];
        let rest = &rest[..rest.find('\'').unwrap_or(rest.len())];
        (rest, start)
    }

    /// 一个音节可以怎么消耗输入段 `rest`：(消耗字节数, 是否靠敲错变体)，按优先级排：
    /// 原样或规范写法（`lue` / `lve`）、模糊音或敲错变体（长的在前）、整段是这个音节没打完的前缀。
    fn syllable_steps(&self, rest: &str, syllable: &str) -> Vec<(usize, bool)> {
        let mut steps = Vec::new();
        let exact = rest.get(..syllable.len()).is_some_and(|typed| {
            typed == syllable
                || cloudime_dictionary::canonical_syllable(typed)
                    == cloudime_dictionary::canonical_syllable(syllable)
        });
        if exact {
            steps.push((syllable.len(), false));
        }
        // 没打完排在变体前面：`shijia` 选 时间 是 jian 没敲完，不是把 jian 敲成了 jia
        if !rest.is_empty() && rest.len() < syllable.len() && syllable.starts_with(rest) {
            steps.push((rest.len(), false));
        }
        for len in (1..=rest.len().min(parser::MAX_SYLLABLE_LEN)).rev() {
            if exact && len == syllable.len() {
                continue;
            }
            let typed = &rest[..len];
            if self.fuzzy.is_variant(typed, syllable) {
                steps.push((len, false));
            } else if typo::is_variant(typed, syllable) {
                steps.push((len, true));
            }
        }
        steps
    }

    /// 整段作用域对应的候选（英文词、云端词、快捷候选）：吃掉全部键，学习键是整段全拼。
    pub(super) fn whole_scope(&self) -> (usize, String) {
        let keys = self.composition.scope();
        (keys.len(), choice_key(keys, keys.len()))
    }

    /// 一个中文词上屏了：记 `times` 份转移、推进链；`auto_word` 为真（用户自己选的词）时，
    /// 紧接着上一个词、合起来词库里没有、且这条接续记够次数还自动造词。
    pub(super) fn record_word(
        &mut self,
        text: &str,
        syllables: &[String],
        times: u32,
        auto_word: bool,
        buffer_left: bool,
    ) {
        self.learner
            .record_transition(self.chain.context(), text, times);
        self.recording
            .push(Transition::new(self.chain.context(), text, times));
        if auto_word {
            let threshold = if self.chain.same_buffer() {
                AUTO_WORD_THRESHOLD_SAME_BUFFER
            } else {
                AUTO_WORD_THRESHOLD
            };
            self.try_auto_word(text, syllables, threshold);
        }
        self.chain.advance(text, syllables, buffer_left);
    }

    /// 上一个词 + 这个词合成用户词的条件见 [`AUTO_WORD_THRESHOLD`]。
    pub(super) fn try_auto_word(&mut self, text: &str, syllables: &[String], threshold: u32) {
        let Some(previous) = self.chain.previous().map(str::to_owned) else {
            return;
        };
        let joined = format!("{previous}{text}");
        let chars = joined.chars().count();
        let mut joined_syllables = self.chain.previous_syllables().to_vec();
        joined_syllables.extend(syllables.iter().cloned());
        if chars > AUTO_WORD_MAX_CHARS || chars != joined_syllables.len() {
            return;
        }
        // 这条转移刚记过，计数已含本次；阈值按「选了几次」算，计数是按份记的
        let seen = self
            .learner
            .user_ngram()
            .map_or(0, |b| b.pair(Some(&previous), text));
        if seen < threshold * EXPLICIT_TRANSITION_WEIGHT {
            return;
        }
        let candidate = Candidate {
            text: joined,
            display: None,
            kind: CandidateKind::Chinese,
            syllables: joined_syllables,
            reading: None,
        };
        if self.knows_word(&candidate) {
            return;
        }
        tracing::debug!(text = %candidate.text, "自动造词");
        let weight = self.initial_user_weight(&candidate.text, &candidate.syllables);
        self.learner
            .learn_word(&candidate.text, &candidate.syllables, weight);
    }

    /// 自造词的初始权重：各字在词库里的词频最大值（查不到的字按 1 算）。
    /// 「每个字的权重」取的是该字在词库里的静态词频（见 `docs/notes/crate-notes.md`）。
    pub(super) fn initial_user_weight(&self, text: &str, syllables: &[String]) -> f64 {
        let dictionaries = self.all_dictionaries();
        let mut max = 1.0f64;
        for (index, ch) in text.chars().enumerate() {
            let Some(syllable) = syllables.get(index) else {
                break;
            };
            let frequency = dictionaries
                .iter()
                .flat_map(|dictionary| dictionary.lookup(&[syllable.as_str()], false))
                .filter(|hit| hit.exact && hit.text.chars().eq(std::iter::once(ch)))
                .map(|hit| hit.frequency)
                .max();
            if let Some(frequency) = frequency {
                max = max.max(f64::from(frequency));
            }
        }
        max
    }

    /// 主词库或用户词里是否已有这个词（同音节）。
    pub(super) fn knows_word(&self, candidate: &Candidate) -> bool {
        let syllables: Vec<&str> = candidate.syllables.iter().map(String::as_str).collect();
        if syllables.is_empty() {
            return true;
        }
        let known = |dictionary: &Dictionary| {
            dictionary
                .lookup(&syllables, false)
                .iter()
                .any(|hit| hit.exact && hit.text == candidate.text)
        };
        self.all_dictionaries().into_iter().any(known)
    }
}

/// 整句路径上的词是英文词（`woxiangxuehaorust` 的 rust）：不是占位音节、全是字母。
fn is_english_word(word: &sentence::SentenceWord) -> bool {
    !word.placeholder && !word.text.is_empty() && word.text.bytes().all(|b| b.is_ascii_alphabetic())
}

/// 删掉重打的键串算不算「同一段拼音打错了」：都够长、不相等、编辑距离不超过 [`RETYPE_MAX_EDITS`]。
/// 差得更多的是在改别的内容，不算。
fn is_retyped(before: &str, after: &str) -> bool {
    before.len() >= RETYPE_MIN_LETTERS
        && after.len() >= RETYPE_MIN_LETTERS
        && before != after
        && edit_distance(before, after) <= RETYPE_MAX_EDITS
}

/// 跨上屏重打的键串最少几个字母才算数。
const RETYPE_MIN_LETTERS: usize = 3;

/// 跨上屏重打最多差几处编辑。
const RETYPE_MAX_EDITS: usize = 2;

/// Levenshtein 编辑距离（按字节，键串都是 ASCII）。
fn edit_distance(a: &str, b: &str) -> usize {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}
