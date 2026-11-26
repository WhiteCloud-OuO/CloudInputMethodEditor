//! 五笔 86 短语码表：只保留单字全码，按五笔规则从万象拼音的纯中文 2–4 字词条生成短语。
//!
//! 现有 `table.tsv` 的单字与旧短语全部保留（兼容旧习惯），再叠加万象里权重 ≥ `weight_min`
//! 的词条；每个编码最多新增 `per_code` 条，避免一个四码翻几十页。
//!
//! 规则（用单字的**全码**，不是一级 / 二级简码）：
//! - 二字词：两字各取前两码；
//! - 三字词：前两字各取第一码 + 第三字前两码；
//! - 四字词：前三字各取第一码 + 末字第一码。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::error::ConvertError;
use crate::wanxiang::{CORE_FILES, OPTIONAL_FILES};

#[derive(Debug, Clone)]
struct Row {
    code: String,
    text: String,
    weight: u32,
}

fn is_han(ch: char) -> bool {
    let value = ch as u32;
    (0x4E00..=0x9FFF).contains(&value)
        || (0x3400..=0x4DBF).contains(&value)
        || (0x20000..=0x2FA1F).contains(&value)
        || (0x2CEB0..=0x2EE5F).contains(&value)
        || (0x30000..=0x323AF).contains(&value)
        || value == 0x3007
}

/// 上游权重可能带后缀（`76N` / `10s`）：取前导数字；没写或不是数字按 1。
fn weight_of(raw: Option<&str>) -> u32 {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return 1;
    };
    let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u32>().unwrap_or(1).max(1)
}

fn read_table(path: &Path) -> Result<Vec<Row>, ConvertError> {
    let mut rows = Vec::new();
    for raw in fs::read_to_string(path)?.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let Some(code) = fields.next().filter(|code| !code.is_empty()) else {
            continue;
        };
        let Some(text) = fields.next().filter(|text| !text.is_empty()) else {
            continue;
        };
        rows.push(Row {
            code: code.to_ascii_lowercase(),
            text: text.to_owned(),
            weight: weight_of(fields.next()),
        });
    }
    Ok(rows)
}

/// 单字全码：取最长编码；同长度取权重最高（权重相同保留文件里靠前的那条）。
fn full_codes(rows: &[Row]) -> HashMap<char, String> {
    let mut full: HashMap<char, (String, u32, usize)> = HashMap::new();
    for (order, row) in rows.iter().enumerate() {
        let mut chars = row.text.chars();
        let Some(ch) = chars.next() else {
            continue;
        };
        if chars.next().is_some() {
            continue;
        }
        let better = full.get(&ch).is_none_or(|(code, weight, _)| {
            row.code.len() > code.len() || (row.code.len() == code.len() && row.weight > *weight)
        });
        if better {
            full.insert(ch, (row.code.clone(), row.weight, order));
        }
    }
    full.into_iter()
        .map(|(ch, (code, _, _))| (ch, code))
        .collect()
}

fn derive_code(text: &str, full: &HashMap<char, String>) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let codes: Vec<&str> = chars
        .iter()
        .map(|ch| full.get(ch).map(String::as_str))
        .collect::<Option<_>>()?;
    match chars.len() {
        2 if codes[0].len() >= 2 && codes[1].len() >= 2 => {
            Some(format!("{}{}", &codes[0][..2], &codes[1][..2]))
        }
        3 if codes[2].len() >= 2 => Some(format!(
            "{}{}{}",
            &codes[0][..1],
            &codes[1][..1],
            &codes[2][..2]
        )),
        4 => Some(format!(
            "{}{}{}{}",
            &codes[0][..1],
            &codes[1][..1],
            &codes[2][..1],
            &codes[3][..1]
        )),
        _ => None,
    }
}

pub fn convert(
    table: &Path,
    dicts: &Path,
    out: &Path,
    weight_min: u32,
    per_code: usize,
) -> Result<(), ConvertError> {
    if per_code == 0 {
        return Err(ConvertError::Verify("--per-code 必须大于 0".to_owned()));
    }
    let current = read_table(table)?;
    let full = full_codes(&current);
    let mut entries: BTreeMap<(String, String), u32> = BTreeMap::new();
    for row in &current {
        let key = (row.code.clone(), row.text.clone());
        let weight = entries.entry(key).or_insert(row.weight);
        *weight = (*weight).max(row.weight);
    }

    let mut wanted: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut candidates = 0usize;
    for name in CORE_FILES.iter().chain(OPTIONAL_FILES) {
        let path = dicts.join(name);
        let text = fs::read_to_string(&path).map_err(|error| {
            ConvertError::Io(std::io::Error::new(
                error.kind(),
                format!("{}: {error}", path.display()),
            ))
        })?;
        let mut body = false;
        for raw in text.lines() {
            let line = raw.trim_end_matches('\r').trim();
            if !body {
                if line == "..." {
                    body = true;
                }
                continue;
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split('\t');
            let Some(word) = fields.next().filter(|word| !word.is_empty()) else {
                continue;
            };
            let chars: Vec<char> = word.chars().collect();
            if !(2..=4).contains(&chars.len()) || !chars.iter().all(|ch| is_han(*ch)) {
                continue;
            }
            let weight = weight_of(fields.nth(1));
            if weight < weight_min {
                continue;
            }
            let Some(code) = derive_code(word, &full) else {
                continue;
            };
            candidates += 1;
            let key = (code, word.to_owned());
            wanted
                .entry(key)
                .and_modify(|existing| *existing = (*existing).max(weight))
                .or_insert(weight);
        }
    }

    let mut grouped: BTreeMap<String, Vec<(u32, String)>> = BTreeMap::new();
    for ((code, word), weight) in wanted {
        grouped.entry(code).or_default().push((weight, word));
    }

    let mut added = 0usize;
    let mut updated = 0usize;
    let mut capped = 0usize;
    for (code, mut words) in grouped {
        words.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        words.truncate(per_code);
        capped += words.len();
        for (weight, text) in words {
            let key = (code.clone(), text);
            match entries.get_mut(&key) {
                Some(existing) => {
                    if weight > *existing {
                        *existing = weight;
                        updated += 1;
                    }
                }
                None => {
                    entries.insert(key, weight);
                    added += 1;
                }
            }
        }
    }

    let mut rows: Vec<((String, String), u32)> = entries.into_iter().collect();
    rows.sort_by(|left, right| {
        left.0
            .0
            .cmp(&right.0.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.0.1.cmp(&right.0.1))
    });

    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(out)?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "# 五笔86 码表")?;
    writeln!(
        writer,
        "# 单字与旧短语来源：KyleBing/rime-wubi86-jidian（MIT，Copyright (c) Kyle Bing）"
    )?;
    writeln!(
        writer,
        "# 新增短语来源：万象拼音 rime-wanxiang-data 的 2–4 字纯中文词条（CC BY 4.0）"
    )?;
    writeln!(
        writer,
        "# 规则：二字词取每字前两码；三字词前两字首码 + 第三字前两码；四字词前三字首码 + 末字首码"
    )?;
    writeln!(
        writer,
        "# 过滤：万象词条权重 ≥ {weight_min}；每个编码最多新增 {per_code} 条；旧短语全部保留"
    )?;
    writeln!(writer, "# 列：编码<TAB>文字<TAB>权重")?;
    for ((code, text), weight) in &rows {
        writeln!(writer, "{code}\t{text}\t{weight}")?;
    }
    writer.flush()?;

    tracing::info!(
        table = %table.display(),
        rows = rows.len(),
        current = current.len(),
        candidates,
        added,
        updated,
        capped,
        out = %out.display(),
        "五笔短语码表生成完成"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_phrase_codes_from_full_codes() {
        let full = HashMap::from([
            ('你', "wqiy".to_owned()),
            ('好', "vbg".to_owned()),
            ('中', "khk".to_owned()),
            ('华', "wxf".to_owned()),
            ('人', "wwww".to_owned()),
            ('民', "nav".to_owned()),
            ('共', "awu".to_owned()),
            ('和', "tkg".to_owned()),
            ('国', "lgyi".to_owned()),
        ]);
        assert_eq!(derive_code("你好", &full).as_deref(), Some("wqvb"));
        assert_eq!(derive_code("中和", &full).as_deref(), Some("khtk"));
        assert_eq!(derive_code("中国人", &full).as_deref(), Some("klww"));
        assert_eq!(derive_code("中华人民共和国", &full), None);
        assert_eq!(derive_code("A中", &full), None);
    }

    #[test]
    fn filters_non_han_and_weight_prefix() {
        assert!(is_han('中'));
        assert!(is_han('𰻝'));
        assert!(!is_han('A'));
        assert_eq!(weight_of(Some("76N")), 76);
        assert_eq!(weight_of(Some("10s")), 10);
        assert_eq!(weight_of(None), 1);
    }
}
