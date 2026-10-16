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
    pub auxiliary: Option<Auxiliary>,
}

/// 候选右侧那一段辅助码：`typed` 是**已经敲上**的那几位（画下划线），`rest` 是还差的那几位。
/// 渲染器在两头补上 `[` `]`（不参与下划线）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Auxiliary {
    /// 已经敲上的那几位（前缀）；一位都没敲就是空串。
    pub typed: String,

    /// 还差的那些。
    pub rest: String,
}

impl Auxiliary {
    /// 按「完整的码 + 已经敲上的那几位」拆：`typed` 不是 `code` 的前缀时（理论上不该发生）
    /// 整条都算没敲，画出来是 `[XH]`。
    pub fn split(code: &str, typed: &str) -> Self {
        match code.get(..typed.len()) {
            Some(head) if head.eq_ignore_ascii_case(typed) => Self {
                typed: head.to_owned(),
                rest: code[head.len()..].to_owned(),
            },
            _ => Self {
                typed: String::new(),
                rest: code.to_owned(),
            },
        }
    }
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
