//! V 模式 `#` 取名：姓氏表 + 名字表，表里没有的字退回自带词库。
//!
//! 输入是「姓拼音 + 名拼音」连写（`v#lixiuqin`），**一段一段地取**：还没选姓时只出姓的候选；
//! 选中姓之后只出名的**第一段**（`xiu`）的候选，选中名字字后剩下的拼音接着取（`qin`）——
//! 不会在选了姓之后就把后面几段拼成整名。候选顺序：姓按表里的排名（复姓优先）；名字表里的字在前，
//! 词库里同读音的其余字（罕见名字用字）按词频排在后面。

use std::collections::{HashMap, HashSet};

use crate::candidate::{Candidate, CandidateKind};
use crate::name::{NAME_PREFIX, NameTables};
use crate::parser;

use super::super::Engine;

/// 取名候选一次最多给多少条（窗口再翻页也是这些）。
const NAME_CANDIDATE_LIMIT: usize = 120;

/// 一个拼音串能匹配到的姓氏。
struct SurnameHit {
    text: String,

    /// 这个姓氏的拼音在输入里占多少字节；选中它之后剩下的拼音从这儿往后取。
    pinyin_len: usize,

    /// 拼音已经整个对上了（`wang`），而不是只打了个开头（`wan` → 王）。
    complete: bool,

    /// 排序用：复姓（拼音长）在前，同长按排名。
    rank: u32,

    /// 来自姓氏表（否则是词库兜底的罕见姓）。
    from_table: bool,
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

    /// 这个候选是不是姓氏（还没选姓时只出姓氏）。
    pub fn name_selection_is_surname(&self, text: &str) -> bool {
        let Some(body) = self.name_body() else {
            return false;
        };
        if self.shape.is_some() {
            return self
                .shape_name_candidates(body)
                .iter()
                .any(|candidate| candidate.text == text);
        }
        if !body.is_ascii() {
            return false;
        }
        let body = normalize(body);
        self.surname_hits(&body).iter().any(|hit| hit.text == text)
    }

    /// 选中这一项之后组句应该变成什么：姓氏与名字字都只吃自己那一段，后面的拼音原样留着。
    /// 返回 `None` 表示这一项是完整名字（或根本不是取名的候选），照常上屏。
    ///
    /// - 姓氏：`v#lixiuqin` 选 `李` → `v#李xiuqin`（姓氏永远不直接上屏）；
    /// - 名字字：`v#李xiuqin` 选 `李秀` → `v#李秀qin`；最后一段吃完（没有剩余拼音）就上屏。
    pub fn name_part_after(&self, text: &str) -> Option<String> {
        let body = self.name_body()?;
        if self.shape.is_some() {
            return self.shape_name_part_after(body, text);
        }
        // 还没选姓：只认姓氏，吃掉它的拼音，后面没打完的留着。
        if body.is_ascii() {
            let body = normalize(body);
            let hit = self
                .surname_hits(&body)
                .into_iter()
                .find(|hit| hit.text == text)?;
            let rest = &body[hit.pinyin_len.min(body.len())..];
            return Some(format!("{text}{rest}"));
        }
        // 已经选了汉字姓：只认「姓（+ 已经取好的名字字）+ 一个字」，这个字还要对得上第一段拼音。
        let tables = self.names.as_ref()?;
        let (surname, rest) = split_literal_surname(tables, body);
        let (literal, rest) = split_literal_given(rest);
        if rest.is_empty() {
            return None;
        }
        let given = text.strip_prefix(surname)?.strip_prefix(literal)?;
        if given.chars().count() != 1 {
            return None;
        }
        let (first, remain) = first_segment(rest);
        let first = normalize(first);
        if first.is_empty() {
            return None;
        }
        if !self
            .single_given_candidates(&first)
            .iter()
            .any(|character| character == given)
        {
            return None;
        }
        // 这一段就是最后一个音节：选中即完整名字，上屏；否则留着剩下的拼音接着取。
        if remain.is_empty() {
            return None;
        }
        Some(format!("{text}{remain}"))
    }

    /// 选中之后应该留在组句里（不上屏）的候选：姓氏，或者还没取完的名字字。
    pub fn name_selection_keeps_composing(&self, text: &str) -> bool {
        self.name_part_after(text).is_some()
    }

    /// 把选中的这一截落在组句里，继续取名。
    pub fn select_name_part(&mut self, text: &str) -> bool {
        match self.name_part_after(text) {
            Some(next) => {
                self.set_input(&format!("{NAME_PREFIX}{next}"));
                // 形码里 `set_input` 从空组句起新一段会清掉 v 模式标志；取名要接着按 v 模式解析。
                if self.shape.is_some() {
                    self.forced_expression = true;
                }
                true
            }
            None => false,
        }
    }

    /// `v#` 后面的输入 → 取名候选。
    pub(super) fn name_candidates(&self, body: &str) -> Vec<Candidate> {
        if body.is_empty() || self.names.is_none() {
            return Vec::new();
        }
        if self.shape.is_some() {
            return self.shape_name_candidates(body);
        }
        let Some(tables) = &self.names else {
            return Vec::new();
        };
        if body.starts_with(|c: char| !c.is_ascii()) {
            return self.literal_name_candidates(tables, body);
        }
        let body = normalize(body);
        if body.is_empty() {
            return Vec::new();
        }
        // 还没选姓：只出姓，不出任何名字候选。
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for hit in self.surname_hits(&body) {
            push_name_candidate(&mut items, &mut seen, hit.text);
            if items.len() >= NAME_CANDIDATE_LIMIT {
                break;
            }
        }
        items
    }

    /// 形码取名：`v#xtay'wfnh`，每段是**一个字的五笔编码**，用 `'` 隔开。
    ///
    /// 还没选姓时只出第一段编码里的姓氏（表里的姓按排名，表里没有就退回这段编码的所有单字）；
    /// 选了姓之后（组句里是 `v#张'wfnh`）只出下一段编码对应的名字字，最后一段选中即整名上屏。
    fn shape_name_candidates(&self, body: &str) -> Vec<Candidate> {
        let (literal, rest) = split_literal_and_rest(body);
        if literal.is_empty() {
            let (code, _) = shape_segment(rest);
            let chars = self.shape_single_chars(code);
            if chars.is_empty() {
                return Vec::new();
            }
            let mut ranked: Vec<(u32, &str)> = chars
                .iter()
                .filter_map(|&text| self.surname_rank(text).map(|rank| (rank, text)))
                .collect();
            if ranked.is_empty() {
                // 姓氏表里没有这个字：退回码表里这一段的所有单字（罕见姓）
                ranked = chars
                    .iter()
                    .enumerate()
                    .map(|(index, &text)| (index as u32, text))
                    .collect();
            }
            ranked.sort_by_key(|(rank, _)| *rank);
            ranked.truncate(NAME_CANDIDATE_LIMIT);
            return ranked
                .into_iter()
                .map(|(_, text)| name_candidate(text.to_owned()))
                .collect();
        }
        // 已经选了姓：后面必须用 `'` 隔开，再来一段编码。
        if rest.is_empty() {
            return vec![name_candidate(literal.to_owned())];
        }
        let Some(codes) = rest.strip_prefix('\'') else {
            return Vec::new();
        };
        let (code, _) = shape_segment(codes);
        let mut ordered: Vec<(usize, usize, &str)> = self
            .shape_single_chars(code)
            .into_iter()
            .enumerate()
            .map(|(index, text)| {
                (
                    self.name_table_order(text).unwrap_or(usize::MAX),
                    index,
                    text,
                )
            })
            .collect();
        ordered.sort_by_key(|(order, index, _)| (*order, *index));
        ordered
            .into_iter()
            .take(NAME_CANDIDATE_LIMIT)
            .map(|(_, _, text)| name_candidate(format!("{literal}{text}")))
            .collect()
    }

    /// 形码取名的「选中这一项之后」：姓氏留下后一段编码，名字字留下更后面的编码；
    /// 最后一段吃完返回 `None`（整名上屏）。
    fn shape_name_part_after(&self, body: &str, text: &str) -> Option<String> {
        let (literal, rest) = split_literal_and_rest(body);
        if literal.is_empty() {
            let (code, remaining) = shape_segment(rest);
            if !self.shape_single_chars(code).contains(&text) {
                return None;
            }
            return Some(format!("{text}{remaining}"));
        }
        let codes = rest.strip_prefix('\'')?;
        if codes.is_empty() {
            return None;
        }
        let (code, remaining) = shape_segment(codes);
        let given = text.strip_prefix(literal)?;
        if given.chars().count() != 1 || !self.shape_single_chars(code).contains(&given) {
            return None;
        }
        if remaining.is_empty() {
            return None;
        }
        Some(format!("{text}{remaining}"))
    }

    /// 形码码表里 `code` 前缀命中的单字（多字词不要）。
    fn shape_single_chars(&self, code: &str) -> Vec<&str> {
        let Some(table) = &self.shape else {
            return Vec::new();
        };
        if code.is_empty() {
            return Vec::new();
        }
        table
            .candidates(code, 200)
            .into_iter()
            .filter(|entry| entry.text.chars().count() == 1)
            .map(|entry| entry.text.as_ref())
            .collect()
    }

    /// 姓氏表里的排名；不是表里的姓为 `None`。
    fn surname_rank(&self, text: &str) -> Option<u32> {
        self.names
            .as_ref()?
            .surnames()
            .iter()
            .find(|entry| entry.text == text)
            .map(|entry| entry.rank)
    }

    /// 名字表里的行序；不在表里为 `None`。
    fn name_table_order(&self, text: &str) -> Option<usize> {
        self.names
            .as_ref()?
            .names()
            .iter()
            .position(|entry| entry.text == text)
    }

    /// 已经选中汉字姓氏（`v#李`）：只出名的**第一段**的字；名字取完（没拼音了）就只剩整名那一条。
    fn literal_name_candidates(&self, tables: &NameTables, body: &str) -> Vec<Candidate> {
        let (surname, rest) = split_literal_surname(tables, body);
        let (literal, rest) = split_literal_given(rest);
        let prefix = format!("{surname}{literal}");
        if rest.is_empty() {
            return vec![name_candidate(prefix)];
        }
        let (first, _) = first_segment(rest);
        let first = normalize(first);
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for given in self.single_given_candidates(&first) {
            push_name_candidate(&mut items, &mut seen, format!("{prefix}{given}"));
            if items.len() >= NAME_CANDIDATE_LIMIT {
                break;
            }
        }
        items
    }

    /// 一个音节（或还没打完的一截）能用的名字字：名字表里的字在前（表内顺序，整音节命中优先），
    /// 词库里同读音的其余字补在后面（罕见名字用字，按词频）。
    fn single_given_candidates(&self, pinyin: &str) -> Vec<String> {
        let mut items = self.table_name_chars_ordered(pinyin);
        let mut seen: HashSet<String> = items.iter().cloned().collect();
        for partial in [false, true] {
            for (text, _) in self.dictionary_chars(pinyin, partial) {
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

    /// 名字表里的字：先整音节命中的（`yi` → 怡 / 艺），再排在后面补上更长音节的前缀命中
    /// （`yi` → 英 / 颖）。
    fn table_name_chars_ordered(&self, pinyin: &str) -> Vec<String> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for text in self.table_name_chars(pinyin, false) {
            if seen.insert(text.clone()) {
                items.push(text);
            }
        }
        for text in self.table_name_chars(pinyin, true) {
            if seen.insert(text.clone()) {
                items.push(text);
            }
        }
        items
    }

    /// 名字表里这个读音的字；表里没有这个读音就是空的。
    fn table_name_chars(&self, pinyin: &str, partial: bool) -> Vec<String> {
        self.names
            .as_ref()
            .map(|tables| {
                tables
                    .name_chars(pinyin, partial)
                    .into_iter()
                    .map(|entry| entry.text.clone())
                    .collect()
            })
            .unwrap_or_default()
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

    /// 姓氏匹配：先查表，表里没有整个对上的姓氏才退回词库（罕姓 / 复姓）。
    fn surname_hits(&self, body: &str) -> Vec<SurnameHit> {
        let mut hits = if let Some(tables) = &self.names {
            table_surname_hits(tables, body)
        } else {
            Vec::new()
        };
        // 表里没有完整对上的姓才退回词库。表里已经有时不再补：`zhangwei` 里的「张伟」「长」「涨」
        // 都不是姓，补进来只会把真正的姓名挤到后面。
        if !hits.iter().any(|hit| hit.complete) {
            let mut seen: HashSet<String> = hits.iter().map(|hit| hit.text.clone()).collect();
            hits.extend(
                self.dictionary_surname_hits(body)
                    .into_iter()
                    .filter(|hit| seen.insert(hit.text.clone())),
            );
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
                .then_with(|| right.from_table.cmp(&left.from_table))
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
                        from_table: false,
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
                                from_table: false,
                            });
                        }
                    }
                }
            }
        }
        hits
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
                from_table: true,
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
            from_table: true,
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

/// 已经取好的名字字（开头连续的非 ASCII 字符）与后面还没打完的名拼音：
/// `石lin` → (`石`, `lin`)；`石` → (`石`, ``)。
fn split_literal_given(body: &str) -> (&str, &str) {
    let end = body
        .char_indices()
        .find(|(_, character)| character.is_ascii())
        .map(|(index, _)| index)
        .unwrap_or(body.len());
    (&body[..end], &body[end..])
}

/// 形码取名：已经选好的汉字（姓 + 已经取好的名）与后面 `'` 开头的剩余编码。
/// `张'wfnh` → (`张`, `'wfnh`)；`xtay'wfnh` → (``, `xtay'wfnh`)。
fn split_literal_and_rest(body: &str) -> (&str, &str) {
    let end = body
        .char_indices()
        .find(|(_, character)| character.is_ascii())
        .map(|(index, _)| index)
        .unwrap_or(body.len());
    (&body[..end], &body[end..])
}

/// 形码取名 / 车牌的当前编码段：`'` 之前的一段，与从 `'` 起的剩余输入。
fn shape_segment(text: &str) -> (&str, &str) {
    match text.find('\'') {
        Some(index) => (&text[..index], &text[index..]),
        None => (text, ""),
    }
}

/// 拼音的第一段（一个完整音节；还没打完就是这个残缺音节）与后面剩下的拼音。
/// 用户用 `'` 显式隔开音节时（`li'xiu'qin`）以它为准。
fn first_segment(pinyin: &str) -> (&str, &str) {
    if let Some(index) = pinyin.find('\'') {
        return (&pinyin[..index], &pinyin[index + 1..]);
    }
    let end = (1..=pinyin.len())
        .rev()
        .find(|&end| parser::is_syllable(&pinyin[..end]))
        .unwrap_or(pinyin.len());
    (&pinyin[..end], &pinyin[end..])
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
