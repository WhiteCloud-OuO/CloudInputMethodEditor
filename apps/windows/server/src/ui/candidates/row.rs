//! 候选窗口的一行：[`Candidate`] → 渲染器的 [`Row`]（序号、候选词、annotation 片段）。

use cloudime_core::Candidate;
use cloudime_platform::ItemNumberStyle;
use cloudime_render::{Row, Tone};

/// `position` 是页内下标（从 0 起）；`style` 是配置里的序号写法；
/// `badge` 是这一行的来源角标（用户短语「短」/ 用户自造词「造」），没有为 `None`。
pub(crate) fn from_candidate(
    position: usize,
    candidate: &Candidate,
    badge: Option<char>,
    style: ItemNumberStyle,
) -> Row {
    let mut annotation = Vec::new();
    // 候选右侧的辅助读音 / 注解；当前没有来源写入，字段保留。
    if let Some(reading) = &candidate.reading {
        annotation.push((reading.clone(), Tone::Gloss));
    }
    Row {
        index: style.format(position + 1),
        text: candidate.display_text().to_owned(),
        annotation,
        badge: badge.map(String::from),
    }
}
