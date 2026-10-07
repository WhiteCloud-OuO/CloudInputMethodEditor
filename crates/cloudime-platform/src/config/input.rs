//! `[input]` 分节：输入相关的开关（简拼、模糊音、简繁、中英混输、标点与符号映射、成对补全）。

use cloudime_core::FuzzyRules;
use cloudime_core::punctuation::Mapping;
use serde::{Deserialize, Serialize};

/// 模糊音各位（`mo_hu_yin_list` 是勾选项数值的和）：**一位一条规则**，按界面顺序 2^0…2^11。
pub const MO_HU_YIN_ZH_Z: u32 = 1 << 0; // zh/z
pub const MO_HU_YIN_CH_C: u32 = 1 << 1; // ch/c
pub const MO_HU_YIN_SH_S: u32 = 1 << 2; // sh/s
pub const MO_HU_YIN_R_L: u32 = 1 << 3; // r/l
pub const MO_HU_YIN_N_L: u32 = 1 << 4; // n/l
pub const MO_HU_YIN_F_H: u32 = 1 << 5; // f/h
pub const MO_HU_YIN_U_V: u32 = 1 << 6; // u/ü
pub const MO_HU_YIN_UO_O: u32 = 1 << 7; // uo/o
pub const MO_HU_YIN_AN_ANG: u32 = 1 << 8; // an/ang
pub const MO_HU_YIN_EN_ENG: u32 = 1 << 9; // en/eng
pub const MO_HU_YIN_IN_ING: u32 = 1 << 10; // in/ing
pub const MO_HU_YIN_WANG_HUANG: u32 = 1 << 11; // wang/huang

/// 模糊音全部可选位，按界面顺序。
pub const MO_HU_YIN_BITS: [(u32, &str); 12] = [
    (MO_HU_YIN_ZH_Z, "zh/z"),
    (MO_HU_YIN_CH_C, "ch/c"),
    (MO_HU_YIN_SH_S, "sh/s"),
    (MO_HU_YIN_R_L, "r/l"),
    (MO_HU_YIN_N_L, "n/l"),
    (MO_HU_YIN_F_H, "f/h"),
    (MO_HU_YIN_U_V, "u/ü"),
    (MO_HU_YIN_UO_O, "uo/o"),
    (MO_HU_YIN_AN_ANG, "an/ang"),
    (MO_HU_YIN_EN_ENG, "en/eng"),
    (MO_HU_YIN_IN_ING, "in/ing"),
    (MO_HU_YIN_WANG_HUANG, "wang/huang"),
];

/// 成对补全各位（`punctuation_marks_pairwise_completion` 是勾选项数值的和），按界面顺序。
/// 位号连续按 2 的幂排列，与界面上的第几项一一对应。
pub const PAIRWISE_COMPLETION_BITS: [(u32, &str, char, char); 10] = [
    (1, "()", '(', ')'),
    (2, "[]", '[', ']'),
    (4, "{}", '{', '}'),
    (8, "\"\"", '"', '"'),
    (16, "（）", '（', '）'),
    (32, "【】", '【', '】'),
    (64, "｛｝", '｛', '｝'),
    (128, "《》", '《', '》'),
    (256, "“”", '“', '”'),
    (512, "‘’", '‘', '’'),
];

/// `open` 在 `mask` 里开了成对补全时，补上的右半边。
pub fn pairwise_completion(mask: u32, open: char) -> Option<char> {
    PAIRWISE_COMPLETION_BITS
        .iter()
        .find(|(bit, _, opening, _)| mask & *bit != 0 && *opening == open)
        .map(|(_, _, _, close)| *close)
}

/// `close` 是某对成对补全的右半边时，对应的左半边。智能引号这类标点是**交替**给出收符号的
/// （`"` 第一次给 `“`、第二次给 `”`），配成对时要靠它把收符号换回开符号。
pub fn pair_open(mask: u32, close: char) -> Option<char> {
    PAIRWISE_COMPLETION_BITS
        .iter()
        .find(|(bit, _, _, closing)| mask & *bit != 0 && *closing == close)
        .map(|(_, _, opening, _)| *opening)
}

/// `(`、`（` 这类左符号对应的位；不在表里返回 `None`。
pub fn pair_bit(open: char) -> Option<u32> {
    PAIRWISE_COMPLETION_BITS
        .iter()
        .find(|(_, _, opening, _)| *opening == open)
        .map(|(bit, ..)| *bit)
}

/// 中文模式符号映射各位（`punctuation_marks_mapping` 是勾选项数值的和），按界面顺序。
/// 只做**单键**替换：敲下来的是这个字符（`{kp}` 前缀 = 小键盘），就直接换成右边那个上屏。
/// 老配置里的 `~=`、`!=`、`<=`、`>=` 这类两键规则已下线（真机上总是失效），不再支持。
pub const PUNCTUATION_MAPPING_BITS: [(u32, &str, &str); 5] = [
    (1, "/", "、"),
    (2, "{kp}/", "÷"),
    (4, "{kp}*", "×"),
    (8, "~", "～"),
    (16, "·", "`"),
];

/// 符号映射的缺省位图。**缺省全关**（新的装机默认值）：要哪几项在「设置 → 输入 → 中文模式下符号映射」里勾。
pub const DEFAULT_PUNCTUATION_MAPPING: u32 = 0;

/// 简体 / 繁体输出。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SimpTrad {
    /// 简体中文（缺省）。
    #[default]
    Simplified,

    /// 繁體中文。
    Traditional,
}

impl SimpTrad {
    /// 界面上按这个顺序排，顺序与配置里的取值无关。
    pub const ALL: [Self; 2] = [Self::Simplified, Self::Traditional];

    pub fn key(self) -> &'static str {
        match self {
            Self::Simplified => "simplified",
            Self::Traditional => "traditional",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Simplified => "简体中文",
            Self::Traditional => "繁體中文",
        }
    }
}

/// 标点符号的全角 / 半角。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FullHalfPunctuation {
    /// 跟随中文 / 英文模式：中文全角、英文半角（缺省）。
    #[default]
    Follow,

    /// 一律全角。
    Full,

    /// 一律半角。
    Half,
}

impl FullHalfPunctuation {
    /// 界面上按这个顺序排。
    pub const ALL: [Self; 3] = [Self::Follow, Self::Full, Self::Half];

    pub fn key(self) -> &'static str {
        match self {
            Self::Follow => "follow",
            Self::Full => "full",
            Self::Half => "half",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Follow => "跟随中文 / 英文模式",
            Self::Full => "强制全角",
            Self::Half => "强制半角",
        }
    }

    /// `english` 是当前模式；「跟随」时中文全角、英文半角（与以前一致）。
    pub fn full_width(self, english: bool) -> bool {
        match self {
            Self::Follow => !english,
            Self::Full => true,
            Self::Half => false,
        }
    }
}

/// `[input]` 分节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputConfig {
    /// 使用简拼：`yd` 也能出「云朵」，不必打全 `yunduo`。关掉只认完整音节。
    pub use_jian_pin: bool,

    /// 模糊音列表：见 [`MO_HU_YIN_BITS`]，勾选项数值累加。
    pub mo_hu_yin_list: u32,

    /// 简 / 繁输出。
    pub simp_trad_chinese_chars_toggle: SimpTrad,

    /// 中英混合输入：中文模式下也出英文词与英文补全（`KALAOK` → 卡拉OK / `hello` → hello）。
    pub mixture_input: bool,

    /// 标点符号全 / 半角。
    pub full_half_punctuation_marks_toggle: FullHalfPunctuation,

    /// 中文模式下的符号映射：见 [`PUNCTUATION_MAPPING_BITS`]，勾选项数值累加。
    /// 只认表里这 5 项固定映射，配置文件里改不出别的花样。
    /// 缺省来自 [`InputConfig::default`]（结构体级的 `#[serde(default)]`）；这里只负责读到老的表时别报错。
    #[serde(deserialize_with = "punctuation_mapping_flags")]
    pub punctuation_marks_mapping: u32,

    /// 符号成对补全：见 [`PAIRWISE_COMPLETION_BITS`]，勾选项数值累加；0 为关闭（缺省）。
    pub punctuation_marks_pairwise_completion: u32,

    /// 数字后标点符号使用半角：`23:06`、`2.36`、`25+9` 里的标点保持半角。
    pub use_half_wide_punctuation_marks_after_digital: bool,

    /// 状态切换提示：中 / 英、大写锁定、全 / 半角、简 / 繁、中文 / 西文标点变化时，
    /// 在输入光标附近弹一个停留 1 秒的提示条（只在处于输入状态时弹）。
    pub show_status_change_tip: bool,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            use_jian_pin: true,
            mo_hu_yin_list: 0,
            simp_trad_chinese_chars_toggle: SimpTrad::Simplified,
            mixture_input: true,
            full_half_punctuation_marks_toggle: FullHalfPunctuation::Follow,
            punctuation_marks_mapping: DEFAULT_PUNCTUATION_MAPPING,
            punctuation_marks_pairwise_completion: 0,
            use_half_wide_punctuation_marks_after_digital: true,
            show_status_change_tip: true,
        }
    }
}

/// 老配置里 `punctuation_marks_mapping` 是「键 = 文本」的表（还带 `~=` 这类两键规则）。
/// 表已下线、改成一个位图：读到表就退回缺省位图，别让整份配置解析失败（用户在设置页重新勾即可）。
fn punctuation_mapping_flags<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(<u32 as Deserialize>::deserialize(deserializer).unwrap_or(DEFAULT_PUNCTUATION_MAPPING))
}

/// Core 的模糊音规则 → 位图（CLI 的 `--fuzzy` 按规则名开完之后回写配置用）。一位一条，与界面一一对应。
pub fn fuzzy_bits(rules: &FuzzyRules) -> u32 {
    let mut bits = 0;
    for (on, bit) in [
        (rules.z_zh, MO_HU_YIN_ZH_Z),
        (rules.c_ch, MO_HU_YIN_CH_C),
        (rules.s_sh, MO_HU_YIN_SH_S),
        (rules.l_r, MO_HU_YIN_R_L),
        (rules.n_l, MO_HU_YIN_N_L),
        (rules.f_h, MO_HU_YIN_F_H),
        (rules.u_v, MO_HU_YIN_U_V),
        (rules.uo_o, MO_HU_YIN_UO_O),
        (rules.an_ang, MO_HU_YIN_AN_ANG),
        (rules.en_eng, MO_HU_YIN_EN_ENG),
        (rules.in_ing, MO_HU_YIN_IN_ING),
        (rules.wang_huang, MO_HU_YIN_WANG_HUANG),
    ] {
        if on {
            bits |= bit;
        }
    }
    bits
}

impl InputConfig {
    /// 模糊音位图 → Core 的模糊音规则。
    pub fn fuzzy_rules(&self) -> FuzzyRules {
        let bit = |mask: u32| self.mo_hu_yin_list & mask != 0;
        FuzzyRules {
            z_zh: bit(MO_HU_YIN_ZH_Z),
            c_ch: bit(MO_HU_YIN_CH_C),
            s_sh: bit(MO_HU_YIN_SH_S),
            l_r: bit(MO_HU_YIN_R_L),
            n_l: bit(MO_HU_YIN_N_L),
            f_h: bit(MO_HU_YIN_F_H),
            u_v: bit(MO_HU_YIN_U_V),
            uo_o: bit(MO_HU_YIN_UO_O),
            an_ang: bit(MO_HU_YIN_AN_ANG),
            en_eng: bit(MO_HU_YIN_EN_ENG),
            in_ing: bit(MO_HU_YIN_IN_ING),
            wang_huang: bit(MO_HU_YIN_WANG_HUANG),
        }
    }

    /// 配置的符号映射位图 → Core 的映射表（只取勾上的那几项固定单键映射）。
    pub fn punctuation_mapping(&self) -> Mapping {
        Mapping::from_pairs(
            PUNCTUATION_MAPPING_BITS
                .iter()
                .filter(|(bit, ..)| self.punctuation_marks_mapping & *bit != 0)
                .map(|(_, key, text)| (*key, *text)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmaps_map_to_fuzzy_rules() {
        // 一位一条规则，位与规则一一对应（zh/z、sh/s、in/ing）
        let config = InputConfig {
            mo_hu_yin_list: MO_HU_YIN_ZH_Z | MO_HU_YIN_SH_S | MO_HU_YIN_IN_ING,
            ..InputConfig::default()
        };
        let rules = config.fuzzy_rules();
        assert!(rules.z_zh && rules.s_sh && rules.in_ing);
        assert!(!rules.c_ch && !rules.l_r && !rules.n_l && !rules.f_h);
        assert!(!rules.u_v && !rules.uo_o && !rules.an_ang && !rules.en_eng && !rules.wang_huang);
        // f/h 与 ü/u
        let config = InputConfig {
            mo_hu_yin_list: MO_HU_YIN_F_H | MO_HU_YIN_U_V,
            ..InputConfig::default()
        };
        let rules = config.fuzzy_rules();
        assert!(rules.f_h && rules.u_v && !rules.z_zh);
        // 位图 → 规则 → 位图 是恒等的
        assert_eq!(fuzzy_bits(&rules), MO_HU_YIN_F_H | MO_HU_YIN_U_V);
    }

    #[test]
    fn pairwise_completion_uses_the_selected_pairs() {
        // 16 = （）、256 = “”
        let mask = 16 | 256;
        assert_eq!(pairwise_completion(mask, '（'), Some('）'));
        assert_eq!(pairwise_completion(mask, '“'), Some('”'));
        assert_eq!(pairwise_completion(mask, '('), None);
        assert_eq!(pairwise_completion(0, '('), None);
        assert_eq!(pairwise_completion(mask, '。'), None);
    }
}
