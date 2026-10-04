//! 中文模式下的标点与符号映射。
//!
//! 组句期间 `-` `=` 是翻页键（由壳先处理）；这里只回答「这个字符该变成什么」：
//! 先是配置里的符号映射（`[input] punctuation_marks_mapping`），再是内置的全角标点表（`，。？！……` 与引号配对）。
//!
//! 两者都受「数字后标点保持半角」约束（配置 `[input] use_half_wide_punctuation_marks_after_digital`，
//! 缺省开）：上一个输入是数字时一律不转，`23:06`、`2.36`、`25+9` 里的标点才留得住半角。
//!
//! 符号映射里「先敲 A 再敲 B」的规则（`~=` → `≈`）要撤掉 A 那一键已经上屏的输出：`symbol` 把要撤掉的字符数
//! 一并返回，壳转成协议里的 `delete_before` 让 DLL 删掉。

/// 配置里的「中文模式下符号映射」。
///
/// 键的写法：单个字符就是那个键（`"/" = "、"`）；`{kp}` 前缀表示小键盘（`"{kp}*" = "×"`）；
/// 两个字符表示先敲前一个再敲后一个（`"~=" = "≈"`）。看不懂的条目直接丢掉。
///
/// 两键写法 Core 里仍然支持（CLI / 测试直接建表时能用），但**配置侧不再产生它**：
/// `[input] punctuation_marks_mapping` 现在是一个只含单键那 5 项的位图（见 `PUNCTUATION_MAPPING_BITS`）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Mapping {
    /// 单键：按键字符、是否小键盘、上屏文本。
    single: Vec<(char, bool, String)>,

    /// 两键：前一个按键、本按键、上屏文本。
    pair: Vec<(char, char, String)>,
}

/// 小键盘条目的前缀。
const KEYPAD_PREFIX: &str = "{kp}";

impl Mapping {
    /// 按配置里的写法建表。
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut mapping = Self::default();
        for (notation, text) in pairs {
            let notation = notation.trim();
            let keypad = notation.starts_with(KEYPAD_PREFIX);
            let rest = notation.strip_prefix(KEYPAD_PREFIX).unwrap_or(notation);
            let mut chars = rest.chars();
            match (chars.next(), chars.next(), chars.next()) {
                (Some(key), None, _) => mapping.single.push((key, keypad, text.to_owned())),
                // 两键规则只在主键区有意义：小键盘上一个一个按算不上「先敲 A 再敲 B」
                (Some(first), Some(second), None) if !keypad => {
                    mapping.pair.push((first, second, text.to_owned()));
                }
                _ => {}
            }
        }
        mapping
    }

    /// 一条规则都没有。
    pub fn is_empty(&self) -> bool {
        self.single.is_empty() && self.pair.is_empty()
    }

    /// 单键规则。
    fn single(&self, key: char, keypad: bool) -> Option<&str> {
        self.single
            .iter()
            .find(|(key_, kp, _)| *key_ == key && *kp == keypad)
            .map(|(.., text)| text.as_str())
    }

    /// 两键规则：先敲 `first` 再敲 `key`。
    fn pair(&self, first: char, key: char) -> Option<&str> {
        self.pair
            .iter()
            .find(|(a, b, _)| *a == first && *b == key)
            .map(|(.., text)| text.as_str())
    }
}

/// 符号映射命中的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedSymbol {
    /// 上屏的文本。
    pub text: String,

    /// 先撤掉光标前这么多个字符：两键规则的第一个键已经上屏了，换掉才看得见 `≈`。
    pub delete_before: u16,
}

/// 引号成对切换与「上一个按键」的状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Punctuation {
    /// 下一个 `"` 是左引号。
    double_quote_open: bool,

    /// 下一个 `'` 是左引号。
    single_quote_open: bool,

    /// 上一个输入的字符（敲的那个，不是转换后的）：数字后的点靠它。
    last: Option<char>,

    /// 上一个按键真正上屏的东西：`(敲的字符, 上了几个字符)`。两键规则要撤掉它，
    /// 所以只在「上一键原样上屏或转换上屏」时有效，上屏过词（`note_committed`）就作废。
    previous_key: Option<(char, usize)>,

    /// 配置的符号映射（中文模式）。
    mapping: Mapping,

    /// 数字后标点保持半角（缺省开，与配置的缺省一致）。
    half_after_digit: bool,
}

impl Default for Punctuation {
    fn default() -> Self {
        Self {
            double_quote_open: false,
            single_quote_open: false,
            last: None,
            previous_key: None,
            mapping: Mapping::default(),
            half_after_digit: true,
        }
    }
}

impl Punctuation {
    /// 换配置里的符号映射。
    pub fn set_mapping(&mut self, mapping: Mapping) {
        self.mapping = mapping;
    }

    /// 数字后标点是否保持半角（`[input] use_half_wide_punctuation_marks_after_digital`）。
    pub fn set_half_after_digit(&mut self, on: bool) {
        self.half_after_digit = on;
    }

    /// 配置的符号映射：`keypad` 的键只认 `{kp}` 条目，两键规则看上一个按键。
    /// 不需要转换返回 `None`，壳把原字符交给应用。
    pub fn symbol(&mut self, c: char, keypad: bool) -> Option<MappedSymbol> {
        let after_digit = self.after_digit();
        // 上一个按键的上下文用完即弃：中间隔了别的键就不再凑成两键规则
        let previous = self.previous_key.take();
        if after_digit {
            return None;
        }
        let (text, delete_before) = match previous.and_then(|(first, chars)| {
            self.mapping
                .pair(first, c)
                .map(|text| (text.to_owned(), chars))
        }) {
            Some((text, chars)) => (text, u16::try_from(chars).unwrap_or(u16::MAX)),
            None => match self.mapping.single(c, keypad) {
                Some(text) => (text.to_owned(), 0),
                None => return None,
            },
        };
        self.last = Some(c);
        self.previous_key = Some((c, text.chars().count()));
        Some(MappedSymbol {
            text,
            delete_before,
        })
    }

    /// 内置的全角标点表：`c` 对应的全角标点；不需要转换的返回 `None`，壳把原字符交给应用。
    pub fn convert(&mut self, c: char) -> Option<String> {
        if self.after_digit() {
            return None;
        }
        let converted = match c {
            ',' => "，",
            '.' => "。",
            '?' => "？",
            '!' => "！",
            ':' => "：",
            ';' => "；",
            '(' => "（",
            ')' => "）",
            '[' => "【",
            ']' => "】",
            '<' => "《",
            '>' => "》",
            '\\' => "、",
            '^' => "……",
            '_' => "——",
            '$' => "￥",
            '~' => "～",
            '"' => {
                self.double_quote_open = !self.double_quote_open;
                if self.double_quote_open { "“" } else { "”" }
            }
            '\'' => {
                self.single_quote_open = !self.single_quote_open;
                if self.single_quote_open { "‘" } else { "’" }
            }
            _ => return None,
        };
        let text = converted.to_owned();
        self.last = Some(c);
        self.previous_key = Some((c, text.chars().count()));
        Some(text)
    }

    /// 壳把没有转换的字符原样交给应用后调用。
    pub fn note_passthrough(&mut self, c: char) {
        self.last = Some(c);
        self.previous_key = Some((c, 1));
    }

    /// 有文本上屏后调用（候选、拼音、英文词）。
    pub fn note_committed(&mut self, text: &str) {
        self.last = text.chars().last();
        // 上屏了词，两键规则的前一键就无从谈起了
        self.previous_key = None;
    }

    /// 上一个输入是数字，且配着「数字后标点保持半角」。
    fn after_digit(&self) -> bool {
        self.half_after_digit && self.last.is_some_and(|last| last.is_ascii_digit())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(pairs: [(&str, &str); 4]) -> Mapping {
        Mapping::from_pairs(pairs)
    }

    #[test]
    fn converts_common_marks_and_toggles_quotes() {
        let mut p = Punctuation::default();
        assert_eq!(p.convert(',').as_deref(), Some("，"));
        assert_eq!(p.convert('"').as_deref(), Some("“"));
        assert_eq!(p.convert('"').as_deref(), Some("”"));
        assert_eq!(p.convert('\'').as_deref(), Some("‘"));
        assert_eq!(p.convert('\'').as_deref(), Some("’"));
        assert_eq!(p.convert('a'), None);
        assert_eq!(p.convert('-'), None);
    }

    #[test]
    fn period_after_digit_stays_ascii() {
        let mut p = Punctuation::default();
        p.note_passthrough('3');
        assert_eq!(p.convert('.'), None);
        // 壳把上一个键原样交给应用（`note_passthrough`）之后，数字的状态就过去了
        p.note_passthrough('.');
        assert_eq!(p.convert('.').as_deref(), Some("。"));
        p.note_committed("第1");
        assert_eq!(p.convert('.'), None);
        p.note_committed("开发");
        assert_eq!(p.convert('.').as_deref(), Some("。"));
    }

    #[test]
    fn every_mark_after_a_digit_stays_half_width() {
        let mut p = Punctuation::default();
        p.note_passthrough('3');
        assert_eq!(p.convert(':'), None);
        p.note_passthrough(':');
        // 只压住紧跟数字的那一个：再下一个就恢复转换
        assert_eq!(p.convert(',').as_deref(), Some("，"));
        p.note_passthrough('3');
        assert_eq!(p.convert(','), None);
        // 关掉之后恢复逐字符转换
        p.set_half_after_digit(false);
        p.note_passthrough('3');
        assert_eq!(p.convert(':').as_deref(), Some("："));
    }

    #[test]
    fn configured_mapping_adds_keys_and_keypad() {
        let mut p = Punctuation::default();
        p.set_mapping(mapping([
            ("/", "、"),
            ("~", "～"),
            ("{kp}*", "×"),
            ("{kp}/", "÷"),
        ]));
        assert_eq!(p.symbol('/', false).map(|m| m.text), Some("、".to_owned()));
        // 主键区的条目不管小键盘：小键盘的 / 走自己的规则
        assert_eq!(p.symbol('/', true).map(|m| m.text), Some("÷".to_owned()));
        assert_eq!(p.symbol('*', true).map(|m| m.text), Some("×".to_owned()));
        assert_eq!(p.symbol('*', false), None);
        // 内置表不受影响（映射里没有的键照旧转换）
        assert_eq!(p.convert(',').as_deref(), Some("，"));
    }

    #[test]
    fn two_key_rule_replaces_the_previous_key() {
        let mut p = Punctuation::default();
        // `~` 由内置表转成 `～`（上屏 1 个字），`=` 再敲时把它换成 ≈
        p.set_mapping(Mapping::from_pairs([("~=", "≈"), ("!=", "≠")]));
        assert_eq!(p.convert('~').as_deref(), Some("～"));
        let mapped = p.symbol('=', false).expect("两键规则命中");
        assert_eq!((mapped.text.as_str(), mapped.delete_before), ("≈", 1));
        // 中间隔了别的键就不再命中：上一键的上下文用完即弃
        p.note_passthrough('!');
        p.note_passthrough('a');
        assert_eq!(p.symbol('=', false), None);
        // 上屏过词之后也不命中（那一键的输出不该被当垫背的删掉）
        p.note_passthrough('!');
        p.note_committed("开发");
        assert_eq!(p.symbol('=', false), None);
    }

    #[test]
    fn mapping_ignores_unreadable_entries() {
        let mapping = mapping([("", "、"), ("abc", "×"), ("{kp}~=", "≈"), ("[", "【")]);
        assert_eq!(mapping.single.len(), 1);
        assert!(mapping.pair.is_empty());
        assert!(!mapping.is_empty());
    }
}
