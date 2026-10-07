//! annotation 片段的深浅。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// 译文。
    Gloss,

    /// 需要强调的注解片段。
    Fresh,

    /// 词性与分隔符，最浅。
    Faint,

    /// 翻译 Tip 里除释义以外的字（词性、释义之间的分隔符、读音括号）：**暂时**统一一个颜色，主题环节再设计。
    TranslateMeta,

    /// 翻译 Tip 的释义：这个词条还没学会。
    TranslateFresh,

    /// 翻译 Tip 的释义：这个词条学会了。
    TranslateLearned,
}
