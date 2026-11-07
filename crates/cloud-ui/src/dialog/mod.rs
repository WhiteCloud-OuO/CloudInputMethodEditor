//! Cloud UI 内置对话框。
//!
//! `dialog` 是统称；每个具体对话框都是一个类：`new` 建实例、改属性，再 `show(self)`。
//! 它们共用 [`layout`] 的尺寸 / 排版、[`style::DialogStyle`] 的属性与
//! [`blocking`] 的同步模态宿主。

mod blocking;
mod layout;
mod message;
mod style;
mod text_input;

pub use message::MessageDialog;
pub use style::DialogStyle;
pub use text_input::TextInputDialog;
