//! **辅助码的匹配方式**：`use_input_scheme.cfg` 的 `auxiliary_mode` 选哪一种，见
//! `docs/design/input-schemes.md` §4。
//!
//! - [`AuxiliaryMode::PerChar`]（逐字，缺省）：每个字一段码、跟在自己的音节后面（`xxXXyyYY…`），
//!   按**字**对齐 —— 匹配逻辑在 [`crate::shuangpin::keeps`]（这里不管）。
//! - [`AuxiliaryMode::DoubleHead`] / [`AuxiliaryMode::DoubleTail`] / [`AuxiliaryMode::FirstLast`]
//!   （半字：双前 / 双后 / 首尾）：**单字**用它本身的码；**多字词**用「首字的码取一位 + 尾字的码取
//!   一位」推出来的**两位**码（取第一位还是最后一位由方式决定）。敲的码只认**整段输入末尾**那串大写的
//!   **最后两位**（多敲的丢掉，见 [`trailing_pair`]），是推出来的那位码的**前缀**就算命中。
//!
//! 中间位置出现的大写字母在 1–3 里**直接丢掉**：不参与断句、也不当码（但原始按键照旧整串参与
//! 「中英混合输入」的英文候选匹配 —— 那条路读的是没动过的按键）。

use std::borrow::Cow;

use crate::shuangpin::{AuxiliaryTable, MAX_AUXILIARY, is_prefix};

/// 辅助码的匹配方式（`use_input_scheme.cfg` 的 `auxiliary_mode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuxiliaryMode {
    /// 逐字：每个字的码跟在自己的音节后面，按字对齐（缺省）。
    #[default]
    PerChar,

    /// 半字（双前）：首字码的第一位 + 尾字码的第一位。
    DoubleHead,

    /// 半字（双后）：首字码的最后一位 + 尾字码的最后一位。
    DoubleTail,

    /// 半字（首尾）：首字码的第一位 + 尾字码的最后一位。
    FirstLast,
}

impl AuxiliaryMode {
    /// 是不是「半字」那三种（按词推码）。
    pub fn is_derived(self) -> bool {
        self != Self::PerChar
    }

    /// 首字取码里的**第一位**（否则取最后一位）。
    fn takes_head_of_first(self) -> bool {
        matches!(self, Self::DoubleHead | Self::FirstLast)
    }

    /// 尾字取码里的**第一位**（否则取最后一位）。
    fn takes_head_of_last(self) -> bool {
        self == Self::DoubleHead
    }
}

/// 断句用的按键：**半字**三种方式下把中间那些大写字母丢掉，末尾那串大写**只留最后两位**
/// （多敲的丢掉；`aiPqgXXH` → `aiqgXH`、`aiqgABCDHU` → `aiqgHU`）；逐字方式原样返回。
/// 没动过就借用原串。
pub fn keys_for_reading<'a>(keys: &'a str, mode: AuxiliaryMode) -> Cow<'a, str> {
    if !mode.is_derived() {
        return Cow::Borrowed(keys);
    }
    let chars: Vec<char> = keys.chars().collect();
    // 末尾那串大写的起点（后面那串是辅助码）；再往前只留两位
    let mut start = chars.len();
    while start > 0 && chars[start - 1].is_ascii_uppercase() {
        start -= 1;
    }
    let keep = (chars.len() - start).min(2);
    let tail_start = chars.len() - keep;
    // 前面没有大写、末尾也不超过两位就不用动
    if keep == chars.len() - start && !chars[..start].iter().any(char::is_ascii_uppercase) {
        return Cow::Borrowed(keys);
    }
    let mut out = String::with_capacity(keys.len());
    out.extend(chars[..start].iter().filter(|c| !c.is_ascii_uppercase()));
    out.extend(chars[tail_start..].iter());
    Cow::Owned(out)
}

/// 敲的辅助码：**整段输入末尾**那串大写（最多 [`MAX_AUXILIARY`] 位）；没有就是 `None`。
pub fn trailing_code(keys: &str) -> Option<String> {
    let tail: String = keys
        .chars()
        .rev()
        .take(MAX_AUXILIARY)
        .take_while(char::is_ascii_uppercase)
        .collect();
    if tail.is_empty() {
        return None;
    }
    Some(tail.chars().rev().collect())
}

/// 半字方式用的码：末尾那串大写里的**最后两位**（推出来的码就两位 —— 前面多敲的丢掉）。
///
/// 例：`yunHU` → `HU`；`yunXXHU` → `HU`（`XX` 丢掉）；`yunH` → `H`。
pub fn trailing_pair(keys: &str) -> Option<String> {
    let code = trailing_code(keys)?;
    let letters: Vec<char> = code.chars().collect();
    let start = letters.len().saturating_sub(2);
    Some(letters[start..].iter().collect())
}

/// 这个候选在这种方式下要对的那条（几条）码：
/// 单字 = 表里它自己的码；多字词 = 首字与尾字的码各取一位拼成的两位（同键多义时几个组合都留着）。
/// 表里没有首字 / 尾字（或推不出字母）就是 `None` —— 敲了码时这种候选对不上、会被筛掉。
pub fn wanted_code(
    text: &str,
    table: &dyn AuxiliaryTable,
    mode: AuxiliaryMode,
) -> Option<Vec<String>> {
    if !mode.is_derived() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    match chars.len() {
        0 => None,
        1 => table.codes(&chars[0].to_string()).map(<[_]>::to_vec),
        _ => {
            let first = table.codes(&chars[0].to_string())?;
            let last = table.codes(&chars[chars.len() - 1].to_string())?;
            let mut codes = Vec::new();
            for head in first {
                let Some(left) = pick(head, mode.takes_head_of_first()) else {
                    continue;
                };
                for tail in last {
                    let Some(right) = pick(tail, mode.takes_head_of_last()) else {
                        continue;
                    };
                    codes.push(format!("{left}{right}"));
                }
            }
            (!codes.is_empty()).then_some(codes)
        }
    }
}

/// 码里取第一位还是最后一位。
fn pick(code: &str, head: bool) -> Option<char> {
    if head {
        code.chars().next()
    } else {
        code.chars().last()
    }
}

/// 半字辅助码的筛选（只筛选、不排序）：敲的那几位是推出来的码的**前缀**就留下。
/// 没敲码（`typed` 空）不参与筛选；推不出码的候选（表里没有首 / 尾字）对不上，丢掉。
pub fn keeps_derived(
    text: &str,
    typed: &str,
    table: &dyn AuxiliaryTable,
    mode: AuxiliaryMode,
) -> bool {
    if typed.is_empty() {
        return true;
    }
    wanted_code(text, table, mode)
        .is_some_and(|codes| codes.iter().any(|code| is_prefix(typed, code)))
}

/// 候选右侧要显示的辅助码（半字方式）：推出来的那条码 + 其中**已经敲上**的那几位
/// （与 [`keeps_derived`] 同一份判定）。推不出码就是 `None`。
pub fn derived_auxiliary(
    text: &str,
    typed: &str,
    table: &dyn AuxiliaryTable,
    mode: AuxiliaryMode,
) -> Option<(String, Option<String>)> {
    let codes = wanted_code(text, table, mode)?;
    let code = codes.first()?.clone();
    let typed = (!typed.is_empty() && is_prefix(typed, &code)).then(|| {
        // 敲的那几位按**码自己**的大小写取，画出来跟整条码一致
        code.get(..typed.len()).unwrap_or_default().to_owned()
    });
    Some((code, typed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// 一张小表：云 / 朵 / 法 / 爱 / 情 + 一个多码的字（用 `云` 有两种码来试组合）。
    struct Codes(BTreeMap<String, Vec<String>>);

    impl Codes {
        fn new(entries: &[(&str, &[&str])]) -> Self {
            let mut map = BTreeMap::new();
            for (word, codes) in entries {
                map.insert(
                    (*word).to_owned(),
                    codes.iter().map(|code| (*code).to_owned()).collect(),
                );
            }
            Self(map)
        }
    }

    impl AuxiliaryTable for Codes {
        fn codes(&self, word: &str) -> Option<&[String]> {
            self.0.get(word).map(Vec::as_slice)
        }
    }

    fn table() -> Codes {
        Codes::new(&[
            ("云", &["HD"]),
            ("朵", &["PM"]),
            ("输", &["QF"]),
            ("入", &["RU"]),
            ("法", &["UT"]),
            ("爱", &["PD"]),
            ("情", &["XH"]),
        ])
    }

    /// 断句用的按键：半字方式丢掉中间的大写、只留末尾那串；逐字方式原样。
    #[test]
    fn derived_modes_keep_only_the_trailing_code() {
        assert_eq!(
            keys_for_reading("aiPDqgXH", AuxiliaryMode::DoubleHead),
            "aiqgXH"
        );
        assert_eq!(
            keys_for_reading("aiPDqg", AuxiliaryMode::DoubleHead),
            "aiqg"
        );
        assert_eq!(
            keys_for_reading("aiP'qg", AuxiliaryMode::DoubleTail),
            "ai'qg"
        );
        assert_eq!(
            keys_for_reading("aipdqgxh", AuxiliaryMode::DoubleHead),
            "aipdqgxh"
        );
        assert_eq!(
            keys_for_reading("aiqgXXH", AuxiliaryMode::DoubleHead),
            "aiqgXH",
            "末尾多敲的丢掉"
        );
        assert_eq!(
            keys_for_reading("aiqgABCDHU", AuxiliaryMode::DoubleHead),
            "aiqgHU",
            "末尾那串超四位也只留最后两位"
        );
        // 逐字方式不动
        assert_eq!(
            keys_for_reading("aiPDqgXH", AuxiliaryMode::PerChar),
            "aiPDqgXH"
        );
        // 没动过就借用原串（不复制）
        assert!(matches!(
            keys_for_reading("yun'duo", AuxiliaryMode::FirstLast),
            Cow::Borrowed("yun'duo")
        ));
    }

    /// 敲的码：末尾那串大写的**最后两位**（多敲的丢掉）；一位都没敲就是 `None`。
    #[test]
    fn the_trailing_pair_keeps_only_the_last_two() {
        assert_eq!(trailing_pair("aiPDqgXH").as_deref(), Some("XH"));
        assert_eq!(
            trailing_pair("yunXXHU").as_deref(),
            Some("HU"),
            "前面多敲的丢掉"
        );
        assert_eq!(trailing_pair("yunABCD").as_deref(), Some("CD"));
        assert_eq!(trailing_pair("yunH").as_deref(), Some("H"));
        assert_eq!(trailing_pair("yunduo").as_deref(), None);
    }

    /// 敲的码：末尾那串大写，最多 4 位。
    #[test]
    fn the_trailing_code_is_the_last_uppercase_run() {
        assert_eq!(trailing_code("aiPDqgXH").as_deref(), Some("XH"));
        assert_eq!(trailing_code("yunHD").as_deref(), Some("HD"));
        assert_eq!(trailing_code("yunduo").as_deref(), None);
        assert_eq!(
            trailing_code("yunHDSAx").as_deref(),
            None,
            "末尾不是大写就没有码"
        );
        assert_eq!(
            trailing_code("yunABCDE").as_deref(),
            Some("BCDE"),
            "最多 4 位"
        );
    }

    /// 推码：单字用它本身的码；多字词按方式取首 / 尾字的一位拼两位。
    #[test]
    fn wanted_codes_follow_the_mode() {
        let table = table();
        let text = "云朵输入法";
        assert_eq!(
            wanted_code(text, &table, AuxiliaryMode::DoubleHead).as_deref(),
            Some(&["HU".to_owned()][..]),
            "双前：云的第一位 H + 法的第一位 U"
        );
        assert_eq!(
            wanted_code(text, &table, AuxiliaryMode::DoubleTail).as_deref(),
            Some(&["DT".to_owned()][..]),
            "双后：云的最后一位 D + 法的最后一位 T"
        );
        assert_eq!(
            wanted_code(text, &table, AuxiliaryMode::FirstLast).as_deref(),
            Some(&["HT".to_owned()][..]),
            "首尾：云的第一位 H + 法的最后一位 T"
        );
        // 单字：本身的码
        assert_eq!(
            wanted_code("云", &table, AuxiliaryMode::DoubleHead).as_deref(),
            Some(&["HD".to_owned()][..])
        );
        // 双字词：首字与尾字（就是第二个字）
        assert_eq!(
            wanted_code("云朵", &table, AuxiliaryMode::DoubleHead).as_deref(),
            Some(&["HP".to_owned()][..])
        );
        // 表里没有尾字 → 推不出来
        assert_eq!(wanted_code("云啊", &table, AuxiliaryMode::DoubleHead), None);
        // 逐字方式不走这条路
        assert_eq!(wanted_code("云朵", &table, AuxiliaryMode::PerChar), None);
    }

    /// 筛选：敲的是推出来那两位的**前缀**就留下；敲错、或推不出码 → 丢掉；没敲码不筛。
    #[test]
    fn derived_keeps_matches_prefixes() {
        let table = table();
        assert!(keeps_derived(
            "云朵输入法",
            "HU",
            &table,
            AuxiliaryMode::DoubleHead
        ));
        assert!(
            keeps_derived("云朵输入法", "H", &table, AuxiliaryMode::DoubleHead),
            "敲一半"
        );
        assert!(!keeps_derived(
            "云朵输入法",
            "HD",
            &table,
            AuxiliaryMode::DoubleHead
        ));
        assert!(!keeps_derived(
            "云朵输入法",
            "HU",
            &table,
            AuxiliaryMode::DoubleTail
        ));
        assert!(keeps_derived(
            "云朵",
            "HP",
            &table,
            AuxiliaryMode::DoubleHead
        ));
        assert!(
            keeps_derived("云", "H", &table, AuxiliaryMode::DoubleHead),
            "单字用本身的码"
        );
        assert!(
            keeps_derived("云", "HD", &table, AuxiliaryMode::DoubleTail),
            "单字不看首尾"
        );
        assert!(
            !keeps_derived("云啊", "HU", &table, AuxiliaryMode::DoubleHead),
            "推不出码"
        );
        assert!(
            keeps_derived("云啊", "", &table, AuxiliaryMode::DoubleHead),
            "没敲码不筛"
        );
    }

    /// 显示用的码：推出来的那条 + 已经敲上的那几位（没敲 / 对不上就没有下划线部分）。
    #[test]
    fn derived_auxiliary_gives_the_code_and_what_is_typed() {
        let table = table();
        assert_eq!(
            derived_auxiliary("云朵输入法", "H", &table, AuxiliaryMode::DoubleHead),
            Some(("HU".to_owned(), Some("H".to_owned())))
        );
        assert_eq!(
            derived_auxiliary("云朵输入法", "", &table, AuxiliaryMode::DoubleHead),
            Some(("HU".to_owned(), None))
        );
        assert_eq!(
            derived_auxiliary("云朵输入法", "X", &table, AuxiliaryMode::DoubleHead),
            Some(("HU".to_owned(), None)),
            "对不上就不画下划线（反正也会被筛掉）"
        );
        assert_eq!(
            derived_auxiliary("云啊", "HU", &table, AuxiliaryMode::DoubleHead),
            None
        );
    }
}
