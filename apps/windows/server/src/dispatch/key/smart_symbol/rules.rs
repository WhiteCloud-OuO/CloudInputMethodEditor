//! 智能符号的自定义规则：读 `%APPDATA%\CloudIME\intelligent-symbol.json`，按光标前那一整句的正则匹配。
//!
//! 文件（见 [`cloudime_platform::smart_symbol::SMART_SYMBOL_TEMPLATE`]）是一组
//! `{ "symbol": "？", "trigger": ["吗$", "呢$"] }`：数组顺序就是优先级，`trigger` 里任意一条正则
//! 命中整句就算这一条；一条都不命中走内置规则（最终兜底句号）。
//! 正则编译失败只记一条警告、跳过那一条，不影响别的规则。

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;

/// 文件里的一条规则：命中任意一条 `trigger` 就上屏 `symbol`。
#[derive(Debug, Deserialize)]
struct RuleFile {
    symbol: String,
    #[serde(default)]
    trigger: Vec<String>,
}

/// 顶层既接受数组，也接受 `{ "rules": [...] }`（手写时两种都顺眼）。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FileShape {
    List(Vec<RuleFile>),
    Wrapped { rules: Vec<RuleFile> },
}

/// 编译好的一份规则表；顺序保持文件里的先后。
#[derive(Debug, Default)]
pub(crate) struct Rules {
    rules: Vec<(Regex, String)>,
}

impl Rules {
    /// 读文件；不在 / 读不动 / 解析失败都记日志并退回空表（只用内置规则）。
    pub(crate) fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match Self::parse(&text) {
            Ok(rules) => rules,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "智能符号规则读不出来，改用内置规则");
                Self::default()
            }
        }
    }

    /// 解析 JSON 文本：坏规则（空符号 / 空 trigger / 正则非法）逐条跳过。
    fn parse(text: &str) -> Result<Self, serde_json::Error> {
        let shape: FileShape = serde_json::from_str(text)?;
        let entries = match shape {
            FileShape::List(entries) | FileShape::Wrapped { rules: entries } => entries,
        };
        let mut rules = Vec::new();
        for (index, entry) in entries.into_iter().enumerate() {
            let symbol = entry.symbol.trim().to_owned();
            if symbol.is_empty() || entry.trigger.is_empty() {
                tracing::warn!(index, "智能符号规则缺 symbol 或 trigger，跳过");
                continue;
            }
            for trigger in entry.trigger {
                match Regex::new(&trigger) {
                    Ok(regex) => rules.push((regex, symbol.clone())),
                    Err(error) => {
                        tracing::warn!(index, %error, trigger, "智能符号的 trigger 不是合法正则，跳过")
                    }
                }
            }
        }
        Ok(Self { rules })
    }

    /// 这一整句命中哪条规则（按文件顺序，第一个命中的赢）。
    pub(crate) fn symbol_for(&self, sentence: &str) -> Option<&str> {
        self.rules
            .iter()
            .find(|(regex, _)| regex.is_match(sentence))
            .map(|(_, symbol)| symbol.as_str())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.rules.len()
    }
}

/// 规则文件路径：用户目录下的 `intelligent-symbol.json`。
pub(crate) fn rules_path(user_dir: &Path) -> PathBuf {
    user_dir.join(cloudime_platform::smart_symbol::SMART_SYMBOL_FILE)
}

/// 光标前的**这一整句**：从最后一个句末标点之后开始。
/// 现代汉语里逗号 / 顿号 / 分号都只隔开句内成分，不断句；句号 / 问号 / 叹号 / 省略号 / 换行才断句。
pub(crate) fn current_sentence(text: &str) -> &str {
    let start = text
        .char_indices()
        .filter(|(_, c)| is_sentence_end(*c))
        .map(|(index, c)| index + c.len_utf8())
        .next_back()
        .unwrap_or(0);
    &text[start..]
}

fn is_sentence_end(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '｡' | '…' | '!' | '?' | '\n' | '\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comma_does_not_split_the_sentence() {
        assert_eq!(
            current_sentence("明天下雨天，干脆不出门了吧！今天也"),
            "今天也"
        );
        assert_eq!(
            current_sentence("明天下雨天，干脆不出门了吧"),
            "明天下雨天，干脆不出门了吧"
        );
        assert_eq!(current_sentence("上一句。云朵输入法"), "云朵输入法");
        assert_eq!(current_sentence(""), "");
    }

    #[test]
    fn rules_match_in_order_and_skip_bad_ones() {
        let rules = Rules::parse(
            r#"[
                { "symbol": "？", "trigger": ["吗$", "呢$"] },
                { "symbol": "……", "trigger": ["吧$", "[不"] }
            ]"#,
        )
        .unwrap();
        // "吧$" 命中第二条；"[不" 不是合法正则，只跳过那一条。
        assert_eq!(rules.symbol_for("干脆不出门了吧"), Some("……"));
        assert_eq!(rules.symbol_for("你好吗"), Some("？"));
        assert_eq!(rules.symbol_for("你好"), None);
        assert_eq!(rules.len(), 3);
    }

    #[test]
    fn wrapped_shape_is_accepted() {
        let rules =
            Rules::parse(r#"{ "rules": [{ "symbol": "！", "trigger": ["啊$"] }] }"#).unwrap();
        assert_eq!(rules.symbol_for("好累啊"), Some("！"));
    }

    /// 缺省规则（安装时放出去的那份）：内置那几条之外，还有「整句就是一个中文数字 → 顿号」。
    #[test]
    fn the_default_rules_cover_the_builtins_and_a_lone_numeral() {
        let rules = Rules::parse(cloudime_platform::smart_symbol::SMART_SYMBOL_TEMPLATE).unwrap();
        assert_eq!(rules.symbol_for("好吗"), Some("？"));
        assert_eq!(rules.symbol_for("好累啊"), Some("！"));
        assert_eq!(rules.symbol_for("轰呜"), Some("——"));
        // `^…$` 锚整句：只有整句就是一个数字才命中。
        for numeral in ["一", "二", "十"] {
            assert_eq!(rules.symbol_for(numeral), Some("、"), "{numeral}");
        }
        for other in ["三四", "五个", "第六", "一，"] {
            assert_eq!(rules.symbol_for(other), None, "{other}");
        }
    }
}
