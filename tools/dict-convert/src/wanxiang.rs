//! 万象拼音中文词库（`rime-wanxiang-data` 的 `dicts/`）→ 云朵 `dict.tsv`。
//!
//! 只取 14 个中文拼音库（`zi` / `jichu` / 分类词库），不碰 `en` / `mixed` / `abbrev` / `t9`：
//! 后四个的“编码”不是拼音，不能进中文词库。
//!
//! 规则：
//! - 带调拼音去调：`ü/ǖǘǚǜ` → `v`；`j/q/x/y` 后的 `u` 是 ü 的书写形式，保持 `u` 不改；
//! - `ń/ň/ǹ` → `en`，`ńg/ňg/ǹg` → `ng`（万象的 `嗯` 读 `en/ng`）；`ḿ/m̀` → `me`；
//! - 权重取前导数字（`76N` → 76），缺省 1，0 抬到 1；
//! - 同一 `(文字, 拼音)` 取最大权重合并；
//! - 补一条 `哼 hng`：万象只收 `hng` 这个叹词读音的 `heng`，但词库要能打 `hng`。

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use cloudime_core::parser::is_syllable;

use crate::error::ConvertError;
use crate::lexicon::tone;

/// 上游默认导入的核心词库（`wanxiang.dict.yaml` 的 `import_tables`）。
const CORE_FILES: &[&str] = &[
    "zi.dict.yaml",
    "jichu.dict.yaml",
    "cuoyin.dict.yaml",
    "duoyin.dict.yaml",
    "diming.dict.yaml",
    "mingren.dict.yaml",
    "yiren.dict.yaml",
];

/// 上游注释掉、由各方案按需开启的分类词库；云朵内置词库沿用旧行为，全部合入。
const OPTIONAL_FILES: &[&str] = &[
    "yixue.dict.yaml",
    "huaxue.dict.yaml",
    "yaopin.dict.yaml",
    "wuzhong.dict.yaml",
    "taifeng.dict.yaml",
    "fangyan.dict.yaml",
    "shici.dict.yaml",
];

/// 一个词的读音有多个音节时按空格拼；拼不出来 / 含非拼音音节的行跳过。
fn normalize_pinyin(pinyin: &str) -> Option<String> {
    let mut syllables = Vec::new();
    for raw in pinyin.split_whitespace() {
        let plain = tone::strip_tone(raw);
        let plain = match plain.as_str() {
            // 万象的独立成字叹词：`嗯` 的 n / ng、`呣` 的 m，按上游迁移说明转成普通音节。
            "n" => "en".to_owned(),
            "m" => "me".to_owned(),
            _ => plain,
        };
        if !is_syllable(&plain) {
            return None;
        }
        syllables.push(plain);
    }
    (!syllables.is_empty()).then(|| syllables.join(" "))
}

/// 上游权重可能带后缀（`76N` / `10s`）：取前导数字；没写或不是数字按 1。
fn weight_of(raw: Option<&str>) -> (u32, bool, bool) {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return (1, true, false);
    };
    let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return (1, false, false);
    }
    let weight = digits.parse::<u32>().unwrap_or(1);
    (weight.max(1), false, weight == 0)
}

#[derive(Default)]
struct Stats {
    rows: usize,
    merged: usize,
    skipped: usize,
    missing_weight: usize,
    zero_weight: usize,
    bad_weight: usize,
    added_hng: usize,
}

pub fn convert(source: &Path, out: &Path, core: bool) -> Result<(), ConvertError> {
    let mut names = CORE_FILES.to_vec();
    if !core {
        names.extend_from_slice(OPTIONAL_FILES);
    }
    let mut pairs: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut stats = Stats::default();
    for name in &names {
        let path = source.join(name);
        let text = fs::read_to_string(&path).map_err(|error| {
            ConvertError::Io(std::io::Error::new(
                error.kind(),
                format!("{}: {error}", path.display()),
            ))
        })?;
        parse_file(&text, &mut pairs, &mut stats);
    }
    if pairs.is_empty() {
        return Err(ConvertError::Verify(format!(
            "{} 里没有解析出词条（目录不对？）",
            source.display()
        )));
    }

    // 万象只收 `哼 hēng`；用户要能按 `hng` 打出来，补一条同权重的叹词读音。
    let hng_weight = pairs
        .iter()
        .filter(|((text, _), _)| text == "哼")
        .map(|(_, weight)| *weight)
        .max()
        .unwrap_or(1);
    if pairs
        .insert(("哼".to_owned(), "hng".to_owned()), hng_weight)
        .is_none()
    {
        stats.added_hng = 1;
    }

    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(out)?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "# 云朵中文词库：万象拼音（rime-wanxiang-data）转换")?;
    writeln!(writer, "# 来源：{}", source.display())?;
    writeln!(
        writer,
        "# 许可：CC BY 4.0（https://creativecommons.org/licenses/by/4.0/）"
    )?;
    writeln!(
        writer,
        "# 修改：去除声调（ü→v，j/q/x/y 后的 u 保持 u）、n/ng→en/ng、m→me、合并词表、权重取前导数字并抬到 ≥1"
    )?;
    writeln!(writer, "# 列：文字<TAB>拼音<TAB>权重")?;
    for ((text, pinyin), weight) in &pairs {
        writeln!(writer, "{text}\t{pinyin}\t{weight}")?;
    }
    writer.flush()?;

    tracing::info!(
        files = names.len(),
        rows = stats.rows,
        pairs = pairs.len(),
        merged = stats.merged,
        skipped = stats.skipped,
        missing_weight = stats.missing_weight,
        zero_weight = stats.zero_weight,
        bad_weight = stats.bad_weight,
        added_hng = stats.added_hng,
        out = %out.display(),
        "万象词库转换完成"
    );
    Ok(())
}

fn parse_file(text: &str, pairs: &mut BTreeMap<(String, String), u32>, stats: &mut Stats) {
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
        let word = fields.next().unwrap_or("").trim();
        let pinyin = fields.next().unwrap_or("").trim();
        if word.is_empty() || pinyin.is_empty() {
            continue;
        }
        stats.rows += 1;
        let (weight, missing, zero) = weight_of(fields.next());
        if missing {
            stats.missing_weight += 1;
        }
        if zero {
            stats.zero_weight += 1;
        }
        let Some(pinyin) = normalize_pinyin(pinyin) else {
            stats.skipped += 1;
            continue;
        };
        let key = (word.to_owned(), pinyin);
        match pairs.get_mut(&key) {
            Some(existing) if *existing >= weight => stats.merged += 1,
            Some(existing) => {
                *existing = weight;
                stats.merged += 1;
            }
            None => {
                pairs.insert(key, weight);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_u_after_j_q_x_y_and_converts_umlaut() {
        assert_eq!(
            normalize_pinyin("jū qún xué yǔ yuán jūn").as_deref(),
            Some("ju qun xue yu yuan jun")
        );
        assert_eq!(
            normalize_pinyin("lǜ nǚ lüè nüè").as_deref(),
            Some("lv nv lve nve")
        );
    }

    #[test]
    fn maps_interjections_and_ng() {
        assert_eq!(normalize_pinyin("ǹ ń ň").as_deref(), Some("en en en"));
        assert_eq!(normalize_pinyin("ǹg ńg ňg").as_deref(), Some("ng ng ng"));
        assert_eq!(normalize_pinyin("ḿ m̀").as_deref(), Some("me me"));
    }

    #[test]
    fn parses_weight_prefix() {
        assert_eq!(weight_of(Some("76N")), (76, false, false));
        assert_eq!(weight_of(Some("10s")), (10, false, false));
        assert_eq!(weight_of(None), (1, true, false));
        assert_eq!(weight_of(Some("0")), (1, false, true));
    }

    #[test]
    fn merges_duplicates_by_max_weight_and_skips_non_pinyin() {
        let text = "---\nname: t\n...\n啊\tā\t5\n啊\tá\t9\n嗯\tǹ\t7\n嗯\tǹg\t6\n呣\tḿ\t3\n呣\tmóu\t2\n西妥昔单抗β\txī tuǒ xī dān kàng beta\t1\n";
        let mut pairs = BTreeMap::new();
        let mut stats = Stats::default();
        parse_file(text, &mut pairs, &mut stats);
        assert_eq!(pairs.get(&("啊".to_owned(), "a".to_owned())), Some(&9));
        assert_eq!(pairs.get(&("嗯".to_owned(), "en".to_owned())), Some(&7));
        assert_eq!(pairs.get(&("嗯".to_owned(), "ng".to_owned())), Some(&6));
        assert_eq!(pairs.get(&("呣".to_owned(), "me".to_owned())), Some(&3));
        assert_eq!(pairs.get(&("呣".to_owned(), "mou".to_owned())), Some(&2));
        assert!(!pairs.keys().any(|(text, _)| text.contains('β')));
        assert_eq!(stats.skipped, 1);
    }
}
