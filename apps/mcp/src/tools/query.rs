//! 从 Engine 查询结果到 MCP 返回结构的转换。

use cloudime_core::{Engine, Query, Segmentation};

use super::{CandidateView, LookupResponse, SegmentResponse, SegmentationView, SyllableView};

/// 从 Engine 查询拼音；输入检查与 Engine 状态都在这里处理。
pub fn query(
    engine: &mut Engine,
    pinyin: &str,
    use_jian_pin: Option<bool>,
) -> Result<Query, String> {
    let input = pinyin.trim();
    if input.is_empty() {
        return Err("拼音不能为空".to_owned());
    }
    if input.chars().count() > 200 {
        return Err("拼音过长（最多 200 个字符）".to_owned());
    }
    engine.set_use_jian_pin(use_jian_pin.unwrap_or(true));
    engine.set_input(input);
    engine
        .query()
        .map_err(|error| format!("拼音无法解析：{error}"))
}

/// 把查询结果转成切分返回结构。
pub fn build_segment_response(input: &str, query: &Query) -> SegmentResponse {
    SegmentResponse {
        input: input.to_owned(),
        preferred: preferred(query),
        segmentations: segmentations(&query.segmentations),
        tail: query.tail.clone(),
    }
}

/// 把查询结果转成候选返回结构。
pub fn build_lookup_response(input: &str, query: &Query, limit: Option<usize>) -> LookupResponse {
    let limit = limit.unwrap_or(20).clamp(1, 100);
    LookupResponse {
        input: input.to_owned(),
        preferred: preferred(query),
        segmentations: segmentations(&query.segmentations),
        candidates: query
            .candidates
            .items
            .iter()
            .take(limit)
            .map(|candidate| CandidateView {
                text: candidate.text.clone(),
                display: candidate.display_text().to_owned(),
                kind: format!("{:?}", candidate.kind),
                syllables: candidate.syllables.clone(),
                weight: query.weights.get(&candidate.text).copied(),
            })
            .collect(),
        tail: query.tail.clone(),
    }
}

fn preferred(query: &Query) -> Option<String> {
    query
        .segmentations
        .first()
        .map(|segmentation| segmentation.joined("'"))
}

fn segmentations(all: &[Segmentation]) -> Vec<SegmentationView> {
    all.iter()
        .take(16)
        .map(|segmentation| SegmentationView {
            joined: segmentation.joined("'"),
            syllables: segmentation
                .syllables
                .iter()
                .map(|syllable| SyllableView {
                    text: syllable.text.clone(),
                    complete: syllable.complete,
                })
                .collect(),
            letters: segmentation.letters(),
            incomplete: segmentation.incomplete_count(),
        })
        .collect()
}
