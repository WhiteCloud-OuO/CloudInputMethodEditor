//! preedit 片段的画法。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreeditStyle {
    /// 敲的拼音。
    Typed,

    /// 不参与当前音节的剩余拼音（特殊输入模式）或光标后剩下的拼音：画角标色。
    Rest,

    /// 被纠错改掉的字母：带删除线。
    Struck,
}
