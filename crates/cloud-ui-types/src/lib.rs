//! `cloud-ui` 与 `cloudime-platform` 共用的轻量 UI 数据类型。
//!
//! 这里不依赖 Windows、词库、配置读取；独立 exe 只拿这些类型就能用 `cloud-ui` 的
//! 渲染设置与主题。

mod item_number;
mod layout;
mod theme;

pub use item_number::ItemNumberStyle;
pub use layout::LayoutMode;
pub use theme::{BarTheme, CandidateTheme, ThemeColor, ThemeError, ThemeFile, TipTheme};
