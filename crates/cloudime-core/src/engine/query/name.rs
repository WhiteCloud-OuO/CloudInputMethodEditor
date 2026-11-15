//! V 模式 `#` 取名：姓氏表 + 名字表，表里没有的字退回自带词库。
//!
//! 输入是「姓拼音 + 名拼音」连写（`v#zhangwei`）；中途也可以先选中姓氏，让汉字留在组句里
//! （`v#王`），再输入名或直接从候选里挑。候选顺序：姓氏按表里的排名（复姓优先），同一姓氏下按
//! 名字表的行序；**名字表里没有的同音字（罕见名字用字）与表里没有的姓也出，只是排在后面**。

use std::collections::{HashMap, HashSet};

use crate::candidate::{Candidate, CandidateKind};
use crate::name::{NAME_PREFIX, NameTables};
use crate::parser;

use super::super::Engine;

/// 取名候选一次最多给多少条（窗口再翻页也是这些）。
///
/// 比别的模式宽：名字表只有几百个字，罕见名字用字全靠词库补在末尾，留够翻页的距离才够得着。
const NAME_CANDIDATE_LIMIT: usize = 120;

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

    /// 来自姓氏表（否则是词库兜底的罕见姓）。
    from_table: bool,

    /// 音节拆开的拼音（`欧阳` → `["ou", "yang"]`），按整词查词库时用。
    syllables: Vec<String>,
}

/// 名字字的来源：名字表（常见用字，含双字名）还是词库（同读音的其余字，罕见用字，只出单字）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GivenSource {
    Table,
    Dictionary,
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

    /// 名字还没打完时选中「姓 + 名字的一个字」：把它留在组句里，返回新的组句文本
    /// （`v#daishilin` 选 `戴石` → `v#戴石lin`），剩下的拼音接着取下一个字。
    ///
    /// 只在拼音还剩**下一个音节**时才这样（`shilin` → 选「石」后还剩 `lin`）；只剩一个音节时
    /// 选中的就是完整名字（`v#戴shi` 选 `戴石`），返回 `None`，照常上屏。
    pub fn name_next_part(&self, text: &str) -> Option<String> {
        let tables = self.names.as_ref()?;
        let body = self.name_body()?;
        // 拆出「已经落在组句里的汉字前缀」与剩下的名拼音。
        let (prefix, pinyin) = if body.is_ascii() {
            let hit = self
                .surname_hits(body)
                .into_iter()
                .find(|hit| hit.pinyin_len < body.len() && text.starts_with(&hit.text))?;
            (hit.text, &body[hit.pinyin_len..])
        } else {
            let (surname, rest) = split_literal_surname(tables, body);
            let (literal, rest) = split_literal_given(rest);
            (format!("{surname}{literal}"), rest)
        };
        if pinyin.is_empty() || !pinyin.is_ascii() {
            return None;
        }
        // 选中的必须是「前缀 + 一个字」，而且这个字对得上后面的第一个音节。
        let given = text.strip_prefix(&prefix)?;
        let mut chars = given.chars();
        let (Some(given), None) = (chars.next(), chars.next()) else {
            return None;
        };
        let given = given.to_string();
        // 取最长的、还能留下后续拼音的音节（`xian…` 里优先按 `xian` 拆，拆不动再看 `xi`）。
        for end in (1..pinyin.len()).rev() {
            let syllable = &pinyin[..end];
            if !parser::is_syllable(syllable) {
                continue;
            }
            if self.given_chars_exact(syllable).contains(&given) {
                return Some(format!("{text}{}", &pinyin[end..]));
            }
        }
        None
    }

    /// 读音正好是这个音节的单字（名字表 + 词库，不算前缀命中）：判断「这个字用掉了哪个音节」用。
    fn given_chars_exact(&self, pinyin: &str) -> Vec<String> {
        let mut items = self.table_name_chars(pinyin, false);
        items.extend(
            self.dictionary_chars(pinyin, false)
                .into_iter()
                .map(|(text, _)| text),
        );
        items
    }

    /// 选中之后应该留在组句里（不上屏）的候选：姓氏，或还没取完的名字字。
    pub fn name_selection_keeps_composing(&self, text: &str) -> bool {
        self.name_selection_is_surname(text) || self.name_next_part(text).is_some()
    }

    /// 把选中的这一截落在组句里（`v#wang` → `v#王`、`v#daishilin` 选 `戴石` → `v#戴石lin`），
    /// 继续取名。
    pub fn select_name_part(&mut self, text: &str) -> bool {
        if self.name_selection_is_surname(text) {
            self.set_input(&format!("{NAME_PREFIX}{text}"));
            return true;
        }
        match self.name_next_part(text) {
            Some(next) => {
                self.set_input(&format!("{NAME_PREFIX}{next}"));
                true
            }
            None => false,
        }
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
        // 词库补的罕见名字字先攒着：常见组合（名字表里的字）全部排完之后，按「同一个名字字 ×
        // 每个姓」交错补在末尾 —— 复姓与同音的多个姓都有机会拿到它，不会全被排名最靠前的那个姓占满。
        // 词库里现成的姓名整词（`张玮`）单独一轮排在前面，免得被「张为」「张未」这类字压到几页之后。
        let mut boosted = Vec::new();
        let mut rare = Vec::new();
        for surname in self.surname_hits(&body) {
            let remainder = &body[surname.pinyin_len.min(body.len())..];
            if remainder.is_empty() {
                push_name_candidate(&mut items, &mut seen, surname.text.clone());
                continue;
            }
            for given in self.given_candidates(remainder, GivenSource::Table) {
                push_name_candidate(&mut items, &mut seen, format!("{}{given}", surname.text));
                if items.len() >= NAME_CANDIDATE_LIMIT {
                    return items;
                }
            }
            self.push_partial_given(&mut items, &mut seen, &surname.text, remainder);
            let boost = self.name_word_given_chars(&surname.text, &surname.syllables, remainder);
            let mut boost_seen: HashSet<String> = boost.iter().cloned().collect();
            let rest: Vec<String> = self
                .given_candidates(remainder, GivenSource::Dictionary)
                .into_iter()
                .filter(|given| boost_seen.insert(given.clone()))
                .collect();
            boosted.push((surname.text.clone(), boost));
            rare.push((surname.text.clone(), rest));
        }
        if push_given_interleaved(&mut items, &mut seen, &boosted) {
            return items;
        }
        push_given_interleaved(&mut items, &mut seen, &rare);
        items
    }

    /// 已经选中汉字姓氏（`v#王`）：后面是名拼音就用名拼音匹配，空着就给表里靠前的名字。
    /// `v#戴石lin` 这种「已经取好的名字字 + 还没打完的拼音」接着往后取。
    fn literal_name_candidates(&self, tables: &NameTables, body: &str) -> Vec<Candidate> {
        let (surname, rest) = split_literal_surname(tables, body);
        let (literal, rest) = split_literal_given(rest);
        if !rest.is_ascii() {
            return vec![name_candidate(format!("{surname}{literal}{rest}"))];
        }
        let rest = normalize(rest);
        // 名字字已经取过、后面也没拼音了：这一条就是完整名字，选中上屏。
        if !literal.is_empty() && rest.is_empty() {
            return vec![name_candidate(format!("{surname}{literal}"))];
        }
        let given = if rest.is_empty() {
            self.default_given_names(tables)
        } else if !literal.is_empty() {
            // 已经取好一个字了：后面只补单字，不再拼双字名（`戴石` + `lin` 只需要一个「麟」）。
            self.single_given_candidates(&rest)
        } else {
            // 表里的字 → 词库里现成的姓名整词（`张玮`）→ 其余同音字。
            let mut given = self.given_candidates(&rest, GivenSource::Table);
            let mut seen: HashSet<String> = given.iter().cloned().collect();
            let syllables: &[String] = tables
                .surnames()
                .iter()
                .find(|entry| entry.text == surname)
                .map(|entry| entry.syllables.as_slice())
                .unwrap_or_default();
            for text in self.name_word_given_chars(surname, syllables, &rest) {
                if seen.insert(text.clone()) {
                    given.push(text);
                }
            }
            for text in self.given_candidates(&rest, GivenSource::Dictionary) {
                if seen.insert(text.clone()) {
                    given.push(text);
                }
            }
            given
        };
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for given in given {
            push_name_candidate(&mut items, &mut seen, format!("{surname}{literal}{given}"));
            if items.len() >= NAME_CANDIDATE_LIMIT {
                break;
            }
        }
        self.push_partial_given(&mut items, &mut seen, &format!("{surname}{literal}"), &rest);
        items
    }

    /// 名还没打完（后面还有音节）时，把第一个音节的字也列出来：`v#daishilin` → `戴石`。
    /// 选中后剩下的拼音接着取 —— 双字名就这样一个字一个字取，不靠词库里的两字整词。
    fn push_partial_given(
        &self,
        items: &mut Vec<Candidate>,
        seen: &mut HashSet<String>,
        prefix: &str,
        remainder: &str,
    ) {
        let Some((first, _)) = split_first_syllable(remainder) else {
            return;
        };
        for source in [GivenSource::Table, GivenSource::Dictionary] {
            for given in self.given_candidates(first, source) {
                push_name_candidate(items, seen, format!("{prefix}{given}"));
                if items.len() >= NAME_CANDIDATE_LIMIT {
                    return;
                }
            }
        }
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
                        syllables: vec![syllable.to_owned()],
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
                                syllables: vec![first.to_owned(), second.to_owned()],
                            });
                        }
                    }
                }
            }
        }
        hits
    }

    /// 词库里现成的「姓 + 单字名」整词（`张玮`、`张巍`）—— 这些名字字排在同音词库字前面，
    /// 免得罕见名字字被「张为」「张未」这类常见的非名字字压到几页之后。
    ///
    /// 只在名正好是一个完整音节时查；返回单字（按整词词频降序）。
    fn name_word_given_chars(
        &self,
        surname: &str,
        syllables: &[String],
        remainder: &str,
    ) -> Vec<String> {
        if syllables.is_empty() || !parser::is_syllable(remainder) {
            return Vec::new();
        }
        let mut pattern: Vec<&str> = syllables.iter().map(String::as_str).collect();
        pattern.push(remainder);
        let surname_chars = surname.chars().count();
        let mut hits: Vec<(String, u32)> = Vec::new();
        let mut seen = HashSet::new();
        for dictionary in self.all_dictionaries() {
            for hit in dictionary.lookup(&pattern, false) {
                if !hit.exact || hit.text.chars().count() != surname_chars + 1 {
                    continue;
                }
                let Some(given) = hit.text.strip_prefix(surname) else {
                    continue;
                };
                if given.is_empty() || !seen.insert(given.to_owned()) {
                    continue;
                }
                hits.push((given.to_owned(), hit.frequency));
            }
        }
        hits.sort_by_key(|(_, frequency)| std::cmp::Reverse(*frequency));
        hits.into_iter().map(|(text, _)| text).collect()
    }

    /// 名字表 + 词库里的单字（不含双字名）：已经取好一个名字字之后，后面只补一个单字。
    fn single_given_candidates(&self, pinyin: &str) -> Vec<String> {
        let mut items = self.table_name_chars_ordered(pinyin);
        let mut seen: HashSet<String> = items.iter().cloned().collect();
        for text in self.dictionary_given_candidates(pinyin) {
            if seen.insert(text.clone()) {
                items.push(text);
            }
        }
        items
    }

    /// 名字部分。`source` 决定只取名字表（常见用字，含双字名）还是只取词库（同读音的其余字，
    /// 罕见名字用字，只出单字）—— 调用方先出表里的，再把词库那份排在后面。
    fn given_candidates(&self, pinyin: &str, source: GivenSource) -> Vec<String> {
        if pinyin.is_empty() {
            return Vec::new();
        }
        match source {
            GivenSource::Table => self.table_given_candidates(pinyin),
            GivenSource::Dictionary => self.dictionary_given_candidates(pinyin),
        }
    }

    /// 名字表里的名字字：单字名（整音节命中 → 更长音节的前缀命中）在前，双字名在后。
    fn table_given_candidates(&self, pinyin: &str) -> Vec<String> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();

        for text in self.table_name_chars_ordered(pinyin) {
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
            let left_chars = self.table_name_chars(left, false);
            if left_chars.is_empty() {
                continue;
            }
            let right_chars = self.table_name_chars_ordered(right);
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

    /// 词库里同读音、名字表里没有的字（罕见名字用字）：整音节命中在前，更长音节的前缀命中在后，
    /// 各按词频降序。
    fn dictionary_given_candidates(&self, pinyin: &str) -> Vec<String> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
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

    /// 名字表里这个读音的字；表里没有这个读音就是空的（词库那份由
    /// [`Self::dictionary_given_candidates`] 补）。
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
                syllables: entry.syllables.clone(),
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
            syllables: entry.syllables.clone(),
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

/// 名拼音的第一个音节（后面还有拼音时）：`shilin` → (`shi`, `lin`)。取最长的那个音节前缀。
fn split_first_syllable(pinyin: &str) -> Option<(&str, &str)> {
    (1..pinyin.len())
        .rev()
        .find(|&end| parser::is_syllable(&pinyin[..end]))
        .map(|end| (&pinyin[..end], &pinyin[end..]))
}

fn push_name_candidate(items: &mut Vec<Candidate>, seen: &mut HashSet<String>, text: String) {
    if !seen.insert(text.clone()) {
        return;
    }
    items.push(name_candidate(text));
}

/// 按名字字交错：同一个名字字依次给每个姓各来一条，再轮到下一个名字字。
/// 返回是否已经到上限（调用方直接收工）。
fn push_given_interleaved(
    items: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
    groups: &[(String, Vec<String>)],
) -> bool {
    let widest = groups
        .iter()
        .map(|(_, given)| given.len())
        .max()
        .unwrap_or(0);
    for index in 0..widest {
        for (surname, given) in groups {
            let Some(given) = given.get(index) else {
                continue;
            };
            push_name_candidate(items, seen, format!("{surname}{given}"));
            if items.len() >= NAME_CANDIDATE_LIMIT {
                return true;
            }
        }
    }
    false
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
