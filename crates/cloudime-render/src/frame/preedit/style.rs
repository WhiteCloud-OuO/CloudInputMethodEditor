//! preedit 片段的画法。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreeditStyle {
    /// 敲的拼音。
    Typed,

    /// 光标后剩下的拼音。
    Rest,

    /// 被纠错改掉的字母：带删除线。
    Struck,
}
