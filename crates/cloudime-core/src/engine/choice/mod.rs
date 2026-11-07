//! 选词模型（知意）：候选页重排与按音节出字。
//!
//! 规则照知意输入法的 laya 重排：只看前 18 条候选里盖住整段输入的，模型分 + 语言模型分 + 原位置先验；
//! 按音节出字则给整句词图提供每个音节的字分布。打分在后台线程做，按键回调不等模型。

mod char_table;
mod model;
mod ranker;
mod request;
mod tier;
mod worker;

use std::sync::Arc;

use crate::candidate::{Candidate, CandidateKind};
use crate::sentence::{self, LanguageModel};

use super::{Engine, SENTENCE_CANDIDATES, take_last_chars};

pub use char_table::CharTable;
pub use model::ChoiceModel;
pub use tier::ChoiceTier;

pub(super) use ranker::ChoiceRanker;
use request::{ChoiceRequest, ComposeRequest};
use worker::{Finished, Job};

#[cfg(test)]
mod tests;

/// 最多看前这么多条候选（知意 `laya` 的 2 × 每页 9 条），各档在这以内取；
/// 短语补窗时可能再多一条词候选。
const WINDOW: usize = 18;

/// 至少这么多条可比的才重排。
const MIN_CANDIDATES: usize = 2;

/// 二元语言模型分的权重（知意 `laya.ngram_weight`）。
const LM_WEIGHT: f64 = 1.0;

/// 中文位置先验的权重。
const RANK_WEIGHT: f64 = 1.0;

/// 整句候选的语言模型分与位置先验再乘这个数，避免同一个判断被数三遍。
const SENTENCE_TERMS_SCALE: f64 = 0.5;

/// 补进模型窗口的词候选：它的原位置被整句候选占满，位置先验和 LM 都不能照原排名算；
/// 几乎不参与，让模型分自己决定。
const FALLBACK_TERMS_SCALE: f64 = 0.1;

/// 短语输入（4–8 音节，约 4–8 个汉字）把最好的词候选补进模型窗口并保在前 3 条；
/// 两三个字的词本来就在窗口里，补进去反而会把正确的词挤掉；再长交给整句候选。
const FALLBACK_MIN_SYLLABLES: usize = 4;
const FALLBACK_MAX_SYLLABLES: usize = 8;

/// 这次输入是不是短语：只有这个长度段才做词候选补窗 / 保底。
fn fallback_length(syllables: usize) -> bool {
    (FALLBACK_MIN_SYLLABLES..=FALLBACK_MAX_SYLLABLES).contains(&syllables)
}

/// 英文位置先验的权重。
const ENGLISH_RANK_WEIGHT: f64 = 0.2;

/// 中文候选原来在第几位的 ln P：冻结输入日志的选中位置分布。
const CHINESE_RANK_LOG_PROB: [f64; WINDOW] = [
    -0.071, -3.443, -4.197, -4.944, -5.683, -6.077, -6.554, -6.611, -6.736, -6.736, -7.652, -7.498,
    -7.365, -7.835, -8.346, -7.835, -8.346, -8.346,
];

/// 英文候选同上。
const ENGLISH_RANK_LOG_PROB: [f64; 5] = [-0.171, -3.02, -3.02, -4.561, -5.659];

/// 整句词图里字分布得分的权重。
const EMISSION_WEIGHT: f64 = 0.5;

/// 字分布里一个字的最低分。
const EMISSION_FLOOR: f64 = -12.0;

/// 读音被敲错边、模糊音改过的字固定扣的分（乘权重前）。
const ALTERED_EMISSION: f64 = -4.0;

/// 语言模型不认识的字。
const UNKNOWN_LOG_PROB: f64 = -18.5;

/// 把 `text` 拉回前 `SENTENCE_CANDIDATES`（3）条：短输入时整句备选会把正确的词挤到第 4 位，
/// 用户第一页看不到。只在它已经掉出去时挪，已经在前面就不动。
fn keep_word_candidate_near_top(items: &mut Vec<Candidate>, text: &str) {
    let Some(index) = items.iter().position(|candidate| candidate.text == text) else {
        return;
    };
    if index < SENTENCE_CANDIDATES {
        return;
    }
    let candidate = items.remove(index);
    items.insert(SENTENCE_CANDIDATES - 1, candidate);
}

impl Engine {
    /// 接上或卸掉选词模型（后台线程随之起停）。
    pub fn set_choice_model(&mut self, model: Option<Arc<dyn ChoiceModel>>) {
        self.choice = model.map(ChoiceRanker::new);
        self.choice_done.set(0);
    }

    /// 换档，下一次查询就按新档比。
    pub fn set_choice_tier(&mut self, tier: ChoiceTier) {
        self.choice_tier = tier;
    }

    /// 接了选词模型。
    pub fn has_choice_model(&self) -> bool {
        self.choice.is_some()
    }

    /// 选词后台线程还有任务没收回。
    pub(super) fn choice_in_flight(&self) -> bool {
        self.choice
            .as_ref()
            .is_some_and(|choice| choice.worker.latest() > self.choice_done.get())
    }

    /// 查询的最后一步：这页有分就按模型重排，没分记下等壳来取。
    pub(super) fn rerank_choices(&self, items: &mut Vec<Candidate>) {
        let Some(choice) = &self.choice else {
            return;
        };
        let keys: String = self
            .composition
            .scope()
            .chars()
            .filter(|c| *c != '\'')
            .collect();
        let english = self.english_mode;
        // 英文模式只比英文补全，和敲的一样的那个词留在原位；中文模式只比中文，夹在里面的英文词、快捷候选不动。
        let mut whole: Vec<(usize, Vec<u32>)> = Vec::new();
        for (index, candidate) in items.iter().enumerate().take(self.choice_tier.window()) {
            let units = match candidate.kind {
                CandidateKind::English
                    if english && !candidate.text.eq_ignore_ascii_case(&keys) =>
                {
                    Some(choice.model.letter_units(&keys))
                }
                CandidateKind::Chinese | CandidateKind::Sentence if !english => {
                    choice.model.align(&keys, &candidate.syllables)
                }
                _ => None,
            };
            if let Some(units) = units {
                whole.push((index, units));
            }
        }
        // 短语输入的整句备选很容易把模型窗口占满，正确的词根本没进比较；把第一个盖住
        // 整段输入的词候选补进去，让模型能给它打分（保底只负责显示，这里才可能改 Top1）。
        let mut added_fallback = None;
        if !english
            && let Some((index, units)) = items
                .iter()
                .enumerate()
                .find(|(index, candidate)| {
                    candidate.kind == CandidateKind::Chinese
                        && fallback_length(candidate.syllables.len())
                        && candidate.syllables.concat() == keys
                        && !whole.iter().any(|(seen, _)| seen == index)
                })
                .and_then(|(index, candidate)| {
                    choice
                        .model
                        .align(&keys, &candidate.syllables)
                        .map(|units| (index, units))
                })
        {
            whole.push((index, units));
            added_fallback = Some(index);
        }
        if whole.len() < MIN_CANDIDATES {
            return;
        }
        let limit = self.choice_tier.context_chars(english);
        let context = match &self.rescoring_before {
            Some(before) => take_last_chars(before, limit),
            None => self.history.recent(limit).to_owned(),
        };
        let request = ChoiceRequest {
            context,
            typed: whole[0].1.clone(),
            texts: whole.iter().map(|(i, _)| items[*i].text.clone()).collect(),
        };
        let Some(logits) = choice.cached(&request) else {
            *choice.wanted.borrow_mut() = Some(request);
            return;
        };
        if logits.len() != whole.len() {
            return;
        }
        let log_probs = log_softmax(&logits);
        let model = &*self.language_model;
        let previous = (!english)
            .then(|| last_word(&request.context, model))
            .flatten();
        let scores: Vec<f64> = (0..whole.len())
            .map(|k| {
                let rank = whole[k].0;
                let (language_model, prior) = if english {
                    (
                        0.0,
                        ENGLISH_RANK_WEIGHT * ENGLISH_RANK_LOG_PROB[rank.min(4)],
                    )
                } else {
                    let lm =
                        LM_WEIGHT * text_log_prob(previous.as_deref(), &request.texts[k], model);
                    let scale = if Some(rank) == added_fallback {
                        FALLBACK_TERMS_SCALE
                    } else if items[rank].kind == CandidateKind::Sentence {
                        SENTENCE_TERMS_SCALE
                    } else {
                        1.0
                    };
                    (
                        scale * lm,
                        scale * RANK_WEIGHT * CHINESE_RANK_LOG_PROB[rank.min(WINDOW - 1)],
                    )
                };
                log_probs[k] + language_model + prior
            })
            .collect();
        // 短语输入（4–8 音节）重排前最好的词候选：整句备选常把正确的词挤到第 4 位
        // （`不差毫厘` / `同一时间`），重排后至少把它保在前 3 条里。
        // 整句候选可能先占满模型窗口，所以这里扫完整列表；只记文本，下面的重排会挪动下标。
        let fallback = (!english && fallback_length(items[whole[0].0].syllables.len()))
            .then(|| {
                items
                    .iter()
                    .find(|candidate| {
                        candidate.kind == CandidateKind::Chinese
                            && candidate.syllables.concat() == keys
                    })
                    .map(|candidate| candidate.text.clone())
            })
            .flatten();
        let best = (0..scores.len())
            .max_by(|&a, &b| scores[a].total_cmp(&scores[b]))
            .unwrap_or(0);
        if best != 0 {
            let candidate = items.remove(whole[best].0);
            items.insert(whole[0].0, candidate);
            self.last_rescored.set(true);
        }
        if let Some(text) = fallback {
            keep_word_candidate_near_top(items, &text);
        }
    }

    /// 这段音节的字分布（选词模型按音节出的），整句词图给路径上的字打分用。
    /// 算好了就给；没算过就记下来，等壳停顿后送去后台，这次先不用。
    pub(in crate::engine) fn char_table(
        &self,
        patterns: &[cloudime_dictionary::SyllablePattern<'_>],
    ) -> Option<Arc<CharTable>> {
        let choice = self.choice.as_ref()?;
        let limit = self.choice_tier.context_chars(false);
        let context = match &self.rescoring_before {
            Some(before) => take_last_chars(before, limit),
            None => self.history.recent(limit).to_owned(),
        };
        let key = compose_key(&context, patterns);
        if let Some(table) = choice.table(&key) {
            return table;
        }
        let mut wanted = choice.compose_wanted.borrow_mut();
        if !wanted.iter().any(|request| request.key == key) {
            wanted.push(ComposeRequest {
                key,
                context,
                syllables: patterns
                    .iter()
                    .map(|p| (p.text.to_owned(), p.complete))
                    .collect(),
            });
        }
        None
    }

    /// 查询开头：上一次查询记下、还没送出的出字请求作废，只留这次要的。
    pub(super) fn forget_stale_compose(&self) {
        if let Some(choice) = &self.choice {
            choice.compose_wanted.borrow_mut().clear();
        }
    }

    /// 有一页等着选词模型打分，或有输入等着出字。
    pub(super) fn choice_pending(&self) -> bool {
        self.choice.as_ref().is_some_and(|choice| {
            choice.wanted.borrow().is_some() || !choice.compose_wanted.borrow().is_empty()
        })
    }

    /// 把等着出字的输入、或等着的那页送去后台；都没有返回 `false`。
    pub(super) fn request_choice(&self) -> bool {
        let Some(choice) = &self.choice else {
            return false;
        };
        let compose: Vec<ComposeRequest> = choice.compose_wanted.borrow_mut().drain(..).collect();
        if !compose.is_empty() {
            choice.wanted.borrow_mut().take();
            choice.worker.submit(Job::Compose(compose));
            return true;
        }
        let Some(request) = choice.wanted.borrow_mut().take() else {
            return false;
        };
        choice.worker.submit(Job::Score(request));
        true
    }

    /// 收后台打好的分与字分布；最近一次请求的到了返回 `true`。
    pub(super) fn poll_choice(&self) -> bool {
        let Some(choice) = &self.choice else {
            return false;
        };
        let latest = choice.worker.latest();
        let mut updated = false;
        while let Some((serial, finished)) = choice.worker.poll() {
            // 先记收到的序号：就算这份结果坏了（长度不对），这个任务也算收过了。
            self.choice_done.set(self.choice_done.get().max(serial));
            match finished {
                Finished::Scored {
                    request, logits, ..
                } => {
                    if logits.len() != request.texts.len() {
                        continue;
                    }
                    *choice.scored.borrow_mut() = Some((request, logits));
                }
                Finished::Composed(tables) => {
                    for (key, table) in tables {
                        choice.insert_table(key, table);
                    }
                }
            }
            updated |= serial == latest;
        }
        updated
    }
}

/// 词 `text`（从第 `start` 个音节起，读音 `syllables`）在字分布里的得分，乘 [`EMISSION_WEIGHT`]。
pub(in crate::engine) fn char_emission(
    table: &CharTable,
    patterns: &[cloudime_dictionary::SyllablePattern<'_>],
    start: usize,
    text: &str,
    syllables: &[String],
) -> f64 {
    text.chars()
        .enumerate()
        .map(|(offset, ch)| {
            let slot = start + offset;
            let as_typed = match (patterns.get(slot), syllables.get(offset)) {
                (Some(typed), Some(reading)) => {
                    if typed.complete {
                        reading == typed.text
                    } else {
                        reading.starts_with(typed.text)
                    }
                }
                _ => false,
            };
            if !as_typed {
                return ALTERED_EMISSION;
            }
            table
                .relative(slot, ch)
                .map_or(EMISSION_FLOOR, f64::from)
                .max(EMISSION_FLOOR)
        })
        .sum::<f64>()
        * EMISSION_WEIGHT
}

/// 字分布的缓存键：前文与音节串（敲全的音节后跟 `|`，没敲全的跟 `~`）。
fn compose_key(context: &str, patterns: &[cloudime_dictionary::SyllablePattern<'_>]) -> String {
    let mut key = String::with_capacity(context.len() + 8 * patterns.len());
    key.push_str(context);
    key.push('\x01');
    for pattern in patterns {
        key.push_str(pattern.text);
        key.push(if pattern.complete { '|' } else { '~' });
    }
    key
}

fn log_softmax(logits: &[f32]) -> Vec<f64> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let sum: f64 = logits.iter().map(|&l| (l as f64 - max).exp()).sum();
    logits.iter().map(|&l| l as f64 - max - sum.ln()).collect()
}

/// 上文最后一个词（按语言模型切词）；上文不是以汉字结尾就当句首。
fn last_word(context: &str, model: &dyn LanguageModel) -> Option<String> {
    let last = context.chars().next_back()?;
    if !sentence::is_han(last) {
        return None;
    }
    sentence::segment_text(context, model)?
        .into_iter()
        .next_back()?
        .into_iter()
        .next_back()
}

fn text_log_prob(previous: Option<&str>, text: &str, model: &dyn LanguageModel) -> f64 {
    let Some(clauses) = sentence::segment_text(text, model) else {
        return 0.0;
    };
    let mut previous = previous.map(str::to_owned);
    let mut total = 0.0;
    for clause in clauses {
        for word in clause {
            total += model
                .log_prob(previous.as_deref(), &word)
                .unwrap_or(UNKNOWN_LOG_PROB);
            previous = Some(word);
        }
    }
    total
}
