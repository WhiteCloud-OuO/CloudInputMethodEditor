//! 附加候选：日期时间等快捷项、中英混输的英文词与补全。

use super::*;

impl Engine {
    /// 精确匹配自定义输入码时，数字键应选择候选。
    pub(super) fn has_custom_phrase(&self) -> bool {
        !self.english_mode
            && self
                .custom_phrases
                .iter()
                .any(|p| p.code == self.composition.scope())
    }

    /// 精确匹配输入码的短语插到你指定的候选位置：`0` 第一位、`1` 第二位……；
    /// 同码多条按位置升序占位，位置相同的按保存顺序依次往后排，越界的排到最后。
    pub(super) fn insert_custom_phrases(&self, items: &mut Vec<Candidate>) {
        if self.english_mode {
            return;
        }
        let mut phrases: Vec<_> = self
            .custom_phrases
            .iter()
            .filter(|p| p.code == self.composition.scope())
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
                _ => phrase.position as usize,
            };
            let index = index.min(items.len());
            items.insert(
                index,
                Candidate {
                    text: phrase.text.clone(),
                    kind: CandidateKind::Custom,
                    syllables: Vec::new(),
                    reading: None,
                },
            );
            last = Some((phrase.position, index));
        }
    }

    /// 日期 / 时间 / 星期这类快捷候选插在本地首选之后：`rq` 首选仍是词库里的词，快捷写法紧随其后。
    pub(super) fn insert_shortcuts(&self, items: &mut Vec<Candidate>, scope: &str) {
        let shortcuts = shortcut::candidates(scope, &jiff::Zoned::now());
        if shortcuts.is_empty() {
            return;
        }
        let position = items.len().min(1);
        items.splice(position..position, shortcuts);
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
