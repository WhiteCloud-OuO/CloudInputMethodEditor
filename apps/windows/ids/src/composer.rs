//! 互动编辑器的状态树：嵌套槽位、当前槽、拆结构、填部件、生成 IDS 串。

use cloudime_ids::IdsOp;

/// 画布上的一个节点。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Node {
    /// 空槽，还没填。
    #[default]
    Empty,

    /// 一个已填部件。
    Char(char),

    /// `？` 通配。
    Wildcard,

    /// 结构运算符和它的子槽（`children.len() == op.arity()`）。
    Op { op: IdsOp, children: Vec<Node> },
}

impl Node {
    /// 按运算符元数生成全空的结构。
    fn empty_op(op: IdsOp) -> Self {
        Self::Op {
            op,
            children: vec![Self::Empty; op.arity()],
        }
    }
}

/// 一次插入的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    /// 填进了空槽。
    Inserted,

    /// 覆盖了原来的部件 / 通配符。
    Replaced,

    /// 当前是结构，填进了它下面第一个空叶子。
    FilledFirst,

    /// 没有可填的槽（结构已经填满）。
    NoSlot,
}

/// 表达式还不能查询。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    /// 画布完全是空的。
    Empty,

    /// 还有空槽没填。
    Unfilled,
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("画布是空的，先点结构或填一个部件"),
            Self::Unfilled => formatter.write_str("还有空槽没填完"),
        }
    }
}

/// 编辑器状态。
#[derive(Debug, Clone, Default)]
pub struct Composer {
    root: Node,
    active: Vec<usize>,
}

impl Composer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn root(&self) -> &Node {
        &self.root
    }

    /// 当前槽的路径；空表示根。
    pub fn active(&self) -> &[usize] {
        &self.active
    }

    /// 按路径取节点。
    pub fn node_at(&self, path: &[usize]) -> Option<&Node> {
        let mut node = &self.root;
        for &index in path {
            let Node::Op { children, .. } = node else {
                return None;
            };
            node = children.get(index)?;
        }
        Some(node)
    }

    /// 选中某个槽；路径越界就当没点。
    pub fn set_active(&mut self, path: Vec<usize>) {
        if self.node_at(&path).is_some() {
            self.active = path;
        }
    }

    /// 点结构按钮：把当前槽变成结构。
    ///
    /// 空槽 → 全是空子槽；已填部件 → 包成第一个子槽；已有结构 → 换成新结构。
    pub fn split(&mut self, op: IdsOp) {
        let path = self.active.clone();
        let tag = match self.node_at(&path) {
            Some(Node::Empty) | Some(Node::Op { .. }) | None => Node::empty_op(op),
            Some(node) => {
                let mut wrapped = Node::empty_op(op);
                if let Node::Op { children, .. } = &mut wrapped {
                    children[0] = node.clone();
                }
                wrapped
            }
        };
        if let Some(node) = self.node_mut(&path) {
            *node = tag;
        }
        match self.first_empty_under(&path) {
            Some(target) => self.active = target,
            None => {
                self.active = path;
                self.active.push(0);
            }
        }
    }

    /// 在当前槽填一个部件（或 `？`）；成功后自动跳到下一个空槽。
    pub fn insert(&mut self, character: Option<char>) -> InsertOutcome {
        let path = self.active.clone();
        let value = match character {
            Some(character) => Node::Char(character),
            None => Node::Wildcard,
        };
        let outcome = match self.node_at(&path) {
            Some(Node::Empty) => {
                if let Some(node) = self.node_mut(&path) {
                    *node = value;
                }
                InsertOutcome::Inserted
            }
            Some(Node::Char(_)) | Some(Node::Wildcard) => {
                if let Some(node) = self.node_mut(&path) {
                    *node = value;
                }
                InsertOutcome::Replaced
            }
            Some(Node::Op { .. }) | None => match self.first_empty_under(&path) {
                Some(target) => {
                    if let Some(node) = self.node_mut(&target) {
                        *node = value;
                    }
                    self.active = target;
                    InsertOutcome::FilledFirst
                }
                None => InsertOutcome::NoSlot,
            },
        };
        if outcome != InsertOutcome::NoSlot {
            self.advance();
        }
        outcome
    }

    /// 退格：当前槽清空；当前是结构就整棵折掉；已经是空槽则退回父槽。
    pub fn backspace(&mut self) {
        let mut path = self.active.clone();
        match self.node_at(&path) {
            Some(Node::Empty) => {
                path.pop();
                self.active = path;
            }
            Some(_) => {
                if let Some(node) = self.node_mut(&path) {
                    *node = Node::Empty;
                }
            }
            None => {}
        }
    }

    /// 清空整个画布。
    pub fn clear(&mut self) {
        self.root = Node::Empty;
        self.active.clear();
    }

    /// 生成规范化 IDS 表达式。
    pub fn to_ids(&self) -> Result<String, ComposeError> {
        if self.root == Node::Empty {
            return Err(ComposeError::Empty);
        }
        let mut text = String::new();
        write_node(&self.root, &mut text)?;
        Ok(text)
    }

    fn node_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        let mut node = &mut self.root;
        for &index in path {
            let Node::Op { children, .. } = node else {
                return None;
            };
            node = children.get_mut(index)?;
        }
        Some(node)
    }

    /// 当前结构下第一个空槽（深度优先）。
    fn first_empty_under(&self, path: &[usize]) -> Option<Vec<usize>> {
        let node = self.node_at(path)?;
        let mut found = Vec::new();
        find_empty(node, path, &mut found);
        found.into_iter().next()
    }

    /// 跳到当前槽之后的第一个空槽；没有就停在原地。
    fn advance(&mut self) {
        let mut empties = Vec::new();
        find_empty(&self.root, &[], &mut empties);
        if let Some(target) = empties
            .into_iter()
            .find(|candidate| candidate.as_slice() > self.active.as_slice())
        {
            self.active = target;
        }
    }
}

fn find_empty(node: &Node, path: &[usize], out: &mut Vec<Vec<usize>>) {
    match node {
        Node::Empty => out.push(path.to_vec()),
        Node::Op { children, .. } => {
            for (index, child) in children.iter().enumerate() {
                let mut child_path = path.to_vec();
                child_path.push(index);
                find_empty(child, &child_path, out);
            }
        }
        Node::Char(_) | Node::Wildcard => {}
    }
}

fn write_node(node: &Node, out: &mut String) -> Result<(), ComposeError> {
    match node {
        Node::Empty => Err(ComposeError::Unfilled),
        Node::Char(character) => {
            out.push(*character);
            Ok(())
        }
        Node::Wildcard => {
            out.push('？');
            Ok(())
        }
        Node::Op { op, children } => {
            out.push(op.symbol());
            for child in children {
                write_node(child, out)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_turns_an_empty_slot_into_structure() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftRight);
        assert_eq!(composer.active(), &[0]);
        assert!(matches!(
            composer.root(),
            Node::Op {
                op: IdsOp::LeftRight,
                children
            } if children.len() == 2
        ));
    }

    #[test]
    fn split_wraps_an_existing_component_and_replaces_structure() {
        let mut composer = Composer::new();
        composer.insert(Some('木'));
        assert_eq!(composer.node_at(composer.active()), Some(&Node::Char('木')));
        composer.split(IdsOp::AboveBelow);
        assert_eq!(
            composer.root(),
            &Node::Op {
                op: IdsOp::AboveBelow,
                children: vec![Node::Char('木'), Node::Empty]
            }
        );
        assert_eq!(composer.active(), &[1]);

        composer.split(IdsOp::SurroundUpperLeft);
        assert!(matches!(
            composer.node_at(&[1]),
            Some(Node::Op {
                op: IdsOp::SurroundUpperLeft,
                ..
            })
        ));
    }

    #[test]
    fn insert_fills_and_advances_in_depth_first_order() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftMiddleRight);
        assert_eq!(composer.active(), &[0]);
        assert_eq!(composer.insert(Some('木')), InsertOutcome::Inserted);
        assert_eq!(composer.active(), &[1]);
        assert_eq!(composer.insert(Some('木')), InsertOutcome::Inserted);
        assert_eq!(composer.active(), &[2]);
        composer.insert(Some('木'));
        assert_eq!(composer.to_ids().unwrap(), "⿲木木木");
        // 全填满后没有空槽，停在原地。
        assert_eq!(composer.insert(Some('林')), InsertOutcome::Replaced);
        assert_eq!(composer.to_ids().unwrap(), "⿲木木林");
    }

    #[test]
    fn insert_on_a_structure_fills_its_first_empty_child() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftRight);
        composer.set_active(Vec::new());
        assert_eq!(composer.insert(Some('木')), InsertOutcome::FilledFirst);
        assert_eq!(composer.to_ids(), Err(ComposeError::Unfilled));
        assert_eq!(composer.active(), &[1]);
    }

    #[test]
    fn insert_into_a_full_structure_reports_no_slot() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftRight);
        composer.insert(Some('木'));
        composer.insert(Some('水'));
        composer.set_active(Vec::new());
        assert_eq!(composer.insert(Some('林')), InsertOutcome::NoSlot);
        assert_eq!(composer.to_ids().unwrap(), "⿰木水");
    }

    #[test]
    fn wildcard_goes_through_the_same_path() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftRight);
        composer.insert(Some('木'));
        composer.insert(None);
        assert_eq!(composer.to_ids().unwrap(), "⿰木？");
    }

    #[test]
    fn backspace_clears_or_collapses_then_walks_up() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftRight);
        composer.insert(Some('木'));
        composer.insert(Some('水'));
        composer.backspace();
        assert_eq!(composer.to_ids(), Err(ComposeError::Unfilled));
        assert_eq!(composer.active(), &[1]);
        // [1] 是根的子槽：空了以后再退格就回到根。
        composer.backspace();
        assert_eq!(composer.active(), &[] as &[usize]);
        assert!(matches!(composer.root(), Node::Op { .. }));
        // 根是结构时，退格把整棵折掉。
        composer.backspace();
        assert_eq!(composer.root(), &Node::Empty);
    }

    #[test]
    fn clear_resets_everything() {
        let mut composer = Composer::new();
        composer.split(IdsOp::AboveBelow);
        composer.insert(Some('艹'));
        composer.clear();
        assert_eq!(composer.root(), &Node::Empty);
        assert_eq!(composer.to_ids(), Err(ComposeError::Empty));
    }

    #[test]
    fn incomplete_expressions_are_rejected() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftRight);
        composer.insert(Some('木'));
        assert_eq!(composer.to_ids(), Err(ComposeError::Unfilled));
        assert!(composer.to_ids().is_err());
    }

    #[test]
    fn nested_expression_serializes_to_ids() {
        let mut composer = Composer::new();
        composer.split(IdsOp::LeftRight);
        composer.insert(Some('木'));
        composer.split(IdsOp::AboveBelow);
        composer.insert(Some('艹'));
        composer.insert(Some('早'));
        assert_eq!(composer.to_ids().unwrap(), "⿰木⿱艹早");
    }
}
