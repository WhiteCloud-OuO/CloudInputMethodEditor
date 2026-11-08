//! `=>` 进制转换：`0b` / `0o` / `0x` / 无前缀 → 其他三种进制。
//!
//! 只认小写前缀；数字先按前缀解析成 `u128`，再按 2 / 8 / 10 / 16 的顺序输出除源进制外的三种。
//! 非十进制输出带对应小写前缀，十进制不带前缀。

const BASES: [u32; 4] = [2, 8, 10, 16];

/// 输入是 `=>0x33ccff` 这类转换式时返回三条候选（其他三种进制）；不是转换式返回 `None`。
pub fn conversion_forms(text: &str) -> Option<Vec<String>> {
    let body = text.strip_prefix("=>")?;
    let (base, digits) = if let Some(rest) = body.strip_prefix("0b") {
        (2, rest)
    } else if let Some(rest) = body.strip_prefix("0o") {
        (8, rest)
    } else if let Some(rest) = body.strip_prefix("0x") {
        (16, rest)
    } else {
        (10, body)
    };
    if digits.is_empty() || !digits.chars().all(|c| valid_digit(c, base)) {
        return None;
    }
    let value = u128::from_str_radix(digits, base).ok()?;
    Some(
        BASES
            .into_iter()
            .filter(|candidate| *candidate != base)
            .map(|candidate| format_radix(value, candidate))
            .collect(),
    )
}

fn valid_digit(c: char, base: u32) -> bool {
    match base {
        2 => matches!(c, '0' | '1'),
        8 => ('0'..='7').contains(&c),
        16 => c.is_ascii_hexdigit(),
        _ => c.is_ascii_digit(),
    }
}

fn format_radix(value: u128, base: u32) -> String {
    match base {
        2 => format!("0b{value:b}"),
        8 => format!("0o{value:o}"),
        16 => format!("0x{value:x}"),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_converts_to_binary_octal_decimal() {
        assert_eq!(
            conversion_forms("=>0x33ccff").unwrap(),
            ["0b1100111100110011111111", "0o14746377", "3394815"]
        );
    }

    #[test]
    fn decimal_converts_to_binary_octal_hex() {
        assert_eq!(
            conversion_forms("=>255").unwrap(),
            ["0b11111111", "0o377", "0xff"]
        );
    }

    #[test]
    fn binary_and_octal_sources_are_supported() {
        assert_eq!(conversion_forms("=>0b1010").unwrap(), ["0o12", "10", "0xa"]);
        assert_eq!(conversion_forms("=>0o17").unwrap(), ["0b1111", "15", "0xf"]);
    }

    #[test]
    fn requires_the_conversion_prefix_and_lowercase_base_prefix() {
        assert!(conversion_forms("0x10").is_none());
        assert!(conversion_forms("=>0X10").is_none());
        assert!(conversion_forms("=>0b102").is_none());
        assert!(conversion_forms("=>0x").is_none());
        assert!(conversion_forms("=>").is_none());
    }
}
