use cloudime_platform::protocol::ScreenRect;

use super::{StatusItems, StatusView};

/// 状态条输出端。Router 在工人线程上调，窗口在 UI 线程上，故要 `Send`。
pub trait StatusSink: Send {
    fn show_status(&self, view: StatusView);

    fn hide_status(&self);

    /// 状态切换提示：在 `anchor`（光标矩形）附近弹一个 1 秒的提示条。
    /// `caps` 是 Caps Lock 亮灭，给「中 / 英」按钮选「A」图标用；
    /// `items` 是要显示哪几项（`全部状态` 四项，`仅切换项` 只显示变的那几项）。
    fn show_status_tip(
        &self,
        _view: StatusView,
        _caps: bool,
        _anchor: ScreenRect,
        _items: StatusItems,
    ) {
    }

    /// 起设置程序（任务栏图标右键菜单用；悬浮条上的设置按钮在 UI 线程直接起）。
    fn open_settings(&self) {}

    /// 用浏览器打开下载页（右键菜单的「有新版本」）。
    fn open_download(&self) {}
}

/// 不画状态条的空实现。
pub struct NoopStatusSink;

impl StatusSink for NoopStatusSink {
    fn show_status(&self, _view: StatusView) {}

    fn hide_status(&self) {}
}
