use super::super::Candidate;

/// 本地候选的分页排布。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CandidateLayout {
    /// 本地候选，顺序就是 Engine 排好的顺序。
    local: Vec<Candidate>,

    /// 每页几格。
    page_size: usize,
}

impl CandidateLayout {
    pub fn new(local: Vec<Candidate>, page_size: usize) -> Self {
        Self {
            local,
            page_size: page_size.max(1),
        }
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    pub fn local(&self) -> &[Candidate] {
        &self.local
    }

    pub fn len(&self) -> usize {
        self.local.len()
    }

    pub fn is_empty(&self) -> bool {
        self.local.is_empty()
    }

    pub fn pages(&self) -> usize {
        self.len().div_ceil(self.page_size)
    }

    /// 第 `index` 个候选；越界返回 `None`。
    pub fn candidate(&self, index: usize) -> Option<&Candidate> {
        self.local.get(index)
    }

    /// 第 `page` 页的候选；越界就是空。
    pub fn page(&self, page: usize) -> &[Candidate] {
        let start = (page * self.page_size).min(self.local.len());
        let end = (start + self.page_size).min(self.local.len());
        &self.local[start..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CandidateKind;

    fn local(text: &str) -> Candidate {
        Candidate {
            text: text.into(),
            display: None,
            kind: CandidateKind::Chinese,
            syllables: vec!["zhang".into(), "tao".into()],
            reading: None,
        }
    }

    fn texts(candidates: &[Candidate]) -> Vec<String> {
        candidates.iter().map(|c| c.text.clone()).collect()
    }

    fn many(count: usize) -> Vec<Candidate> {
        (0..count).map(|i| local(&format!("本{i}"))).collect()
    }

    #[test]
    fn candidates_fill_the_page_and_overflow_to_the_next() {
        let layout = CandidateLayout::new(many(12), 9);
        assert_eq!(
            texts(layout.page(0)),
            [
                "本0", "本1", "本2", "本3", "本4", "本5", "本6", "本7", "本8"
            ]
        );
        assert_eq!(layout.pages(), 2);
        assert_eq!(texts(layout.page(1)), ["本9", "本10", "本11"]);
    }

    #[test]
    fn the_last_page_can_be_short() {
        let layout = CandidateLayout::new(many(11), 9);
        assert_eq!(layout.pages(), 2);
        assert_eq!(texts(layout.page(1)), ["本9", "本10"]);
        assert!(layout.page(2).is_empty());
    }

    #[test]
    fn candidates_are_indexed_across_pages() {
        let layout = CandidateLayout::new(many(12), 9);
        assert_eq!(layout.candidate(0).unwrap().text, "本0");
        assert_eq!(layout.candidate(11).unwrap().text, "本11");
        assert!(layout.candidate(12).is_none());
    }
}
