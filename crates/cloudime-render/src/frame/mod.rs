//! 一帧要画的全部内容：顶部拼音行、候选行、高亮、页脚、右侧状态。只是展示形态，不含排序或查词。

mod highlight;
mod preedit;
mod row;
mod tone;

pub use highlight::{HighlightAnimation, HighlightRect};
pub use preedit::{Preedit, PreeditSegment, PreeditStyle};
pub use row::Row;
pub use tone::Tone;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    /// 顶部拼音行；配置成只在行内显示时为 `None`。
    pub preedit: Option<Preedit>,

    /// 候选行。
    pub rows: Vec<Row>,

    /// 高亮行；`None` 不高亮。
    pub highlighted: Option<usize>,

    /// 高亮条移动动画；`None` 直接画在 `highlighted`。见 [`HighlightAnimation`]。
    pub highlight_animation: Option<HighlightAnimation>,

    /// 横排展开成矩阵时每行几格，`rows` 按行优先排开、空位是空行；0 为没展开。
    pub columns: usize,

    /// 矩阵各列要留几个候选字宽（壳按整份候选估的，滚动、移动高亮时不变，窗口才不跳）；空着就按视口里的内容实测。
    pub column_ems: Vec<f32>,

    /// 右下角页码。
    pub footer: Option<String>,

    /// 拼音行右侧的一句临时状态（删了什么词）。
    pub status: Option<String>,
}

impl Frame {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.preedit.is_none() && self.status.is_none()
    }

    /// 顶部要不要画一行（拼音或右侧状态任一存在）。
    pub fn has_top_line(&self) -> bool {
        self.preedit.is_some() || self.status.is_some()
    }
}
