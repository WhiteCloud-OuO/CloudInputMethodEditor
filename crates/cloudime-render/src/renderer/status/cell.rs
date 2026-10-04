//! 状态条的一格：一张 SVG 图标。

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusCell {
    /// 一段 SVG 源码画的图标按钮；方框边长由渲染器定（`BUTTON_SIZE`）。
    Icon { svg: String },
}

impl StatusCell {
    /// `svg` 是整份 `<svg>` 文本；按它自己的宽高比缩进方框、居中画出来。
    pub fn icon(svg: impl Into<String>) -> Self {
        Self::Icon { svg: svg.into() }
    }

    /// 这一格的 SVG 源码。
    pub fn svg(&self) -> &str {
        match self {
            Self::Icon { svg } => svg,
        }
    }
}
