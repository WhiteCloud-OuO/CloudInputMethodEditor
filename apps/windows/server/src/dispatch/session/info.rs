//! 会话信息。

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
}
