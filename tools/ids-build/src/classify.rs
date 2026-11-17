//! 部件位置分类：从 IDS 树统计「圍 / 頭 / 旁 / 底 / 其它」的分布。
//!
//! 每个字只算一次：同一个部件在同一个字里出现多次时取最大权重，避免 `森` / `⿲木木木`
//! 把「木」刷成三倍。权重随深度衰减；包围结构里的「被围内容」按其它计、再打 5 折。

use std::collections::HashMap;

use cloudime_ids::{ComponentRole, IdsNode, IdsOp, parse};

use crate::read::Rows;

/// 一个部件的分类结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    /// 五个分类的千分比，总和 1000。
    pub roles: [u16; 5],

    /// 主分类下标（0..=4）。
    pub primary: u8,
}

/// 统计所有 IDS 树，得到每个部件的位置分布。
pub fn classify(rows: &Rows) -> HashMap<char, Classification> {
    let mut samples: HashMap<char, [f32; 5]> = HashMap::new();
    for (ids, characters) in &rows.keys {
        let Ok(tree) = parse(ids) else {
            continue;
        };
        for _ in characters.keys() {
            // 同一个字内：同一个部件在同一分类出现多次只算一次（取最大权重）；
            // 跨字：把每个字的最大权重累加，得到使用频度。
            let mut local: HashMap<char, [f32; 5]> = HashMap::new();
            walk(
                &tree,
                ComponentRole::Other,
                1.0,
                0,
                &mut |leaf, role, weight| {
                    let slot = &mut local.entry(leaf).or_insert([0f32; 5])[role.index() as usize];
                    if weight > *slot {
                        *slot = weight;
                    }
                },
            );
            for (leaf, weights) in local {
                let merged = samples.entry(leaf).or_insert([0f32; 5]);
                for (slot, value) in merged.iter_mut().zip(weights) {
                    *slot += value;
                }
            }
        }
    }

    samples
        .into_iter()
        .map(|(character, weights)| (character, normalize(&weights)))
        .collect()
}

/// 递归遍历：`role` 是当前节点在自己父节点里的位置，`discount` 是包围内缩的额外折扣。
fn walk(
    node: &IdsNode,
    role: ComponentRole,
    discount: f32,
    depth: usize,
    visit: &mut impl FnMut(char, ComponentRole, f32),
) {
    match node {
        IdsNode::Leaf(character) => {
            visit(*character, role, discount / (1 + depth) as f32);
        }
        IdsNode::Unencoded(_) | IdsNode::Wildcard => {}
        IdsNode::Op { op, children } => {
            for (index, child) in children.iter().enumerate() {
                let (child_role, child_discount) = child_slot(*op, index, role, discount);
                walk(child, child_role, child_discount, depth + 1, visit);
            }
        }
    }
}

/// 某个子节点在父运算符里的位置。
fn child_slot(
    op: IdsOp,
    index: usize,
    inherited: ComponentRole,
    discount: f32,
) -> (ComponentRole, f32) {
    use ComponentRole::{Bottom, Other, Side, Surround, Top};
    match op {
        IdsOp::LeftRight | IdsOp::LeftMiddleRight => (Side, discount),
        IdsOp::AboveBelow => {
            if index == 0 {
                (Top, discount)
            } else {
                (Bottom, discount)
            }
        }
        IdsOp::AboveMiddleBelow => match index {
            0 => (Top, discount),
            1 => (Other, discount),
            _ => (Bottom, discount),
        },
        IdsOp::FullSurround
        | IdsOp::SurroundAbove
        | IdsOp::SurroundBelow
        | IdsOp::SurroundLeft
        | IdsOp::SurroundUpperLeft
        | IdsOp::SurroundUpperRight
        | IdsOp::SurroundLowerLeft
        | IdsOp::SurroundRight
        | IdsOp::SurroundLowerRight => {
            if index == 0 {
                (Surround, discount)
            } else {
                // 「内」不计入围；按其它计，并且权重打 5 折。
                (Other, discount * 0.5)
            }
        }
        IdsOp::Overlaid => (Other, discount),
        IdsOp::Subtraction => {
            if index == 0 {
                (inherited, discount)
            } else {
                (Other, discount)
            }
        }
        IdsOp::HorizontalReflection | IdsOp::Rotation => (inherited, discount),
    }
}

fn normalize(weights: &[f32; 5]) -> Classification {
    let total: f32 = weights.iter().sum();
    if total <= 0.0 {
        return Classification {
            roles: [0, 0, 0, 0, 1000],
            primary: ComponentRole::Other.index(),
        };
    }
    let mut roles = [0u16; 5];
    for (index, weight) in weights.iter().enumerate() {
        roles[index] = (weight / total * 1000.0).round() as u16;
    }
    // 四舍五入可能多 1 或少 1，补到分值最大的桶上，保证总和正好 1000。
    let sum: u16 = roles.iter().sum();
    let difference = 1000i32 - i32::from(sum);
    if difference != 0
        && let Some((index, _)) = roles.iter().enumerate().max_by_key(|(_, score)| **score)
    {
        roles[index] = (i32::from(roles[index]) + difference).clamp(0, 1000) as u16;
    }

    let primary = choose_primary(&roles);
    Classification { roles, primary }
}

/// 主分类：分值最高者；平票按 旁 > 頭 > 底 > 圍 > 其它；最高不到 300‰ 归其它。
fn choose_primary(roles: &[u16; 5]) -> u8 {
    const PREFERENCE: [usize; 5] = [2, 1, 3, 0, 4];
    let mut best = ComponentRole::Other.index() as usize;
    let mut best_score = 0u16;
    for index in PREFERENCE {
        if roles[index] > best_score {
            best = index;
            best_score = roles[index];
        }
    }
    if best_score < 300 {
        return ComponentRole::Other.index();
    }
    best as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str) -> Rows {
        crate::read::parse(text).unwrap()
    }

    fn primary_of(rows: &Rows, character: char) -> Classification {
        let all = classify(rows);
        *all.get(&character).unwrap()
    }

    #[test]
    fn assigns_positions_from_surrounding_operators() {
        let sample = rows(
            "U+6797\t林\t^⿰木木$(G)\n\
             U+8349\t草\t^⿱艹早$(G)\n\
             U+56FD\t国\t^⿴囗玉$(G)\n\
             U+70B9\t点\t^⿱占灬$(G)\n\
             U+5305\t包\t^⿹勹巳$(G)\n",
        );
        assert_eq!(
            primary_of(&sample, '木').primary,
            ComponentRole::Side.index()
        );
        assert_eq!(
            primary_of(&sample, '艹').primary,
            ComponentRole::Top.index()
        );
        assert_eq!(
            primary_of(&sample, '囗').primary,
            ComponentRole::Surround.index()
        );
        assert_eq!(
            primary_of(&sample, '灬').primary,
            ComponentRole::Bottom.index()
        );
        // 被围内容不算圍，只能算其它。
        assert_eq!(
            primary_of(&sample, '巳').primary,
            ComponentRole::Other.index()
        );
    }

    #[test]
    fn inner_content_is_discounted_against_a_real_surrounding_use() {
        // 囗 只做包围；玉既做被围内容、又做左旁，左旁应当赢。
        let sample = rows(
            "U+56FD\t国\t^⿴囗玉$(G)\n\
             U+73A9\t玩\t^⿰王玉$(G)\n",
        );
        let jade = primary_of(&sample, '玉');
        assert_eq!(jade.primary, ComponentRole::Side.index());
        assert_eq!(jade.roles[0], 0);
        assert!(jade.roles[2] > jade.roles[4]);
    }

    #[test]
    fn middle_of_three_rows_is_other() {
        let sample = rows("U+4EAB\t享\t^⿳亠口子$(G)\n");
        assert_eq!(
            primary_of(&sample, '亠').primary,
            ComponentRole::Top.index()
        );
        assert_eq!(
            primary_of(&sample, '口').primary,
            ComponentRole::Other.index()
        );
        assert_eq!(
            primary_of(&sample, '子').primary,
            ComponentRole::Bottom.index()
        );
    }

    #[test]
    fn subtraction_base_inherits_the_parent_role() {
        let sample = rows("U+6797\t林\t^⿰木㇯一丨$(G)\n");
        assert_eq!(
            primary_of(&sample, '木').primary,
            ComponentRole::Side.index()
        );
        assert_eq!(
            primary_of(&sample, '一').primary,
            ComponentRole::Side.index()
        );
        assert_eq!(
            primary_of(&sample, '丨').primary,
            ComponentRole::Other.index()
        );
    }

    #[test]
    fn duplicate_occurrences_in_one_character_count_once() {
        let triple = rows("U+68EE\t森\t^⿲木木木$(G)\n");
        let single = rows("U+6797\t林\t^⿰木木$(G)\n");
        assert_eq!(primary_of(&triple, '木'), primary_of(&single, '木'));
    }

    #[test]
    fn scores_are_permille_and_sum_to_one_thousand() {
        let sample = rows(
            "U+6797\t林\t^⿰木木$(G)\n\
             U+6821\t校\t^⿰木交$(G)\n\
             U+76F8\t相\t^⿰木目$(G)\n",
        );
        let classification = primary_of(&sample, '木');
        assert_eq!(
            classification
                .roles
                .iter()
                .map(|v| u32::from(*v))
                .sum::<u32>(),
            1000
        );
        assert!(classification.roles[ComponentRole::Side.index() as usize] > 900);
    }

    #[test]
    fn scattered_distribution_falls_back_to_other() {
        // 一 在四个不同位置各出现一次，没有一种占绝对多数。
        let sample = rows(
            "U+6797\t林\t^⿰木一$(G)\n\
             U+6749\t杉\t^⿱木一$(G)\n\
             U+4E0A\t上\t^⿱一木$(G)\n\
             U+672A\t未\t^⿳木一木$(G)\n",
        );
        assert_eq!(
            primary_of(&sample, '一').primary,
            ComponentRole::Other.index()
        );
    }
}
