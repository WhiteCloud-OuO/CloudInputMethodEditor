//! 智能符号：中文模式下，按空格时按光标前的内容换成标点。
//!
//! 两个空格分两次按键完成：第一次先上屏逗号并记住原来光标前的字，第二次再用
//! `delete_before` 把那个逗号换掉 —— 第一下不用延迟上屏。
//!
//! 两次空格必须在 [`SMART_SPACE_INTERVAL`] 内；超时后第二下按普通空格处理，不会误替换。
//! 计时不另起后台线程 / 定时器：记下第一击的时刻，第二击进来时比一下即可。

use std::time::{Duration, Instant};

use cloudime_core::{SmartSymbol, smart_space_double, smart_space_single};
use cloudime_platform::pairwise_completion;
use cloudime_platform::protocol::KeyEvent;

use super::input::keyboard_close;
use super::{Effect, codes};
use crate::dispatch::Router;

/// 两次空格的最大间隔：超过就当成两次独立的空格。
const SMART_SPACE_INTERVAL: Duration = Duration::from_secs(1);

/// 上一次智能空格插入了什么、插入前光标前是哪个字。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending {
    /// 第一击之前光标前的字符，双击时按它选符号。
    pub(super) before: Option<char>,
    /// 第一击真正插进去的字符（双击时要用 `delete_before` 撤掉）。
    pub(super) inserted: char,
    /// 第一击的时刻：第二击离它超过 [`SMART_SPACE_INTERVAL`] 就不再换符号。
    pub(super) at: Instant,
}

impl Router {
    /// 任何键进 Router 时先作废「双击空格」的候选；只有紧接着再按空格才算第二击。
    pub(crate) fn note_key_for_smart_symbol(&mut self, event: &KeyEvent) {
        if event.virtual_key != codes::SPACE {
            self.smart_space = None;
        }
    }

    /// DLL 送来的光标前文更新「光标前一个字符」；空串不覆盖本进程刚上屏的文字
    /// （记事本 / OpenCode 这类宿主不给前文，本地记录还得留着）。
    ///
    /// 刚上屏时 DLL 可能还送来一份**上屏之前**读的前文（编辑会话是异步的，读和写谁先跑不保证）。
    /// 那份的末尾字对不上刚上屏的结果，忽略一次；下一份前文再恢复采用。
    pub(crate) fn note_text_before_caret(&mut self, before: Option<char>) {
        if let Some(expected) = self.smart_guard.take()
            && before != Some(expected)
        {
            return;
        }
        if before.is_some() {
            self.smart_before = before;
        }
    }

    /// 一次按键结束：把「光标前一个字符」推进到按键后的状态。
    pub(crate) fn note_key_result(&mut self, event: &KeyEvent, commit: Option<&str>) {
        if let Some(text) = commit.filter(|text| !text.is_empty()) {
            let chars: Vec<char> = text.chars().collect();
            // 成对补全把光标退回到引号中间：光标前是开引号，不是最后的收引号。
            let caret = if self.caret_shift < 0 {
                chars
                    .len()
                    .saturating_sub(self.caret_shift.unsigned_abs() as usize)
            } else {
                chars.len()
            };
            self.smart_before = caret
                .checked_sub(1)
                .and_then(|index| chars.get(index))
                .copied();
            self.smart_guard = self.smart_before;
            return;
        }
        // 交给应用的可见字符最终也会落在光标前；带 Ctrl / Alt / Win 的热键不算。
        if let Some(c) = event.character.filter(|c| !c.is_control())
            && !event.modifiers.has_command_key()
        {
            self.smart_before = Some(c);
            return;
        }
        if clears_caret(event.virtual_key) {
            self.smart_before = None;
        }
    }

    /// 中文模式、没在组句时按空格：智能符号命中就返回上屏结果，否则交还应用。
    pub(crate) fn apply_smart_space(&mut self) -> Option<Effect> {
        if !self.config.smart_symbols {
            return None;
        }
        // 第二击：把第一击插进去的逗号删掉再换成对应标点。超过 1 秒就当成新的空格，
        // 第一击的逗号留着，这一下按普通空格交给应用。
        if let Some(pending) = self.smart_space.take() {
            if pending.at.elapsed() >= SMART_SPACE_INTERVAL {
                return None;
            }
            if let Some(symbol) = smart_space_double(pending.before) {
                return Some(self.commit_smart_symbol(symbol, Some(pending.inserted)));
            }
        }
        let symbol = smart_space_single(self.smart_before)?;
        let inserted = symbol.text().chars().next_back()?;
        self.smart_space = Some(Pending {
            before: self.smart_before,
            inserted,
            at: Instant::now(),
        });
        Some(self.commit_smart_symbol(symbol, None))
    }

    /// 上屏一个智能符号；`replaced` 是双击时要先删掉的第一击输出。
    fn commit_smart_symbol(&mut self, symbol: SmartSymbol, replaced: Option<char>) -> Effect {
        let mut text = symbol.text().to_owned();
        let mut caret_shift = 0;
        // 「说」后面的开引号跟着成对补全：右半边一起上屏，光标停在中间。
        if symbol.opens_quote()
            && let Some(close) = pairwise_completion(self.config.pairwise_completion, '“')
        {
            text.push(close);
            caret_shift = -1;
            let close_key = keyboard_close(close);
            if close_key != ' ' {
                self.pending_close = Some(close_key);
            }
        }
        if let Some(inserted) = replaced {
            self.delete_before = inserted.len_utf16() as u16;
        }
        self.caret_shift = caret_shift;
        self.engine.commit_symbol(&text);
        Effect::Changed(Some(text))
    }
}

/// 这些键之后光标会离开原来的位置，本地记的「光标前一个字符」不再可信。
fn clears_caret(virtual_key: u32) -> bool {
    matches!(
        virtual_key,
        codes::BACK
            | codes::DELETE
            | codes::RETURN
            | codes::TAB
            | codes::ESCAPE
            | codes::PRIOR
            | codes::NEXT
            | codes::END
            | codes::HOME
            | codes::LEFT
            | codes::UP
            | codes::RIGHT
            | codes::DOWN
    )
}
