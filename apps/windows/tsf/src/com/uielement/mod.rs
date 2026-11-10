//! TSF UI-less（P1）：宿主状态与最新一帧。
//!
//! 状态（`ui_less` / `host_draws` / `host_reads`）只在变化时上报；最新一帧缓存在这里，
//! 候选元素的 getter 直接读它，不做 IPC（元素对象在 P1-C 接入）。

mod state;

pub(crate) use self::state::UiElementState;
