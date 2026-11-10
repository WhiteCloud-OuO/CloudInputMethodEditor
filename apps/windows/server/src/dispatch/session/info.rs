//! 会话信息。

/// UIElement 状态（P1 UI-less）：宿主声明 UI-less、自己画候选、或读走过候选串。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UiElementFlags {
    /// 宿主在 `BeginUIElement` 回了 `pbShow=FALSE` / 之后 `Show(FALSE)`。
    pub(crate) host_draws: bool,

    /// 宿主线程以 `TF_TMAE_UIELEMENTENABLEDONLY` 激活。
    pub(crate) ui_less: bool,

    /// 宿主调用过 `GetString` 把候选串读走（推断，不等于会画）。
    pub(crate) host_reads: bool,
}

/// 一个活跃会话在 Server 侧记下的信息，开会话时由 DLL 报来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionInfo {
    /// 宿主应用的 exe 文件名（如 `Code.exe`），查 `[apps]` 用；取不到为 `None`。
    pub(crate) app: Option<String>,

    /// 该会话当前落在私密输入框里（DLL 随 `ClientMessage::Privacy` 报来）；焦点切回来时按它重设 Engine。
    pub(crate) private: bool,

    /// DLL 报来的协议版本。比 Server 低的（应用还没重启、加载着旧 DLL）仍照常服务，
    /// 但要按老协议能力做降级，并在第一次出候选时提示重启这个应用。
    pub(crate) protocol: u32,

    /// 这条会话是否已经提示过「应用需重启」（旧协议 DLL 只提示一次）。
    pub(crate) restart_notice_shown: bool,

    /// UIElement 状态（P1 UI-less）；`ClientMessage::UiElementState` 更新它。
    pub(crate) ui_element: UiElementFlags,
}
