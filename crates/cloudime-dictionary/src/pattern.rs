/// 把键盘输入常用的 `lue` / `nue`、以及 `j/q/x/y + u` 的写法转成词库规范形式。
///
/// 词库用 `v` 表示 ü：`lüe` / `nüe` 写作 `lve` / `nve`；`j/q/x/y` 后的 `u` 实际是 ü，
/// 也统一写成 `v`（`ju → jv`、`jue → jve`、`juan → jvan`、`jun → jvn`，`q/x/y` 同理）。
/// 输入时 `ju` 与 `jv` 都接受，查询都会被归一成 `jv`。其他音节原样返回。
pub fn canonical_syllable(text: &str) -> &str {
    match text {
        "lue" => "lve",
        "nue" => "nve",
        "ju" => "jv",
        "jue" => "jve",
        "juan" => "jvan",
        "jun" => "jvn",
        "qu" => "qv",
        "que" => "qve",
        "quan" => "qvan",
        "qun" => "qvn",
        "xu" => "xv",
        "xue" => "xve",
        "xuan" => "xvan",
        "xun" => "xvn",
        "yu" => "yv",
        "yue" => "yve",
        "yuan" => "yvan",
        "yun" => "yvn",
        _ => text,
    }
}

/// 模型 / 显示用的标准写法：`jv → ju`、`jve → jue`…；`lve` / `nve` 保持 v
/// （知意模型的词表就是标准写法，且 `lve` 是它自己的写法）。
pub fn standard_syllable(text: &str) -> &str {
    match text {
        "jv" => "ju",
        "jve" => "jue",
        "jvan" => "juan",
        "jvn" => "jun",
        "qv" => "qu",
        "qve" => "que",
        "qvan" => "quan",
        "qvn" => "qun",
        "xv" => "xu",
        "xve" => "xue",
        "xvan" => "xuan",
        "xvn" => "xun",
        "yv" => "yu",
        "yve" => "yue",
        "yvan" => "yuan",
        "yvn" => "yun",
        _ => text,
    }
}

/// 模型输入里没打完的前缀：`j/q/x/y` 后只可能把 `v` 写回 `u`（模型词表没有 `p:jv`），
/// `lve` / `nve` 的前缀保持 v。
pub fn standard_prefix(text: &str) -> String {
    if text.starts_with(['j', 'q', 'x', 'y']) {
        text.replace('v', "u")
    } else {
        text.to_owned()
    }
}

/// `typed` 是否是这个音节 / 音节前缀：`j/q/x/y` 后 `u` 与 `v` 等价，`lue/nue` 也对 `lve/nve`。
pub fn matches_syllable(typed: &str, syllable: &str, complete: bool) -> bool {
    if complete {
        return canonical_syllable(typed) == canonical_syllable(syllable);
    }
    let target = canonical_syllable(syllable);
    let mapped: String = typed
        .chars()
        .enumerate()
        .map(|(index, character)| {
            if target.chars().nth(index) == Some('v') && character == 'u' {
                'v'
            } else {
                character
            }
        })
        .collect();
    target.starts_with(&mapped)
}

/// `keys`（已去分隔符的按键串）是否正好由这些音节组成；`u` / `v` 按规范写法比较。
pub fn syllables_match(keys: &str, syllables: &[String]) -> bool {
    let mut rest = keys;
    for syllable in syllables {
        // 规范写法只替换同长度的 u/v，不会改变音节长度。
        let Some(chunk) = rest.get(..syllable.len()) else {
            return false;
        };
        if canonical_syllable(chunk) != canonical_syllable(syllable) {
            return false;
        }
        rest = &rest[syllable.len()..];
    }
    rest.is_empty()
}

/// 一个音节的查询条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyllablePattern<'a> {
    /// 用户敲的字母：完整音节，或音节前缀 / 声母。
    pub text: &'a str,

    /// 为真时要求音节与 `text` 完全相等，否则只要求以 `text` 开头（简拼、未打完的音节）。
    pub complete: bool,
}

impl<'a> SyllablePattern<'a> {
    pub fn complete(text: &'a str) -> Self {
        Self {
            text,
            complete: true,
        }
    }

    pub fn prefix(text: &'a str) -> Self {
        Self {
            text,
            complete: false,
        }
    }

    pub fn accepts(&self, syllable: &str) -> bool {
        let text = canonical_syllable(self.text);
        let syllable = canonical_syllable(syllable);
        if self.complete {
            syllable == text
        } else {
            syllable.starts_with(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_keyboard_spelling_for_u_umlaut() {
        assert_eq!(canonical_syllable("lue"), "lve");
        assert_eq!(canonical_syllable("nue"), "nve");
        assert_eq!(canonical_syllable("lve"), "lve");
        assert_eq!(canonical_syllable("nve"), "nve");
        // j/q/x/y 后的 u 实际是 ü，统一写 v；y 的 you / ya 等不受影响
        for (typed, canonical) in [
            ("ju", "jv"),
            ("jue", "jve"),
            ("juan", "jvan"),
            ("jun", "jvn"),
            ("qu", "qv"),
            ("que", "qve"),
            ("quan", "qvan"),
            ("qun", "qvn"),
            ("xu", "xv"),
            ("xue", "xve"),
            ("xuan", "xvan"),
            ("xun", "xvn"),
            ("yu", "yv"),
            ("yue", "yve"),
            ("yuan", "yvan"),
            ("yun", "yvn"),
        ] {
            assert_eq!(canonical_syllable(typed), canonical, "{typed}");
            assert_eq!(canonical_syllable(canonical), canonical, "{canonical}");
        }
        assert_eq!(canonical_syllable("you"), "you");
        assert_eq!(canonical_syllable("ya"), "ya");
    }

    #[test]
    fn accepts_keyboard_and_canonical_spellings_equally() {
        assert!(SyllablePattern::complete("lue").accepts("lve"));
        assert!(SyllablePattern::complete("nve").accepts("nue"));
        assert!(SyllablePattern::complete("xue").accepts("xve"));
        assert!(SyllablePattern::complete("ju").accepts("jv"));
        assert!(SyllablePattern::complete("yun").accepts("yvn"));
    }

    #[test]
    fn matches_prefixes_and_full_readings_across_u_and_v() {
        assert!(matches_syllable("ju", "jv", true));
        assert!(matches_syllable("jue", "jve", true));
        assert!(matches_syllable("jua", "jvan", false));
        assert!(matches_syllable("yun", "yvn", true));
        assert!(!matches_syllable("ji", "jv", true));
        assert_eq!(standard_syllable("jv"), "ju");
        assert_eq!(standard_syllable("yvn"), "yun");
        assert_eq!(standard_syllable("lve"), "lve");
        assert_eq!(standard_prefix("jva"), "jua");
        assert_eq!(standard_prefix("lv"), "lv");
        let syllables = vec!["qv".to_owned(), "xve".to_owned()];
        assert!(syllables_match("quxue", &syllables));
        assert!(!syllables_match("quxva", &syllables));
    }
}
