//! 互动画布：把状态树切成嵌套矩形，并画成可点的槽位。

mod layout;
mod paint;

pub(crate) use layout::child_rect;
pub use paint::{paint, parse_slot};
