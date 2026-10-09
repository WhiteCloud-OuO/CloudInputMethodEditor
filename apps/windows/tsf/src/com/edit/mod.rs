//! 编辑会话：TSF 不允许直接改文档，要经 `RequestEditSession` 申请、在回调里拿着 edit cookie 读写。
//! 写组句 / 上屏在 [`update`]，读光标前文与输入框私密判定（本地整句模型 / 密码框）在 [`surrounding`]，候选窗口的定位锚点在 [`anchor`]。

mod anchor;
mod surrounding;
mod update;

pub(crate) use self::anchor::{anchor_rect, caret_rect};
pub(crate) use self::surrounding::{InputContext, input_context, request_surrounding_now};
pub(crate) use self::update::request_update;
