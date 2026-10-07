//! 释义的数据模型：一个词条对应一到多条「词性 + 译文」。

use serde::{Deserialize, Serialize};

/// 一条释义。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sense {
    /// 词性缩写（`n.` / `adj.` / `v.`…）；数据源没给就是 `None`。
    /// 词典里存的就是缩写本身，所以这里不解析成枚举——显示要的正是这串。
    pub pos: Option<String>,

    /// 译文本身。
    pub text: String,

    /// 读音（日语假名之类）；没有就是 `None`。
    pub reading: Option<String>,
}

impl Sense {
    /// 拼成一行纯文本：`adj. sad`（有读音就接 `|读音`）。
    pub fn joined(&self) -> String {
        let mut text = String::new();
        if let Some(pos) = &self.pos {
            text.push_str(pos);
            text.push(' ');
        }
        text.push_str(&self.text);
        if let Some(reading) = &self.reading {
            text.push('|');
            text.push_str(reading);
        }
        text
    }
}

/// 候选窗底部那一行左侧要显示的翻译提示：词条 + 释义 + 学没学过。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tip {
    /// 词条本身（候选文本）。
    pub word: String,

    /// 学过没有：决定 Tip 的颜色（`candidate_translate_tip_color`）。
    pub learned: bool,

    /// 释义，按词典里的顺序。
    pub senses: Vec<Sense>,
}

impl Tip {
    /// 各条释义拼成一段：`adj. sad; n. sorrow`。
    pub fn joined(&self) -> String {
        self.senses
            .iter()
            .map(Sense::joined)
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sense_joins_its_pos_and_reading() {
        let sense = Sense {
            pos: Some("n.".to_owned()),
            text: "開発".to_owned(),
            reading: Some("かいはつ".to_owned()),
        };
        assert_eq!(sense.joined(), "n. 開発|かいはつ");
        let plain = Sense {
            pos: None,
            text: "hello".to_owned(),
            reading: None,
        };
        assert_eq!(plain.joined(), "hello");
    }

    #[test]
    fn a_tip_joins_its_senses() {
        let tip = Tip {
            word: "悲伤的".to_owned(),
            learned: false,
            senses: vec![
                Sense {
                    pos: Some("adj.".to_owned()),
                    text: "sad".to_owned(),
                    reading: None,
                },
                Sense {
                    pos: Some("n.".to_owned()),
                    text: "sorrow".to_owned(),
                    reading: None,
                },
            ],
        };
        assert_eq!(tip.joined(), "adj. sad; n. sorrow");
    }
}
