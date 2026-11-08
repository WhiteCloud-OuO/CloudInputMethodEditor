//! V 模式 `#` 取名：姓氏表 + 名字表，表里没有的字退回自带词库。
//!
//! 输入是「姓拼音 + 名拼音」连写（`v#zhangwei`）；中途也可以先选中姓氏，让汉字留在组句里
//! （`v#王`），再输入名或直接从候选里挑。候选顺序：姓氏按表里的排名（复姓优先），同一姓氏下按
//! 名字表的行序。

use std::collections::{HashMap, HashSet};

use crate::candidate::{Candidate, CandidateKind};
use crate::name::{NAME_PREFIX, NameTables};
use crate::parser;

use super::super::Engine;

/// 取名候选一次最多给多少条（窗口再翻页也是这些）。
const NAME_CANDIDATE_LIMIT: usize = 60;

/// 姓选好、名还没敲时，先给多少个单字名 / 双字名。
const DEFAULT_GIVEN_SINGLE: usize = 24;
const DEFAULT_GIVEN_PAIR: usize = 8;

/// 一个拼音串能匹配到的姓氏。
struct SurnameHit {
    text: String,

    /// 这个姓氏在输入里占多少字节；等于输入长度时它是「只出姓」的候选。
    pinyin_len: usize,

    /// 拼音已经整个对上了（`wang`），而不是只打了个开头（`wan` → 王）。
    complete: bool,

    /// 排序用：复姓（拼音长）在前，同长按排名。
    rank: u32,
}

impl Engine {
    /// 当前是不是 `v#` 取名模式。
    pub fn name_mode(&self) -> bool {
        self.expression_mode() && self.name_body().is_some()
    }

    /// `v#` 后面的部分；不在取名模式为 `None`。
    fn name_body(&self) -> Option<&str> {
        self.composition
            .scope()
            .strip_prefix(crate::shortcut::EXPRESSION_PREFIX)?
            .strip_prefix('#')
    }

    /// 这个候选是不是还没带名字的姓氏（选中后应留在组句里继续取名）。
    pub fn name_selection_is_surname(&self, text: &str) -> bool {
        let Some(body) = self.name_body() else {
            return false;
        };
        if body.is_empty() || body.chars().any(|c| !c.is_ascii_alphabetic() && c != '\'') {
            return false;
        }
        let body = normalize(body);
        self.surname_hits(&body)
            .iter()
            .any(|hit| hit.pinyin_len == body.len() && hit.text == text)
    }

    /// 选中一个姓氏：把它作为汉字放进组句（`v#wang` → `v#王`），继续取名。
    pub fn select_name_surname(&mut self, text: &str) -> bool {
        if !self.name_selection_is_surname(text) {
            return false;
        }
        self.set_input(&format!("{NAME_PREFIX}{text}"));
        true
    }

    /// `v#` 后面的输入 → 取名候选。
    pub(super) fn name_candidates(&self, body: &str) -> Vec<Candidate> {
        let Some(tables) = &self.names else {
            return Vec::new();
        };
        if body.is_empty() {
            return Vec::new();
        }
        if body.starts_with(|c: char| !c.is_ascii()) {
            return self.literal_name_candidates(tables, body);
        }
        let body = normalize(body);
        if body.is_empty() {
            return Vec::new();
        }

        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for surname in self.surname_hits(&body) {
            let remainder = &body[surname.pinyin_len.min(body.len())..];
            if remainder.is_empty() {
                push_name_candidate(&mut items, &mut seen, surname.text.clone());
                continue;
            }
            for given in self.given_candidates(remainder) {
                push_name_candidate(&mut items, &mut seen, format!("{}{given}", surname.text));
                if items.len() >= NAME_CANDIDATE_LIMIT {
                    return items;
                }
            }
        }
        items
    }

    /// 已经选中汉字姓氏（`v#王`）：后面是名拼音就用名拼音匹配，空着就给表里靠前的名字。
    fn literal_name_candidates(&self, tables: &NameTables, body: &str) -> Vec<Candidate> {
        let (surname, rest) = split_literal_surname(tables, body);
        if !rest.is_ascii() {
            return vec![name_candidate(format!("{surname}{rest}"))];
        }
        let rest = normalize(rest);
        let given = if rest.is_empty() {
            self.default_given_names(tables)
        } else {
            self.given_candidates(&rest)
        };
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for given in given {
            push_name_candidate(&mut items, &mut seen, format!("{surname}{given}"));
            if items.len() >= NAME_CANDIDATE_LIMIT {
                break;
            }
        }
        items
    }

    /// 姓氏匹配：先查表，表里没有整个对上的姓氏才退回词库（罕姓 / 复姓）。
    fn surname_hits(&self, body: &str) -> Vec<SurnameHit> {
        let mut hits = if let Some(tables) = &self.names {
            table_surname_hits(tables, body)
        } else {
            Vec::new()
        };
        if !hits.iter().any(|hit| hit.complete) {
            hits.extend(self.dictionary_surname_hits(body));
        }
        // 同一段输入有更长的完整姓氏时，短姓氏只是把多出来的字母当成了名的开头：
        // `wang` 是 王 / 汪，不该再出「万 + g」。
        let longest = hits
            .iter()
            .filter(|hit| hit.complete)
            .map(|hit| hit.pinyin_len)
            .max();
        if let Some(longest) = longest {
            hits.retain(|hit| !hit.complete || hit.pinyin_len == longest);
        }
        hits.sort_by(|left, right| {
            right
                .pinyin_len
                .cmp(&left.pinyin_len)
                .then_with(|| left.rank.cmp(&right.rank))
        });
        hits
    }

    /// 词库兜底：单字姓（`cuan` → 爨）与复姓双字词（`dantai` → 澹台）。
    fn dictionary_surname_hits(&self, body: &str) -> Vec<SurnameHit> {
        let mut hits = Vec::new();
        let mut seen = HashSet::new();

        for end in 1..=body.len().min(parser::MAX_SYLLABLE_LEN) {
            let syllable = &body[..end];
            if !parser::is_syllable(syllable) {
                continue;
            }
            for (text, frequency) in self.dictionary_chars(syllable, false) {
                if text.chars().count() == 1 && seen.insert(text.clone()) {
                    hits.push(SurnameHit {
                        text,
                        pinyin_len: end,
                        complete: true,
                        rank: u32::MAX.saturating_sub(frequency),
                    });
                }
            }
        }

        for first_end in 1..=body.len().min(parser::MAX_SYLLABLE_LEN) {
            let first = &body[..first_end];
            if !parser::is_syllable(first) {
                continue;
            }
            for second_end in first_end + 1..=body.len().min(first_end + parser::MAX_SYLLABLE_LEN) {
                let second = &body[first_end..second_end];
                if !parser::is_syllable(second) {
                    continue;
                }
                let syllables = [first, second];
                for dictionary in self.all_dictionaries() {
                    for hit in dictionary.lookup(&syllables, false) {
                        if !hit.exact || hit.text.chars().count() != 2 {
                            continue;
                        }
                        let text = hit.text.to_owned();
                        if seen.insert(text.clone()) {
                            hits.push(SurnameHit {
                                text,
                                pinyin_len: second_end,
                                complete: true,
                                rank: u32::MAX.saturating_sub(hit.frequency),
                            });
                        }
                    }
                }
            }
        }
        hits
    }

    /// 名字部分：单字名与双字名都出；表里的字优先，表里没有该读音才查词库。
    fn given_candidates(&self, pinyin: &str) -> Vec<String> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        if pinyin.is_empty() {
            return items;
        }

        for text in self.name_chars_ordered(pinyin) {
            if seen.insert(text.clone()) {
                items.push(text);
            }
            if items.len() >= NAME_CANDIDATE_LIMIT {
                return items;
            }
        }

        // 双字名：前一个字必须是完整音节，后一个字可以还没打完。
        for split in 1..pinyin.len() {
            let (left, right) = pinyin.split_at(split);
            let left_chars = self.name_chars(left, false);
            if left_chars.is_empty() {
                continue;
            }
            let right_chars = self.name_chars_ordered(right);
            for first in &left_chars {
                for second in &right_chars {
                    let text = format!("{first}{second}");
                    if seen.insert(text.clone()) {
                        items.push(text);
                    }
                    if items.len() >= NAME_CANDIDATE_LIMIT {
                        return items;
                    }
                }
            }
        }
        items
    }

    /// 名字字：先整音节命中的（`yi` → 怡 / 艺），再排在后面补上更长音节的前缀命中
    /// （`yi` → 英 / 颖）。
    fn name_chars_ordered(&self, pinyin: &str) -> Vec<String> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for text in self.name_chars(pinyin, false) {
            if seen.insert(text.clone()) {
                items.push(text);
            }
        }
        for text in self.name_chars(pinyin, true) {
            if seen.insert(text.clone()) {
                items.push(text);
            }
        }
        items
    }

    /// 姓已经选好、名还空着：按名字表顺序先给单字名，再给少量双字名。
    fn default_given_names(&self, tables: &NameTables) -> Vec<String> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let top = tables.top_name_chars(DEFAULT_GIVEN_SINGLE + DEFAULT_GIVEN_PAIR);
        for entry in top.iter().take(DEFAULT_GIVEN_SINGLE) {
            if seen.insert(entry.text.clone()) {
                items.push(entry.text.clone());
            }
        }
        let pair_source = &top[..top.len().min(DEFAULT_GIVEN_PAIR)];
        for first in pair_source {
            for second in pair_source {
                let text = format!("{}{}", first.text, second.text);
                if seen.insert(text.clone()) {
                    items.push(text);
                }
                if items.len() >= NAME_CANDIDATE_LIMIT {
                    return items;
                }
            }
        }
        items
    }

    /// 名字字的候选：名字表里有这个读音就只用表里的，没有才退回词库。
    fn name_chars(&self, pinyin: &str, partial: bool) -> Vec<String> {
        if let Some(tables) = &self.names {
            let hits: Vec<String> = tables
                .name_chars(pinyin, partial)
                .into_iter()
                .map(|entry| entry.text.clone())
                .collect();
            if !hits.is_empty() {
                return hits;
            }
        }
        self.dictionary_chars(pinyin, partial)
            .into_iter()
            .map(|(text, _)| text)
            .collect()
    }

    /// 词库里读音匹配的单字，按词频降序（跨主词库 / 稀有词库 / 附加词库去重）。
    fn dictionary_chars(&self, pinyin: &str, partial: bool) -> Vec<(String, u32)> {
        let mut items: Vec<(String, u32)> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();
        for dictionary in self.all_dictionaries() {
            for hit in dictionary.lookup(&[pinyin], partial) {
                if !partial && !hit.exact {
                    continue;
                }
                if hit.text.chars().count() != 1 {
                    continue;
                }
                match positions.get(hit.text) {
                    Some(&index) => items[index].1 = items[index].1.max(hit.frequency),
                    None => {
                        positions.insert(hit.text.to_owned(), items.len());
                        items.push((hit.text.to_owned(), hit.frequency));
                    }
                }
            }
        }
        items.sort_by_key(|item| std::cmp::Reverse(item.1));
        items
    }
}

fn table_surname_hits(tables: &NameTables, body: &str) -> Vec<SurnameHit> {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    for entry in tables.surnames_leading(body) {
        if seen.insert(entry.text.clone()) {
            hits.push(SurnameHit {
                text: entry.text.clone(),
                pinyin_len: entry.pinyin.len(),
                complete: true,
                rank: entry.rank,
            });
        }
    }
    // 姓还没打完（`wan` → 王 / 汪）：只出姓，选中后留在组句里继续。
    for entry in tables.surnames_with_prefix(body) {
        if entry.pinyin.len() <= body.len() || !seen.insert(entry.text.clone()) {
            continue;
        }
        hits.push(SurnameHit {
            text: entry.text.clone(),
            pinyin_len: body.len(),
            complete: false,
            rank: entry.rank,
        });
    }
    hits
}

/// `v#` 后面已经有一个汉字姓氏时，切出姓氏与剩下的名。两个字的复姓优先。
fn split_literal_surname<'a>(tables: &NameTables, body: &'a str) -> (&'a str, &'a str) {
    let mut boundaries: Vec<usize> = body.char_indices().map(|(index, _)| index).collect();
    boundaries.push(body.len());
    let first_end = boundaries.get(1).copied().unwrap_or(body.len());
    if let Some(&second_end) = boundaries.get(2) {
        let two = &body[..second_end];
        if tables.surnames().iter().any(|entry| entry.text == two) {
            return (two, &body[second_end..]);
        }
    }
    (&body[..first_end], &body[first_end..])
}

fn push_name_candidate(items: &mut Vec<Candidate>, seen: &mut HashSet<String>, text: String) {
    if !seen.insert(text.clone()) {
        return;
    }
    items.push(name_candidate(text));
}

fn name_candidate(text: String) -> Candidate {
    Candidate {
        text,
        display: None,
        kind: CandidateKind::Shortcut,
        syllables: Vec::new(),
        reading: None,
        auxiliary: None,
        auxiliary_typed: None,
    }
}

/// 去掉拼音分隔符，统一小写。
fn normalize(body: &str) -> String {
    body.chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
