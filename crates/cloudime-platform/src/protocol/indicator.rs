use serde::{Deserialize, Serialize};

/// 任务栏「中 / 英」图标右键菜单里要交给 Server 办的项（中 / 英切换在 DLL 侧自己做）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndicatorCommand {
    /// 翻转当前模式的全角标点，与悬浮状态条上点「中文标点 / 英文标点」按钮一样
    /// （内置热键 `Ctrl + Alt + ,`）。
    TogglePunctuation,

    /// 翻转「全角 / 半角」字符，与状态条上那一格一样（内置热键 `Shift + Space`）。
    ToggleCharWidthType,

    /// 翻转简 / 繁输出，与状态条那一格、设置里那一项一样（内置热键 `Ctrl + Alt + .`）。
    ToggleSimpTrad,

    /// 打开设置程序。DLL 可能在 UWP 沙箱里起不了进程，交给 Server 起。
    OpenSettings,

    /// 查到新版本时菜单里的「有新版本」：打开下载页，同样交给 Server。
    /// 界面上已不再有入口（任务栏右键菜单固定四项），协议与 Server 处理保留。
    OpenDownload,

    /// 重启输入法服务：Server 起一个新的 `cloudime-server.exe`（带 `--wait-pid` 等本进程退出再占管道），
    /// 本进程回完这条消息后干净退出。
    RestartServer,
}

/// 右键菜单打勾用的开关状态。DLL 不读配置文件（UWP 沙箱里读不到），由 Server 随
/// [`super::ServerMessage::ModeSync`] 每一拍带下来。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndicatorState {
    /// 中文模式下标点转全角（Server 的会话内状态，重启回缺省）。
    pub full_width_punctuation: bool,

    /// 英文模式下标点转全角（同上，缺省半角）。
    pub english_full_width_punctuation: bool,

    /// 检查更新查到了新版本，菜单里露出「有新版本」。
    pub update_available: bool,
}
