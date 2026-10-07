//! Server ↔ DLL 的 IPC 协议类型。
//!
//! Windows 的 TSF DLL 会被加载进每一个应用进程，所以 [`cloudime_core::Engine`] 不能待在 DLL 里，
//! 得跑在独立的 Server 进程；DLL 只做 IPC，把系统按键翻译成 [`ClientMessage`] 发给 Server，
//! 把 Server 回的 [`ServerMessage`] 画到候选窗口。结构与 Weasel（WeaselServer）/ 水杉（Server 进程）一致。
//!
//! 这里的类型两端共用，必须可序列化（serde）。协议只描述「按键进、要画什么出」这一层，
//! 不复制 Core 的排序 / 词库逻辑：候选直接用 [`cloudime_core::CandidateList`]，preedit 分段用
//! [`PreeditSegment`]（Core 内部的 `MarkedSegment` 的可序列化镜像，避免协议耦合 Core 的内部枚举）。

mod client;
mod codec;
mod indicator;
mod mode;
mod screen_rect;
mod server;
mod session;

/// 协议版本，DLL 开会话时带上。加消息 / 改字段语义时 +1，一个版本周期只升一次（本周期已升过就不再升）；
/// Server 只对不上时记警告（老 DLL 在没重启的
/// 应用里还会活很久，serde 的缺省字段 / 忽略未知字段让两边仍能对话）。
///
/// **加枚举变体不在「仍能对话」之列**：`cloudime_core::Candidate` 是线上格式的一部分（见本模块文档），
/// 给它加一个 `kind` 变体，老 DLL 解不出来会整条帧失败、按键直接放行——测试时看到的「输入法突然只出英文」
/// 就是这么来的（`unknown variant `Code``）。加变体必须同时 +1 并重装 DLL，否则连警告都不会有。
///
/// 同理，**删字段也不同样能对话**：老 DLL 的 `Frame` 缺字段会整条解析失败（v8 去掉 `theme` 就是这么升的）。
/// **删枚举变体同理**：v9 删掉了 `CandidateKind::Emoji`（emoji 候选整体下线），还没重启的应用里那份旧 Server
/// 仍会发它，新 DLL 解不出来就是整条帧失败、按键直接放行，所以一并 +1 并重装 DLL。
///
/// **加字段本身能对话，但要新 DLL 才有行为**：v10 给 [`InputSettings`] 加了 `full_width_chars`（状态条的
/// 「全角 / 半角」开关）。老 DLL 会忽略这个字段、表现得像没开——静默少一个功能，只能靠版本号认出来，所以同样 +1。
///
/// v11 给 [`ServerMessage::KeyResult`] 加了 `caret_shift`（成对补全把光标停到括号中间）与 `delete_before`
/// （符号映射的两键规则把上一个键的输出换掉）：老 DLL 会忽略它们，表现成「括号补上了但光标在末尾」「`~=` 出成
/// `～≈`」，同样是静默少了半个功能。
///
/// v12 给 [`InputSettings`] 加了 `raw_input`（「在下列程序中不显示候选框」名单里的程序完全不接管）：老 DLL 会当它
/// 不存在，照旧吃键出候选——这个程序里用户想用应用自己的补全列表就用不成。
///
/// v13 去掉 [`cloudime_core::CandidateKind::Custom`] 的载荷（短语不再占固定候选格，改成按权重整体排在最前）：
/// 变体形状变了，老 DLL 解不出来会整条帧失败、按键直接放行，必须 +1 并重装 DLL。
///
/// v14 把中英的布尔换成三态 [`InputMode`]（中文 / 英文 / 禁用）：`ModeChanged` 与 `ModeSync` 的字段变了，
/// 另外 [`IndicatorCommand`] 加了「全角 / 半角」与「简 / 繁」两项（`Shift + Space`、`Ctrl + Alt + .` 两个内置热键）。
///
/// v15 给 [`IndicatorCommand`] 加了 [`IndicatorCommand::RestartServer`]（任务栏右键菜单的「重启输入法服务」）：
/// 加枚举变体老 DLL / 老 Server 解不出来，整条帧失败、按键直接放行，必须 +1 并重装 DLL。老 Server 下点了
/// 这一项没反应（它不认识这个变体，整帧解析失败）。
///
/// v15 之后给 [`cloudime_core::Candidate`] 加了 `display`（候选里显示的内容，上屏仍用 `text`）：给结构体加
/// **带 `serde(default)` 的字段**两边仍能对话，且老 DLL 只读候选条数、不读这条内容（候选窗由 Server 自绘），
/// 行为完全不变，所以不 +1。
///
/// v16 给 [`ServerMessage::Update`] 加了 `commit`（鼠标点候选窗上屏）：候选窗是 Server 自绘的、收不到按键，
/// 只能等 DLL 那一拍 `Poll` 时把要上屏的文本带回去。老 DLL 会忽略它，表现成「点了候选窗没反应、组句还
/// 悄悄清掉了」——静默少了半个功能，必须 +1 并重装 DLL。同一版里给 [`Frame`] 加了 `columns`
/// （展开「更多候选项」的网格列数）：它只影响 Server 自绘，加字段本身不改老 DLL 的行为，跟着这次一起走。
///
/// v17 给 [`ClientMessage::SyncMode`] 加了 `in_text_input` 与 `caps`（「状态切换提示」判断在不在输入状态、
/// Caps Lock 亮不亮用）：老 DLL 不带这两个字段，读成 `false`，表现成「在那台机器上提示条从来不弹」——
/// 同样是静默少一个功能，所以 +1 并重装 DLL。
pub const PROTOCOL_VERSION: u32 = 17;

/// 从哪个协议版本起 DLL 会在 `OpenSession` 后阻塞读一条 [`ServerMessage::SessionOpened`]。
/// 门槛是固定值而不是当前版本：以后版本再升，没重启的应用里那些旧 DLL 仍在等这条回包，
/// 不回它们会卡在 `open()` 里（宿主 UI 线程）。
pub const SESSION_OPENED_SINCE: u32 = 6;

/// 从哪个协议版本起 DLL 认 [`ServerMessage::Update`] 里的 `commit`（鼠标点候选窗上屏）。
/// 老 DLL 收不到这段文本却又会跟着把组句清掉，所以 Server 宁可整条鼠标上屏都拦下来（见 `supports_candidate_click`）。
pub const CANDIDATE_CLICK_SINCE: u32 = 16;

pub mod frame;
pub mod key;

pub use client::ClientMessage;
pub use codec::{CodecError, DEFAULT_PIPE_NAME, read_message, write_message};
pub use frame::{Frame, PreeditKind, PreeditSegment, TipChoices};
pub use indicator::{IndicatorCommand, IndicatorState};
pub use key::{KeyEvent, KeyModifiers, KeyOutcome};
pub use mode::InputMode;
pub use screen_rect::ScreenRect;
pub use server::{InputSettings, ServerMessage};
pub use session::SessionId;
