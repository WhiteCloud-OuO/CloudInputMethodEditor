//! 页面树节点与布局参数。
//!
//! 节点只描述结构、尺寸和布局；控件内容由上层（脚本运行时）按 `NodeId` 另行保存。
//! 这样 `cloud-ui-core` 不依赖 JSON、Lua 或任何具体控件。

use crate::geometry::Size;
use crate::layout::{Align, Axis, Justify, LayoutKind, Position};

/// 页面树节点编号（`PageTree::nodes` 的下标）。
pub type NodeId = usize;

/// 节点类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// 窗口主页面根。
    Page,

    /// 无外观的逻辑分组。
    Stem,

    /// 有外观的容器控件（例如 `panel`）。
    Container,

    /// 叶子控件。
    Control,

    /// 另一个页面布局的实例；具体内容由上层提供。
    Subpage,
}

/// 一个节点的尺寸来源。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum SizeSpec {
    /// 由内容 / 子节点测量。
    #[default]
    Auto,

    /// 固定点值。
    Fixed(f32),

    /// 相对父节点 content box 的百分比，存 0..=1。
    Percent(f32),
}

/// 四边值（padding / margin，点）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub const fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    pub const fn horizontal(self) -> f32 {
        self.left + self.right
    }

    pub const fn vertical(self) -> f32 {
        self.top + self.bottom
    }
}

/// 绝对定位的四个锚点；`None` 表示没写。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Insets {
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
}

/// 网格轨道。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrack {
    Fixed(f32),
    Percent(f32),
    Auto,
    Fraction(f32),
}

/// 页面 / 容器的布局算法。
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutSpec {
    Linear {
        gap: f32,
    },
    Flex {
        direction: Axis,
        wrap: bool,
        justify: Justify,
        align: Align,
        gap: f32,
    },
    Grid {
        columns: Vec<GridTrack>,
        rows: Vec<GridTrack>,
        gap: f32,
    },
    Responsive {
        gap: f32,
        min_item_width: Option<f32>,
    },
}

impl Default for LayoutSpec {
    fn default() -> Self {
        Self::Flex {
            direction: Axis::Row,
            wrap: false,
            justify: Justify::Start,
            align: Align::Start,
            gap: 0.0,
        }
    }
}

impl LayoutSpec {
    pub fn kind(&self) -> LayoutKind {
        match self {
            Self::Linear { .. } => LayoutKind::Linear,
            Self::Flex { .. } => LayoutKind::Flex,
            Self::Grid { .. } => LayoutKind::Grid,
            Self::Responsive { .. } => LayoutKind::Responsive,
        }
    }
}

/// 页面树节点。
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,

    /// 控件 / 分组名；Page 和匿名 Stem 可以为 `None`。
    pub name: Option<String>,

    pub layout: LayoutSpec,
    pub padding: Edges,
    pub margin: Edges,
    pub width: SizeSpec,
    pub height: SizeSpec,
    pub min_width: f32,
    pub max_width: f32,
    pub min_height: f32,
    pub max_height: f32,
    pub grow: f32,
    pub shrink: f32,
    pub align_self: Option<Align>,
    pub position: Position,
    pub insets: Insets,
    pub visible: bool,
    pub grid_column: Option<usize>,
    pub grid_row: Option<usize>,
    pub column_span: usize,
    pub row_span: usize,
    pub children: Vec<NodeId>,
}

impl Node {
    pub fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            name: None,
            layout: LayoutSpec::default(),
            padding: Edges::default(),
            margin: Edges::default(),
            width: SizeSpec::Auto,
            height: SizeSpec::Auto,
            min_width: 0.0,
            max_width: f32::INFINITY,
            min_height: 0.0,
            max_height: f32::INFINITY,
            grow: 0.0,
            shrink: 1.0,
            align_self: None,
            position: Position::Flow,
            insets: Insets::default(),
            visible: true,
            grid_column: None,
            grid_row: None,
            column_span: 1,
            row_span: 1,
            children: Vec::new(),
        }
    }

    pub fn is_layout_host(&self) -> bool {
        matches!(
            self.kind,
            NodeKind::Page | NodeKind::Stem | NodeKind::Container
        )
    }

    pub fn content_width(&self, width: f32) -> f32 {
        (width - self.padding.horizontal()).max(0.0)
    }

    pub fn content_height(&self, height: f32) -> f32 {
        (height - self.padding.vertical()).max(0.0)
    }

    pub fn content_size(&self, size: Size) -> Size {
        Size::new(
            self.content_width(size.width),
            self.content_height(size.height),
        )
    }
}

/// 页面树：节点按编号放在 arena 里，根节点缺省为 0。
#[derive(Debug, Clone)]
pub struct PageTree {
    pub nodes: Vec<Node>,
    pub root: NodeId,
}

impl PageTree {
    pub fn new(root: Node) -> Self {
        Self {
            nodes: vec![root],
            root: 0,
        }
    }

    pub fn push(&mut self, node: Node) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(node);
        id
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    pub fn set_children(&mut self, id: NodeId, children: Vec<NodeId>) {
        self.nodes[id].children = children;
    }
}
