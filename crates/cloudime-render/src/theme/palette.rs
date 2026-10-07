//! 一套配色，取自调研期在 macOS 上实测的系统语义色 sRGB 值（label 0.847、secondaryLabel 0.498…）。

use crate::color::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// 候选词。
    pub text: Color,

    /// 拼音串（候选窗口顶部那一行）。
    pub pinyin: Color,

    /// 译文。
    pub gloss: Color,

    /// 词性，比译文更浅。
    pub pos: Color,

    /// 强调色：注解里需要更醒目的片段用它。
    pub fresh: Color,

    /// 序号。
    pub index: Color,

    /// 页码等页脚小字，比序号更弱。
    pub footer: Color,

    /// 候选右侧来源角标（用户短语「短」/ 用户自造词「造」）。页码也用这个颜色。
    pub badge: Color,

    /// 翻译 Tip 里除释义以外的字（词性、分隔符、读音括号）：暂时统一 `#333333`，主题环节再一起设计。
    pub translate_meta: Color,

    /// 翻译 Tip 的释义：这个词条还没学会（缺省橙）。
    pub translate_fresh: Color,

    /// 翻译 Tip 的释义：这个词条学会了（缺省深灰）。
    pub translate_learned: Color,

    /// 强调色：注解里需要更醒目的片段用它。
    pub accent: Color,

    /// 窗口背景。
    pub background: Color,

    /// 当前候选的高亮底色。
    pub highlight: Color,
}

impl Palette {
    pub const fn new() -> Self {
        Self {
            text: Color::gray(0, 216),
            // 拼音串与序号用纯黑：它们原先的 alpha（127 / 66）在白底上太淡，用户看不清
            pinyin: Color::rgb(0, 0, 0),
            gloss: Color::gray(0, 127),
            pos: Color::gray(0, 66),
            fresh: Color::rgb(255, 141, 40),
            index: Color::rgb(0, 0, 0),
            footer: Color::gray(0, 66),
            badge: Color::rgb(0x88, 0x88, 0x88),
            translate_meta: Color::rgb(0x33, 0x33, 0x33),
            translate_fresh: Color::rgb(0xff, 0x7f, 0x27),
            translate_learned: Color::rgb(0x33, 0x33, 0x33),
            accent: Color::rgb(0, 195, 208),
            background: Color::rgb(255, 255, 255),
            highlight: Color::rgba(200, 241, 255, 240),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}
