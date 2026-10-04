//! 一处编辑的纠错读法：换位、相邻键替换、多敲、少敲，作为额外读法进候选池，按权重打折。
//!
//! 原样读法的候选一直保留（不再整段替换切分），纠错读法只是同一池子里的额外候选：换位折扣
//! [`TRANSPOSE_FACTOR`]、相邻键替换 [`SUBSTITUTE_FACTOR`]、多敲 / 少敲 [`EXTRA_MISSING_FACTOR`]；
//! 原样输入本身能整段对上 一个词或短语时折扣保留，对不上时再乘回去（换位 ×1.2 / 其余 ×1.15），
//! 让明显敲错的串更容易被纠。
//!
//! 纠错读法只是「额外读法」：排序时它们排在原样读法之后（`Scored::tier` = 2），所以额外读法不会
//! 把原样候选挤走，只在那段拼音本来就讲不通时顶上来。只由末尾落单字母触发的（多半还没敲完）
//! 只试相邻换位，试不起整串的替换 / 增删变体；拼音真的切不干净时才四类都试。

use super::*;
use crate::correction::{Correction, Edit};

/// 换位读法的权重折扣（再乘回补时见模块注释）。
pub(super) const TRANSPOSE_FACTOR: f64 = 0.75;

/// 相邻键替换读法的权重折扣。
pub(super) const SUBSTITUTE_FACTOR: f64 = 0.70;

/// 多敲 / 少敲一个键的权重折扣。
pub(super) const EXTRA_MISSING_FACTOR: f64 = 0.60;

/// 一处编辑纠错读法的类别，决定权重折扣。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CorrectionKind {
    /// 相邻两键敲反（`kagn` → `kang`）。
    Transpose,

    /// 敲到相邻键（`ksng` → `kang`）。
    Substitute,

    /// 多敲了一个键（`kagn` → `kan`）。
    Extra,

    /// 少敲了一个键（`kan` → `kang`）。
    Missing,
}

impl CorrectionKind {
    fn of(edit: &Edit) -> Option<Self> {
        match edit {
            Edit::Transpose { .. } => Some(Self::Transpose),
            Edit::Substitute { .. } => Some(Self::Substitute),
            Edit::Delete { .. } => Some(Self::Extra),
            Edit::Insert { .. } => Some(Self::Missing),
        }
    }

    /// 这一类的基础折扣。
    fn discount(self) -> f64 {
        match self {
            Self::Transpose => TRANSPOSE_FACTOR,
            Self::Substitute => SUBSTITUTE_FACTOR,
            Self::Extra | Self::Missing => EXTRA_MISSING_FACTOR,
        }
    }

    /// 原样输入整段对不上词 / 短语时的回补倍数。
    fn backfill(self) -> f64 {
        if self == Self::Transpose { 1.2 } else { 1.15 }
    }
}

impl Engine {
    /// 触发条件成立（拼音不像话或末尾落单字母）时的一处编辑纠错读法：换位 +（`full` 时）相邻键替换 /
    /// 多敲 / 少敲，能整段（换位可放宽到末尾没敲完）切分；每条的权重折扣按「原样能否整段对上」算好。
    pub(super) fn correction_hits(
        &self,
        scope: &str,
        original_matches: bool,
        full: bool,
    ) -> Vec<(Correction, f64)> {
        self.correction_readings(scope, full)
            .into_iter()
            .map(|correction| {
                let kind =
                    CorrectionKind::of(&correction.edit).unwrap_or(CorrectionKind::Substitute);
                let factor = correction_factor(kind, original_matches);
                (correction, factor)
            })
            .collect()
    }

    /// 与某个候选音节序列对得上的纠错读法（整段或它的前缀）：上屏按它换算消耗与敲错对。
    pub(in crate::engine) fn correction_reading_for(
        &self,
        scope: &str,
        syllables: &[String],
    ) -> Option<Correction> {
        self.correction_readings(scope, full_variants_wanted(scope))
            .into_iter()
            .find(|correction| {
                correction.segmentation.syllables.len() >= syllables.len()
                    && correction
                        .segmentation
                        .syllables
                        .iter()
                        .zip(syllables)
                        .all(|(syllable, wanted)| &syllable.text == wanted)
            })
    }

    /// 纠错读法里有没有能把整段转通的：给「句末英文词」判断用（`shiide` 是敲错，不是英文）。
    pub(super) fn has_correction(&self, scope: &str) -> bool {
        self.correction_readings(scope, full_variants_wanted(scope))
            .iter()
            .any(|correction| {
                self.convert_sentence(&correction.segmentation.patterns())
                    .is_some_and(|conversion| !conversion.has_placeholder())
            })
    }

    /// 一处编辑的纠错读法：换位取 [`correction::loose_segmentation`]（末尾可没敲完）；
    /// 替换只留键盘相邻键、多敲 / 少敲只留还能完整切分的，都取 [`correction::complete_segmentation`]。
    fn correction_readings(&self, scope: &str, full: bool) -> Vec<Correction> {
        if !correction::eligible(scope) {
            return Vec::new();
        }
        let mut readings = Vec::new();
        for (edit, corrected) in correction::variants(scope) {
            let segmentation = match &edit {
                Edit::Transpose { .. } => {
                    // 先用不分配切分的可达性检查挡掉绝大多数变体，剩下的才做真正的切分
                    if !loosely_segmentable(&corrected) {
                        continue;
                    }
                    match correction::loose_segmentation(&corrected) {
                        Some(segmentation) => segmentation,
                        None => continue,
                    }
                }
                Edit::Substitute { index, from } if full => {
                    let to = corrected.as_bytes()[*index] as char;
                    // 只有敲到相邻键才算替换敲错
                    if !correction::typo::adjacent(*from, to) {
                        continue;
                    }
                    match fully_segmented(&corrected) {
                        Some(segmentation) => segmentation,
                        None => continue,
                    }
                }
                // 多敲 / 少敲：能完整切分才算（`kagn` 删掉 g 是 `kan`，`meiganxi` 补 u 是 `mei guan xi`）
                Edit::Delete { .. } | Edit::Insert { .. } if full => {
                    match fully_segmented(&corrected) {
                        Some(segmentation) => segmentation,
                        None => continue,
                    }
                }
                _ => continue,
            };
            readings.push(Correction {
                original: scope.to_owned(),
                corrected,
                edit,
                segmentation,
            });
        }
        readings
    }
}

/// 能完整切分就返回第一种切分；先用无分配的可达性检查挡掉绝大多数变体。
fn fully_segmented(text: &str) -> Option<crate::parser::Segmentation> {
    parser::is_fully_segmentable(text)
        .then(|| correction::complete_segmentation(text))
        .flatten()
}

/// 这个作用域的纠错读法要不要带「替换 / 多敲 / 少敲」类变体：拼音不像话时带，
/// 仅末尾落单字母时只试换位（省每键开销）。
fn full_variants_wanted(scope: &str) -> bool {
    match segment_longest_prefix(scope, true) {
        Ok((segmentations, tail)) => correction::unlikely_pinyin(segmentations.first(), tail),
        Err(_) => false,
    }
}

/// 能不能切成「每个音节都完整，或只有末尾一个没敲完且前面至少一个完整音节」（同 [`correction::loose_segmentation`]）。
/// 不分配切分，给换位变体先过滤用。
fn loosely_segmentable(text: &str) -> bool {
    if parser::is_fully_segmentable(text) {
        return true;
    }
    (1..text.len()).any(|split| {
        parser::is_fully_segmentable(&text[..split]) && parser::is_syllable_prefix(&text[split..])
    })
}

/// 一条纠错读法的权重折扣（见 [`CorrectionKind::discount`]）；原样输入本身对不上整段词 / 短语时
/// 再乘回去（见 [`CorrectionKind::backfill`]），让明显敲错的串更容易被纠。
pub(super) fn correction_factor(kind: CorrectionKind, original_matches: bool) -> f64 {
    let base = kind.discount();
    if original_matches {
        base
    } else {
        base * kind.backfill()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 原样 > 换位 > 替换 > 多敲/少敲；原样对不上时折扣往回收一点，但顺序不变。
    #[test]
    fn correction_discounts_are_ordered() {
        let exact = 1.0;
        let transpose = correction_factor(CorrectionKind::Transpose, true);
        let substitute = correction_factor(CorrectionKind::Substitute, true);
        let extra = correction_factor(CorrectionKind::Extra, true);
        assert_eq!(transpose, TRANSPOSE_FACTOR);
        assert_eq!(substitute, SUBSTITUTE_FACTOR);
        assert_eq!(extra, EXTRA_MISSING_FACTOR);
        assert!(exact > transpose && transpose > substitute && substitute > extra);
        // 回补只加不减：对不上整段时每一种都比对得上时更接近原样
        for kind in [
            CorrectionKind::Transpose,
            CorrectionKind::Substitute,
            CorrectionKind::Extra,
            CorrectionKind::Missing,
        ] {
            assert!(correction_factor(kind, false) > correction_factor(kind, true));
        }
    }
}
