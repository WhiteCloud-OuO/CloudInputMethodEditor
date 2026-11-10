//! 一个激活周期里的 UIElement 状态：三个标志 + 最新一帧。
//!
//! `host_reads` 只在 `GetString` 被调用时置位（推断宿主可能自己画）；`host_draws` 只在
//! `BeginUIElement` 回 `FALSE` / `Show(FALSE)` 时置位（宿主声明自己画）；`ui_less` 来自
//! `ActivateEx` 的 `TF_TMAE_UIELEMENTENABLEDONLY`。三者都 sticky 到 `Deactivate`。

use cloudime_platform::protocol::Frame;

/// 要报给 Server 的三个标志。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UiElementFlags {
    /// 宿主线程以 `TF_TMAE_UIELEMENTENABLEDONLY` 激活。
    pub(crate) ui_less: bool,

    /// 宿主在 `BeginUIElement` 回了 `pbShow=FALSE` / 之后 `Show(FALSE)`。
    pub(crate) host_draws: bool,

    /// 宿主调用过 `GetString` 把候选串读走。
    pub(crate) host_reads: bool,
}

/// 一个激活周期的 UIElement 状态。
#[derive(Debug, Default)]
pub(crate) struct UiElementState {
    /// 最新一帧（`KeyResult` / `Update` / `Poll` 的应答）；空帧表示组句结束。
    pub(crate) frame: Frame,

    /// 宿主线程以 `TF_TMAE_UIELEMENTENABLEDONLY` 激活。
    pub(crate) ui_less: bool,

    /// 宿主在 `BeginUIElement` 回了 `pbShow=FALSE` / 之后 `Show(FALSE)`。
    pub(crate) host_draws: bool,

    /// 宿主调用过 `GetString` 把候选串读走。
    pub(crate) host_reads: bool,

    /// 上次上报成功的标志；`None` = 还没报过（或刚 `reset`）。
    sent: Option<UiElementFlags>,
}

impl UiElementState {
    /// 换一帧：只换缓存，不决定发不发消息。
    pub(crate) fn set_frame(&mut self, frame: Frame) {
        self.frame = frame;
    }

    /// 当前三个标志。
    pub(crate) fn flags(&self) -> UiElementFlags {
        UiElementFlags {
            ui_less: self.ui_less,
            host_draws: self.host_draws,
            host_reads: self.host_reads,
        }
    }

    /// 与上次上报的不同就返回新标志；没变化返回 `None`。
    pub(crate) fn changed(&self) -> Option<UiElementFlags> {
        let flags = self.flags();
        (self.sent != Some(flags)).then_some(flags)
    }

    /// 上报成功后记下这一份，下一次只有再变才报。
    pub(crate) fn mark_sent(&mut self) {
        self.sent = Some(self.flags());
    }

    /// 新会话 / 重连：Server 那边没有状态，下一次必须重报。
    pub(crate) fn forget_sent(&mut self) {
        self.sent = None;
    }

    /// 激活周期结束 / 重新激活：清空缓存与上报记录，下一次变化会重新报。
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use cloudime_platform::protocol::Frame;

    use super::UiElementState;

    #[test]
    fn flags_only_report_when_they_change() {
        let mut state = UiElementState::default();
        let first = state.changed().expect("第一次上报");
        assert!(!first.ui_less && !first.host_draws && !first.host_reads);
        state.mark_sent();
        assert!(state.changed().is_none(), "没变不再报");

        state.ui_less = true;
        let changed = state.changed().expect("ui_less 变了");
        assert!(changed.ui_less);
        assert!(!changed.host_draws && !changed.host_reads);
        state.mark_sent();
        assert!(state.changed().is_none());
    }

    #[test]
    fn reset_forces_the_next_report_and_drops_the_frame() {
        let mut state = UiElementState {
            frame: Frame::default(),
            ui_less: true,
            host_draws: true,
            host_reads: true,
            ..UiElementState::default()
        };
        state.mark_sent();
        assert!(state.changed().is_none());

        state.reset();
        let flags = state.changed().expect("reset 后重报");
        assert!(!flags.ui_less && !flags.host_draws && !flags.host_reads);
        assert!(state.frame.preedit.is_empty());
    }

    #[test]
    fn frame_cache_keeps_the_latest_frame() {
        let mut state = UiElementState::default();
        let frame = Frame {
            highlight: 3,
            ..Frame::default()
        };
        state.set_frame(frame);
        assert_eq!(state.frame.highlight, 3);
    }
}
