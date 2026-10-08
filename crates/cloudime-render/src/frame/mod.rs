//! 一帧要画的全部内容：顶部拼音行、候选行、高亮、页脚、右侧状态。只是展示形态，不含排序或查词。

mod highlight;
mod preedit;
mod row;
mod tip;
mod tone;

pub use highlight::{HighlightAnimation, HighlightRect};
pub use preedit::{Preedit, PreeditSegment, PreeditStyle};
pub use row::Row;
pub use tip::TipSegment;
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

    /// 展开成矩阵时每行几格，`rows` 按行优先排开；0 为没展开（按竖排 / 横排画）。
    pub columns: usize,

    /// 矩阵每格的最小宽度（内容区像素），`0` 不限。壳把「收起时候选高亮条有多宽」传进来：
    /// 展开成网格后每格就以它为准——比它还长的候选截尾加「…」，短的原样留白。
    pub min_cell_width: f32,

    /// 右下角页码。
    pub footer: Option<String>,

    /// 底部那一行左侧的翻译 Tip（一段一段画）；`None` 表示这个词条没有译文。
    pub tip: Option<Vec<TipSegment>>,

    /// 候选窗底部**再下面一行**的在线翻译（`Ctrl+T`）：整行留给它，一段一段画。
    /// `None` = 没按过 `Ctrl+T`（或已经收起）。
    pub online: Option<Vec<TipSegment>>,

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
