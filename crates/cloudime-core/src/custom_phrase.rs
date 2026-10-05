//! 用户短语（输入码 + 原样上屏的文本 + 可选的候选显示内容 + 固定的候选位置）；由 Core 匹配，平台负责存储与编辑。
//!
//! 输入码敲全时短语插到你指定的候选位置（`1` 第一位、`2` 第二位……），不再按权重排；同码多条位置相同的，
//! 按保存顺序依次占位。`title` 非空时候选里显示它，上屏仍是 `text`。

use serde::{Deserialize, Serialize};

/// 位置的缺省值：2 = 候选项的第二位。
pub const DEFAULT_POSITION: u32 = 2;

/// 位置的下限：1 = 候选项的第一位。
pub const MIN_POSITION: u32 = 1;

/// 位置的上限；再靠后的位置没人翻，也不会出现。
pub const MAX_POSITION: u32 = 9;

/// 按原始输入码匹配、原样上屏的文本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomPhrase {
    /// 小写英文字母输入码，1–32 个字符。
    pub code: String,

    /// 原样上屏的文本，保留空格和换行。
    pub text: String,

    /// 候选里显示的内容：为空时候选显示 [`Self::text`]，非空时显示它、上屏仍是 `text`。
    #[serde(default)]
    pub title: Option<String>,

    /// 固定的候选位置：`1` 第一位、`2` 第二位……（[`MIN_POSITION`]–[`MAX_POSITION`]）。
    #[serde(default = "default_position")]
    pub position: u32,
}

fn default_position() -> u32 {
    DEFAULT_POSITION
}

/// 保存和加载使用同一校验；同一输入码下的重复文本不允许。
pub fn validate_phrases(phrases: &[CustomPhrase]) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for phrase in phrases {
        if !valid_code(&phrase.code) {
            return Err("输入码须为 1–32 个小写英文字母".into());
        }
        if phrase.text.is_empty() {
            return Err("自定义短语不能为空".into());
        }
        if !(MIN_POSITION..=MAX_POSITION).contains(&phrase.position) {
            return Err(format!("短语位置须为 {MIN_POSITION}–{MAX_POSITION}"));
        }
        if !seen.insert((&phrase.code, &phrase.text)) {
            return Err(format!(
                "输入码 {} 下已有同样的短语，不能重复保存",
                phrase.code
            ));
        }
    }
    Ok(())
}

/// 把 `title` 去掉首尾空白，空串归一成 `None`；保存与加载共用。
pub fn normalize_phrases(phrases: &mut [CustomPhrase]) {
    for phrase in phrases {
        phrase.title = phrase
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned);
    }
}

/// 输入码能不能当自定义短语用：1–32 个小写英文字母。
fn valid_code(code: &str) -> bool {
    !code.is_empty() && code.len() <= 32 && code.bytes().all(|c| c.is_ascii_lowercase())
}

/// 把系统的文本替换（Windows 设置里的「文本替换」这类「输入码 → 短语」表）并进自定义短语：
/// 输入码不是小写字母、短语为空、或已有同码同文本的规则时跳过，新条目用缺省位置。
/// 结果保证通过 [`validate_phrases`]（前提是 `base` 本身合法）。
pub fn merge_replacements<'a>(
    base: &[CustomPhrase],
    replacements: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<CustomPhrase> {
    let mut phrases = base.to_vec();
    for (code, text) in replacements {
        if !valid_code(code) || text.is_empty() {
            continue;
        }
        if phrases.iter().any(|p| p.code == code && p.text == text) {
            continue;
        }
        phrases.push(CustomPhrase {
            code: code.to_owned(),
            text: text.to_owned(),
            title: None,
            position: DEFAULT_POSITION,
        });
    }
    phrases
}

impl CustomPhrase {
    /// 单行预览保留 Unicode 字符边界，用可见符号表示换行和制表符。
    pub fn preview(text: &str, max_chars: usize) -> String {
        let mut chars = text.chars().peekable();
        let mut preview = String::new();
        for _ in 0..max_chars {
            let Some(c) = chars.next() else {
                break;
            };
            preview.push(match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    '↵'
                }
                '\n' => '↵',
                '\t' => '⇥',
                _ => c,
            });
        }
        if chars.next().is_some() {
            preview.push('…');
        }
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CustomPhrase, DEFAULT_POSITION, MAX_POSITION, merge_replacements, normalize_phrases,
        validate_phrases,
    };

    fn phrase(code: &str, text: &str, position: u32) -> CustomPhrase {
        CustomPhrase {
            code: code.into(),
            text: text.into(),
            title: None,
            position,
        }
    }

    #[test]
    fn replacements_are_deduped_and_get_the_default_position() {
        let base = vec![phrase("yx", "第一位", 1), phrase("ee", "：", 2)];
        let merged = merge_replacements(
            &base,
            [
                ("yx", "qi@example.com"),
                ("omw", "On my way!"),
                ("ee", "："),
                ("Gs", "大写不算"),
                ("a1", "带数字不算"),
                ("", "空码不算"),
                ("kong", ""),
            ],
        );
        validate_phrases(&merged).unwrap();
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[2], phrase("yx", "qi@example.com", DEFAULT_POSITION));
        assert_eq!(merged[3], phrase("omw", "On my way!", DEFAULT_POSITION));
    }

    #[test]
    fn the_same_code_and_text_cannot_repeat() {
        let phrases = vec![phrase("aa", "，", 1), phrase("aa", "，", 2)];
        assert!(validate_phrases(&phrases).is_err());
    }

    #[test]
    fn position_must_be_in_range() {
        assert!(validate_phrases(&[phrase("aa", "，", MAX_POSITION + 1)]).is_err());
        assert!(validate_phrases(&[phrase("aa", "，", 0)]).is_err());
        assert!(validate_phrases(&[phrase("aa", "，", DEFAULT_POSITION)]).is_ok());
        assert!(validate_phrases(&[phrase("aa", "，", MAX_POSITION)]).is_ok());
    }

    #[test]
    fn the_same_code_can_carry_several_texts() {
        let phrases = vec![phrase("ee", "：", 1), phrase("ee", "；", 2)];
        assert!(validate_phrases(&phrases).is_ok());
    }

    #[test]
    fn titles_are_trimmed_and_blank_ones_become_none() {
        let mut phrases = vec![
            CustomPhrase {
                code: "aa".into(),
                text: "正文".into(),
                title: Some("  候选  ".into()),
                position: 1,
            },
            CustomPhrase {
                code: "bb".into(),
                text: "正文".into(),
                title: Some("   ".into()),
                position: 2,
            },
        ];
        normalize_phrases(&mut phrases);
        assert_eq!(phrases[0].title.as_deref(), Some("候选"));
        assert_eq!(phrases[1].title, None);
    }
}
