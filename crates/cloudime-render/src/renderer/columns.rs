//! 竖排的列宽与行高（像素）。

pub(super) struct Columns {
    pub index_width: f32,

    pub text_width: f32,

    pub annotation_width: f32,

    /// 右对齐的来源角标宽度（取最宽一行），没有角标为 0。
    pub badge_width: f32,

    pub row_height: f32,
}
