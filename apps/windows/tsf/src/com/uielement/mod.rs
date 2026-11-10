//! TSF UI-less（P1）：宿主状态、候选元素与生命周期。
//!
//! 状态（`ui_less` / `host_draws` / `host_reads`）只在变化时上报；最新一帧缓存在 `Shared` 里，
//! 候选元素的 getter 直接读它，不做 IPC。

mod element;
mod lifecycle;
mod state;

pub(crate) use self::lifecycle::ActiveElement;
pub(crate) use self::state::UiElementState;
