//! 候选排序。
//!
//! 词级排序分两级：先按**结构键**，同一结构下再按**权重**降序、文本升序。结构键必须先于权重：
//! 中文单字（是 = 334 万）与整词（输入法 = 1508）差两三个数量级，纯按权重排会让高频单字与模糊 /
//! 前缀扩展命中霸屏（`putonghua` 出 普、`xian` 出被当成没打完的 想）。权重只负责「同一档里谁更靠前」。
//!
//! 结构键（前面的压倒后面）：
//!
//! 1. 覆盖的输入字母数降序：完整覆盖整段输入的词优先（`kaif` 的 开发 先于只覆盖 `kai` 的 开）。
//! 2. 非末尾的简拼音节数升序：越少越像敲的原话（`kaifa` 按 `kai fa` 读的 开发 先于按 `kai f a` 读的）。
//! 3. 末音节完整匹配降序：`xian` 的 先（末音节 xian 就是敲的那个）先于被当成没打完的 想（xiang）。
//! 4. 词库命中的 `exact`（音节数正好等于查询位置数）降序。
//! 5. 读法层级升序：原样 > 模糊音 > 一处编辑纠错。
//!
//! 不再单独加「音节数少优先」：覆盖满且末音节对上（第 1、3 项）已经等价于「整段输入被完整读出来」，
//! `exact` 只是它下面的兜底键，重复一条只会让长词被短词无谓压过。
//!
//! 一个词级命中的权重是若干因子的乘积：
//!
//! ```text
//! weight = frequency                                        // 词库静态词频
//!        × learner.rank_weight(text)                        // 用户权重因子
//!        × (1 + learner.choice_weight(input_key, text))     // 同输入串选过次数
//!        × context_factor                                   // 上下文系数
//!        × correction_factor                                // 模糊音 / 纠错折扣
//!        × association_factor                               // 联想折扣 0.8^k
//! ```
//!
//! - 用户权重因子 `Learner::rank_weight`：自造词的权重就是它在用户词库里的词频（每次重选 ×1.2），
//!   其余词按全局重复次数每次 ×1.15；两者相乘替代了原来的「1 + 全局选过次数」。
//! - 上下文系数 = `clamp(exp(log P(词 | 上一个上屏词) − fallback), e^-4, e^4)`，`fallback` 是模型
//!   不认识这个词时的词频兜底；相等（模型不认识）时系数是 1。
//! - 模糊音命中的折扣是 `exp(-ln 2) = 0.5`；一处编辑的纠错读法由 Engine 定（换位 / 相邻键替换），
//!   音节级敲错边（多敲 / 少敲，只进整句词图）按 `correction::TypoCosts` 扣。
//! - 联想折扣 `0.8^k`：词比产生它的读法多 `k` 个字（查前缀时带出的更长的词），少字不算（`k` 取 0）。
//! - 预选只按便宜的因子（词频 × 选择次数 × 纠错折扣 × 联想折扣），命中太多时先砍到够排的量，不查上下文。

mod scored;

use std::cmp::Ordering;
use std::collections::HashSet;

pub use scored::Scored;

use crate::sentence::fallback_log_prob;

/// 联想折扣：查前缀带出的更长词每多一个字打这个折。
pub const ASSOCIATION_DISCOUNT: f64 = 0.8;

/// 模糊音命中的折扣（对数形式）：一个模糊音节扣 ln 2，换算成权重因子就是 0.5。
/// 词级权重取 `exp(-penalty)`，整句词图里也按它扣分。
pub const FUZZY_PENALTY: f64 = std::f64::consts::LN_2;

/// 上下文系数与模型系数的下上限（`e^-4` / `e^4`）：模型不认识或差得很远时最多把权重压到 1/55 或抬到 55 倍。
pub fn clamp_factor(value: f64) -> f64 {
    value.clamp((-4.0f64).exp(), 4.0f64.exp())
}

/// 联想折扣：`text` 比产生它的读法多出的字数每字乘 [`ASSOCIATION_DISCOUNT`]；不多或更短返回 1。
pub fn association_factor(text: &str, reading_syllables: usize) -> f64 {
    let extra = text.chars().count().saturating_sub(reading_syllables);
    ASSOCIATION_DISCOUNT.powi(i32::try_from(extra).unwrap_or(i32::MAX))
}

/// 排序并按词文本去重（同一个词可能被多种切分命中，保留名次最高的一条），最多留 `limit` 条。
///
/// 排序键（前面的压倒后面，同键才比下一项），顺序见模块文档：
/// 覆盖字母数降序 → 非末尾简拼数升序 → 末音节完整匹配降序 → 命中 `exact` 降序 →
/// 读法层级升序 → 权重降序 → 文本升序。
///
/// `log_total` 是全部词库词频之和的对数；`context` 给剩下的那些算（同输入串选择次数, 上下文 log 概率）。
/// `association_limit` 是「联想候选」（词比读法更长）最多留几条（`[candidate] candidate_association_counts`）。
/// 返回每条命中的权重，供与整句 / 英文候选合并排序。
pub fn rank<'a>(
    items: &mut Vec<Scored<'a>>,
    limit: usize,
    association_limit: usize,
    log_total: f64,
    context: impl Fn(&Scored<'a>) -> (u32, f64),
) -> Vec<(Scored<'a>, f64)> {
    // 远超上限时先按「结构 + 便宜权重」选出前面一段：同一个词会被多种切分命中，多选一倍留给去重。
    // 结构键必须先于权重参与预选，否则高频单字会把完整匹配挤出去。
    let preselect = limit.saturating_mul(2);
    if items.len() > preselect.saturating_mul(2) {
        let mut keyed: Vec<(Scored<'a>, f64)> = items
            .drain(..)
            .map(|item| {
                let weight = item.preselect_weight();
                (item, weight)
            })
            .collect();
        keyed.select_nth_unstable_by(preselect, compare_structural);
        keyed.truncate(preselect);
        items.extend(keyed.into_iter().map(|(item, _)| item));
    }
    let mut keyed: Vec<(f64, &str, Scored<'a>)> = items
        .drain(..)
        .map(|item| {
            let (choice, log_prob) = context(&item);
            let fallback = fallback_log_prob(item.hit.frequency, log_total);
            let context_factor = clamp_factor((log_prob - fallback).exp());
            let weight = item.weight(choice, context_factor);
            (weight, item.hit.text, item)
        })
        .collect();
    keyed.sort_unstable_by(|a, b| {
        b.2.coverage
            .cmp(&a.2.coverage)
            .then_with(|| a.2.abbreviated.cmp(&b.2.abbreviated))
            .then_with(|| b.2.full_last.cmp(&a.2.full_last))
            .then_with(|| b.2.hit.exact.cmp(&a.2.hit.exact))
            .then_with(|| a.2.tier.cmp(&b.2.tier))
            .then_with(|| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal))
            .then_with(|| a.1.cmp(b.1))
    });
    let mut seen: HashSet<&str> = HashSet::with_capacity(limit.min(keyed.len()));
    let mut associations = 0usize;
    keyed
        .into_iter()
        .filter(|(_, text, _)| seen.insert(*text))
        // 联想候选（词比产生它的读法更长）另限个数：单字母输入这类能到几千条，会把要选的字顶出窗口
        .filter(|(_, _, item)| {
            if !is_association(item) {
                return true;
            }
            associations += 1;
            associations <= association_limit
        })
        .take(limit)
        .map(|(weight, _, item)| (item, weight))
        .collect()
}

/// 这条命中是不是「联想候选」：词覆盖的音节比产生它的读法多（`kaifa` 里按前缀带出的 开发者）。
/// 前缀词（输入更长、词只覆盖前面一段）不算——它是给逐词上屏用的，不是联想。
fn is_association(item: &Scored<'_>) -> bool {
    item.hit.syllable_count() > item.reading_syllables
}

/// 预选比较器，与 [`rank`] 的排序键同序（结构键 → 便宜权重）。
fn compare_structural(a: &(Scored<'_>, f64), b: &(Scored<'_>, f64)) -> Ordering {
    b.0.coverage
        .cmp(&a.0.coverage)
        .then_with(|| a.0.abbreviated.cmp(&b.0.abbreviated))
        .then_with(|| b.0.full_last.cmp(&a.0.full_last))
        .then_with(|| b.0.hit.exact.cmp(&a.0.hit.exact))
        .then_with(|| a.0.tier.cmp(&b.0.tier))
        .then_with(|| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloudime_dictionary::Match;

    /// 让上下文系数恰好为 1 的 `log_total`：闭包返回 fallback 本身。
    const LOG_TOTAL: f64 = 1.0;

    fn fallback(frequency: u32) -> f64 {
        crate::sentence::fallback_log_prob(frequency, LOG_TOTAL)
    }

    fn hit<'a>(text: &'a str, pinyin: &'a str, frequency: u32, exact: bool) -> Match<'a> {
        Match {
            text,
            pinyin,
            frequency,
            exact,
        }
    }

    fn scored<'a>(
        text: &'a str,
        pinyin: &'a str,
        frequency: u32,
        reading_syllables: usize,
        weight: u32,
        correction: f64,
    ) -> Scored<'a> {
        Scored {
            hit: hit(text, pinyin, frequency, true),
            coverage: pinyin.replace(' ', "").len(),
            full_last: true,
            abbreviated: 0,
            reading_syllables,
            tier: if correction >= 1.0 {
                0
            } else if correction == 0.5 {
                1
            } else {
                2
            },
            weight: 1.0 + f64::from(weight),
            correction,
        }
    }

    #[test]
    fn coverage_beats_frequency() {
        // 覆盖字母多的先，哪怕词频低得多（`putonghua` 不能被只覆盖 `pu` 的 普 顶掉）
        let mut items = vec![
            scored("开发者", "kai fa zhe", 99999, 3, 0, 1.0),
            scored("开放", "kai fang", 20000, 2, 0, 1.0),
            scored("开发", "kai fa", 9000, 2, 0, 1.0),
        ];
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency))
        });
        let texts: Vec<&str> = ranked.iter().map(|(s, _)| s.hit.text).collect();
        assert_eq!(texts, ["开发者", "开放", "开发"]);
    }

    fn structural<'a>(
        text: &'a str,
        pinyin: &'a str,
        frequency: u32,
        coverage: usize,
        full_last: bool,
        abbreviated: usize,
    ) -> Scored<'a> {
        Scored {
            hit: hit(text, pinyin, frequency, true),
            coverage,
            full_last,
            abbreviated,
            reading_syllables: pinyin.split(' ').count(),
            tier: 0,
            weight: 1.0,
            correction: 1.0,
        }
    }

    #[test]
    fn full_last_beats_frequency_among_the_same_coverage() {
        // `xian`：先（末音节 xian 就是敲的那个）压过被当成没打完的 想（xiang），哪怕 想 词频高得多
        let mut items = vec![
            structural("想", "xiang", 9_000_000, 4, false, 0),
            structural("先", "xian", 100_000, 4, true, 0),
        ];
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency))
        });
        assert_eq!(ranked[0].0.hit.text, "先");
    }

    #[test]
    fn fewer_abbreviated_syllables_rank_first() {
        // 同一个词按两种切法命中：`kai fa` 读法（非末尾简拼 0）先于 `kai f a` 读法（1）
        let mut items = vec![
            structural("开放啊", "kai fang a", 5000, 7, true, 1),
            structural("开放", "kai fang", 4000, 7, true, 0),
        ];
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency))
        });
        assert_eq!(ranked[0].0.hit.text, "开放");
    }

    #[test]
    fn tier_dominates_and_frequency_orders_within_a_tier() {
        // 同一层级内词频高者先
        let mut items = vec![
            scored("高频模糊", "ba", 5_000_000, 1, 0, 0.5),
            scored("低频模糊", "ba", 2_000_000, 1, 0, 0.5),
        ];
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency))
        });
        assert_eq!(ranked[0].0.hit.text, "高频模糊");
        // 层级优先：原样哪怕词频只有零头也压过模糊（`xian` 的 先 不被 想 顶掉）
        let mut items = vec![
            scored("原样", "ba", 100_000, 1, 0, 1.0),
            scored("模糊", "ba", 5_000_000, 1, 0, 0.5),
        ];
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency))
        });
        assert_eq!(ranked[0].0.hit.text, "原样");
    }

    #[test]
    fn context_and_choice_are_multiplicative_factors() {
        let build = || {
            vec![
                scored("把", "ba", 3_000_000, 1, 0, 1.0),
                scored("吧", "ba", 2_000_000, 1, 0, 1.0),
            ]
        };
        // 上下文说 吧 更像：词频高的 把 让位
        let mut items = build();
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (
                0,
                fallback(s.hit.frequency) + if s.hit.text == "吧" { -1.0 } else { -6.0 },
            )
        });
        assert_eq!(ranked[0].0.hit.text, "吧");
        // 同输入串下选过 把 很多次：(1+choice) 因子压过上下文
        let mut items = build();
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (
                if s.hit.text == "把" { 100 } else { 0 },
                fallback(s.hit.frequency) + if s.hit.text == "吧" { -1.0 } else { -6.0 },
            )
        });
        assert_eq!(ranked[0].0.hit.text, "把");
    }

    #[test]
    fn association_discount_grows_with_extra_characters() {
        assert_eq!(association_factor("开发", 2), 1.0);
        assert_eq!(association_factor("开发者", 2), ASSOCIATION_DISCOUNT);
        assert!((association_factor("普通话水平等级考试", 3) - 0.8f64.powi(6)).abs() < 1e-12);
        // 比读法短的前缀词不打折
        assert_eq!(association_factor("开", 2), 1.0);
    }

    #[test]
    fn context_factor_is_capped_at_e_pm_four() {
        let mut items = vec![scored("开发", "kai fa", 1000, 2, 0, 1.0)];
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency) + 1000.0)
        });
        assert!((ranked[0].1 - 1000.0 * 4.0f64.exp()).abs() < 1e-6);
        let mut items = vec![scored("开发", "kai fa", 1000, 2, 0, 1.0)];
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency) - 1000.0)
        });
        assert!((ranked[0].1 - 1000.0 * (-4.0f64).exp()).abs() < 1e-6);
    }

    #[test]
    fn association_limit_caps_the_longer_words() {
        // 联想候选（词比读法长）只留 `association_limit` 条，原样命中不受限
        let mut items = vec![
            scored("开", "kai", 100, 1, 0, 1.0),
            scored("开发", "kai fa", 90, 1, 0, 1.0),
            scored("开发票", "kai fa piao", 80, 1, 0, 1.0),
        ];
        let ranked = rank(&mut items, usize::MAX, 1, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency))
        });
        let texts: Vec<&str> = ranked.iter().map(|(s, _)| s.hit.text).collect();
        // 覆盖字母多的联想先占名额，原样的 开 一直留着
        assert_eq!(texts, ["开发票", "开"]);
        // 0 表示不显示联想候选
        let mut items = vec![
            scored("开", "kai", 100, 1, 0, 1.0),
            scored("开发", "kai fa", 90, 1, 0, 1.0),
        ];
        let ranked = rank(&mut items, usize::MAX, 0, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency))
        });
        let texts: Vec<&str> = ranked.iter().map(|(s, _)| s.hit.text).collect();
        assert_eq!(texts, ["开"]);
    }

    #[test]
    fn deduplicates_by_text_keeping_the_highest_weight() {
        let mut items = vec![
            scored("西安", "xi an", 4000, 2, 0, 1.0),
            scored("西安", "xi an", 4000, 2, 0, 0.5),
        ];
        let ranked = rank(&mut items, usize::MAX, usize::MAX, LOG_TOTAL, |s| {
            (0, fallback(s.hit.frequency))
        });
        assert_eq!(ranked.len(), 1);
        assert!((ranked[0].1 - 4000.0).abs() < 1e-6);
    }
}
