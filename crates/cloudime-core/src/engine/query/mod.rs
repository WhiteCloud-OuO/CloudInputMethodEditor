//! 候选生成：按模式分派查询，整句转换与词级查找，位置展开。

use super::*;

mod english_tail;
mod result;
mod snapshot;
mod spelling;

pub(crate) use english_tail::EnglishTail;
pub use result::Query;
pub(super) use result::join_marked;
pub(super) use result::join_marked_typed;
pub(super) use snapshot::QuerySnapshot;

use std::collections::HashSet;

impl Engine {
    /// 解析当前缓冲区并生成排好序的候选。**不带译文**，译文由 [`Self::annotate`] 补。
    ///
    /// 光标停在拼音中间时只按光标前的那段算候选（`ni|hao` 出 你），光标后的拼音留着，
    /// 上屏之后接着组句；见 [`Composition::scope`]。
    pub fn query(&self) -> Result<Query, ParseError> {
        self.last_rescored.set(false);
        let mut query = match self.query_inner() {
            Ok(query) => query,
            Err(error) => {
                if !self
                    .custom_phrases
                    .iter()
                    .any(|p| p.code == self.composition.scope())
                {
                    return Err(error);
                }
                Query::custom_only(
                    &self.composition.typed_text(),
                    self.composition.cursor(),
                    self.composition.scope(),
                    self.marked_rest(self.composition.rest()),
                )
            }
        };
        self.insert_custom_phrases(&mut query.candidates.items);
        // 给输入日志留个摘要：上屏时才知道选了什么，这里才知道看到了什么
        let pinyin = match &query.correction {
            Some(correction) => correction.segmentation.joined("'"),
            None => join_marked(&query.segmentations, &query.tail),
        };
        *self.last_query.borrow_mut() = Some(QuerySnapshot {
            scope: self.composition.scope().to_owned(),
            pinyin,
            corrected: query.correction.is_some(),
            candidates: query
                .candidates
                .items
                .iter()
                .take(QuerySnapshot::MAX_CANDIDATES)
                .map(|c| c.text.clone())
                .collect(),
            rescored: self.last_rescored.get(),
        });

        if self.traditional
            && let Some(opencc) = &self.opencc
        {
            for candidate in &mut query.candidates.items {
                if matches!(
                    candidate.kind,
                    CandidateKind::Chinese | CandidateKind::Sentence
                ) {
                    let traditional_text = opencc.convert(&candidate.text);
                    self.traditional_map
                        .borrow_mut()
                        .insert(traditional_text.clone(), candidate.text.clone());
                    candidate.text = traditional_text;
                }
            }
        }

        Ok(query)
    }

    pub(super) fn query_inner(&self) -> Result<Query, ParseError> {
        let start = Instant::now();
        let keys = self.composition.scope();
        let rest = self.marked_rest(self.composition.rest());
        if self.english_mode {
            return Ok(self.query_english(keys, rest, start));
        }
        if keys.starts_with(EXPRESSION_PREFIX) {
            return Ok(self.query_expression(keys, rest, start));
        }
        if is_raw(keys) {
            return Ok(self.query_raw(keys, rest, start));
        }
        self.query_phonetic(keys, rest, start)
    }

    /// 拼音侧的候选生成：整段作用域是一串读音。
    fn query_phonetic(
        &self,
        keys: &str,
        rest: String,
        start: Instant,
    ) -> Result<Query, ParseError> {
        let scope: &str = keys;
        // 末尾是英文词（`woxiangxuehaorust`）：拼音候选与整句只按头段算，尾段整个跟在整句后面。
        // 整段也能读成拼音时（`database`、`…rust` 当简拼）两种读法比分，英文赢了才按头段算，
        // 输了整段按拼音读、英文读法排在拼音整句后面
        let english_tail = self.split_english_tail(keys);
        let head_wins = english_tail
            .as_ref()
            .is_some_and(|t| !t.competes || self.mixed_beats_plain(keys, t));
        let parsed = match &english_tail {
            Some(tail) if head_wins => {
                parser::segment_with(&keys[..tail.head_len], self.use_jian_pin).map(|s| (s, ""))
            }
            _ => segment_longest_prefix(keys, self.use_jian_pin),
        };
        // 连第一个字母都切不动（`impor`）：拼音这边没戏，但英文词 / 补全、快捷候选还可以有
        let (segmentations, tail) = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                let mut pool: Vec<(Candidate, PoolRank)> = Vec::new();
                let full = keys.chars().filter(|c| *c != '\'').count();
                self.push_english(&mut pool, true, full, 0, false);
                let mut items = rank_pool(pool);
                self.insert_shortcuts(&mut items, keys);
                if items.is_empty() {
                    return Err(error);
                }
                return Ok(Query {
                    segmentations: Vec::new(),
                    candidates: CandidateList { items },
                    tail: keys.to_owned(),
                    text: self.composition.typed_text(),
                    cursor: self.composition.cursor(),
                    rest,
                    typed_display: None,
                    correction: None,
                    timings: Timings {
                        parse: start.elapsed(),
                        lookup: Duration::ZERO,
                        rank: Duration::ZERO,
                    },
                });
            }
        };
        let unlikely = correction::unlikely_pinyin(segmentations.first(), tail)
            || correction::trailing_single_letter(segmentations.first());
        // 仅末尾落单字母时只试换位，省每键的变体枚举开销
        let substitutes = correction::unlikely_pinyin(segmentations.first(), tail);
        let parse = start.elapsed();

        let start = Instant::now();
        let mut scored = Vec::new();
        // 不同切分共享很多前缀（`zh g d o…` 的各种切法前几段一样），同一次查询里同一个模式只查一遍
        let mut memo: HashMap<String, Vec<Match<'_>>> = HashMap::new();
        // 原样输入本身能否整段对一个词（决定纠错读法要不要回补折扣）
        let scope_letters: usize = scope.chars().filter(|c| *c != '\'').count();
        let mut original_exact = false;
        for segmentation in &segmentations {
            let mut patterns = segmentation.patterns();
            let count = patterns.len();
            let last = &segmentation.syllables[count - 1];
            // 最后一个音节即使打完了也可能还没打完（`xia` 可能是 `xiang` 的前缀），按前缀查
            if last.complete && parser::is_syllable_prefix(&last.text) {
                patterns[count - 1].complete = false;
            }
            // 词级候选只按敲的原样与模糊音查；一处编辑的纠错读法由 [`Self::correction_readings`] 另加
            let expanded = self.fuzzy.expand(&patterns);
            let positions = expanded.positions();
            let abbreviated = abbreviated_count(&patterns);
            let hits = self.lookup_all(&positions);
            scored.reserve(hits.len());
            for hit in hits {
                if hit.exact && segmentation.letters() == scope_letters {
                    original_exact = true;
                }
                let penalty = expanded.penalty(hit.syllables());
                scored.push(Scored {
                    coverage: segmentation.letters(),
                    full_last: last.complete
                        && hit.syllables().nth(count - 1) == Some(patterns[count - 1].text),
                    abbreviated,
                    reading_syllables: count,
                    tier: u8::from(penalty > 0.0),
                    correction: (-penalty).exp(),
                    weight: self.learner.rank_weight(hit.text),
                    hit,
                });
            }
            // 输入的前缀也出候选（`kaifazhe` → 开发、开），否则长句没法逐词上屏。
            // 只收音节数正好等于前缀长度的词，更长的词会与输入后面的音节冲突。
            // 前缀不含最后一个位置，因此可复用上面的扩展结果。
            for prefix_len in (1..count).rev() {
                let prefix = &patterns[..prefix_len];
                let prefix_letters: usize = prefix.iter().map(|p| p.text.len()).sum();
                let hits = memo
                    .entry(pattern_key(prefix))
                    .or_insert_with(|| self.lookup_exact_all(&positions[..prefix_len]));
                let abbreviated = abbreviated_count(prefix);
                for hit in hits.iter().copied() {
                    let penalty = expanded.penalty(hit.syllables());
                    scored.push(Scored {
                        // 对整个输入来说它不是精确命中，只是覆盖了前面一部分
                        hit: Match {
                            exact: false,
                            ..hit
                        },
                        coverage: prefix_letters,
                        // 前缀词的读法在覆盖到的那一段里是完整的，末音节自然对得上
                        full_last: true,
                        abbreviated,
                        reading_syllables: prefix_len,
                        tier: u8::from(penalty > 0.0),
                        correction: (-penalty).exp(),
                        weight: self.learner.rank_weight(hit.text),
                    });
                }
            }
        }
        // 原样能整段对上（有音节数正好拼满整段的精确命中，或自定义短语的输入码正好是整段）
        let original_matches =
            original_exact || self.custom_phrases.iter().any(|p| p.code == scope);
        // 拼音「不像话」时把一处编辑的纠错读法作为额外读法加进同一个候选池，按权重打折。
        // 原样输入本身就能整段拼出一个词时（`Cpan` 的 C盘）不再纠：那多半是简拼 / 字母混输，
        // 一处编辑只会用一个补出来的字母凑出别的词，凭更多覆盖字母把原样的词挤下去。
        if unlikely && !original_matches {
            for (correction, factor) in self.correction_hits(scope, original_matches, substitutes) {
                let patterns = reading_positions(&correction.segmentation);
                let positions: Vec<Vec<cloudime_dictionary::SyllablePattern<'_>>> =
                    patterns.iter().map(|p| vec![*p]).collect();
                let count = patterns.len();
                let letters: usize = patterns.iter().map(|p| p.text.len()).sum();
                let last = &correction.segmentation.syllables[count - 1];
                let abbreviated = abbreviated_count(&patterns);
                for hit in self.lookup_all(&positions) {
                    scored.push(Scored {
                        coverage: letters,
                        full_last: last.complete
                            && hit.syllables().nth(count - 1) == Some(patterns[count - 1].text),
                        abbreviated,
                        reading_syllables: count,
                        tier: 2,
                        correction: factor,
                        weight: self.learner.rank_weight(hit.text),
                        hit,
                    });
                }
            }
        }
        let lookup = start.elapsed();

        let start = Instant::now();
        // 再往后翻也翻不到的候选不必再造：单字母简拼能命中两万个词，排完序只留前面这些。
        // 同输入串（候选覆盖的那段字母）下选过的优先；上下文是上一个上屏的词（句首为 None）
        let log_total = (self.total_frequency() as f64).max(1.0).ln();
        let letters = choice_key(scope, scope.len());
        let ranked = ranking::rank(
            &mut scored,
            MAX_CANDIDATES,
            self.association_counts(),
            log_total,
            |item| {
                let covered = item.coverage.min(letters.len());
                let choice = letters
                    .get(..covered)
                    .map_or(0, |input| self.learner.choice_weight(input, item.hit.text));
                let log_prob = sentence::transition_log_prob(
                    &*self.language_model,
                    self.personal(),
                    self.chain.context(),
                    item.hit.text,
                    sentence::fallback_log_prob(item.hit.frequency, log_total),
                );
                (choice, log_prob)
            },
        );
        let mut pool: Vec<(Candidate, PoolRank)> = ranked
            .iter()
            .map(|(item, weight)| {
                (
                    chinese_candidate(item),
                    (
                        item.coverage,
                        item.abbreviated,
                        item.full_last,
                        item.hit.exact,
                        item.tier,
                        *weight,
                    ),
                )
            })
            .collect();
        // 整句候选与词候选进同一个池子（同文本同读音不重复插；同文本不同读音的去掉词级那条）
        self.push_sentences(&mut pool, &segmentations, english_tail.as_ref(), head_wins);
        // 英文候选按英文词频 ×(1+用户次数) 一起排。结构键借用整段读法（敲的字母是同一串），
        // 否则英文词凭「没有简拼 / 末音节必然完整」天然压过同覆盖的中文简拼读法（`mp` 的 MP 压 门票）。
        let reading_structure = segmentations.first().map_or((0, false), |segmentation| {
            let patterns = segmentation.patterns();
            (
                abbreviated_count(&patterns),
                segmentation
                    .syllables
                    .last()
                    .is_some_and(|last| last.complete),
            )
        });
        self.push_english(
            &mut pool,
            unlikely,
            scope_letters,
            reading_structure.0,
            reading_structure.1,
        );
        let mut items = rank_pool(pool);
        let rank = start.elapsed();
        // 快捷候选按敲的键认（`rq` 日期），插在本地首选之后
        self.insert_shortcuts(&mut items, keys);

        // 按头段算时英文尾段不参与拼音候选，显示上跟在切分后面：`wo'xiang'xue'hao'rust`
        let tail = english_tail
            .as_ref()
            .filter(|_| head_wins)
            .map_or(tail, |t| &keys[t.head_len..]);
        let typed_display = self.composition.has_shifted().then(|| {
            // 中文模式下 Shift 敲的大写：匹配按小写算，拼音行仍按敲的样子显示（`Cpan`）
            join_marked_typed(&self.composition.typed_scope(), &segmentations, tail)
        });
        Ok(Query {
            segmentations,
            candidates: CandidateList { items },
            tail: tail.to_owned(),
            text: self.composition.typed_text(),
            cursor: self.composition.cursor(),
            rest,
            typed_display,
            correction: None,
            timings: Timings {
                parse,
                lookup,
                rank,
            },
        })
    }

    /// 表达式模式（`v` 开头）：不解析拼音，候选是算式结果 / 中文数字，再加上整段是英文词的情况（`very`）。
    /// preedit 原样显示输入。
    pub(super) fn query_expression(&self, scope: &str, rest: String, start: Instant) -> Query {
        let mut items = shortcut::candidates(scope, &jiff::Zoned::now());
        if let Some(word) = self.english.as_ref().and_then(|english| english.get(scope)) {
            items.push(Candidate {
                text: word.to_owned(),
                kind: CandidateKind::English,
                syllables: Vec::new(),
                reading: None,
            });
        }
        Query {
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            tail: scope.to_owned(),
            text: self.composition.text().to_owned(),
            cursor: self.composition.cursor(),
            rest,
            typed_display: None,
            correction: None,
            timings: Timings {
                parse: Duration::ZERO,
                lookup: Duration::ZERO,
                rank: start.elapsed(),
            },
        }
    }

    /// 英文直输段：唯一候选就是原文（`no-way`），空格 / 回车都上屏它；preedit 原样显示。
    pub(super) fn query_raw(&self, scope: &str, rest: String, start: Instant) -> Query {
        let items = vec![Candidate {
            text: scope.to_owned(),
            kind: CandidateKind::English,
            syllables: Vec::new(),
            reading: None,
        }];
        Query {
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            tail: scope.to_owned(),
            text: self.composition.text().to_owned(),
            cursor: self.composition.cursor(),
            rest,
            typed_display: None,
            correction: None,
            timings: Timings {
                parse: Duration::ZERO,
                lookup: Duration::ZERO,
                rank: start.elapsed(),
            },
        }
    }

    /// 英文模式：敲的字母原样显示，候选是英文词表的精确词、前缀补全与拼错纠正（见 [`english::suggest`]），
    /// 词表没装就没有候选。
    pub(super) fn query_english(&self, scope: &str, rest: String, start: Instant) -> Query {
        let items: Vec<Candidate> = english::suggest(
            &self.english_lists(),
            scope,
            |text| self.learner.weight(text),
            ENGLISH_MODE_CANDIDATES,
        )
        .into_iter()
        .map(|text| Candidate {
            text,
            kind: CandidateKind::English,
            syllables: Vec::new(),
            reading: None,
        })
        .collect();
        Query {
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            tail: scope.to_owned(),
            text: self.composition.text().to_owned(),
            cursor: self.composition.cursor(),
            rest,
            typed_display: None,
            correction: None,
            timings: Timings {
                parse: Duration::ZERO,
                lookup: Duration::ZERO,
                rank: start.elapsed(),
            },
        }
    }

    /// 整句候选进候选池。没有英文尾段时是整段拼音的转换（[`Self::plain_sentence`]）；
    /// 有英文尾段且英文读法胜出（`head_wins`）时，头段的转换加上那个词（`woxiangxuehaorust` → 我想学好rust），
    /// 整段也能读成拼音的再把拼音读法的整句也放进去；英文读法输了就不出（`diaoyong` 不出 掉Yong）。
    pub(super) fn push_sentences(
        &self,
        pool: &mut Vec<(Candidate, PoolRank)>,
        segmentations: &[Segmentation],
        english_tail: Option<&EnglishTail>,
        head_wins: bool,
    ) {
        let Some(best) = segmentations.first() else {
            return;
        };
        let keys = self.composition.scope();
        let full = keys.chars().filter(|c| *c != '\'').count();
        // 整句按整段读法算，结构键给它最好的一档：覆盖满、无简拼、末音节完整、命中精确
        let sentence_rank = |weight: f64| (full, 0, true, true, 0, weight);
        let first_segmentation = |text: &str| {
            parser::segment_with(text, self.use_jian_pin)
                .ok()?
                .into_iter()
                .next()
        };
        match english_tail {
            Some(tail) if head_wins => {
                if let Some((mixed, weight)) = self.mixed_sentence(best, tail) {
                    push_sentence(pool, mixed, sentence_rank(weight));
                }
                if tail.competes
                    && let Some(full_seg) = first_segmentation(keys)
                    && let Some((plain, weight)) = self.plain_sentence(pool, &full_seg)
                {
                    push_sentence(pool, plain, sentence_rank(weight));
                }
            }
            _ => {
                if let Some((plain, weight)) = self.plain_sentence(pool, best) {
                    push_sentence(pool, plain, sentence_rank(weight));
                }
            }
        }
    }

    /// 整段拼音的整句候选：最优切分至少两个音节、且最优路径不止一个词时才有。
    /// 整段本身就是词库里的词时不重复；有音节没转成字的不算句子。返回候选与它的权重。
    pub(super) fn plain_sentence(
        &self,
        pool: &[(Candidate, PoolRank)],
        best: &Segmentation,
    ) -> Option<(Candidate, f64)> {
        if best.syllables.len() < 2 {
            return None;
        }
        let conversion = self.convert_sentence(&best.patterns())?;
        // 不按原样读的路径（模糊音）不许压过「敲的拼音本身就是一个词」
        if conversion.altered() {
            let letters = best.joined("");
            let spelled_exactly = pool
                .iter()
                .any(|(c, _)| c.kind == CandidateKind::Chinese && c.syllables.concat() == letters);
            if spelled_exactly {
                return None;
            }
        }
        if conversion.has_placeholder() {
            return None;
        }
        // 整段本来就是一个词时不出整句；只读了一部分（末尾没打完的音节没算进去）的不插
        let kind = if conversion.word_count() >= 2 {
            CandidateKind::Sentence
        } else if conversion.altered() && conversion.syllables.len() == best.syllables.len() {
            CandidateKind::Chinese
        } else {
            return None;
        };
        let weight = conversion.weight();
        Some((
            Candidate {
                text: conversion.text,
                kind,
                syllables: conversion.syllables,
                reading: None,
            },
            weight,
        ))
    }

    /// 跑一次整句转换：主词库 + 用户词（含模糊音写法，命中的按代价扣分），静态语言模型与个人 n-gram 插值。
    pub(super) fn convert_sentence(
        &self,
        patterns: &[cloudime_dictionary::SyllablePattern<'_>],
    ) -> Option<Conversion> {
        self.convert_sentence_with(patterns, false)
    }

    /// 同 [`Self::convert_sentence`]，`whole` 为真时末尾单字母也读（[`sentence::convert_whole`]），只给比分用。
    /// 接了神经重打分器时取前 [`RESCORE_PATHS`] 条路径，按「路径分 + λ·(神经分 − 静态分)」重排（[`Self::rescore_paths`]）：
    /// 返回重排后的第一条，并带上模型系数（见 `Conversion::neural_factor`）；只有一条路径或模型还没给分时原样返回。
    pub(super) fn convert_sentence_with(
        &self,
        patterns: &[cloudime_dictionary::SyllablePattern<'_>],
        whole: bool,
    ) -> Option<Conversion> {
        let dictionaries = self.all_dictionaries();
        let expanded = self.expand_positions(patterns);
        let k = if self.has_sentence_scorer() {
            RESCORE_PATHS
        } else {
            1
        };
        let mut paths = sentence::convert_paths(
            &dictionaries,
            &expanded.positions(),
            whole,
            k,
            &*self.language_model,
            self.personal(),
            |text| self.learner.weight(text),
            |index, syllable| expanded.cost(index, syllable),
            &mut self.span_cache.borrow_mut(),
        );
        // 与最优路径差得太远的不参与：那种差距多半是个人 n-gram 拉开的
        if paths.len() > 1 {
            let floor = paths[0].score - self.neural_margin;
            paths.retain(|p| p.score >= floor);
            self.rescore_paths(&mut paths);
        }
        paths.into_iter().next()
    }

    /// 一个位置的写法：敲的原样、模糊音扩展（配置的模糊音规则），再给每个完整音节补上
    /// **多敲 / 少敲** 一个键仍是合法音节的变体（`gan` → `guan`），按 [`TypoCosts`] 打折进词图。
    /// 换位与相邻键替换不在这里补：它们走整段一处编辑的纠错读法（[`Self::correction_readings`]），
    /// 两边都放会重复。多敲 / 少敲是「音节级」的（`meiganxi` 的 `gan` 少一个键就是 `guan`），
    /// 整段编辑那一路反而是切不干净时才试，补不出 没关系。
    ///
    /// 太短（不到 [`correction::MIN_LETTERS`]）或非末尾带简拼 / 残缺音节的切分不加：短串一处
    /// 编辑几乎总能凑出别的词，非末尾的残缺音节本来就不是用户敲的原话。
    pub(super) fn expand_positions(
        &self,
        patterns: &[cloudime_dictionary::SyllablePattern<'_>],
    ) -> Expanded {
        let mut expanded = self.fuzzy.expand(patterns);
        let letters: usize = patterns.iter().map(|p| p.text.len()).sum();
        let inner_abbreviated = patterns
            .iter()
            .take(patterns.len().saturating_sub(1))
            .any(|p| !p.complete);
        if letters < correction::MIN_LETTERS || inner_abbreviated {
            return expanded;
        }
        for (index, pattern) in patterns.iter().enumerate() {
            if !pattern.complete {
                continue;
            }
            for (text, kind) in correction::typo::variants(pattern.text) {
                if !matches!(
                    *kind,
                    correction::TypoKind::Extra | correction::TypoKind::Missing
                ) {
                    continue;
                }
                let accepted = self.learner.typo_count(pattern.text, text);
                expanded.push_alternative(index, text, self.typo_costs.typo_cost(*kind, accepted));
            }
        }
        expanded
    }

    /// 主词库与用户词一起查（每个位置多种写法），按 [`Self::all_dictionaries`] 的顺序跨词库去重。
    pub(super) fn lookup_all(
        &self,
        positions: &[Vec<cloudime_dictionary::SyllablePattern<'_>>],
    ) -> Vec<Match<'_>> {
        self.lookup_across_dictionaries(positions, false)
    }

    /// 只要音节数正好等于位置数的词，主词库与用户词一起查，同样按词库顺序跨词库去重。
    pub(super) fn lookup_exact_all(
        &self,
        positions: &[Vec<cloudime_dictionary::SyllablePattern<'_>>],
    ) -> Vec<Match<'_>> {
        self.lookup_across_dictionaries(positions, true)
    }

    /// 按 [`Self::all_dictionaries`] 的顺序逐本查词，做「跨词库、靠前优先」的过滤：处理第 N 本时，
    /// `text` 在前 N-1 本里命中过的丢掉；**同一本里同一个 `text` 的不同命中照旧全收**，
    /// 留给 [`crate::ranking::rank`] 按名次去重（它按 `text` 保留名次最高的一条）。
    fn lookup_across_dictionaries(
        &self,
        positions: &[Vec<cloudime_dictionary::SyllablePattern<'_>>],
        exact: bool,
    ) -> Vec<Match<'_>> {
        let mut hits = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for dictionary in self.all_dictionaries() {
            let found = if exact {
                dictionary.lookup_exact_alt(positions)
            } else {
                dictionary.lookup_pattern_alt(positions)
            };
            let mut current: HashSet<&str> = HashSet::with_capacity(found.len());
            for hit in found {
                if seen.contains(hit.text) {
                    continue;
                }
                current.insert(hit.text);
                hits.push(hit);
            }
            seen.extend(current);
        }
        hits
    }

    /// 中英混输：整段输入是英文词就加进候选池，拼音不像话时顺带给出前缀补全。
    /// 权重是英文词频 ×(1+用户选择次数)，没有词频的按 1.0。
    pub(super) fn push_english(
        &self,
        pool: &mut Vec<(Candidate, PoolRank)>,
        unlikely_pinyin: bool,
        full: usize,
        abbreviated: usize,
        full_last: bool,
    ) {
        // 关掉中英混输：中文模式下不再掺英文词与英文补全
        if !self.mixture_input {
            return;
        }
        let lists = self.english_lists();
        if lists.is_empty() {
            return;
        }
        let text = self.composition.scope();
        if text.contains('\'') {
            return;
        }
        // `exact` 为真时整段就是这个词；补全比输入长，排在整段精确词之后
        let case = self.english_case;
        let push =
            |pool: &mut Vec<(Candidate, PoolRank)>, word: &str, frequency: u32, exact: bool| {
                // 反引号轮换的大小写套在展示与上屏文本上；选择次数按这个文本记，查也按它查
                let text = case.apply(word);
                let weight =
                    f64::from(frequency.max(1)) * (1.0 + f64::from(self.learner.weight(&text)));
                pool.push((
                    Candidate {
                        text,
                        kind: CandidateKind::English,
                        syllables: Vec::new(),
                        reading: None,
                    },
                    // 结构键借用整段读法；`exact` 为真时整段就是这个词，补全比输入长视作不精确
                    (full, abbreviated, full_last, exact, 0, weight),
                ));
            };
        if let Some((word, frequency)) = lists.iter().find_map(|words| {
            words
                .get(text)
                .map(|word| (word.to_owned(), words.frequency(text).unwrap_or(0)))
        }) {
            push(pool, &word, frequency, true);
        }
        // 英文补全：拼音不像话时（`compa` 切成 co'm'pa），整段多半是在打英文词的前面几个字母，补全紧跟在精确词之后；
        // 个人词表在前，两张表里都有的只出一次。
        if unlikely_pinyin && text.len() >= MIN_COMPLETION_LETTERS {
            let mut budget = ENGLISH_COMPLETIONS;
            for words in &lists {
                for (_code, word, frequency) in words.complete_entries(text, budget) {
                    if pool.iter().any(|(c, _)| {
                        c.kind == CandidateKind::English && c.text.eq_ignore_ascii_case(word)
                    }) {
                        continue;
                    }
                    push(pool, word, frequency, false);
                    budget -= 1;
                    if budget == 0 {
                        break;
                    }
                }
                if budget == 0 {
                    break;
                }
            }
        }
    }
}

/// 候选池里一条的排序键：与 [`ranking::rank`] 同序——覆盖字母 → 非末尾简拼数 → 末音节完整 →
/// 命中精确 → 读法层级 → 权重。候选池把词、整句、英文放在一起比，结构键必须带过来，
/// 否则最后一步的权重排序又会让高频单字霸屏。
type PoolRank = (usize, usize, bool, bool, u8, f64);

/// 候选池按结构键与权重排，同文本只留名次最高的那条，最后 map 成候选列表。
fn rank_pool(mut pool: Vec<(Candidate, PoolRank)>) -> Vec<Candidate> {
    pool.sort_by(|a, b| {
        b.1.0
            .cmp(&a.1.0)
            .then_with(|| a.1.1.cmp(&b.1.1))
            .then_with(|| b.1.2.cmp(&a.1.2))
            .then_with(|| b.1.3.cmp(&a.1.3))
            .then_with(|| a.1.4.cmp(&b.1.4))
            .then_with(|| {
                b.1.5
                    .partial_cmp(&a.1.5)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.0.text.cmp(&b.0.text))
    });
    let mut seen: HashSet<String> = HashSet::with_capacity(pool.len());
    pool.into_iter()
        .filter(|(candidate, _)| seen.insert(candidate.text.clone()))
        .map(|(candidate, _)| candidate)
        .collect()
}

/// 整句候选进池：词级候选里已有同文本同读音的不重复插；同文本不同读音的去掉词级那条，整句顶上。
fn push_sentence(pool: &mut Vec<(Candidate, PoolRank)>, candidate: Candidate, rank: PoolRank) {
    if let Some(index) = pool.iter().position(|(c, _)| c.text == candidate.text) {
        if pool[index].0.syllables == candidate.syllables {
            return;
        }
        pool.remove(index);
    }
    pool.push((candidate, rank));
}

/// 一个切分的查询模式；最后一个音节即使打完了也可能还没打完（`xia` 是 `xiang` 的前缀），按前缀查。
fn reading_positions(segmentation: &Segmentation) -> Vec<cloudime_dictionary::SyllablePattern<'_>> {
    let mut patterns = segmentation.patterns();
    let count = patterns.len();
    if count == 0 {
        return patterns;
    }
    let last = &segmentation.syllables[count - 1];
    if last.complete && parser::is_syllable_prefix(&last.text) {
        patterns[count - 1].complete = false;
    }
    patterns
}

/// 切分里非末尾的简拼音节数（`kai f a` 是 1，`kai fa` 是 0）：非末尾的残缺音节越多越不像
/// 用户敲的原话，排序时少者优先（见 [`crate::ranking::Scored::abbreviated`]）。
fn abbreviated_count(patterns: &[cloudime_dictionary::SyllablePattern<'_>]) -> usize {
    patterns
        .iter()
        .rev()
        .skip(1)
        .filter(|p| !p.complete)
        .count()
}

/// 词库命中的中文候选。
fn chinese_candidate(item: &Scored<'_>) -> Candidate {
    Candidate {
        text: item.hit.text.to_owned(),
        kind: CandidateKind::Chinese,
        syllables: item.hit.syllables().map(str::to_owned).collect(),
        reading: None,
    }
}
