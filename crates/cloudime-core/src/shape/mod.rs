//! 形码输入（五笔 / 仓颉 / 声笔）：一张「编码 → 文字」的码表。
//!
//! 码表一行 `编码<TAB>文字<TAB>权重`，编码只认小写 `a`–`z`、最长 [`Table::max_code_length`] 位
//! （方案文件里写）。查询是**前缀匹配**：敲 1–4 键都拿已经敲的整串当前缀，命中所有以它开头的编码；
//! 同一个文字会出现在好几个编码下（五笔的一级到三级简码与全码），同一页里只留权重最高的那一条。
//!
//! 形码没有拼音切分、没有整句转换，候选只按静态权重排；码表由平台层从方案目录读出来，
//! Core 不碰文件（与双键拼音的键位表同一条分工）。

use std::collections::HashMap;

/// 一次查询最多回多少条候选：一两个键的前缀能命中几千条，收窄到前几百条足够翻好几页。
pub const MAX_CANDIDATES: usize = 500;

/// 码表里的一条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// 完整编码（小写 `a`–`z`，1–[`Table::max_code_length`] 位）。
    pub code: Box<str>,

    /// 上屏文字：单字或词组。
    pub text: Box<str>,

    /// 静态权重，越大越靠前。
    pub weight: u32,

    /// 源文件里的行序：权重相同的条目按上游词库的顺序排。
    order: u32,
}

/// 一张形码码表。条目按编码升序、同一编码内按权重降序排好，前缀查询靠二分定位。
#[derive(Debug, Default)]
pub struct Table {
    entries: Vec<Entry>,
    max_code_length: usize,
}

impl Table {
    /// 解析码表。`#` 开头的注释行与空行忽略；格式不对的行跳过，原因收在返回的第二个值里。
    ///
    /// `max_code_length` 取方案文件的 `MaxCodeLength`（五笔 4），超过它的编码按写错处理，
    /// 免得一条 5 位的怪编码让前缀查询多出一层。
    pub fn parse(source: &str, max_code_length: usize) -> (Self, Vec<String>) {
        let max_code_length = max_code_length.max(1);
        let mut entries: Vec<Entry> = Vec::new();
        let mut problems = Vec::new();
        for (index, raw) in source.lines().enumerate() {
            let line = raw.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split('\t');
            let code = fields.next().map(str::trim).unwrap_or_default();
            let text = fields.next().map(str::trim).unwrap_or_default();
            if !is_valid_code(code, max_code_length) {
                problems.push(format!(
                    "第 {} 行：编码 `{code}` 只认小写 a–z、1–{max_code_length} 位，已跳过",
                    index + 1
                ));
                continue;
            }
            if text.is_empty() {
                problems.push(format!("第 {} 行：没有文字，已跳过", index + 1));
                continue;
            }
            let weight = match fields.next().map(str::trim) {
                Some(value) if !value.is_empty() => match value.parse::<u32>() {
                    Ok(weight) => weight,
                    Err(_) => {
                        problems.push(format!(
                            "第 {} 行：权重 `{value}` 不是非负整数，已跳过",
                            index + 1
                        ));
                        continue;
                    }
                },
                _ => 1,
            };
            entries.push(Entry {
                code: code.into(),
                text: text.into(),
                weight,
                order: entries.len() as u32,
            });
        }
        // 编码升序；同一编码内权重降序；权重相同保持源文件顺序（`order` 唯一，排序稳定）
        entries.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| right.weight.cmp(&left.weight))
                .then_with(|| left.order.cmp(&right.order))
        });
        (
            Self {
                entries,
                max_code_length,
            },
            problems,
        )
    }

    /// 码表条数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 方案文件里的最长编码长度。
    pub fn max_code_length(&self) -> usize {
        self.max_code_length
    }

    /// 编码以 `prefix` 开头的候选，按权重降序排、最多 `limit` 条。
    ///
    /// 同一个文字在好几个编码下命中时只留一条：优先权重高的，权重相同时取源文件里靠前的
    /// （五笔的一级简码通常排在前面）。
    pub fn candidates(&self, prefix: &str, limit: usize) -> Vec<&Entry> {
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        let start = self
            .entries
            .partition_point(|entry| entry.code.as_ref() < prefix);
        // 文字 → 选中的条目下标
        let mut best: HashMap<&str, usize> = HashMap::new();
        for (offset, entry) in self.entries[start..].iter().enumerate() {
            if !entry.code.starts_with(prefix) {
                break;
            }
            let index = start + offset;
            match best.get(entry.text.as_ref()) {
                Some(&chosen) if !self.better(index, chosen) => {}
                _ => {
                    best.insert(entry.text.as_ref(), index);
                }
            }
        }
        let mut chosen: Vec<usize> = best.into_values().collect();
        chosen.sort_by(|&left, &right| {
            let left = &self.entries[left];
            let right = &self.entries[right];
            right
                .weight
                .cmp(&left.weight)
                .then_with(|| left.code.len().cmp(&right.code.len()))
                .then_with(|| left.order.cmp(&right.order))
        });
        chosen.truncate(limit);
        chosen
            .into_iter()
            .map(|index| &self.entries[index])
            .collect()
    }

    /// 条目 `candidate` 是否比已经选中的 `chosen` 更适合代表这个文字：权重高者优先，
    /// 权重相同取源文件里靠前的。
    fn better(&self, candidate: usize, chosen: usize) -> bool {
        let candidate = &self.entries[candidate];
        let chosen = &self.entries[chosen];
        if candidate.weight != chosen.weight {
            candidate.weight > chosen.weight
        } else {
            candidate.order < chosen.order
        }
    }
}

/// 编码只认小写 `a`–`z`，长度 1 到 `max_code_length`。
fn is_valid_code(code: &str, max_code_length: usize) -> bool {
    !code.is_empty()
        && code.len() <= max_code_length
        && code.bytes().all(|byte| byte.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# 注释行
a\t工\t20
a\t戈\t10
aa\t式\t20
aaa\t工\t10
aaaa\t工\t20
aaaa\t恭恭敬敬\t10
ggll\t一\t20
rqyy\t的\t30
rqyy\t换句话说\t20
";

    fn table() -> Table {
        Table::parse(SAMPLE, 4).0
    }

    fn texts(entries: Vec<&Entry>) -> Vec<&str> {
        entries
            .into_iter()
            .map(|entry| entry.text.as_ref())
            .collect()
    }

    #[test]
    fn the_table_is_sorted_by_code_and_weight() {
        let table = table();
        assert_eq!(table.len(), 9);
        assert_eq!(table.max_code_length(), 4);
        let codes: Vec<&str> = table.entries.iter().map(|entry| &*entry.code).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        assert_eq!(codes, sorted);
        let aaaa: Vec<u32> = table
            .entries
            .iter()
            .filter(|entry| entry.code.as_ref() == "aaaa")
            .map(|entry| entry.weight)
            .collect();
        assert_eq!(aaaa, [20, 10], "同一编码内按权重降序");
    }

    #[test]
    fn a_prefix_matches_every_code_that_starts_with_it() {
        let table = table();
        // 一键 `a`：a / aa / aaa / aaaa 全算；工 在三个编码下出现，只留权重最高的那条
        assert_eq!(
            texts(table.candidates("a", 10)),
            ["工", "式", "戈", "恭恭敬敬"]
        );
        // 三键 `aaa`：aaa 与 aaaa；工 取权重 20 的那条（aaaa）
        let candidates = table.candidates("aaa", 10);
        assert_eq!(texts(candidates.clone()), ["工", "恭恭敬敬"]);
        assert_eq!(candidates[0].code.as_ref(), "aaaa");
        // 四键是完整码：只命中 aaaa
        assert_eq!(texts(table.candidates("aaaa", 10)), ["工", "恭恭敬敬"]);
        // 一码多词的按权重排：的 30 在 换句话说 20 前面
        assert_eq!(texts(table.candidates("rqyy", 10)), ["的", "换句话说"]);
    }

    #[test]
    fn candidates_are_capped_and_unmatched_prefixes_are_empty() {
        let table = table();
        assert_eq!(table.candidates("a", 1).len(), 1);
        assert!(table.candidates("zzzz", 10).is_empty());
        assert!(table.candidates("", 10).is_empty(), "空前缀不查");
        assert!(table.candidates("a", 0).is_empty());
    }

    #[test]
    fn malformed_lines_are_skipped_with_a_reason() {
        let source = "\
A\t大写编码
aaaaa\t太长的编码
ab\t
ab\t文本\t不是数字
ab\t文本\t5
";
        let (table, problems) = Table::parse(source, 4);
        assert_eq!(table.len(), 1);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert_eq!(texts(table.candidates("ab", 10)), ["文本"]);
    }

    #[test]
    fn a_bom_and_crlf_do_not_break_the_first_line() {
        let source = "\u{feff}a\t工\t20\r\n";
        let (table, problems) = Table::parse(source, 4);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(texts(table.candidates("a", 10)), ["工"]);
    }
}
