//! 一套配色，取自调研期在 macOS 上实测的系统语义色 sRGB 值（label 0.847、secondaryLabel 0.498…）。
//!
//! 与主题文件（`Themes\*.json`）一一对应的是三个窗口那 21 项；其余（`gloss` / `pos` / `fresh` 这些
//! 注解色）暂时没进主题文件，仍是这里的缺省值。

use crate::color::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    // ── 候选窗口（主题文件里的 `candidate`） ──
    /// 窗口背景。
    pub background: Color,

    /// 当前候选的高亮条底色。
    pub highlight: Color,

    /// 窗口阴影。
    pub shadow: Color,

    /// 拼音串（候选窗口顶部那一行）。
    pub pinyin: Color,

    /// 拼音串里的光标。
    pub pinyin_caret: Color,

    /// 高亮的那条候选项文字。
    pub highlight_text: Color,

    /// 普通候选项文字。
    pub text: Color,

    /// 高亮的那条候选项的序号。
    pub highlight_index: Color,

    /// 普通候选项的序号。
    pub index: Color,

    /// 页码等页脚小字。
    pub footer: Color,

    /// 候选右侧来源角标（用户短语「短」/ 用户自造词「造」）。
    pub badge: Color,

    /// 翻译 Tip 里除释义以外的字（词性、分隔符、读音括号）。
    pub translate_meta: Color,

    /// 翻译 Tip 的释义：这个词条还没学会（缺省橙）。
    pub translate_fresh: Color,

    /// 翻译 Tip 的释义：这个词条学会了（缺省深灰）。
    pub translate_learned: Color,

    /// 候选项额外内容（如云翻译返回的文本、在线翻译那一行）。
    pub extra: Color,

    // ── 悬浮工具栏（主题文件里的 `bar`） ──
    /// 工具栏背景。
    pub bar_background: Color,

    /// 工具栏图标。
    pub bar_icon: Color,

    /// 工具栏阴影。
    pub bar_shadow: Color,

    // ── 状态切换提示（主题文件里的 `tip`） ──
    /// 提示窗背景。
    pub tip_background: Color,

    /// 提示窗图标。
    pub tip_icon: Color,

    /// 提示窗阴影。
    pub tip_shadow: Color,

    // ── 暂不进主题文件的注解色 ──
    /// 译文。
    pub gloss: Color,

    /// 词性，比译文更浅。
    pub pos: Color,

    /// 强调色：注解里需要更醒目的片段用它。
    pub fresh: Color,
}

impl Palette {
    pub const fn new() -> Self {
        Self {
            background: Color::rgb(255, 255, 255),
            highlight: Color::rgba(200, 241, 255, 240),
            shadow: Color::gray(0, 90),
            // 拼音串与序号用纯黑：它们原先的 alpha（127 / 66）在白底上太淡，用户看不清
            pinyin: Color::rgb(0, 0, 0),
            pinyin_caret: Color::gray(0, 216),
            highlight_text: Color::gray(0, 216),
            text: Color::gray(0, 216),
            highlight_index: Color::rgb(0, 0, 0),
            index: Color::rgb(0, 0, 0),
            footer: Color::rgb(0x88, 0x88, 0x88),
            badge: Color::rgb(0x88, 0x88, 0x88),
            translate_meta: Color::rgb(0x33, 0x33, 0x33),
            translate_fresh: Color::rgb(0xff, 0x7f, 0x27),
            translate_learned: Color::rgb(0x33, 0x33, 0x33),
            extra: Color::rgb(0x0f, 0x6c, 0xbd),
            bar_background: Color::rgb(255, 255, 255),
            bar_icon: Color::rgb(0x23, 0x1f, 0x20),
            bar_shadow: Color::gray(0, 90),
            tip_background: Color::rgb(255, 255, 255),
            tip_icon: Color::rgb(0x23, 0x1f, 0x20),
            tip_shadow: Color::gray(0, 90),
            gloss: Color::gray(0, 127),
            pos: Color::gray(0, 66),
            fresh: Color::rgb(255, 141, 40),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}
