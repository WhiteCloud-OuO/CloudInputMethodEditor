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

/// 样式按钮上的样例：每种样式的前三个字母（A、B、C）。
pub fn label(style: &[char]) -> String {
    style.iter().take(3).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(prefix: &str) -> Vec<char> {
        let mut chars: Vec<char> = prefix.chars().take(52).collect();
        while chars.len() < 52 {
            chars.push('?');
        }
        chars
    }

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
    fn labels_show_the_first_three_letters() {
        let chars = style("𝐀𝐁𝐂𝐃");
        assert_eq!(label(&chars), "𝐀𝐁𝐂");
    }
}
