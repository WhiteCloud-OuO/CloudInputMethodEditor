//! annotation 片段的深浅。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// 译文。
    Gloss,

    /// 需要强调的注解片段。
    Fresh,

    /// 词性与分隔符，最浅。
    Faint,
}
