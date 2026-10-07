//! 往文档写字：按键结果经异步编辑会话写上屏文本 + 组句拼音行；失焦 / 停用 / 切模式时让 Server 交出缓冲区原样落定。

use windows::Win32::UI::TextServices::ITfContext;
use windows::core::Ref;

use super::TextService_Impl;
use crate::com::composition::{Update, preedit_string};
use crate::com::edit::request_update;
use crate::com::log::log;
use cloudime_platform::protocol::Frame;

impl TextService_Impl {
    /// 失焦 / 停用 / 切模式：让 Server 交出缓冲区，原样落进最近收键的文档并收掉组句。
    /// 组句已被应用终止的（拼音已是普通文本）只清 Server 不再插。
    pub(super) fn commit_pending(&self) {
        let stale = self.shared.take_server_stale();
        if !self.shared.composing() && !stale {
            return;
        }
        let text = {
            let mut guard = self.engine.borrow_mut();
            let Some(client) = guard.as_mut() else {
                self.shared.reset();
                return;
            };
            match client.commit() {
                Ok(text) => text,
                Err(error) => {
                    log(&format!("失焦上屏失败，断开，下一键重连: {error}"));
                    drop(guard);
                    self.disconnect();
                    return;
                }
            }
        };
        if stale {
            return;
        }
        self.shared.end_composing();
        let Some(context) = self.shared.last_context() else {
            log(&format!("失焦上屏没有上下文，丢弃: {text:?}"));
            self.shared.reset();
            return;
        };
        log(&format!("失焦上屏: {text:?}"));
        let requested = request_update(
            &context,
            self.client_id.get(),
            self.engine.clone(),
            self.shared.clone(),
            Update {
                commit: text.filter(|t| !t.is_empty()),
                ..Update::default()
            },
        );
        if let Err(error) = requested {
            log(&format!("失焦上屏的编辑会话没被受理: {error}"));
            self.shared.reset();
        }
    }

    /// 轮询里搭回来的「不用按键的上屏」：鼠标点在 Server 自绘的候选窗上的一格。
    ///
    /// 候选窗在 Server 手里、收不到按键，所以那一格要上屏的文本只能挂在 `Poll` 的回包里回来。
    /// 只选了一半（候选并进组句）时拼音行也变了，所以帧里那条新拼音行一并落进文档。
    pub(crate) fn apply_poll_commit(&self, frame: &Frame, commit: Option<String>) {
        let preedit = if frame.preedit_mode.inline() {
            preedit_string(frame)
        } else {
            String::new()
        };
        self.shared.set_composing(!frame.is_empty());
        let Some(context) = self.shared.last_context() else {
            log(&format!("鼠标上屏没有上下文，丢弃: {commit:?}"));
            self.shared.reset();
            return;
        };
        log(&format!("鼠标点选上屏: {commit:?} 拼音行={preedit:?}"));
        let requested = request_update(
            &context,
            self.client_id.get(),
            self.engine.clone(),
            self.shared.clone(),
            Update {
                commit,
                preedit,
                ..Update::default()
            },
        );
        if let Err(error) = requested {
            log(&format!("鼠标上屏的编辑会话没被受理: {error}"));
            self.shared.reset();
        }
    }

    /// 经异步编辑会话把上屏文本 + 组句拼音行写进文档；`update` 里还有光标位移与要删的字符数
    /// （见 [`Update`](crate::com::composition::Update)）。
    pub(super) fn update_document(&self, pic: Ref<ITfContext>, update: Update) {
        // 退到最后一个字母时帧已空但组句句柄还在，得跑一次把它收掉；
        // 成对补全跳过右半边（只有光标要挪）与两键符号规则（要删前一个字）也走这一趟。
        if update.is_empty() && !self.shared.composing() && !self.shared.has_composition() {
            return;
        }
        let Ok(context) = pic.ok() else {
            log(&format!("无上下文，丢弃更新: {update:?}"));
            return;
        };
        if let Err(error) = request_update(
            context,
            self.client_id.get(),
            self.engine.clone(),
            self.shared.clone(),
            update,
        ) {
            log(&format!("请求组句更新失败: {error}"));
        }
    }
}
