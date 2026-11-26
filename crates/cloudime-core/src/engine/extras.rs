//! 附加候选：中英混输的英文词与补全。

use super::*;
use crate::custom_phrase;

impl Engine {
    /// 形码（五笔）里输入码超过 4 个字母的短语可以只敲「前 3 + 最后一个字母」；
    /// z 模式是临时拼音，v / u / i 面板也不走码表，仍按完整输入码匹配。
    fn abbreviates_phrase_code(&self) -> bool {
        self.shape.is_some() && !self.shape_z_mode() && !self.forced_expression && !self.panel_mode
    }

    /// 当前输入是否命中这条短语的输入码（拼音只认完整码，五笔可走长码缩写）。
    pub(super) fn phrase_matches(&self, phrase: &crate::CustomPhrase) -> bool {
        custom_phrase::code_matches(
            &phrase.code,
            self.composition.scope(),
            self.abbreviates_phrase_code(),
        )
    }

    /// 当前输入命中了自定义短语（或五笔里的长码缩写）时，数字键应选择候选。
    pub(super) fn has_custom_phrase(&self) -> bool {
        !self.english_mode && self.custom_phrases.iter().any(|p| self.phrase_matches(p))
    }

    /// 命中输入码的短语插到你指定的候选位置：`1` 第一位、`2` 第二位……；
    /// 同码多条按位置升序占位，位置相同的按保存顺序依次往后排，越界的排到最后。
    /// `title` 非空时把它作为候选的显示内容，上屏仍是短语文本。
    pub(super) fn insert_custom_phrases(&self, items: &mut Vec<Candidate>) {
        if self.english_mode {
            return;
        }
        let mut phrases: Vec<_> = self
            .custom_phrases
            .iter()
            .filter(|p| self.phrase_matches(p))
            .collect();
        if phrases.is_empty() {
            return;
        }
        phrases.sort_by_key(|p| p.position);
        let mut last: Option<(u32, usize)> = None;
        for phrase in phrases {
            let index = match last {
                // 位置撞车：后面那条往后挪一格，保持保存顺序
                Some((position, index)) if position == phrase.position => index + 1,
                _ => phrase.position.saturating_sub(custom_phrase::MIN_POSITION) as usize,
            };
            let index = index.min(items.len());
            items.insert(
                index,
                Candidate {
                    text: phrase.text.clone(),
                    display: phrase.title.clone(),
                    kind: CandidateKind::Custom,
                    syllables: Vec::new(),
                    reading: None,
                    auxiliary: None,
                    auxiliary_typed: None,
                },
            );
            last = Some((phrase.position, index));
        }
    }

    /// 给英文候选用的词表，个人的在前、随包的在后；一张都没有就是空。
    pub(super) fn english_lists(&self) -> Vec<&WordList> {
        self.learner
            .user_english()
            .into_iter()
            .chain(self.english.as_ref())
            .collect()
    }
}
