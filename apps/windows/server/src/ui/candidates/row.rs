//! 候选窗口的一行：[`Candidate`] → 渲染器的 [`Row`]（序号、候选词、annotation 片段）。

use cloudime_core::Candidate;
use cloudime_platform::ItemNumberStyle;
use cloudime_render::{Auxiliary, Row, Tone};

/// `position` 是页内下标（从 0 起）；`style` 是配置里的序号写法；
/// `badge` 是这一行的来源角标（用户短语「短」/ 用户自造词「造」），没有为 `None`；
/// `candidate_auxiliary` 为真时把候选带的辅助码画在候选词右边（`[XH]` 这样，Core 已经填好）。
pub(crate) fn from_candidate(
    position: usize,
    candidate: &Candidate,
    badge: Option<char>,
    style: ItemNumberStyle,
    candidate_auxiliary: bool,
) -> Row {
    let mut annotation = Vec::new();
    // 候选右侧的辅助读音 / 注解；当前没有来源写入，字段保留。
    if let Some(reading) = &candidate.reading {
        annotation.push((reading.clone(), Tone::Gloss));
    }
    // 候选项显示辅助码：Core 只把码填在候选里，画不画由这里的那道开关决定。
    // 已经敲上的那几位交给渲染器画下划线（`Row::auxiliary` 的两个部分）。
    let auxiliary = candidate_auxiliary
        .then(|| {
            candidate.auxiliary.as_ref().map(|code| {
                Auxiliary::split(
                    code,
                    candidate.auxiliary_typed.as_deref().unwrap_or_default(),
                )
            })
        })
        .flatten();
    Row {
        index: style.format(position + 1),
        text: candidate.display_text().to_owned(),
        annotation,
        badge: badge.map(String::from),
        auxiliary,
    }
}
