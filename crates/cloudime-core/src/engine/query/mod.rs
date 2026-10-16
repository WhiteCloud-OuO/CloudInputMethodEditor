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

use std::collections::{HashMap, HashSet};

/// 候选右侧要显示的辅助码：`related` 是**光标前那个音节**在候选里的字下标，取那个字的码。
/// 单字表里没有（表里只有整词码）时按「一格一位」切开取那一位 —— 与 [`crate::shuangpin::keeps`]
/// 的「先找词」一致。表里没有就是 `None`（角标位置空着）。
///
/// 返回 `(完整码, 已经敲上的那几位)`：`typed` 是这个词每个字敲的辅助码（与 `keeps` 同一份），
/// 已经敲上的那几位填进 `Candidate::auxiliary_typed`，候选窗口画下划线用。
fn candidate_auxiliary(
    candidate: &Candidate,
    related: usize,
    typed: &[String],
    codes: &dyn crate::shuangpin::AuxiliaryTable,
) -> Option<(String, Option<String>)> {
    let chars: Vec<char> = candidate.display_text().chars().collect();
    // 候选比那个位置短（`开` 配两个音节）：这个位置上没有字，不显示
    if related >= chars.len() {
        return None;
    }
    let code = chars
        .get(related)
        .and_then(|character| codes.codes(&character.to_string()))
        .and_then(|codes| codes.first())
        .cloned()
        .or_else(|| {
            // 表里只有整词码：按「一格一位」切开取那一位
            let letters: Vec<char> = codes
                .codes(candidate.display_text())?
                .first()?
                .chars()
                .collect();
            (letters.len() == chars.len()).then(|| letters[related].to_string())
        })?;
    // 那个字敲了几位：空串 / 没这一项就是一位都没敲（不画下划线）
    let typed = typed
        .get(related)
        .filter(|typed| !typed.is_empty())
        .cloned();
    Some((code, typed))
}

/// 辅助码筛选：候选整体当一个**词**（先找词），没词再按字查（见 [`crate::shuangpin::keeps`]）——
/// **逐字对应**：候选的字跟敲的辅助码按顺序对齐（候选总是从这一串的开头算）。
fn keeps_candidate(
    candidate: &Candidate,
    typed: &[String],
    table: &dyn crate::shuangpin::AuxiliaryTable,
) -> bool {
    let chars = candidate.text.chars().count();
    let codes: Vec<String> = typed[..chars.min(typed.len())].to_vec();
    crate::shuangpin::keeps(&[(candidate.text.clone(), codes)], table)
}

/// 一段**给定**的读法：双键拼音把方案解出的音节直接交进来，[`Engine::query_reading`] 不再自己切。
struct Reading {
    /// 这一段按键的几种切法。同一个位置有几种可能时（尾巴上那半个音节）就列几份，候选按**并集**出。
    segmentations: Vec<Segmentation>,

    /// `spans[n]` = 前 n 个音节在按键串里占几个键（`spans[0]` 恒为 0）—— 打分时的「覆盖多少」用它，
    /// 不用读法里的字母数，见 [`crate::shuangpin::Table::typed_covered`]。
    spans: Vec<usize>,
}

impl Engine {
    /// 解析当前缓冲区并生成排好序的候选。**不带译文**，译文由 [`Self::annotate`] 补。
    ///
    /// 光标停在拼音中间时只按光标前的那段算候选（`ni|hao` 出 你），光标后的拼音留着，
    /// 选上之后接着组句；见 [`Composition::scope`]。
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
        // 已选文本显示在未选拼音之前；候选与光标只对未选拼音算。
        query.selected = self.composition.selected_text();
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
        // 双键拼音：先按方案的键位表把按键还原成完整拼音，再走后面的拼音查询。
        // 同键多义（`t` = ve/ue）会有几种解法，逐个试、取第一个能出候选的 —— 非法那一支自然落选。
        // 放在 `is_raw` **之前**：双拼的按键（含大写辅助码）就是这套方案的输入，别被「直通段」截走；
        // 也放在表达式 **之前**：`v` / `u` / `i` 那三种面板（表达式、区位、符号）仅全键拼音可用。
        // 喂给解码器的是**大小写原样**的那一份（`typed_scope`）：大写字母进缓冲区时被压成小写
        // 只记了 shifted 标记，辅助码全靠大小写区分。
        if let Some(table) = &self.shuangpin {
            return self.query_shuangpin(table, &self.composition.typed_scope(), &rest, start);
        }
        if keys.starts_with(EXPRESSION_PREFIX) {
            return Ok(self.query_expression(keys, rest, start));
        }
        if is_raw(keys) {
            return Ok(self.query_raw(keys, rest, start));
        }
        self.query_phonetic(keys, rest, start, true)
    }

    /// 双键拼音的查询：每种解法都按方案的切分跑一遍拼音侧的候选生成。
    ///
    /// **按键只按方案解释**：解法解出的音节序列直接当切分用（[`crate::shuangpin::Table::segmentations`]），
    /// 不再交给全键那套切分器自由切 —— 不认简拼、不再改刀、末尾音节也不放宽，否则按键会被读出方案
    /// 没解出来的音节（见 `docs/design/input-schemes.md` §6）。
    /// 同键多义会有几种解法，逐个试、取第一个能出候选的 —— 非法那一支自然落选。
    ///
    /// **英文**：`allow_extras=false`（快捷候选与英文尾段都不掺）；`english` = `.cfg` 的「中英混合输入」
    /// （`use_input_scheme.cfg` 的 `mixture_input`，缺省开），开着才**按敲的原始按键**匹配英文词
    /// （整段英文词 + 前缀补全，**不纠错**）；关着一律不匹配（见 [`Self::push_english`]）。
    ///
    /// 开了辅助码时**第一个键敲了大写**（Caps Lock / Shift 卡着）→ 只把那个键压成小写，**后面照旧**：
    /// 大写只能当辅助码，开头一个大写本来整段都切不出来；后面的辅助码（`YpdoYP`）照样认。
    /// 候选窗里也跟着显示小写那一份。
    fn query_shuangpin(
        &self,
        table: &crate::shuangpin::Table,
        keys: &str,
        rest: &str,
        start: Instant,
    ) -> Result<Query, ParseError> {
        let english = self.shuangpin_mixture_input;
        let lowered = table.lowered_leading_key(keys);
        let keys = lowered.as_ref();
        let mut last: Option<Result<Query, ParseError>> = None;
        let mut found: Option<(Query, Vec<String>)> = None;
        for expansion in table.expand(keys) {
            let pinyin = expansion.to_pinyin();
            let segmentations = table.segmentations(&expansion);
            // 这个解法连一个音节都不出来（按键当不了首键）：没有读法，看下一个解法
            let Some(count) = segmentations
                .first()
                .map(|segmentation| segmentation.syllables.len())
            else {
                continue;
            };
            let reading = Reading {
                // 覆盖多少按**按键数**算：同一段按键的几种切法用同一把尺子（见 `Table::typed_covered`）
                spans: (0..=count)
                    .map(|syllables| table.typed_covered(keys, syllables))
                    .collect(),
                segmentations,
            };
            // `auxiliary` 是**这个词每个字敲的辅助码**（不是 `segmentations` 里的那个字形 ——
            // 候选匹配用不到它）；尾巴上那半个音节敲的码接在最后一个（`yH` = 「要」的码 `H`）。
            let mut typed_auxiliary: Vec<String> = expansion
                .syllables
                .iter()
                .map(|syllable| syllable.auxiliary.clone())
                .collect();
            if !expansion.trailing.is_empty() {
                typed_auxiliary.push(expansion.trailing_auxiliary.clone());
            }
            match self.query_reading(
                &pinyin,
                Some(reading),
                rest.to_owned(),
                start,
                false,
                english,
            ) {
                Ok(query) if !query.candidates.items.is_empty() => {
                    found = Some((query, typed_auxiliary));
                    break;
                }
                other => last = Some(other),
            }
        }
        // 一种解法都出不了候选（或一种解法都没有）：**不再按全拼读一遍** ——
        // 双键拼音下的按键不是拼音字母，按全拼读就是「另一种解释」，正是要挡掉的那种混进候选。
        let (mut query, typed_auxiliary) = match found {
            Some(found) => found,
            None => match last {
                Some(last) => (last?, Vec::new()),
                None => {
                    // 按键根本不是这套方案的键：拼音这边没戏，但开着「中英混合输入」时
                    // **原始按键照样查英文词**（`hello` 在双拼表里解不出来，英文词表里有）
                    if !english {
                        return Err(ParseError::NoSegmentation);
                    }
                    let query = self.english_only_query(keys, rest.to_owned(), start, false);
                    if query.candidates.items.is_empty() {
                        return Err(ParseError::NoSegmentation);
                    }
                    (query, Vec::new())
                }
            },
        };
        // 配置要求显示原始按键：候选照旧按展开后的拼音出，拼音行按敲的显示（大小写按敲的来），
        // 音节之间补上半角 `'` —— 光给一串按键看不出打了几段
        if self.show_raw_input {
            query.typed_display = Some(table.typed_marked(keys));
        } else {
            // 关掉时拼音行显示**展开后的完整拼音**（`es` → `shou`）：显示串的字母跟敲的按键对不上，
            // 光标得按「作用域那一整段显示完」算（`expanded_pinyin`）；也不能让 Shift 那套显示串顶上来 ——
            // 它按展开后的拼音长度去切按键串，切出来是乱的。
            query.typed_display = None;
            query.expanded_pinyin = true;
        }
        // 辅助码：只筛选（不排序）——「先找词、没词再拼字」**逐字对应**，不过关的候选直接丢掉
        if let Some(codes) = self.auxiliary_codes.as_deref()
            && table.auxiliary
        {
            query
                .candidates
                .items
                .retain(|candidate| keeps_candidate(candidate, &typed_auxiliary, codes));
        }
        // 候选右侧要显示的辅助码：作用域里**光标前那个音节**对应的那个字（`ai|qg` 显示「爱」的码、
        // `aiqg|` 显示「情」的码），外加那条码里**已经敲上**的前几位（候选窗口画下划线）。
        // 填不填与显示设置无关 —— 画不画由平台层看设置决定。
        if table.auxiliary
            && let Some(codes) = self.auxiliary_codes.as_deref()
            && let Some(related) = query
                .segmentations
                .first()
                .map(|segmentation| segmentation.syllables.len())
                .and_then(|count| count.checked_sub(1))
        {
            for candidate in &mut query.candidates.items {
                if let Some((code, typed)) =
                    candidate_auxiliary(candidate, related, &typed_auxiliary, codes)
                {
                    candidate.auxiliary = Some(code);
                    candidate.auxiliary_typed = typed;
                }
            }
        }
        Ok(query)
    }

    /// 拼音侧的候选生成：整段作用域是一串读音。
    ///
    /// `allow_extras` 决定要不要上「外挂」：中英混输的英文尾段（`woxiangxuehaorust`）与按原始按键认的
    /// 快捷候选（表达式 `v1+2`、日期 `rq`）—— 这两样只在全键拼音下有效。整段英文词与英文补全另看
    /// 「中英文混合输入」（全键拼音读 `[input] mixture_input`，双键拼音读 `use_input_scheme.cfg`）。
    fn query_phonetic(
        &self,
        keys: &str,
        rest: String,
        start: Instant,
        allow_extras: bool,
    ) -> Result<Query, ParseError> {
        self.query_reading(keys, None, rest, start, allow_extras, self.mixture_input)
    }

    /// 同 [`Self::query_phonetic`]，但 `reading` 给定时**不再自己切**：它就是这一段读音的切分
    /// （双键拼音把方案解出的音节直接交进来）。同一段按键的几种切法放在一起按**并集**查
    /// （尾巴上那半个音节的几种可能各算一份切分）。
    ///
    /// 这时候简拼、末尾音节的放宽、一处编辑的纠错读法都不参与 —— 它们都会从同一串字母里凑出
    /// **方案没解出来的读法**。模糊音照旧（那是用户自己配的读音习惯）。
    ///
    /// `english` 为真时**按敲的原始按键匹配英文词**（整段英文词 + 前缀补全，不纠错）：
    /// 全键拼音传配置里的「中英文混合输入」、双键拼音传 `.cfg` 里那一份（见 [`Self::push_english`]）。
    /// 它只放行「整段英文词 + 英文补全」，不放英文尾段与快捷候选（那两样看 `allow_extras`）。
    fn query_reading(
        &self,
        keys: &str,
        reading: Option<Reading>,
        rest: String,
        start: Instant,
        allow_extras: bool,
        english: bool,
    ) -> Result<Query, ParseError> {
        let strict = reading.is_some();
        let scope: &str = keys;
        // 末尾是英文词（`woxiangxuehaorust`）：拼音候选与整句只按头段算，尾段整个跟在整句后面。
        // 整段也能读成拼音时（`database`、`…rust` 当简拼）两种读法比分，英文赢了才按头段算，
        // 输了整段按拼音读、英文读法排在拼音整句后面
        let english_tail = allow_extras
            .then(|| self.split_english_tail(keys))
            .flatten();
        let head_wins = english_tail
            .as_ref()
            .is_some_and(|t| !t.competes || self.mixed_beats_plain(keys, t));
        let parsed = match &reading {
            Some(reading) => Ok((reading.segmentations.clone(), "")),
            None => match &english_tail {
                Some(tail) if head_wins => {
                    parser::segment_with(&keys[..tail.head_len], self.use_jian_pin).map(|s| (s, ""))
                }
                _ => segment_longest_prefix(keys, self.use_jian_pin),
            },
        };
        // 连第一个字母都切不动（`impor`）：拼音这边没戏，但英文词 / 补全、快捷候选还可以有
        let (segmentations, tail) = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                if !english {
                    return Err(error);
                }
                let query = self.english_only_query(keys, rest, start, allow_extras);
                if query.candidates.items.is_empty() {
                    return Err(error);
                }
                return Ok(query);
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
            // 最后一个音节即使打完了也可能还没打完（`xia` 可能是 `xiang` 的前缀），按前缀查。
            // 交进来的读法不放宽：音节是方案解出来的，按键不会让它再长。
            if !strict && last.complete && parser::is_syllable_prefix(&last.text) {
                patterns[count - 1].complete = false;
            }
            // 覆盖多少：交进来的读法按**按键数**（同一段按键的几种切法得用同一把尺子），
            // 自己切的那种按读法里的字母数
            let coverage = match &reading {
                Some(reading) => reading.spans.last().copied().unwrap_or(0),
                None => segmentation.letters(),
            };
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
                    coverage,
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
                let coverage = match &reading {
                    Some(reading) => reading
                        .spans
                        .get(prefix_len)
                        .copied()
                        .unwrap_or(prefix_letters),
                    None => prefix_letters,
                };
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
                        coverage,
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
        // 交进来的读法不纠：那串字母不是用户敲的（按键才对），「敲错一个字母」无从谈起。
        if !strict && unlikely && !original_matches {
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

        // 脚本给的加权 / 降权（`Engine::set_word_adjustments`）：乘进用户权重因子那一层，
        // 结构键、读法层级、纠错与联想折扣都不动 —— 排序规则仍在 Core，脚本只调参数。
        if !self.word_adjustments.is_empty() {
            for item in &mut scored {
                if let Some(factor) = self.word_adjustments.get(item.hit.text) {
                    item.weight *= factor;
                }
            }
        }

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
        if english {
            // 英文候选按**敲的按键**算结构键：双拼下 `scope` 是展开后的拼音，不是用户敲的
            let (full, abbreviated, full_last) = if allow_extras {
                (scope_letters, reading_structure.0, reading_structure.1)
            } else {
                let keys = self.composition.scope();
                let full = keys.chars().filter(|key| *key != '\'').count();
                (full, 0, true)
            };
            self.push_english(&mut pool, unlikely, full, abbreviated, full_last);
        }
        let ranked = rank_pool(pool);
        let weights = pool_weights(&ranked);
        let mut items: Vec<Candidate> =
            ranked.into_iter().map(|(candidate, _)| candidate).collect();
        let rank = start.elapsed();
        // 快捷候选按敲的键认（`rq` 日期），插在本地首选之后
        if allow_extras {
            self.insert_shortcuts(&mut items, keys);
        }

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
            weights,
            tail: tail.to_owned(),
            text: self.composition.typed_text(),
            cursor: self.composition.cursor(),
            rest,
            typed_display,
            expanded_pinyin: false,
            selected: String::new(),
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
        // 表达式计算面板（Tab 进来的）：只给结果那一条候选，拼音行显示算式本身（去掉 `v` 前缀）。
        // 算式空着显示 `0`，算错 / 溢出就是没有候选。
        if self.calculator {
            let body = scope
                .strip_prefix(shortcut::EXPRESSION_PREFIX)
                .unwrap_or(scope);
            return Query {
                segmentations: Vec::new(),
                candidates: CandidateList {
                    items: shortcut::result_candidates(body),
                },
                weights: HashMap::new(),
                tail: String::new(),
                // 拼音行显示算式本身：`typed_display` 是它，`text` / `cursor` 也跟着去掉 `v` 那一个字节，
                // 光标位置才对得上（`segments_cursor` 是数 `text` 里的字符）。
                text: body.to_owned(),
                cursor: self.composition.cursor().saturating_sub(1),
                rest,
                typed_display: Some(body.to_owned()),
                expanded_pinyin: false,
                selected: String::new(),
                correction: None,
                timings: Timings {
                    parse: Duration::ZERO,
                    lookup: Duration::ZERO,
                    rank: start.elapsed(),
                },
            };
        }
        let mut items = shortcut::candidates(scope, &jiff::Zoned::now());
        if let Some(word) = self.english.as_ref().and_then(|english| english.get(scope)) {
            items.push(Candidate {
                text: word.to_owned(),
                display: None,
                kind: CandidateKind::English,
                syllables: Vec::new(),
                reading: None,
                auxiliary: None,
                auxiliary_typed: None,
            });
        }
        Query {
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            weights: HashMap::new(),
            tail: scope.to_owned(),
            text: self.composition.text().to_owned(),
            cursor: self.composition.cursor(),
            rest,
            typed_display: None,
            expanded_pinyin: false,
            selected: String::new(),
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
            display: None,
            kind: CandidateKind::English,
            syllables: Vec::new(),
            reading: None,
            auxiliary: None,
            auxiliary_typed: None,
        }];
        Query {
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            weights: HashMap::new(),
            tail: scope.to_owned(),
            text: self.composition.text().to_owned(),
            cursor: self.composition.cursor(),
            rest,
            typed_display: None,
            expanded_pinyin: false,
            selected: String::new(),
            correction: None,
            timings: Timings {
                parse: Duration::ZERO,
                lookup: Duration::ZERO,
                rank: start.elapsed(),
            },
        }
    }

    /// 英文模式：敲的字母原样显示，候选是英文词表的精确词与前缀补全（见 [`english::suggest`]，**不做拼错纠正**），
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
            display: None,
            kind: CandidateKind::English,
            syllables: Vec::new(),
            reading: None,
            auxiliary: None,
            auxiliary_typed: None,
        })
        .collect();
        Query {
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            weights: HashMap::new(),
            tail: scope.to_owned(),
            text: self.composition.text().to_owned(),
            cursor: self.composition.cursor(),
            rest,
            typed_display: None,
            expanded_pinyin: false,
            selected: String::new(),
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
                display: None,
                kind,
                syllables: conversion.syllables,
                reading: None,
                auxiliary: None,
                auxiliary_typed: None,
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
    ///
    /// **不做拼错纠正**（`hallo` 不会出 hello）—— 只在词表里找整段那个词与它的前缀补全；
    /// 要不要调用由调用方定（`query_reading` 的 `english`：全键拼音读 `[input] mixture_input`、
    /// 双键拼音读 `use_input_scheme.cfg` 的 `mixture_input`）。
    pub(super) fn push_english(
        &self,
        pool: &mut Vec<(Candidate, PoolRank)>,
        unlikely_pinyin: bool,
        full: usize,
        abbreviated: usize,
        full_last: bool,
    ) {
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
                        display: None,
                        kind: CandidateKind::English,
                        syllables: Vec::new(),
                        reading: None,
                        auxiliary: None,
                        auxiliary_typed: None,
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

    /// 「拼音这边一个候选都出不来」时的兜底查询：只按**整段英文词 + 前缀补全**出候选（不纠错），
    /// 拼音行留原样的按键（`tail`），没有切分。`shortcuts` 为真时顺带插按原始按键认的快捷候选
    /// （`rq` 这类，只有全键拼音传真）。候选为空由调用方决定报不报错。
    pub(super) fn english_only_query(
        &self,
        keys: &str,
        rest: String,
        start: Instant,
        shortcuts: bool,
    ) -> Query {
        let mut pool: Vec<(Candidate, PoolRank)> = Vec::new();
        let full = keys.chars().filter(|c| *c != '\'').count();
        self.push_english(&mut pool, true, full, 0, false);
        let ranked = rank_pool(pool);
        let weights = pool_weights(&ranked);
        let mut items: Vec<Candidate> =
            ranked.into_iter().map(|(candidate, _)| candidate).collect();
        if shortcuts {
            self.insert_shortcuts(&mut items, keys);
        }
        Query {
            segmentations: Vec::new(),
            candidates: CandidateList { items },
            weights,
            tail: keys.to_owned(),
            text: self.composition.typed_text(),
            cursor: self.composition.cursor(),
            rest,
            typed_display: None,
            expanded_pinyin: false,
            selected: String::new(),
            correction: None,
            timings: Timings {
                parse: start.elapsed(),
                lookup: Duration::ZERO,
                rank: Duration::ZERO,
            },
        }
    }
}

/// 候选池里一条的排序键：与 [`ranking::rank`] 同序——覆盖字母 → 非末尾简拼数 → 末音节完整 →
/// 命中精确 → 读法层级 → 权重。候选池把词、整句、英文放在一起比，结构键必须带过来，
/// 否则最后一步的权重排序又会让高频单字霸屏。
type PoolRank = (usize, usize, bool, bool, u8, f64);

/// 候选池按结构键与权重排，同文本只留名次最高的那条；返回「候选 + 它的排名权重」
/// （权重给平台层看 Core 的排序依据，见 [`Query::weights`]）。
fn rank_pool(mut pool: Vec<(Candidate, PoolRank)>) -> Vec<(Candidate, f64)> {
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
        .map(|(candidate, rank)| (candidate, rank.5))
        .collect()
}

/// 候选池里的排名权重按文本记一份（给平台层看 Core 的排序依据）。
fn pool_weights(ranked: &[(Candidate, f64)]) -> HashMap<String, f64> {
    ranked
        .iter()
        .map(|(candidate, weight)| (candidate.text.clone(), *weight))
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
        display: None,
        kind: CandidateKind::Chinese,
        syllables: item.hit.syllables().map(str::to_owned).collect(),
        reading: None,
        auxiliary: None,
        auxiliary_typed: None,
    }
}
