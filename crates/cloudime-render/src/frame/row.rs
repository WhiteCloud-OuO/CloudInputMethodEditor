/// 候选窗口的一行：序号、候选词、annotation 片段、右侧来源角标。
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
}

impl Row {
    /// 只有序号和候选词的一行。
    pub fn plain(index: usize, text: impl Into<String>) -> Self {
        Self {
            index: (index + 1).to_string(),
            text: text.into(),
            annotation: Vec::new(),
            badge: None,
        }
    }
}
