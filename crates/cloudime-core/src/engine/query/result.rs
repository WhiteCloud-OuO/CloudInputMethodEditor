use crate::candidate::CandidateList;
use crate::correction::Correction;
use crate::parser::Segmentation;

/// 最优切分的音节用 `'` 连接，再接未切分尾部。
pub(crate) fn join_marked(segmentations: &[Segmentation], tail: &str) -> String {
    let mut text = segmentations
        .first()
        .map(|s| s.joined("'"))
        .unwrap_or_default();
    if !tail.is_empty() {
        if !text.is_empty() {
            text.push('\'');
        }
        text.push_str(tail);
    }
    text
}

/// 与 [`join_marked`] 相同的分段，但用原样大小写的输入（`Cpan`）：切分是按小写算的，
/// 大小写只影响显示，逐段按同样的字节长度取回原样文本。
pub(crate) fn join_marked_typed(typed: &str, segmentations: &[Segmentation], tail: &str) -> String {
    let mut text = String::new();
    let mut offset = 0;
    if let Some(first) = segmentations.first() {
        for (index, syllable) in first.syllables.iter().enumerate() {
            if index > 0 {
                text.push('\'');
            }
            let end = (offset + syllable.text.len()).min(typed.len());
            text.push_str(&typed[offset..end]);
            offset = end;
        }
    }
    if !tail.is_empty() {
        if !text.is_empty() {
            text.push('\'');
        }
        text.push_str(&typed[offset.min(typed.len())..]);
    }
    text
}

use crate::engine::timings::Timings;
use crate::engine::{MarkedKind, MarkedSegment};

/// 不带译文的候选查询结果。
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// 参与候选生成的所有切分，索引 0 为首选切分。
    pub segmentations: Vec<Segmentation>,

    /// 排好序的候选，`translation` 均为 `None`。
    pub candidates: CandidateList,

    /// 输入末尾无法切分的字母（如 `kaifv` 的 `v`），不参与本次候选，留给后续输入。
    pub tail: String,

    /// 查询时缓冲区里的原始文本。
    pub text: String,

    /// 查询时的光标位置（`text` 的字节下标）。
    pub cursor: usize,

    /// 光标停在中间时，作用域之后剩下的拼音的显示形式（已按音节用 `'` 连好）；候选不管它，只画出来。
    pub rest: String,

    /// 各阶段耗时。
    pub timings: Timings,

    /// 生效的拼写纠正：`segmentations` 与候选都来自纠正后的拼音，`text` 仍是用户敲的。
    pub correction: Option<Correction>,

    /// Shift 敲的大写字母的显示字串（`Cpan`）：匹配按小写算，显示仍按敲的样子。有此值时 preedit 优先显示它。
    pub typed_display: Option<String>,

    /// 组句里已选定的文本（`云朵`），显示在未选拼音之前，也是 `Typed` 段。
    /// 拆成单独字段而非 `text` 的一部分：`text` 仍是未选拼音，候选与光标只对它算。
    pub selected: String,
}

impl Query {
    /// 无法解析为拼音但精确匹配自定义短语时，保留原始输入和光标。
    pub(super) fn custom_only(text: &str, cursor: usize, scope: &str, rest: String) -> Self {
        Self {
            text: text.to_owned(),
            cursor,
            tail: scope.to_owned(),
            rest,
            ..Self::default()
        }
    }

    /// 给 marked text（应用输入框未上屏文本）用的显示形式：最优切分的音节用 `'` 连接，再接未切分尾部，
    /// 光标后的剩余拼音跟在最后。
    /// `kaifa` → `kai'fa`，`kf` → `k'f`，`ni|hao` → `ni'hao`。
    pub fn marked_text(&self) -> String {
        self.marked_segments()
            .iter()
            .map(|s| s.text.as_str())
            .collect()
    }

    /// [`Self::marked_text`] 的分段形式：已选文本一段（`Typed`，排在前面），敲的拼音一段（`Typed`），
    /// 光标后剩下的拼音连同前面的 `'` 一段（`Rest`）。壳按段画样式；[`Self::segments_cursor`] 的位置按各段拼接后的字符数算。
    ///
    /// 已选文本刻意也用 `Typed`：DLL 把非 `Corrected` 段拼起来当组句文本，这样应用里自然显示
    /// 「云朵shurufa」。以后想给已选文本单独样式再加 `MarkedKind` 变体并升 `PROTOCOL_VERSION`。
    pub fn marked_segments(&self) -> Vec<MarkedSegment> {
        let mut segments = Vec::with_capacity(3);
        if !self.selected.is_empty() {
            segments.push(MarkedSegment::new(self.selected.clone(), MarkedKind::Typed));
        }
        match &self.correction {
            Some(correction) => segments.extend(correction.marked_segments()),
            None => {
                let typed = if let Some(display) = &self.typed_display {
                    display.clone()
                } else {
                    join_marked(&self.segmentations, &self.tail)
                };
                if !typed.is_empty() {
                    segments.push(MarkedSegment::new(typed, MarkedKind::Typed));
                }
            }
        }
        if !self.rest.is_empty() {
            let rest = if segments.is_empty() {
                self.rest.clone()
            } else {
                format!("'{}", self.rest)
            };
            segments.push(MarkedSegment::new(rest, MarkedKind::Rest));
        }
        segments
    }

    /// 光标在 [`Self::marked_segments`] 拼接文本里的字符下标（候选窗口顶部拼音行使用）。
    pub fn segments_cursor(&self) -> usize {
        let selected = self.selected.chars().count();
        // 纠错生效时显示串与敲的不一样长，作用域又总在光标前：光标就在敲的部分末尾
        // （光标在开头时作用域是整段，光标仍在开头）
        if self.correction.is_some() {
            return self
                .marked_segments()
                .iter()
                .filter(|s| s.kind != MarkedKind::Rest)
                .map(|s| s.text.chars().count())
                .sum();
        }
        let letters_before = self.text[..self.cursor.min(self.text.len())]
            .chars()
            .filter(|c| *c != '\'')
            .count();
        let after_apostrophe = self.text[..self.cursor.min(self.text.len())].ends_with('\'');
        // 只在未选拼音那几段里数光标，跳过排在最前面的已选文本。
        let segments = self.marked_segments();
        let skip = usize::from(!self.selected.is_empty());
        let segments_text: String = segments
            .iter()
            .skip(skip)
            .map(|s| s.text.as_str())
            .collect();
        let chars: Vec<char> = segments_text.chars().collect();
        let mut seen = 0;
        let mut position = 0;
        while position < chars.len() && seen < letters_before {
            if chars[position] != '\'' {
                seen += 1;
            }
            position += 1;
        }
        if after_apostrophe && chars.get(position) == Some(&'\'') {
            position += 1;
        }
        selected + position
    }

    /// 光标在 [`Self::marked_text`] 里的字符下标（给平台层传给宿主应用输入框用的，所以按字符算，不是字节）。
    pub fn marked_cursor(&self) -> usize {
        self.segments_cursor()
    }
}
