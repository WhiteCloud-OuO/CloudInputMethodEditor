//! IDS 表达式解析：标准符号、两字母缩写、`？` 通配与 `{n}` 未编码部件。
//!
//! 解析是前缀递归下降：遇到运算符就按元数取子节点，叶子是单个字符。整串必须正好消费完。

mod node;
pub(crate) mod op;

use crate::error::ParseError;

pub use node::IdsNode;
pub use op::IdsOp;

/// 解析一个 IDS 表达式。
///
/// 允许开头的 `〾`（变体标记），解析时直接去掉；`？` / `?` / `⬚` 都当通配符。
pub fn parse(input: &str) -> Result<IdsNode, ParseError> {
    let text = input.trim();
    if text.is_empty() {
        return Err(ParseError::Empty);
    }
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0;
    let node = parse_node(&characters, &mut index, None)?;
    if index < characters.len() {
        return Err(ParseError::ExtraInput(characters[index..].iter().collect()));
    }
    Ok(node)
}

/// 规范化：解析再写回，顺便展开缩写、统一通配符、去掉 `〾`。
pub fn normalize(input: &str) -> Result<String, ParseError> {
    Ok(parse(input)?.to_ids())
}

/// 原串是不是带 `〾` 变体标记（可以出现在任意子树前）。
pub fn is_variation(input: &str) -> bool {
    input.contains('〾')
}

fn parse_node(
    characters: &[char],
    index: &mut usize,
    parent: Option<IdsOp>,
) -> Result<IdsNode, ParseError> {
    let Some(&current) = characters.get(*index) else {
        return Err(ParseError::Incomplete { op: parent });
    };

    let operator = IdsOp::from_char(current).or_else(|| abbreviation_at(characters, *index));
    if let Some(op) = operator {
        *index += if IdsOp::from_char(current).is_some() {
            1
        } else {
            2
        };
        let mut children = Vec::with_capacity(op.arity());
        for _ in 0..op.arity() {
            children.push(parse_node(characters, index, Some(op))?);
        }
        return Ok(IdsNode::Op { op, children });
    }

    match current {
        '？' | '?' | '⬚' => {
            *index += 1;
            Ok(IdsNode::Wildcard)
        }
        // `〾` 是变体标记，可以前缀在任意子树上（BabelStone 数据里就有
        // `⿸{13}〾⿻一㇄` 这种写法）；去掉标记，结构按后面的节点算。
        '〾' => {
            *index += 1;
            parse_node(characters, index, parent)
        }
        '{' => parse_unencoded(characters, index),
        leaf => {
            *index += 1;
            Ok(IdsNode::Leaf(leaf))
        }
    }
}

fn abbreviation_at(characters: &[char], index: usize) -> Option<IdsOp> {
    let first = *characters.get(index)?;
    let second = *characters.get(index + 1)?;
    if !first.is_ascii_uppercase() || !second.is_ascii_uppercase() {
        return None;
    }
    IdsOp::from_abbreviation_bytes([first as u8, second as u8])
}

fn parse_unencoded(characters: &[char], index: &mut usize) -> Result<IdsNode, ParseError> {
    let start = *index;
    *index += 1;
    let mut digits = String::new();
    while let Some(&current) = characters.get(*index) {
        if current == '}' {
            break;
        }
        if !current.is_ascii_digit() {
            return Err(ParseError::BadUnencoded(
                characters[start..=*index].iter().collect(),
            ));
        }
        digits.push(current);
        *index += 1;
    }
    if characters.get(*index) != Some(&'}') || digits.is_empty() {
        return Err(ParseError::BadUnencoded(
            characters[start..].iter().collect(),
        ));
    }
    *index += 1;
    let raw: String = characters[start..*index].iter().collect();
    let number: u8 = digits
        .parse()
        .map_err(|_| ParseError::BadUnencoded(raw.clone()))?;
    if number == 0 {
        return Err(ParseError::BadUnencoded(raw));
    }
    Ok(IdsNode::Unencoded(number))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(op: IdsOp, children: Vec<IdsNode>) -> IdsNode {
        IdsNode::Op { op, children }
    }

    #[test]
    fn parses_nested_expression_and_round_trips() {
        let node = parse("⿰木⿱艹早").unwrap();
        let expected = op(
            IdsOp::LeftRight,
            vec![
                IdsNode::Leaf('木'),
                op(
                    IdsOp::AboveBelow,
                    vec![IdsNode::Leaf('艹'), IdsNode::Leaf('早')],
                ),
            ],
        );
        assert_eq!(node, expected);
        assert_eq!(node.to_ids(), "⿰木⿱艹早");
        assert_eq!(normalize("⿰木⿱艹早").unwrap(), "⿰木⿱艹早");
    }

    #[test]
    fn expands_two_letter_abbreviations() {
        assert_eq!(normalize("LR木木").unwrap(), "⿰木木");
        assert_eq!(normalize("UD艹早").unwrap(), "⿱艹早");
        assert_eq!(
            normalize("UULLLR").unwrap_err(),
            ParseError::Incomplete {
                op: Some(IdsOp::LeftRight)
            }
        );
        assert_eq!(normalize("LL木木木").unwrap(), "⿲木木木");
    }

    #[test]
    fn strips_variation_marker_and_normalizes_wildcards() {
        assert!(is_variation("〾⿻一乚"));
        assert_eq!(normalize("〾⿻一乚").unwrap(), "⿻一乚");
        assert_eq!(normalize("⿰木?").unwrap(), "⿰木？");
        assert_eq!(normalize("⿰木⬚").unwrap(), "⿰木？");

        // 数据里 `〾` 也可以前缀在子树上（`⿸{13}〾⿻一㇄`）。
        assert!(is_variation("⿸{13}〾⿻一㇄"));
        assert_eq!(normalize("⿸{13}〾⿻一㇄").unwrap(), "⿸{13}⿻一㇄");
        assert_eq!(
            parse("⿰木〾").unwrap_err(),
            ParseError::Incomplete {
                op: Some(IdsOp::LeftRight)
            }
        );
    }

    #[test]
    fn keeps_unencoded_parts() {
        let node = parse("⿱{1}口").unwrap();
        assert_eq!(node.first_unencoded(), Some(1));
        assert_eq!(node.to_ids(), "⿱{1}口");
        assert_eq!(
            parse("⿱{0}口").unwrap_err(),
            ParseError::BadUnencoded("{0}".to_owned())
        );
        assert!(matches!(
            parse("⿱{x}口").unwrap_err(),
            ParseError::BadUnencoded(_)
        ));
    }

    #[test]
    fn reports_incomplete_and_extra_input() {
        assert_eq!(
            parse("⿰木").unwrap_err(),
            ParseError::Incomplete {
                op: Some(IdsOp::LeftRight)
            }
        );
        assert_eq!(
            parse("⿰木木木").unwrap_err(),
            ParseError::ExtraInput("木".to_owned())
        );
        assert_eq!(parse("").unwrap_err(), ParseError::Empty);
        assert_eq!(parse("   ").unwrap_err(), ParseError::Empty);
        assert_eq!(
            parse("⿾").unwrap_err(),
            ParseError::Incomplete {
                op: Some(IdsOp::HorizontalReflection)
            }
        );
    }

    #[test]
    fn wildcard_and_unencoded_helpers_scan_the_tree() {
        let wildcard = parse("⿰木？").unwrap();
        assert!(wildcard.contains_wildcard());
        assert_eq!(wildcard.first_unencoded(), None);
        assert!(!parse("⿰木木").unwrap().contains_wildcard());
        assert_eq!(parse("⿰{2}{3}").unwrap().first_unencoded(), Some(2));
    }

    #[test]
    fn real_data_shapes_parse() {
        // 取自 BabelStone IDS.TXT 的几类写法：多来源、变体、未编码件、包围、减除。
        for text in [
            "⿰木木",
            "⿱艹早",
            "⿰氵毎",
            "⿸丆𠃌",
            "⿹{22}一",
            "⿻一乚",
            "⿱{1}口",
            "⿰木⿱一丨",
            "⿸{13}〾⿻一㇄",
        ] {
            let node = parse(text).unwrap();
            assert_eq!(normalize(text).unwrap(), node.to_ids());
        }
    }
}
