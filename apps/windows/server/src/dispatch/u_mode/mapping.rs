//! u 模式六个分类的符号表。
//!
//! 键盘位置固定为四排（见 [`KEY_ROWS`]）；每个分类按「键 → 正常 / Shift 符号」查表。
//! 当前表先留空，等维护者给出正式映射后直接往对应数组里填，不用改按键流程。

/// u 模式的六个候选分类，顺序就是候选窗口里从上到下的顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SymbolCategory {
    /// ① 常用符号
    Common,
    /// ② 序号 | 角标
    Number,
    /// ③ 希腊字母
    Greek,
    /// ④ 西里尔字母
    Cyrillic,
    /// ⑤ 日文假名
    Kana,
    /// ⑥ 制表符 | 箭头
    TabArrow,
}

impl SymbolCategory {
    /// 候选窗口里的固定顺序。
    pub(super) const ALL: [Self; 6] = [
        Self::Common,
        Self::Number,
        Self::Greek,
        Self::Cyrillic,
        Self::Kana,
        Self::TabArrow,
    ];

    /// 分类在候选窗口里的显示名（带圈序号）。
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Common => "① 常用符号",
            Self::Number => "② 序号|角标",
            Self::Greek => "③ 希腊字母",
            Self::Cyrillic => "④ 西里尔字母",
            Self::Kana => "⑤ 日文假名",
            Self::TabArrow => "⑥ 制表符|箭头",
        }
    }
}

/// 日文假名的两套映射。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Kana {
    /// 平假名
    #[default]
    Hiragana,
    /// 片假名
    Katakana,
}

impl Kana {
    /// 按 Tab 切换平假名 / 片假名。
    pub(super) fn toggle(self) -> Self {
        match self {
            Self::Hiragana => Self::Katakana,
            Self::Katakana => Self::Hiragana,
        }
    }
}

/// 四排键盘，顺序与 u 模式的符号输入窗口一致。
pub(super) const KEY_ROWS: [&str; 4] =
    ["1234567890-=", "QWERTYUIOP[]", "ASDFGHJKL;'", "ZXCVBNM,./"];

/// 一个键在某个分类里的正常 / Shift 符号。
#[derive(Debug, Clone, Copy)]
pub(super) struct SymbolEntry {
    /// 键帽（与 [`KEY_ROWS`] 里的字符一致，字母用大写）。
    pub(super) key: &'static str,
    /// 正常按下时上屏的符号。
    pub(super) normal: &'static str,
    /// Shift + 键上屏的符号；空 = 与正常相同。
    pub(super) shifted: &'static str,
}

/// 查这个分类当前这套映射。
///
/// `variant` 是「制表符|箭头」用 Tab 循环的套数；别的分类忽略。
pub(super) fn entries(
    category: SymbolCategory,
    kana: Kana,
    variant: usize,
) -> &'static [SymbolEntry] {
    match category {
        SymbolCategory::Common => COMMON,
        SymbolCategory::Number => NUMBER,
        SymbolCategory::Greek => GREEK,
        SymbolCategory::Cyrillic => CYRILLIC,
        SymbolCategory::Kana => match kana {
            Kana::Hiragana => HIRAGANA,
            Kana::Katakana => KATAKANA,
        },
        SymbolCategory::TabArrow => match variant % 2 {
            0 => TAB_ARROW,
            _ => TAB_ARROW_ALT,
        },
    }
}

/// 查一个键帽的符号；`shift` 时优先取 `shifted`，表里没写就退回正常符号。
pub(super) fn symbol(
    category: SymbolCategory,
    kana: Kana,
    variant: usize,
    key: &str,
    shift: bool,
) -> Option<&'static str> {
    let entry = entries(category, kana, variant)
        .iter()
        .find(|entry| entry.key.eq_ignore_ascii_case(key))?;
    match (shift, entry.shifted.is_empty()) {
        (true, false) => Some(entry.shifted),
        _ => (!entry.normal.is_empty()).then_some(entry.normal),
    }
}

// 正式映射待补。数组里的 `key` 用 KEY_ROWS 的字符，`normal` / `shifted` 直接写要上屏的符号；
// 同一个键在正常与 Shift 下相同就把 `shifted` 留空。映射没填时按键仍会被吞掉，不会漏进应用。

const COMMON: &[SymbolEntry] = &[];

const NUMBER: &[SymbolEntry] = &[];

const GREEK: &[SymbolEntry] = &[];

const CYRILLIC: &[SymbolEntry] = &[];

const HIRAGANA: &[SymbolEntry] = &[];

const KATAKANA: &[SymbolEntry] = &[];

const TAB_ARROW: &[SymbolEntry] = &[];

const TAB_ARROW_ALT: &[SymbolEntry] = &[];
