//! 部件书写变体：同一个部件在 IDS 里的不同写法。
//!
//! BabelStone 的 IDS 数据大量使用偏旁形（`釒`、`訁`、`糹`、`飠`），用户在画布上拼的是
//! 独立成字的写法（`金`、`言`、`糸`、`食`）；只按字面比较会漏掉「鑫」这类字
//! （数据里是 `⿱金鍂`、`鍂` 是 `⿰釒金`）。这里只收「同一个部件的书写变体」，
//! 不收简繁对应字（那本来就是不同的字）。
//!
//! 只在精确查询未命中后的递归展开里比较叶子时使用，字面完全相同的命中不受影响。

/// `(变体, 基准)`：两个叶子规范到同一基准就算同一个部件。
const VARIANTS: &[(char, char)] = &[
    // 人 / 水 / 心 / 手
    ('亻', '人'),
    ('氵', '水'),
    ('氺', '水'),
    ('忄', '心'),
    ('㣺', '心'),
    ('扌', '手'),
    // 言 / 金 / 示 / 衣
    ('讠', '言'),
    ('訁', '言'),
    ('钅', '金'),
    ('釒', '金'),
    ('礻', '示'),
    ('衤', '衣'),
    // 刀 / 火 / 艸 / 辵
    ('刂', '刀'),
    ('灬', '火'),
    ('艹', '艸'),
    ('辶', '辵'),
    // 食 / 糸 / 爪 / 网
    ('饣', '食'),
    ('飠', '食'),
    ('纟', '糸'),
    ('糹', '糸'),
    ('爫', '爪'),
    ('罒', '网'),
];

/// 两个叶子部件是否是同一个：字面相同，或只是同一部件的不同写法。
pub fn same_component(left: char, right: char) -> bool {
    left == right || canonical(left) == canonical(right)
}

fn canonical(character: char) -> char {
    VARIANTS
        .iter()
        .find(|(variant, _)| *variant == character)
        .map_or(character, |(_, base)| *base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radical_forms_match_their_standalone_component() {
        // 「鑫」的数据写法：`⿱金鍂`，`鍂` 是 `⿰釒金`。
        assert!(same_component('釒', '金'));
        assert!(same_component('金', '釒'));
        assert!(same_component('钅', '釒'));
        assert!(same_component('氵', '水'));
        assert!(same_component('讠', '訁'));
        assert!(same_component('木', '木'));
    }

    #[test]
    fn different_components_stay_different() {
        assert!(!same_component('木', '水'));
        assert!(!same_component('金', '全'));
        assert!(!same_component('心', '必'));
    }
}
