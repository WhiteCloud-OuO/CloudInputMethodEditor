use std::io::{Read, Write};

use cloudime_platform::protocol::{
    ClientMessage, DocumentText, IndicatorCommand, InputMode, InputSettings, KeyEvent,
    PROTOCOL_VERSION, ScreenRect, ServerMessage, SessionId, read_message, write_message,
};

use super::{KeyResponse, ModeSyncReply, PollReply};
use crate::error::ClientError;

/// 连 Server 的一个会话客户端，开在一条已连好的双工流上（Windows 下是命名管道，测试里是内存流）。
/// 传输是一问一答；开关会话与通知类消息单向发。
pub struct EngineClient<S> {
    stream: S,

    /// 本会话标识，随每条消息带上。
    session: SessionId,

    /// 上次报给 Server 的私密状态；`None` 是还没报过（Server 按不私密起算）。
    private: Option<bool>,
}

impl<S: Read + Write> EngineClient<S> {
    /// 开一个会话；Server 随即回一次按键行为设置（切换键、内置英文模式），带出来交给调用方。
    /// `app` 是宿主应用的 exe 文件名，Server 据此查按应用的设置。
    pub fn open(
        mut stream: S,
        session: SessionId,
        app: Option<String>,
    ) -> Result<(Self, InputSettings), ClientError> {
        write_message(
            &mut stream,
            &ClientMessage::OpenSession {
                session,
                app,
                protocol: PROTOCOL_VERSION,
            },
        )?;
        let input = match read_message(&mut stream)?.ok_or(ClientError::Closed)? {
            ServerMessage::SessionOpened { input, .. } => input,
            _ => InputSettings::default(),
        };
        Ok((
            Self {
                stream,
                session,
                private: None,
            },
            input,
        ))
    }

    pub fn session(&self) -> SessionId {
        self.session
    }

    /// 在一条临时流上通知 Server「本线程切成了别的输入法」（状态条收起）。不开会话、不回话。
    pub fn notify_ime_switched(mut stream: S, session: SessionId) -> Result<(), ClientError> {
        write_message(&mut stream, &ClientMessage::ImeSwitched { session })?;
        Ok(())
    }

    /// 送一个按键等结果。
    pub fn key(&mut self, event: KeyEvent) -> Result<KeyResponse, ClientError> {
        let message = ClientMessage::Key {
            session: self.session,
            event,
        };
        match self.call(&message)? {
            ServerMessage::KeyResult {
                outcome,
                commit,
                caret_shift,
                delete_before,
                frame,
                ..
            } => Ok(KeyResponse {
                outcome,
                commit,
                caret_shift,
                delete_before,
                frame,
            }),
            _ => Err(ClientError::Unexpected("expected key result")),
        }
    }

    /// 组句期间定时轮询最新一帧（本地整句重排到达后候选顺序可能变了），
    /// 顺路取回「不用按键的上屏」（鼠标点了 Server 自绘的候选窗）。
    pub fn poll(&mut self) -> Result<PollReply, ClientError> {
        match self.call(&ClientMessage::Poll {
            session: self.session,
        })? {
            ServerMessage::Update { frame, commit, .. } => Ok(PollReply { frame, commit }),
            _ => Err(ClientError::Unexpected("expected update for poll")),
        }
    }

    /// 让 Server 清空缓冲，拿回要原样上屏的文本（没在组句时为 `None`）。
    pub fn commit(&mut self) -> Result<Option<String>, ClientError> {
        match self.call(&ClientMessage::Commit {
            session: self.session,
        })? {
            ServerMessage::Committed { text, .. } => Ok(text),
            _ => Err(ClientError::Unexpected("expected committed for commit")),
        }
    }

    /// 组句起始时把应用光标前的文字送给 Server（本地整句模型的前文），
    /// 以及（Server 请过的话）一份整篇文本快照（脚本的 `cloudime.text.*`）。不回话。
    pub fn surrounding(
        &mut self,
        text: String,
        document: Option<DocumentText>,
    ) -> Result<(), ClientError> {
        self.send(&ClientMessage::Surrounding {
            session: self.session,
            text,
            document,
        })
    }

    /// 起组句时报输入框私密与否；与上次报的相同就不发（Server 缺省按不私密）。不回话。
    pub fn set_private(&mut self, private: bool) -> Result<(), ClientError> {
        if self.private == Some(private) || (self.private.is_none() && !private) {
            self.private = Some(private);
            return Ok(());
        }
        self.send(&ClientMessage::Privacy {
            session: self.session,
            private,
        })?;
        self.private = Some(private);
        Ok(())
    }

    /// 报组句范围的屏幕矩形，Server 据此摆候选窗口。不回话。
    pub fn position_candidates(&mut self, rect: ScreenRect) -> Result<(), ClientError> {
        self.send(&ClientMessage::PositionCandidates {
            session: self.session,
            rect,
        })
    }

    /// 问 Server 有没有待处理的目标模式，顺路取回最新的按键行为设置（每一拍都带）。
    /// `in_text_input` / `caps` 是本线程此刻的焦点状态（状态切换提示用，见 [`ClientMessage::SyncMode`]）。
    pub fn sync_mode(
        &mut self,
        in_text_input: bool,
        caps: bool,
    ) -> Result<ModeSyncReply, ClientError> {
        match self.call(&ClientMessage::SyncMode {
            session: self.session,
            in_text_input,
            caps,
        })? {
            ServerMessage::ModeSync {
                mode,
                input,
                indicator,
                want_document,
                ..
            } => Ok(ModeSyncReply {
                mode,
                input,
                indicator,
                want_document,
            }),
            _ => Err(ClientError::Unexpected("expected mode sync")),
        }
    }

    /// 把当前会话的输入法状态推给 Server（悬浮状态条）。不回话。
    pub fn mode_changed(&mut self, mode: InputMode) -> Result<(), ClientError> {
        self.send(&ClientMessage::ModeChanged {
            session: self.session,
            mode,
        })
    }

    /// 任务栏图标右键菜单里点的项交给 Server。不回话。
    pub fn indicator(&mut self, command: IndicatorCommand) -> Result<(), ClientError> {
        self.send(&ClientMessage::Indicator {
            session: self.session,
            command,
        })
    }

    /// 让 Server 收起候选窗口（组句在 DLL 侧结束、Server 无从知晓时用）。不回话。
    pub fn hide_candidates(&mut self) -> Result<(), ClientError> {
        self.send(&ClientMessage::HideCandidates {
            session: self.session,
        })
    }

    /// 关闭会话，释放 Server 侧状态。不回话。
    pub fn close(mut self) -> Result<(), ClientError> {
        self.send(&ClientMessage::CloseSession {
            session: self.session,
        })
    }

    fn send(&mut self, message: &ClientMessage) -> Result<(), ClientError> {
        write_message(&mut self.stream, message)?;
        Ok(())
    }

    /// 一问一答；对端在帧边界关闭算 [`ClientError::Closed`]。
    fn call(&mut self, message: &ClientMessage) -> Result<ServerMessage, ClientError> {
        self.send(message)?;
        read_message(&mut self.stream)?.ok_or(ClientError::Closed)
    }
}
