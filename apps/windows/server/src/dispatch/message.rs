//! 按消息类型分派：会话开关、按键、轮询、失焦上屏、选区 / 光标矩形 / 中英模式的通知。

use cloudime_platform::protocol::{
    ClientMessage, Frame, KeyEvent, KeyOutcome, PROTOCOL_VERSION, SESSION_OPENED_SINCE,
    ServerMessage, SessionId,
};

use super::Router;
use super::key::Effect;
use super::session::SessionInfo;

impl Router {
    pub(super) fn dispatch(&mut self, message: ClientMessage) -> Option<ServerMessage> {
        match message {
            ClientMessage::OpenSession {
                session,
                app,
                protocol,
            } => {
                tracing::debug!(?session, app, protocol, "会话打开");
                if protocol != PROTOCOL_VERSION {
                    tracing::warn!(
                        ?session,
                        app,
                        dll = protocol,
                        server = PROTOCOL_VERSION,
                        "DLL 与 Server 的协议版本不同（应用还没重启、用着旧 DLL？），照常服务"
                    );
                }
                // 同一会话重开（DLL 断线重连）：从干净状态起。
                if self.focused == Some(session) {
                    self.reset_composition();
                    self.focused = None;
                }
                self.sessions.insert(
                    session,
                    SessionInfo {
                        app,
                        private: false,
                        protocol,
                    },
                );
                // 按键行为设置回一次，让 DLL 不必自己读配置文件。**只回给会读这条回包的 DLL**：
                // 更老的 DLL 的 `open` 是只写不读，多回一条会被它当成下一次 `Poll` 的应答而报错，
                // 那条连接就废了（老 DLL 在没重启的应用里还会活很久）。它们从 `ModeSync` 那一拍
                // 也能拿到同一份（新字段它直接忽略），只是慢一拍。
                (protocol >= SESSION_OPENED_SINCE).then(|| ServerMessage::SessionOpened {
                    session,
                    input: self.input_settings(session),
                })
            }
            ClientMessage::Key { session, event } => Some(self.handle_key(session, event)),
            ClientMessage::Poll { session } => Some(self.handle_poll(session)),
            ClientMessage::Commit { session } => {
                let text = self.commit_raw_for(session);
                tracing::debug!(?session, ?text, "焦点离开，结束组句");
                Some(ServerMessage::Committed { session, text })
            }
            ClientMessage::Surrounding { session, text } => {
                tracing::trace!(?session, chars = text.chars().count(), "收到光标前文");
                self.set_surrounding(session, text);
                None
            }
            ClientMessage::Privacy { session, private } => {
                tracing::debug!(?session, private, "输入框私密状态");
                self.set_privacy(session, private);
                None
            }
            ClientMessage::PositionCandidates { session, rect } => {
                self.position_candidates(session, rect);
                None
            }
            ClientMessage::HideCandidates { session } => {
                // 组句在 DLL 侧结束（应用终止组句）：只收窗口；缓冲留给下一键的 Commit 清。
                if self.focused == Some(session) {
                    self.hide_candidate_window();
                }
                None
            }
            ClientMessage::ModeChanged { session, mode } => {
                tracing::debug!(?session, ?mode, "输入法状态");
                self.handle_mode_changed(mode);
                None
            }
            ClientMessage::SyncMode { session } => {
                self.handle_ime_active();
                Some(ServerMessage::ModeSync {
                    session,
                    mode: Some(self.mode),
                    input: self.input_settings(session),
                    indicator: self.indicator_state(),
                })
            }
            ClientMessage::ImeSwitched { session } => {
                tracing::debug!(?session, "切成了别的输入法");
                self.handle_ime_switched();
                None
            }
            ClientMessage::Indicator { session, command } => {
                tracing::debug!(?session, ?command, "任务栏图标菜单");
                self.handle_indicator(command);
                None
            }
            ClientMessage::CloseSession { session } => {
                self.sessions.remove(&session);
                if self.focused == Some(session) {
                    self.reset_composition();
                    self.focused = None;
                }
                self.flush_learning();
                tracing::debug!(?session, "会话关闭");
                None
            }
        }
    }

    fn handle_key(&mut self, session: SessionId, event: KeyEvent) -> ServerMessage {
        self.ensure_focus(session);
        self.notice = None;
        self.caret_shift = 0;
        self.delete_before = 0;
        let (commit, outcome) = match self.apply_key(&event) {
            Effect::Changed(commit) => {
                self.recompose();
                (commit, KeyOutcome::Consumed)
            }
            Effect::Navigated => (None, KeyOutcome::Consumed),
            Effect::Passthrough => (None, KeyOutcome::Passthrough),
        };
        // 自绘窗吃未降级的帧；发给 DLL 的那份按老协议降级（见 composed 的 current_frame）
        let shown = self.self_drawn_frame();
        self.reconcile_candidates(&shown);
        ServerMessage::KeyResult {
            session,
            outcome,
            commit,
            caret_shift: self.caret_shift,
            delete_before: self.delete_before,
            frame: self.current_frame(),
        }
    }

    /// 轮询：聚焦会话回最新一帧（本地整句重排到达后候选顺序可能变了），否则回空帧。
    fn handle_poll(&mut self, session: SessionId) -> ServerMessage {
        self.tick();
        let frame = if self.focused == Some(session) {
            let shown = self.self_drawn_frame();
            self.reconcile_candidates(&shown);
            self.current_frame()
        } else {
            Frame::default()
        };
        ServerMessage::Update { session, frame }
    }
}
