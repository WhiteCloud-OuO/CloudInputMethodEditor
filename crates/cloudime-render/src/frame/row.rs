/// 候选窗口的一行：序号、候选词、annotation 片段、右侧来源角标、紧跟候选词的辅助码。
use super::tone::Tone;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// 显示用序号文本，如 `1`。
    pub index: String,

    /// 候选词。
    pub text: String,

    /// 右侧 annotation，按顺序绘制；没有译文时为空。
    pub annotation: Vec<(String, Tone)>,

    /// 右侧来源角标（用户短语「短」/ 用户自造词「造」），没有为 `None`；右对齐到候选那一格的圆角矩形内。
    pub badge: Option<String>,

    /// 紧跟候选词的辅助码（`[XH]` 这样），与候选词**隔一个空格**；字体与颜色跟角标一致。
    /// 没有为 `None`（见 `docs/design/input-schemes.md` §4）。
    pub auxiliary: Option<String>,
}

impl Row {
    /// 只有序号和候选词的一行。
    pub fn plain(index: usize, text: impl Into<String>) -> Self {
        Self {
            index: (index + 1).to_string(),
            text: text.into(),
            annotation: Vec::new(),
            badge: None,
            auxiliary: None,
        }
    }
}
