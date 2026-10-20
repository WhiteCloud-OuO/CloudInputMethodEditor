//! u 模式六个分类的符号表。
//!
//! 键盘位置固定为四排（见 [`KEY_ROWS`]）；每个分类按「键 → 正常 / Shift 符号」查表。
//! 用户给的映射里小写键是正常按下、大写键是 Shift + 键；这里统一收进
//! [`SymbolEntry::normal`] / [`SymbolEntry::shifted`]。

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

    /// 分类在候选窗口里的显示名。
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Common => "常用符号",
            Self::Number => "序号|角标",
            Self::Greek => "希腊字母",
            Self::Cyrillic => "西里尔字母",
            Self::Kana => "日文假名",
            Self::TabArrow => "制表符|箭头",
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

/// 希腊字母的重音 / 隔音状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct GreekState {
    /// 重音输入状态（`;` 进入 / 退出）。
    pub(super) accent: bool,
    /// 隔音输入状态（`'` 进入 / 退出）。
    pub(super) diaeresis: bool,
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
    /// Shift + 键上屏的符号；空 = 这个键没有 Shift 映射。
    pub(super) shifted: &'static str,
}

const fn entry(key: &'static str, normal: &'static str, shifted: &'static str) -> SymbolEntry {
    SymbolEntry {
        key,
        normal,
        shifted,
    }
}

/// 查这个分类当前这套映射。
///
/// `variant` 是「制表符|箭头」用 Tab 循环的套数；`greek` 是希腊字母的重音 / 隔音状态。
pub(super) fn entries(
    category: SymbolCategory,
    kana: Kana,
    variant: usize,
    greek: GreekState,
) -> &'static [SymbolEntry] {
    match category {
        SymbolCategory::Common => COMMON,
        SymbolCategory::Number => NUMBER,
        SymbolCategory::Greek => match (greek.accent, greek.diaeresis) {
            (false, false) => GREEK,
            (true, false) => GREEK_ACCENT,
            (false, true) => GREEK_DIAERESIS,
            (true, true) => GREEK_BOTH,
        },
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

/// 查一个键帽的符号。Shift 时只认 `shifted`；没有 Shift 映射就返回 `None`。
pub(super) fn symbol(
    category: SymbolCategory,
    kana: Kana,
    variant: usize,
    greek: GreekState,
    key: &str,
    shift: bool,
) -> Option<&'static str> {
    let entry = entries(category, kana, variant, greek)
        .iter()
        .find(|entry| entry.key.eq_ignore_ascii_case(key))?;
    if shift {
        (!entry.shifted.is_empty()).then_some(entry.shifted)
    } else {
        (!entry.normal.is_empty()).then_some(entry.normal)
    }
}

const COMMON: &[SymbolEntry] = &[
    entry("1", "〈", ""),
    entry("2", "〉", ""),
    entry("3", "〔", ""),
    entry("4", "〕", ""),
    entry("5", "「", ""),
    entry("6", "」", ""),
    entry("7", "『", ""),
    entry("8", "』", ""),
    entry("9", "〖", ""),
    entry("0", "〗", ""),
    entry("-", "¦", ""),
    entry("=", "‖", ""),
    entry("Q", "◰", ""),
    entry("W", "◳", ""),
    entry("E", "◴", ""),
    entry("R", "◷", ""),
    entry("T", "◇", ""),
    entry("Y", "○", ""),
    entry("U", "□", ""),
    entry("I", "△", ""),
    entry("O", "☆", ""),
    entry("P", "⊙", ""),
    entry("[", "╱", ""),
    entry("]", "╲", ""),
    entry("A", "◱", ""),
    entry("S", "◲", ""),
    entry("D", "◵", ""),
    entry("F", "◶", ""),
    entry("G", "◆", ""),
    entry("H", "●", ""),
    entry("J", "■", ""),
    entry("K", "▲", ""),
    entry("L", "★", ""),
    entry(";", "♀", ""),
    entry("'", "♂", ""),
    entry("Z", "♦", ""),
    entry("X", "♠", ""),
    entry("C", "♣", ""),
    entry("V", "♥", ""),
    entry("B", "±", ""),
    entry("N", "≠", ""),
    entry("M", "≈", ""),
    entry(",", "≤", ""),
    entry(".", "≥", ""),
    entry("/", "¤", ""),
];

const NUMBER: &[SymbolEntry] = &[
    entry("1", "①", ""),
    entry("2", "②", ""),
    entry("3", "③", ""),
    entry("4", "④", ""),
    entry("5", "⑤", ""),
    entry("6", "⑥", ""),
    entry("7", "⑦", ""),
    entry("8", "⑧", ""),
    entry("9", "⑨", ""),
    entry("0", "⑩", ""),
    entry("Q", "⑴", ""),
    entry("W", "⑵", ""),
    entry("E", "⑶", ""),
    entry("R", "⑷", ""),
    entry("T", "⑸", ""),
    entry("Y", "⑹", ""),
    entry("U", "⑺", ""),
    entry("I", "⑻", ""),
    entry("O", "⑼", ""),
    entry("P", "⑽", ""),
    entry("A", "¹", ""),
    entry("S", "²", ""),
    entry("D", "³", ""),
    entry("F", "⁴", ""),
    entry("G", "⁵", ""),
    entry("H", "⁶", ""),
    entry("J", "⁷", ""),
    entry("K", "⁸", ""),
    entry("L", "⁹", ""),
    entry(";", "⁰", ""),
    entry("Z", "₁", ""),
    entry("X", "₂", ""),
    entry("C", "₃", ""),
    entry("V", "₄", ""),
    entry("B", "₅", ""),
    entry("N", "₆", ""),
    entry("M", "₇", ""),
    entry(",", "₈", ""),
    entry(".", "₉", ""),
    entry("/", "₀", ""),
    entry("-", "₋", "⁻"),
    entry("=", "₊", "⁺"),
    entry("[", "₍", "⁽"),
    entry("]", "₎", "⁾"),
];

const GREEK: &[SymbolEntry] = &[
    entry("W", "ς", ""),
    entry("E", "ε", "Ε"),
    entry("R", "ρ", "Ρ"),
    entry("T", "τ", "Τ"),
    entry("Y", "υ", "Υ"),
    entry("U", "θ", "Θ"),
    entry("I", "ι", "Ι"),
    entry("O", "ο", "Ο"),
    entry("P", "π", "Π"),
    entry("A", "α", "Α"),
    entry("S", "σ", "Σ"),
    entry("D", "δ", "Δ"),
    entry("F", "φ", "Φ"),
    entry("G", "γ", "Υ"),
    entry("H", "η", "Η"),
    entry("J", "ξ", "Ξ"),
    entry("K", "κ", "Κ"),
    entry("L", "λ", "Λ"),
    entry("Z", "ζ", "Ζ"),
    entry("X", "χ", "Χ"),
    entry("C", "ψ", "Ψ"),
    entry("V", "ω", "Ω"),
    entry("B", "β", "Β"),
    entry("N", "ν", "Ν"),
    entry("M", "μ", "Μ"),
    entry(";", "【重音】", ""),
    entry("'", "【隔音】", ""),
];

const GREEK_ACCENT: &[SymbolEntry] = &[
    entry("E", "έ", "Έ"),
    entry("Y", "ύ", "Ύ"),
    entry("I", "ί", "Ί"),
    entry("O", "ό", "Ό"),
    entry("A", "ά", "Ά"),
    entry("H", "ή", "Ή"),
    entry("V", "ώ", "Ώ"),
    entry(";", "【重音】", ""),
    entry("'", "【隔音】", ""),
];

const GREEK_DIAERESIS: &[SymbolEntry] = &[
    entry("I", "ϊ", "Ϊ"),
    entry("Y", "ϋ", "Ϋ"),
    entry(";", "【重音】", ""),
    entry("'", "【隔音】", ""),
];

const GREEK_BOTH: &[SymbolEntry] = &[
    entry("I", "ΐ", ""),
    entry("Y", "ΰ", ""),
    entry(";", "【重音】", ""),
    entry("'", "【隔音】", ""),
];

const CYRILLIC: &[SymbolEntry] = &[
    entry("Q", "й", "Й"),
    entry("W", "ц", "Ц"),
    entry("E", "у", "У"),
    entry("R", "к", "К"),
    entry("T", "е", "Е"),
    entry("Y", "н", "Н"),
    entry("U", "г", "Г"),
    entry("I", "ш", "Ш"),
    entry("O", "щ", "Щ"),
    entry("P", "з", "З"),
    entry("[", "х", "Х"),
    entry("]", "ъ", "Ъ"),
    entry("A", "ф", "Ф"),
    entry("S", "ы", "Ы"),
    entry("D", "в", "В"),
    entry("F", "а", "А"),
    entry("G", "п", "П"),
    entry("H", "р", "Р"),
    entry("J", "о", "О"),
    entry("K", "л", "Л"),
    entry("L", "д", "Д"),
    entry(";", "ж", "Ж"),
    entry("'", "э", "Э"),
    entry("Z", "я", "Я"),
    entry("X", "ч", "Ч"),
    entry("C", "с", "С"),
    entry("V", "м", "М"),
    entry("B", "и", "И"),
    entry("N", "т", "Т"),
    entry("M", "ь", "Ь"),
    entry(",", "б", "Б"),
    entry(".", "ю", "Ю"),
    entry("/", "ё", "Ё"),
];

const HIRAGANA: &[SymbolEntry] = &[
    entry("Q", "あ", "ぁ"),
    entry("W", "い", "ぃ"),
    entry("E", "う", "ぅ"),
    entry("R", "え", "ぇ"),
    entry("T", "お", "ぉ"),
    entry("Y", "た", "ま"),
    entry("U", "ち", "み"),
    entry("I", "つ", "む"),
    entry("O", "て", "め"),
    entry("P", "と", "も"),
    entry("A", "か", "ら"),
    entry("S", "き", "り"),
    entry("D", "く", "る"),
    entry("F", "け", "れ"),
    entry("G", "こ", "ろ"),
    entry("H", "な", "や"),
    entry("J", "に", "ゆ"),
    entry("K", "ぬ", "よ"),
    entry("L", "ね", "わ"),
    entry(";", "の", "を"),
    entry("'", "゛", "゜"),
    entry("Z", "さ", "ゃ"),
    entry("X", "し", "ゅ"),
    entry("C", "す", "ょ"),
    entry("V", "せ", "っ"),
    entry("B", "そ", "ゎ"),
    entry("N", "は", "ゐ"),
    entry("M", "ひ", "ゑ"),
    entry(",", "ふ", ""),
    entry(".", "へ", ""),
    entry("/", "ほ", ""),
    entry("[", "ん", ""),
    entry("-", "ー", ""),
];

const KATAKANA: &[SymbolEntry] = &[
    entry("Q", "ア", "ァ"),
    entry("W", "イ", "ィ"),
    entry("E", "ウ", "ゥ"),
    entry("R", "エ", "ェ"),
    entry("T", "オ", "ォ"),
    entry("Y", "タ", "マ"),
    entry("U", "チ", "ミ"),
    entry("I", "ツ", "ム"),
    entry("O", "テ", "メ"),
    entry("P", "ト", "モ"),
    entry("A", "カ", "ラ"),
    entry("S", "キ", "リ"),
    entry("D", "ク", "ル"),
    entry("F", "ケ", "レ"),
    entry("G", "コ", "ロ"),
    entry("H", "ナ", "ヤ"),
    entry("J", "ニ", "ユ"),
    entry("K", "ヌ", "ヨ"),
    entry("L", "ネ", "ワ"),
    entry(";", "ノ", "ヲ"),
    entry("'", "゛", "゜"),
    entry("Z", "サ", "ャ"),
    entry("X", "シ", "ュ"),
    entry("C", "ス", "ョ"),
    entry("V", "セ", "ッ"),
    entry("B", "ソ", "ヮ"),
    entry("N", "ハ", "ヰ"),
    entry("M", "ヒ", "ヱ"),
    entry(",", "フ", ""),
    entry(".", "ヘ", ""),
    entry("/", "ホ", ""),
    entry("[", "ン", ""),
    entry("-", "ー", ""),
];

const TAB_ARROW: &[SymbolEntry] = &[
    entry("Q", "┌", "╔"),
    entry("W", "┬", "╦"),
    entry("E", "┐", "╗"),
    entry("A", "├", "╠"),
    entry("S", "┼", "╬"),
    entry("D", "┤", "╣"),
    entry("Z", "└", "╚"),
    entry("X", "┴", "╩"),
    entry("C", "┘", "╝"),
    entry("T", "┏", ""),
    entry("Y", "┳", ""),
    entry("U", "┓", ""),
    entry("G", "┣", ""),
    entry("H", "╋", ""),
    entry("J", "┫", ""),
    entry("B", "┗", ""),
    entry("N", "┻", ""),
    entry("M", "┛", ""),
    entry("O", "╭", ""),
    entry("P", "╮", ""),
    entry("L", "╰", ""),
    entry(";", "╯", ""),
    entry(".", "┉", ""),
    entry("/", "┊", ""),
];

const TAB_ARROW_ALT: &[SymbolEntry] = &[
    entry("Q", "↖", "⇖"),
    entry("W", "↑", "⇑"),
    entry("E", "↗", "⇗"),
    entry("A", "←", "⇐"),
    entry("D", "→", "⇒"),
    entry("Z", "↙", "⇙"),
    entry("X", "↓", "⇓"),
    entry("C", "↘", "⇘"),
    entry("T", "⤣", "⇍"),
    entry("Y", "⇈", "⇧"),
    entry("U", "⤤", "⇏"),
    entry("G", "⇇", "⇦"),
    entry("J", "⇉", "⇨"),
    entry("B", "⤦", "↚"),
    entry("N", "⇊", "⇩"),
    entry("M", "⤥", "↛"),
    entry("O", "↔", "⇎"),
    entry("P", "↕", "⇔"),
    entry("L", "⇆", "⇋"),
    entry(";", "⇄", "⇌"),
    entry(".", "⇵", "«"),
    entry("/", "⇅", "»"),
];
