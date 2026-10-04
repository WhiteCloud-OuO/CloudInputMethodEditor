use cloudime_platform::protocol::{Frame, IndicatorState, InputMode, InputSettings, KeyOutcome};

/// Server 对一次「同步输入法状态」轮询的答复。
pub struct ModeSyncReply {
    /// `Some(...)` 切到该状态；`None` 没有待处理的切换。
    pub mode: Option<InputMode>,

    /// 当前的按键行为设置（切换键、内置英文模式）。每一拍都带，配置改了靠它生效——
    /// DLL 不读配置文件，`%APPDATA%\CloudIME` 对 AppContainer 里的商店应用本来也读不到。
    pub input: InputSettings,

    /// 右键菜单打勾用的开关状态，同样每一拍都带。
    pub indicator: IndicatorState,
}

/// Server 对一次按键的处理结果。
pub struct KeyResponse {
    /// 吃掉还是放行给应用。
    pub outcome: KeyOutcome,

    /// 本次要立即上屏到文档的文本。
    pub commit: Option<String>,

    /// 上屏后把光标再挪几个字符（成对补全用，见 `protocol::ServerMessage::KeyResult`）。
    pub caret_shift: i16,

    /// 上屏前先删掉光标前这么多个字符（符号映射的两键规则用）。
    pub delete_before: u16,

    /// 处理后的组句状态（preedit + 候选）；空帧表示收起候选窗口。
    pub frame: Frame,
}
