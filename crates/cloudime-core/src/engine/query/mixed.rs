//! 中英混合组句：`ni'de'dian'nao'cpu'xing'hao'shi'shen'me` → 你的电脑CPU型号是什么。
//!
//! 纯拼音切不动（`cpu` / `github` 不是音节）时，把整段重新切成「拼音音节 + 英文词」：英文词来自
//! 随包 / 个人英文词表，作为词图里的一个 span——本次查询专用一本词库，词的读法就是它自己的字母，
//! 词频按词表里的 Zipf 值（×1000）给，交给整句模型当兜底分。这样汉字与英文由同一个 Viterbi 排，
//! 跨段的上文也是连续的（`电脑 → CPU → 型号`）。
//!
//! 只在纯拼音切分失败或留下尾巴时才走这条路：能整段读成拼音的输入行为完全不变。
//! 英文词要查得到（`github` 在随包表里）；查不到的拼写先不认，交给原来的英文候选 / 补全。

use super::*;
use crate::parser::Syllable;

/// 英文词最短几个字母：两个字母的（`ok` / `id`）太容易撞上简拼。
const MIN_MIXED_ENGLISH_LETTERS: usize = 2;

/// 英文词最长几个字母：再长多半是几段拼在一起。
const MAX_MIXED_ENGLISH_LETTERS: usize = 24;

/// DP 里一个位置是怎么来的。
struct Step {
    /// 上一个位置（字节下标）。
    from: usize,

    /// 这一段在切分里的读法：拼音音节，或英文词的小写字母。
    code: String,

    /// 是英文词（切分里占一个音节位）。
    latin: bool,

    /// 拼音末尾没打完的音节为 `false`。
    complete: bool,

    /// 英文词的（显示写法，词频），派生词库用。
    entry: Option<(String, u32)>,

    /// 到这里的段数，用来挑段数最少的切分。
    tokens: usize,
}

impl Engine {
    /// 把整段切成「拼音音节 + 英文词」：要求覆盖全部字母、两种都出现。
    /// 返回切分与本次专用的英文词库（给整句词图用）。
    pub(super) fn split_mixed(&self, keys: &str) -> Option<(Segmentation, Dictionary)> {
        if !self.mixture_input {
            return None;
        }
        let lists = self.english_lists();
        if lists.is_empty() {
            return None;
        }
        // `'` 只是分隔符：切分按去掉它的字母串算（上屏消耗仍按原串对齐）。
        // 同时记下每个字母在**原缓冲区**里的位置：要回去看这一段是不是 Shift 敲的大写。
        let mut text = String::with_capacity(keys.len());
        let mut raw_of: Vec<usize> = Vec::with_capacity(keys.len());
        for (index, c) in keys.char_indices() {
            if c == '\'' {
                continue;
            }
            text.push(c);
            raw_of.push(index);
        }
        if text.len() < MIN_MIXED_ENGLISH_LETTERS + 1
            || !text.bytes().all(|b| b.is_ascii_lowercase())
        {
            return None;
        }
        let n = text.len();
        let mut best: Vec<Option<Step>> = Vec::with_capacity(n + 1);
        best.resize_with(n + 1, || None);
        best[0] = Some(Step {
            from: 0,
            code: String::new(),
            latin: false,
            complete: true,
            entry: None,
            tokens: 0,
        });
        for start in 0..n {
            let Some(tokens) = best[start].as_ref().map(|step| step.tokens) else {
                continue;
            };
            for len in 1..=parser::MAX_SYLLABLE_LEN.min(n - start) {
                let piece = &text[start..start + len];
                if parser::is_syllable(piece) {
                    improve(
                        &mut best,
                        start + len,
                        Step {
                            from: start,
                            code: piece.to_owned(),
                            latin: false,
                            complete: true,
                            entry: None,
                            tokens: tokens + 1,
                        },
                    );
                } else if start + len == n && parser::is_syllable_prefix(piece) {
                    improve(
                        &mut best,
                        n,
                        Step {
                            from: start,
                            code: piece.to_owned(),
                            latin: false,
                            complete: false,
                            entry: None,
                            tokens: tokens + 1,
                        },
                    );
                }
            }
            for len in MIN_MIXED_ENGLISH_LETTERS..=MAX_MIXED_ENGLISH_LETTERS.min(n - start) {
                let code = &text[start..start + len];
                // 本身就是一个合法拼音音节的英文词（`men` / `an` / `bei`）不在这里认：
                // 拼音读法更自然，认了会让正常拼音输入凭空多出一条混合整句（回放里掉 3 个点）。
                if parser::is_syllable(code) {
                    continue;
                }
                let Some((word, frequency)) = lists.iter().find_map(|list| {
                    let word = list.get(code)?;
                    Some((word.to_owned(), list.frequency(code)?))
                }) else {
                    continue;
                };
                improve(
                    &mut best,
                    start + len,
                    Step {
                        from: start,
                        code: code.to_owned(),
                        latin: true,
                        complete: true,
                        entry: Some((word, frequency)),
                        tokens: tokens + 1,
                    },
                );
            }
        }
        best[n].as_ref()?;
        // 回溯：全英文 / 全拼音都不算混合，那是别的路径的地盘。
        let mut steps = Vec::new();
        let mut cursor = n;
        while cursor > 0 {
            let step = best[cursor].as_ref()?;
            steps.push((
                step.latin,
                step.code.clone(),
                step.complete,
                step.entry.clone(),
                step.from,
            ));
            if step.from >= cursor {
                return None;
            }
            cursor = step.from;
        }
        steps.reverse();
        let latin = steps.iter().filter(|(latin, ..)| *latin).count();
        if latin == 0 || latin == steps.len() || steps.len() < 2 {
            return None;
        }
        let mut syllables = Vec::with_capacity(steps.len());
        let mut source = String::new();
        for (is_latin, code, complete, entry, from) in steps {
            if is_latin {
                let (word, frequency) = entry?;
                // 按敲的大写就保留大写（`CPU` 一直是 `CPU`），不跟反引号轮换；
                // 小写敲的才按 `english_case` 走。读法（`code`）始终小写匹配。
                let word = if self.typed_upper(&raw_of, from, &code) {
                    code.to_ascii_uppercase()
                } else {
                    self.english_case().apply(&word)
                };
                source.push_str(&format!("{word}\t{code}\t{frequency}\n"));
                syllables.push(Syllable::complete(&code));
            } else if complete {
                syllables.push(Syllable::complete(&code));
            } else {
                syllables.push(Syllable::partial(&code));
            }
        }
        let dictionary = Dictionary::parse(&source).ok()?;
        Some((Segmentation { syllables }, dictionary))
    }

    /// 这一段英文词是不是整段按 Shift 敲的（大写）：是就不跟反引号轮换。
    fn typed_upper(&self, raw_of: &[usize], from: usize, code: &str) -> bool {
        let Some(&start) = raw_of.get(from) else {
            return false;
        };
        let Some(&last) = raw_of.get(from + code.len().saturating_sub(1)) else {
            return false;
        };
        self.composition.is_shifted_span(start, last + 1)
    }

    /// 中英混合的整句候选：整段一个 Viterbi，英文词与汉字一起排。
    /// 交给 [Self::query_reading] 插进候选池（词级候选照常保留），不单独成一次查询。
    /// 返回（候选、权重、路径分）：路径分给调用方跟拼音读法比。
    ///
    /// `anchor` 是拼音读法「出问题的那一格」在去掉分隔符后的字节偏移（中间没打完的音节、
    /// 或切不动留下的尾巴起点）：只有英文词正好盖住这里，混合读法才算把这处异常解释掉了；
    /// 别处的英文词（`me` / `an` 这类也拼得出来的）不足以推翻一个正常的拼音读法。
    pub(super) fn mixed_candidate(
        &self,
        keys: &str,
        anchor: Option<(usize, Option<usize>)>,
    ) -> Option<(Candidate, f64, f64)> {
        let (segmentation, dictionary) = self.split_mixed(keys)?;
        if let Some((anchor, partial)) = anchor {
            // 中间没打完的音节只有一个字母时才算「这段压根不是拼音」（`c` + `pu` 的 `cpu`）；
            // 更长的残缺音节多半是敲错的拼音，交给纠错那一路，别让英文读法抢。
            if partial.is_some_and(|len| len != 1) {
                return None;
            }
            // 中间残缺又正好能纠成通顺拼音的（`ananjain` → 按按键）：按纠错读。
            // `Jain` 这种冷门英文词会把敲错的拼音读法顶掉。
            if partial.is_some() && self.has_correction(keys) {
                return None;
            }
            let covered = segmentation
                .syllables
                .iter()
                .scan(0usize, |offset, syllable| {
                    let start = *offset;
                    *offset += syllable.text.len();
                    Some((start, syllable.text.as_str()))
                })
                .any(|(start, text)| {
                    start <= anchor
                        && anchor < start + text.len()
                        && text.len() >= 3
                        && !parser::is_syllable(text)
                        && !parser::is_fully_segmentable(text)
                });
            if !covered {
                return None;
            }
        }
        let patterns = segmentation.patterns();
        let conversions = self.sentence_paths_with(&patterns, false, 1, Some(&dictionary));
        let conversion = conversions.into_iter().next()?;
        if conversion.has_placeholder() || conversion.words.len() < 2 {
            return None;
        }
        let weight = conversion.weight();
        let score = conversion.score;
        Some((
            Candidate {
                text: conversion.text,
                display: None,
                kind: CandidateKind::Sentence,
                syllables: conversion.syllables,
                reading: None,
                auxiliary: None,
                auxiliary_typed: None,
            },
            weight,
            score,
        ))
    }
}

/// 段数更少才收：少一段意味着多一个更长的词，切分更像话。
fn improve(best: &mut [Option<Step>], to: usize, step: Step) {
    if best[to]
        .as_ref()
        .is_some_and(|existing| existing.tokens <= step.tokens)
    {
        return;
    }
    best[to] = Some(step);
}
