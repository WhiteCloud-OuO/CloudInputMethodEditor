//! Server 的库部分：Engine 装配（[`assembly`]）、协议分派（[`Router`]）与传输（[`ipc`]），供 bin 与集成测试共用。

pub mod assembly;
pub mod dispatch;
pub mod error;
/// 前台全屏形态判定（D3D 独占 / 铺满）；仅 Windows。
#[cfg(windows)]
pub(crate) mod fullscreen;
pub mod ipc;
/// MCP Client sidecar 代理（`cloudime.mcp.*` 的传输层）。
pub mod mcp;
/// 系统语音（SAPI）：`Shift + 反引号` 念候选的译文。
pub mod speech;
pub(crate) mod timing;
/// 候选窗口 / 状态条的自绘线程；仅 Windows。
#[cfg(windows)]
pub mod ui;

pub use assembly::{AssemblySpec, LanguageModelFiles, NameFiles};
pub use dispatch::{Router, RouterConfig};
pub use error::ServerError;
