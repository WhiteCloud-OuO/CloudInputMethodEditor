//! 日文假名的浊音 / 半浊音变体。
//!
//! 第 5 页右侧三个按钮（无 / 浊゛ / 半゜）是模式开关：选「浊゛」后点 か 输入 が、
//! 选「半゜」后点 は 输入 ぱ；找不到变体的假名原样输入。

/// 假名变体模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    /// 无：原样。
    #[default]
    Plain,
    /// 浊゛：か→が、は→ば、う→ゔ。
    Voiced,
    /// 半゜：は→ぱ（只有は行）。
    SemiVoiced,
}

/// 平假名 / 片假名的浊音对照。
const VOICED: &[(char, char)] = &[
    // 平假名
    ('か', 'が'),
    ('き', 'ぎ'),
    ('く', 'ぐ'),
    ('け', 'げ'),
    ('こ', 'ご'),
    ('さ', 'ざ'),
    ('し', 'じ'),
    ('す', 'ず'),
    ('せ', 'ぜ'),
    ('そ', 'ぞ'),
    ('た', 'だ'),
    ('ち', 'ぢ'),
    ('つ', 'づ'),
    ('て', 'で'),
    ('と', 'ど'),
    ('は', 'ば'),
    ('ひ', 'び'),
    ('ふ', 'ぶ'),
    ('へ', 'べ'),
    ('ほ', 'ぼ'),
    ('う', 'ゔ'),
    ('ゝ', 'ゞ'),
    // 片假名
    ('カ', 'ガ'),
    ('キ', 'ギ'),
    ('ク', 'グ'),
    ('ケ', 'ゲ'),
    ('コ', 'ゴ'),
    ('サ', 'ザ'),
    ('シ', 'ジ'),
    ('ス', 'ズ'),
    ('セ', 'ゼ'),
    ('ソ', 'ゾ'),
    ('タ', 'ダ'),
    ('チ', 'ヂ'),
    ('ツ', 'ヅ'),
    ('テ', 'デ'),
    ('ト', 'ド'),
    ('ハ', 'バ'),
    ('ヒ', 'ビ'),
    ('フ', 'ブ'),
    ('ヘ', 'ベ'),
    ('ホ', 'ボ'),
    ('ウ', 'ヴ'),
    ('ワ', 'ヷ'),
    ('ヰ', 'ヸ'),
    ('ヱ', 'ヹ'),
    ('ヲ', 'ヺ'),
    ('ヽ', 'ヾ'),
];

/// 半浊音对照（は行 / ハ行）。
const SEMI_VOICED: &[(char, char)] = &[
    ('は', 'ぱ'),
    ('ひ', 'ぴ'),
    ('ふ', 'ぷ'),
    ('へ', 'ぺ'),
    ('ほ', 'ぽ'),
    ('ハ', 'パ'),
    ('ヒ', 'ピ'),
    ('フ', 'プ'),
    ('ヘ', 'ペ'),
    ('ホ', 'ポ'),
];

/// 把 `text` 里的假名换成当前模式的变体；非假名 / 没有对应变体的字符原样保留。
pub fn apply(text: &str, variant: Variant) -> String {
    if variant == Variant::Plain {
        return text.to_owned();
    }
    let table = match variant {
        Variant::Plain => return text.to_owned(),
        Variant::Voiced => VOICED,
        Variant::SemiVoiced => SEMI_VOICED,
    };
    text.chars()
        .map(|character| {
            table
                .iter()
                .find(|(base, _)| *base == character)
                .map_or(character, |(_, variant)| *variant)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_keeps_everything() {
        assert_eq!(apply("かはカハ", Variant::Plain), "かはカハ");
    }

    #[test]
    fn voiced_maps_hiragana_and_katakana() {
        assert_eq!(apply("か", Variant::Voiced), "が");
        assert_eq!(apply("は", Variant::Voiced), "ば");
        assert_eq!(apply("う", Variant::Voiced), "ゔ");
        assert_eq!(apply("カ", Variant::Voiced), "ガ");
        assert_eq!(apply("ワ", Variant::Voiced), "ヷ");
    }

    #[test]
    fn semi_voiced_only_maps_the_h_rows() {
        assert_eq!(apply("は", Variant::SemiVoiced), "ぱ");
        assert_eq!(apply("ハ", Variant::SemiVoiced), "パ");
        assert_eq!(apply("か", Variant::SemiVoiced), "か");
        assert_eq!(apply("ン", Variant::SemiVoiced), "ン");
    }

    #[test]
    fn non_kana_is_left_alone() {
        assert_eq!(apply("ー", Variant::Voiced), "ー");
        assert_eq!(apply("A1", Variant::Voiced), "A1");
    }
}
