//! 日文假名的浊音 / 半浊音变体，与旧版 VFB 的 `Form6` 对照表一致。
//!
//! 第 5 页右侧三个按钮（无 / 浊゛ / 半゜）是模式开关：
//! - 「无」：显示全部假名，按原样输入；
//! - 「浊゛」：只显示能加浊点的假名（う 行、か / さ / た / は 行，平假名 + 片假名），
//!   按钮上的字同时换成浊音写法（か→が）；
//! - 「半゜」：只显示は行 / ハ行，按钮换成半浊音写法（は→ぱ）。
//!
//! 对照表按旧工程 `KVidx / KVp / KVd / KVh` 抄录：旧版没有 ワ→ヷ 这类扩展映射，这里也不收。

/// 假名变体模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    /// 无：原样。
    #[default]
    Plain,
    /// 浊゛。
    Voiced,
    /// 半゜。
    SemiVoiced,
}

impl Variant {
    /// 这个假名在该模式下的写法；没有对应变体返回 `None`（调用方据此隐藏按钮）。
    pub fn map(self, character: char) -> Option<char> {
        let table = match self {
            Self::Plain => return None,
            Self::Voiced => VOICED,
            Self::SemiVoiced => SEMI_VOICED,
        };
        table
            .iter()
            .find(|(base, _)| *base == character)
            .map(|(_, variant)| *variant)
    }
}

/// 平假名 / 片假名的浊音对照（旧版 42 项）。
const VOICED: &[(char, char)] = &[
    ('う', 'ゔ'),
    ('ウ', 'ヴ'),
    ('か', 'が'),
    ('き', 'ぎ'),
    ('く', 'ぐ'),
    ('け', 'げ'),
    ('こ', 'ご'),
    ('カ', 'ガ'),
    ('キ', 'ギ'),
    ('ク', 'グ'),
    ('ケ', 'ゲ'),
    ('コ', 'ゴ'),
    ('さ', 'ざ'),
    ('し', 'じ'),
    ('す', 'ず'),
    ('せ', 'ぜ'),
    ('そ', 'ぞ'),
    ('サ', 'ザ'),
    ('シ', 'ジ'),
    ('ス', 'ズ'),
    ('セ', 'ゼ'),
    ('ソ', 'ゾ'),
    ('た', 'だ'),
    ('ち', 'ぢ'),
    ('つ', 'づ'),
    ('て', 'で'),
    ('と', 'ど'),
    ('タ', 'ダ'),
    ('チ', 'ヂ'),
    ('ツ', 'ヅ'),
    ('テ', 'デ'),
    ('ト', 'ド'),
    ('は', 'ば'),
    ('ひ', 'び'),
    ('ふ', 'ぶ'),
    ('へ', 'べ'),
    ('ほ', 'ぼ'),
    ('ハ', 'バ'),
    ('ヒ', 'ビ'),
    ('フ', 'ブ'),
    ('ヘ', 'ベ'),
    ('ホ', 'ボ'),
];

/// 半浊音对照（旧版 10 项，只有は行 / ハ行）。
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
    text.chars()
        .map(|character| variant.map(character).unwrap_or(character))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_keeps_everything() {
        assert_eq!(apply("かはカハ", Variant::Plain), "かはカハ");
        assert_eq!(Variant::Plain.map('か'), None);
    }

    #[test]
    fn voiced_maps_the_same_rows_as_the_old_form() {
        assert_eq!(apply("か", Variant::Voiced), "が");
        assert_eq!(apply("は", Variant::Voiced), "ば");
        assert_eq!(apply("う", Variant::Voiced), "ゔ");
        assert_eq!(apply("カ", Variant::Voiced), "ガ");
        assert_eq!(apply("ハ", Variant::Voiced), "バ");
    }

    #[test]
    fn voiced_mode_hides_kanas_without_a_voiced_form() {
        // 旧版没有 ワ→ヷ、ゝ→ゞ 这些扩展映射。
        assert_eq!(Variant::Voiced.map('ワ'), None);
        assert_eq!(Variant::Voiced.map('ゝ'), None);
        assert_eq!(Variant::Voiced.map('ん'), None);
        assert_eq!(Variant::Voiced.map('ア'), None);
    }

    #[test]
    fn semi_voiced_only_maps_the_h_rows() {
        assert_eq!(apply("は", Variant::SemiVoiced), "ぱ");
        assert_eq!(apply("ハ", Variant::SemiVoiced), "パ");
        assert_eq!(Variant::SemiVoiced.map('か'), None);
        assert_eq!(Variant::SemiVoiced.map('ン'), None);
    }

    #[test]
    fn non_kana_is_left_alone() {
        assert_eq!(apply("ー", Variant::Voiced), "ー");
        assert_eq!(apply("A1", Variant::Voiced), "A1");
    }
}
