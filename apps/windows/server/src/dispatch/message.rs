//! 按消息类型分派：会话开关、按键、轮询、失焦上屏、选区 / 光标矩形 / 中英模式的通知。

use cloudime_platform::protocol::{
    ClientMessage, Frame, KeyEvent, KeyOutcome, PROTOCOL_VERSION, SESSION_OPENED_SINCE,
    ServerMessage, SessionId,
};

use super::Router;
use super::key::Effect;
use super::key::input::reserved_combo;
use super::script::{ScriptActions, ThemeCommand};
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
                    self.sync_focused_app();
                    // 重连：DLL 的文本快照重新读一份
                    self.document.clear();
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
            ClientMessage::Surrounding {
                session,
                text,
                document,
            } => {
                tracing::trace!(?session, chars = text.chars().count(), "收到光标前文");
                self.set_surrounding(session, text);
                // 整篇快照只认聚焦会话那份（DLL 在起组句时读，天然就是当前文档）
                if self.focused == Some(session) {
                    self.document.store(document);
                }
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
            ClientMessage::SyncMode {
                session,
                in_text_input,
                caps,
            } => {
                self.handle_ime_active();
                // 顺路记下焦点状态：状态切换提示据此决定弹不弹、以及「中 / 英」按钮画不画「A」。
                self.in_text_input = in_text_input;
                self.caps = caps;
                self.check_status_tip();
                Some(ServerMessage::ModeSync {
                    session,
                    mode: Some(self.mode),
                    input: self.input_settings(session),
                    indicator: self.indicator_state(),
                    // 只要还有脚本登记，就每段组句请 DLL 带一份整篇：快照得跟着文档走，读一次就冻结会
                    // 拿到旧的（`copy all` 这类会少掉后来敲进去的字）。代价是每段组句起始读一次文档，
                    // 与文档大小成正比、硬上限 20 万 UTF-16 单元；没脚本时永远是 `false`，零读取。
                    want_document: self.scripts.has_any_handlers(),
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
                    self.sync_focused_app();
                }
                // 会话没了：文本快照也作废
                self.document.clear();
                self.flush_learning();
                tracing::debug!(?session, "会话关闭");
                None
            }
        }
    }

    fn handle_key(&mut self, session: SessionId, event: KeyEvent) -> ServerMessage {
        self.ensure_focus(session);
        // 脚本先看一眼这一键：返回的表能改这一拍的结果（见 `script`）。
        // **输入法自己占用的组合键（`Ctrl+数字` / `Ctrl+Enter` / `Ctrl+反引号` / `Shift+反引号`）
        // 不派发给脚本** —— 谁先定义谁优先，脚本抢不走（见 `docs/design/script.md`）。
        let actions = if reserved_combo(&event) {
            ScriptActions::default()
        } else {
            self.script_actions(&event)
        };
        // 脚本给的加权 / 降权：排序仍在 Core，这里只把参数递过去（没给就不动 Core 那份）
        if let Some(adjustments) = &actions.adjust {
            self.engine
                .set_word_adjustments(adjustments.iter().cloned());
        }
        self.notice = None;
        self.caret_shift = 0;
        self.delete_before = 0;
        // 脚本把这一键接管了：不吃，原样交给应用（游戏里抢键）。
        if actions.passthrough {
            let shown = self.self_drawn_frame();
            self.reconcile_candidates(&shown);
            return ServerMessage::KeyResult {
                session,
                outcome: KeyOutcome::Passthrough,
                commit: None,
                caret_shift: 0,
                delete_before: 0,
                frame: self.current_frame(),
            };
        }
        // 脚本把这一键接管了：上屏它的文本，当前组句作废。
        if let Some(text) = actions.commit {
            self.reset_composition();
            return ServerMessage::KeyResult {
                session,
                outcome: KeyOutcome::Consumed,
                commit: Some(text),
                caret_shift: 0,
                delete_before: 0,
                frame: self.current_frame(),
            };
        }
        self.notice = actions.notice.clone();
        let (commit, outcome) = match self.apply_key(&event) {
            Effect::Changed(commit) => {
                self.recompose();
                (commit, KeyOutcome::Consumed)
            }
            Effect::Navigated => (None, KeyOutcome::Consumed),
            Effect::Passthrough => (None, KeyOutcome::Passthrough),
        };
        // 脚本要的在线翻译那一行（`online`）：放在按键派发**之后**，脚本看到的是这一拍之后的状态
        self.apply_online_actions(&actions);
        // 脚本换主题：动作表里的 `theme` 是这一拍的最终决定，其次才看 `cloudime.apply_theme` 提的请求
        let requested = ThemeCommand::from_request(self.scripts.take_theme_request());
        self.apply_script_theme(actions.theme.clone().or(requested));
        // 这一拍之后不在组句了（Esc / 上屏完 / 断线）：候选窗都没了，那一行与脚本设的尺寸都收掉 ——
        // 它们只活在一次组句里（脚本自己写的那一份也归这条规则）
        if !self.composing() {
            self.online.clear();
            self.clear_script_size();
            self.engine.set_calculator(false);
            // 候选窗关掉了：脚本在组句里要换的主题这时才补上（不打断刚才在看候选的人）
            self.flush_pending_theme();
        }
        // 脚本要了「重画」（`cloudime.candidate.redraw()`）：按它最新的状态把这一屏重算一遍，
        // 这样异步回调里改的状态能立刻反映到候选窗上（重算会重新派发 `candidates` 事件）。
        if self.scripts.take_redraw_request() {
            self.recompose();
        }
        // 脚本设的候选窗尺寸（`cloudime.candidate.set_*`）：本组句内有效
        let size = self.scripts.take_size_request();
        self.apply_size_request(size);
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
    /// 顺带把攒着的鼠标点选上屏带回去（候选窗在 Server 手里，没有按键可以捎它）。
    fn handle_poll(&mut self, session: SessionId) -> ServerMessage {
        self.tick();
        let focused = self.focused == Some(session);
        let frame = if focused {
            let shown = self.self_drawn_frame();
            self.reconcile_candidates(&shown);
            self.current_frame()
        } else {
            Frame::default()
        };
        let commit = if focused {
            self.pending_commit.take()
        } else {
            None
        };
        ServerMessage::Update {
            session,
            frame,
            commit,
        }
    }
}
