//! 英文花体：把 A–Z / a–z 换成某一种 Unicode 数学字母，其它字符原样保留。

/// 把 `text` 按 `style`（52 个字符：A–Z + a–z）转换。
pub fn convert(text: &str, style: &[char]) -> String {
    if style.len() != 52 {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        let mapped = match character {
            'A'..='Z' => style.get((character as u8 - b'A') as usize),
            'a'..='z' => style.get(26 + (character as u8 - b'a') as usize),
            _ => None,
        };
        out.push(mapped.copied().unwrap_or(character));
    }
    out
}

/// 样式按钮上的样例：大写 A + 小写 b、c（旧版界面就是这三种字母的映射结果）。
pub fn label(style: &[char]) -> String {
    convert("Abc", style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_ascii_letters_and_keeps_everything_else() {
        let mut chars: Vec<char> = ('A'..='Z').collect();
        chars.extend('a'..='z');
        assert_eq!(convert("Az 09!中", &chars), "Az 09!中");
    }

    #[test]
    fn uses_lowercase_slots_for_lowercase_letters() {
        let chars: Vec<char> = ('\u{1D400}'..='\u{1D433}').collect();
        assert_eq!(chars.len(), 52);
        assert_eq!(convert("AB ab", &chars), "𝐀𝐁 𝐚𝐛");
    }

    #[test]
    fn labels_mix_uppercase_a_with_lowercase_bc() {
        let chars: Vec<char> = ('\u{1D400}'..='\u{1D433}').collect();
        assert_eq!(label(&chars), "𝐀𝐛𝐜");
        // 大写 / 小写共用一个字形的样式：Abc 与 ABC 显示相同。
        let mut chars: Vec<char> = "⒜⒝⒞".chars().collect();
        chars.resize(26, '?');
        chars.extend("⒜⒝⒞".chars().chain(std::iter::repeat('?')).take(26));
        assert_eq!(label(&chars), "⒜⒝⒞");
    }
}
