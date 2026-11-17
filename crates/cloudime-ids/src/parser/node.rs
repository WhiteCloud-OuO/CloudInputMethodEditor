use crate::parser::op::IdsOp;

/// 解析后的 IDS 结构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdsNode {
    /// 一个已编码部件（汉字 / 部首 / 笔画）。
    Leaf(char),

    /// `{n}`：没有对应 Unicode 字的部件。
    Unencoded(u8),

    /// `？`：任意单个子树（只在查询里出现）。
    Wildcard,

    /// 一个运算符和它的子节点，`children.len() == op.arity()`。
    Op { op: IdsOp, children: Vec<IdsNode> },
}

impl IdsNode {
    /// 规范化前缀表达式：运算符用符号，通配符用全角 `？`，未编码部件用 `{n}`。
    pub fn to_ids(&self) -> String {
        let mut text = String::new();
        self.write_ids(&mut text);
        text
    }

    /// 是否含 `？`。
    pub fn contains_wildcard(&self) -> bool {
        match self {
            Self::Wildcard => true,
            Self::Leaf(_) | Self::Unencoded(_) => false,
            Self::Op { children, .. } => children.iter().any(Self::contains_wildcard),
        }
    }

    /// 第一个未编码部件编号。
    pub fn first_unencoded(&self) -> Option<u8> {
        match self {
            Self::Unencoded(number) => Some(*number),
            Self::Op { children, .. } => children.iter().find_map(Self::first_unencoded),
            Self::Leaf(_) | Self::Wildcard => None,
        }
    }

    fn write_ids(&self, out: &mut String) {
        match self {
            Self::Leaf(character) => out.push(*character),
            Self::Unencoded(number) => {
                out.push('{');
                out.push_str(&number.to_string());
                out.push('}');
            }
            Self::Wildcard => out.push('？'),
            Self::Op { op, children } => {
                out.push(op.symbol());
                for child in children {
                    child.write_ids(out);
                }
            }
        }
    }
}
