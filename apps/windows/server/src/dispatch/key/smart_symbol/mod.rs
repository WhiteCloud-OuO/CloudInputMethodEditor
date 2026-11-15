//! 智能符号：中文模式下，按空格时按光标前的内容换成标点。
//!
//! 两个空格分两次按键完成：第一次先上屏逗号并记住原来光标前的字，第二次再用
//! `delete_before` 把那个逗号换掉 —— 第一下不用延迟上屏。
//!
//! 双击的符号先按用户的规则文件（[`rules`]：`intelligent-symbol.json`，按光标前**整句**正匹配），
//! 没命中再走内置规则（`smart_space_double`），最后兜底句号。
//!
//! 两次空格必须在 [`SMART_SPACE_INTERVAL`] 内；超时后第二下按普通空格处理，不会误替换。
//! 计时不另起后台线程 / 定时器：记下第一击的时刻，第二击进来时比一下即可。

mod rules;

use std::time::{Duration, Instant};

use cloudime_core::{SmartSymbol, smart_space_double, smart_space_single};
use cloudime_platform::pairwise_completion;
use cloudime_platform::protocol::KeyEvent;

use super::input::keyboard_close;
use super::{Effect, codes};
use crate::dispatch::Router;

pub(crate) use rules::{Rules, rules_path};

/// 两次空格的最大间隔：超过就当成两次独立的空格。
const SMART_SPACE_INTERVAL: Duration = Duration::from_secs(1);

/// 本地记的「光标前文」最多留多少个字：匹配一句够了，也免得长文档越攒越大。
const SMART_TEXT_CHARS: usize = 160;

/// 上一次智能空格插入了什么、插入前光标前是哪个字。
#[derive(Debug, Clone)]
pub(crate) struct Pending {
    /// 第一击之前光标前的字符，双击时按它选符号。
    pub(super) before: Option<char>,
    /// 第一击之前光标前的本地前文：用户规则按它的**整句**匹配（第一击插进去的逗号不算）。
    pub(super) sentence: String,
    /// 第一击真正插进去的字符（双击时要用 `delete_before` 撤掉）。
    pub(super) inserted: char,
    /// 第一击的时刻：第二击离它超过 [`SMART_SPACE_INTERVAL`] 就不再换符号。
    pub(super) at: Instant,
}

impl Router {
    /// 读 `intelligent-symbol.json` 换上用户规则（启动与热加载时调；文件不在就是空表）。
    pub fn load_smart_symbol_rules(&mut self, user_dir: &std::path::Path) {
        self.smart_symbols = Rules::load(&rules_path(user_dir));
    }

    /// 任何键进 Router 时先作废「双击空格」的候选；只有紧接着再按空格才算第二击。
    pub(crate) fn note_key_for_smart_symbol(&mut self, event: &KeyEvent) {
        if event.virtual_key != codes::SPACE {
            self.smart_space = None;
        }
    }

    /// DLL 送来的光标前文：更新「光标前一个字符」与本地那份「光标前文」（智能符号按整句匹配用）。
    /// 空串不覆盖本进程刚上屏的文字（记事本 / OpenCode 这类宿主不给前文，本地记录还得留着）。
    ///
    /// 刚上屏时 DLL 可能还送来一份**上屏之前**读的前文（编辑会话是异步的，读和写谁先跑不保证）。
    /// 那份的末尾字对不上刚上屏的结果，忽略一次；下一份前文再恢复采用。
    pub(crate) fn note_text_before_caret(&mut self, before: &str) {
        let last = before.chars().next_back();
        if let Some(expected) = self.smart_guard.take()
            && last != Some(expected)
        {
            return;
        }
        if !before.is_empty() {
            self.smart_before = last;
            self.smart_text = tail(before, SMART_TEXT_CHARS);
        }
    }

    /// 一次按键结束：把「光标前一个字符」与本地那份前文推进到按键后的状态。
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
            // 上屏的文字接在本地前文后面：它可能带开引号，光标停在中间时只算到光标前。
            let committed: String = chars[..caret].iter().collect();
            self.push_smart_text(&committed);
            return;
        }
        // 交给应用的可见字符最终也会落在光标前；带 Ctrl / Alt / Win 的热键不算。
        if let Some(c) = event.character.filter(|c| !c.is_control())
            && !event.modifiers.has_command_key()
        {
            self.smart_before = Some(c);
            self.push_smart_text(&c.to_string());
            return;
        }
        if clears_caret(event.virtual_key) {
            self.smart_before = None;
            self.smart_text.clear();
        }
    }

    /// 把刚落到光标前的文字接进本地前文，只留最后 [`SMART_TEXT_CHARS`] 个字。
    fn push_smart_text(&mut self, text: &str) {
        self.smart_text.push_str(text);
        let extra = self
            .smart_text
            .chars()
            .count()
            .saturating_sub(SMART_TEXT_CHARS);
        if extra > 0 {
            let cut = self
                .smart_text
                .char_indices()
                .nth(extra)
                .map_or(self.smart_text.len(), |(index, _)| index);
            self.smart_text.drain(..cut);
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
            // 先看用户规则（按光标前那一整句的正则），没命中再走内置规则（最终兜底句号）。
            // 用的是第一击之前的前文：第一击的逗号是待替换的占位，不算进句子里。
            if let Some(symbol) = self
                .smart_symbols
                .symbol_for(rules::current_sentence(&pending.sentence))
            {
                let symbol = symbol.to_owned();
                return Some(self.commit_smart_text(&symbol, Some(pending.inserted)));
            }
            if let Some(symbol) = smart_space_double(pending.before) {
                return Some(self.commit_smart_symbol(symbol, Some(pending.inserted)));
            }
        }
        let symbol = smart_space_single(self.smart_before)?;
        let inserted = symbol.text().chars().next_back()?;
        self.smart_space = Some(Pending {
            before: self.smart_before,
            sentence: self.smart_text.clone(),
            inserted,
            at: Instant::now(),
        });
        Some(self.commit_smart_symbol(symbol, None))
    }

    /// 上屏一个智能符号；`replaced` 是双击时要先删掉的第一击输出。
    fn commit_smart_symbol(&mut self, symbol: SmartSymbol, replaced: Option<char>) -> Effect {
        self.commit_smart_text(symbol.text(), replaced)
    }

    /// 上屏一段智能符号文本（用户规则或内置符号）；`replaced` 是双击时要先删掉的第一击输出。
    fn commit_smart_text(&mut self, symbol: &str, replaced: Option<char>) -> Effect {
        let mut text = symbol.to_owned();
        let mut caret_shift = 0;
        // 「说」后面的开引号跟着成对补全：右半边一起上屏，光标停在中间。
        if symbol == SmartSymbol::OpeningQuote.text()
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

/// 一段文本的最后 `limit` 个字（本地前文只留尾部）。
fn tail(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_owned();
    }
    let cut = text
        .char_indices()
        .nth(count - limit)
        .map_or(0, |(index, _)| index);
    text[cut..].to_owned()
}
