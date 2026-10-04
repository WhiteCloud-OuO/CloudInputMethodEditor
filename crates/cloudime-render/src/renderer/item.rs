//! 横排时每一项的尺寸（像素）。

pub(super) struct Item {
    pub index_width: f32,

    pub text_width: f32,

    /// 该项的来源角标宽度，没有为 0。
    pub badge_width: f32,
}
