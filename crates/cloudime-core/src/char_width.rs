//! 全角 / 半角字符（悬浮状态条上的「全角 / 半角」开关）。
//!
//! 只管「本来就直通给应用的可打印 ASCII」：中文标点优先（壳先问 [`crate::punctuation::Punctuation`]，
//! 转成 `，。` 的就不再走这里），空格与功能键不动。

/// 半角可打印 ASCII（`!` 到 `~`）到全角形的码点差。
const OFFSET: u32 = 0xFEE0;

/// 半角可打印 ASCII 对应的全角字符；空格与别的字符返回 `None`（空格转 U+3000 太容易误伤）。
pub fn full_width(c: char) -> Option<char> {
    match c as u32 {
        code @ 0x21..=0x7E => char::from_u32(code + OFFSET),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_printable_ascii_to_its_full_width_form() {
        assert_eq!(full_width('a'), Some('ａ'));
        assert_eq!(full_width('A'), Some('Ａ'));
        assert_eq!(full_width('1'), Some('１'));
        assert_eq!(full_width('-'), Some('－'));
        assert_eq!(full_width('~'), Some('～'));
        // 空格、汉字与功能字符不动
        assert_eq!(full_width(' '), None);
        assert_eq!(full_width('中'), None);
        assert_eq!(full_width('\t'), None);
        assert_eq!(full_width('\n'), None);
    }
}
