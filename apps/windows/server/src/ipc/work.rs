use std::sync::mpsc::Sender;

use cloudime_platform::protocol::{ClientMessage, ServerMessage};

use crate::dispatch::{CandidateEvent, StatusEvent};

/// 工人线程（独占 [`crate::dispatch::Router`]）的一件活：DLL 的一条消息，或 UI 线程发来的一次操作。
pub enum Work {
    /// 某条连接收到的消息 + 回结果的通道（`None` 表示不用回话）。
    Client(ClientMessage, Sender<Option<ServerMessage>>),

    /// UI 线程发来的状态条操作。
    Status(StatusEvent),

    /// UI 线程发来的候选窗鼠标操作（悬停 / 单击）。
    Candidate(CandidateEvent),
}
