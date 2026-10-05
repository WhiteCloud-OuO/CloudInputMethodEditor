//! 按键怎么作用到 Engine / 高亮上。

use cloudime_core::{CandidateKind, char_width, shortcut};
use cloudime_platform::pairwise_completion;
use cloudime_platform::protocol::KeyEvent;

use super::{Effect, codes, with_prefix};
use crate::dispatch::Router;

/// 成对补全里右半边对应键盘上的哪个键：`）` → `)`、`”` → `"`；本来就是半角（或没有对应）的原样返回。
fn keyboard_close(close: char) -> char {
    match close {
        '”' => '"',
        '’' => '\'',
        other => char::from_u32(u32::from(other).wrapping_sub(0xFEE0))
            .filter(|half| half.is_ascii_graphic())
            .unwrap_or(other),
    }
}

/// 组句里敲这个键要「先把高亮候选上屏、再上屏这个标点」：可打印 ASCII 标点。
///
/// `-` / `=` 是固定的翻页键（[`codes::page_key`] 在前面先处理掉），`_` / `+` 是它们上档的键、
/// 也是标识符里常见的字符，沿用原来的英文直输段；`'` 是拼音的分隔符（`xi'an`），不能抢；
/// `` ` `` 留给「轮换英文候选大小写」（见 [`Router::apply_printable`]），没英文候选时才进直输段。
fn is_commit_punctuation(c: char) -> bool {
    c.is_ascii_punctuation() && !matches!(c, '\'' | '-' | '=' | '_' | '+' | '`')
}

impl Router {
    /// 功能键靠键码，其余靠字符。带 Ctrl / Alt / Win 的键归应用。
    /// 表达式模式里 Shift + 数字打的是 `^ * ( )`，进算式。
    pub(crate) fn apply_key(&mut self, event: &KeyEvent) -> Effect {
        // 成对补全：上一键补上的右半边，紧接着又敲了一次就跳过去（不再插一个）。别的键也会把它作废。
        if let Some(close) = self.pending_close.take()
            && event.character == Some(close)
            && !self.composing()
            && !event.modifiers.has_command_key()
        {
            self.caret_shift = 1;
            return Effect::Changed(None);
        }
        if let Some(effect) = self.ctrl_digit(event) {
            return effect;
        }
        if let Some(effect) = self.ctrl_enter(event) {
            return effect;
        }
        if event.modifiers.has_command_key() {
            return Effect::Passthrough;
        }
        let Some(c) = event.character.filter(|c| !c.is_control()) else {
            return self.apply_function_key(event);
        };
        let english = event.modifiers.english_mode;
        if english {
            self.apply_english(c, event)
        } else {
            self.apply_chinese(c, event)
        }
    }

    /// Ctrl + 候选数字：用户短语直接上屏（短语没有可杀的学习）；其余中文 / 英文候选「杀掉」——
    /// 自造词从用户词库整个删掉（下次不再出），词库已有的词清掉对它的用户学习（选择次数、同输入串选择、
    /// 个人 n-gram 里与它相关的转移），权重回到词库原始词频；然后由调用方重排。
    /// 没这一格、或这一格既不是短语也不是中文 / 英文候选时返回 `None`，这一键照常交还应用。
    fn ctrl_digit(&mut self, event: &KeyEvent) -> Option<Effect> {
        let modifiers = event.modifiers;
        if !modifiers.ctrl || modifiers.alt || modifiers.win || !self.composing() {
            return None;
        }
        let digit = codes::digit_virtual_key(event.virtual_key)?;
        let index = self.slot_index(digit)?;
        let candidate = self.layout_candidate(index)?;
        if candidate.kind == CandidateKind::Custom {
            // 短语原样上屏，与敲它的序号一样
            return Some(Effect::Changed(self.engine.commit(&candidate)));
        }
        if candidate.kind == CandidateKind::Sentence {
            // 整句：先记一次（记够两次收进用户自造词库），再照常上屏
            let learned = self.engine.remember_sentence(&candidate);
            tracing::debug!(text = %candidate.text, learned, "Ctrl+数字 上屏整句候选");
            return Some(Effect::Changed(self.engine.commit(&candidate)));
        }
        if !matches!(
            candidate.kind,
            CandidateKind::Chinese | CandidateKind::English
        ) {
            return None;
        }
        let forgotten = self.engine.forget(&candidate);
        tracing::debug!(text = %candidate.text, ?forgotten, "杀词");
        self.notice = Some(kill_notice(
            &candidate.text,
            forgotten.user_word,
            forgotten.learning,
        ));
        Some(Effect::Changed(None))
    }

    /// Ctrl + 回车：原样上屏当前字母串（去掉手敲的 `'`），并记一次；同一串记够两次收进自造词库（英文）。
    /// 没在组句、或不是纯 Ctrl+回车时返回 `None`（交还应用）。
    fn ctrl_enter(&mut self, event: &KeyEvent) -> Option<Effect> {
        let modifiers = event.modifiers;
        if !modifiers.ctrl
            || modifiers.alt
            || modifiers.win
            || event.virtual_key != codes::RETURN
            || !self.composing()
        {
            return None;
        }
        Some(Effect::Changed(Some(self.engine.take_raw_english())))
    }

    /// 退格 / Esc / 回车 / Tab / 方向键；没在组句时都交还应用。
    fn apply_function_key(&mut self, event: &KeyEvent) -> Effect {
        if !self.composing() {
            // 回车交给应用：文本流里是一个段落边界
            if event.virtual_key == codes::RETURN {
                self.engine.note_passthrough('\n');
            }
            return Effect::Passthrough;
        }
        match event.virtual_key {
            codes::BACK => {
                self.engine.backspace();
                Effect::Changed(None)
            }
            codes::ESCAPE => {
                self.engine.clear();
                Effect::Changed(None)
            }
            codes::RETURN => Effect::Changed(Some(self.engine.take_raw())),
            codes::TAB if event.modifiers.shift => {
                self.page(-1);
                Effect::Navigated
            }
            codes::TAB => {
                self.page(1);
                Effect::Navigated
            }
            codes::DOWN => {
                self.move_highlight(1);
                Effect::Navigated
            }
            codes::UP => {
                self.move_highlight(-1);
                Effect::Navigated
            }
            codes::NEXT => {
                self.page(1);
                Effect::Navigated
            }
            codes::PRIOR => {
                self.page(-1);
                Effect::Navigated
            }
            codes::LEFT => {
                self.engine.move_cursor_left();
                Effect::Changed(None)
            }
            codes::RIGHT => {
                self.engine.move_cursor_right();
                Effect::Changed(None)
            }
            codes::HOME => {
                self.engine.move_cursor_home();
                Effect::Changed(None)
            }
            codes::END => {
                self.engine.move_cursor_end();
                Effect::Changed(None)
            }
            _ => Effect::Passthrough,
        }
    }

    /// 中文模式：字母进拼音。Shift 敲的大写也收进缓冲区（Core 按小写匹配、原样上屏时还原大小写）；
    /// Caps Lock 亮着的字母仍然直通给应用（应用自己按大小写位插入；全角字符开着时由我们转全角）。
    /// 没在组句时的其他字符走全角标点与全角字符（组句中的标点仍进英文直输段）。
    fn apply_chinese(&mut self, c: char, event: &KeyEvent) -> Effect {
        if c.is_ascii_alphabetic() {
            if !event.modifiers.caps {
                self.engine.push(c);
                return Effect::Changed(None);
            }
            // Caps 亮着：组句里的拼音先原样上屏，字母按「全角字符」设置交给应用或由我们插全角形
            let raw = self.composing().then(|| self.engine.take_raw());
            self.engine.note_passthrough(c);
            return with_prefix(raw, self.passthrough(c), c);
        }
        if !self.composing() {
            return self.apply_punctuation(c, event);
        }
        self.apply_printable(c, event)
    }

    /// 当前模式开着全角就让 Core 转标点（数字后的 `.` 与小键盘的键保持半角）；转不了的原样交给应用并告知 Core。
    /// 标点的全角 / 半角跟着模式走（英文模式半角、中文模式全角），Caps Lock 不参与；标点没转的再看「全角字符」。
    /// 中文模式先走配置里的符号映射（`[input] punctuation_marks_mapping`），小键盘的键只认里面的 `{kp}` 条目。
    fn apply_punctuation(&mut self, c: char, event: &KeyEvent) -> Effect {
        let english = event.modifiers.english_mode;
        let keypad = codes::is_keypad(event.virtual_key);
        if !english && let Some(mapped) = self.engine.map_symbol(c, keypad) {
            // 两键规则要把上一个键的输出换掉
            self.delete_before = mapped.delete_before;
            return self.complete_pair(c, mapped.text);
        }
        if !keypad
            && self.full_width_punctuation_for(english)
            && let Some(text) = self.engine.punctuate(c)
        {
            return self.complete_pair(c, text);
        }
        self.engine.note_passthrough(c);
        // 没转换的左半边也要补全：半角标点模式下 `(` 不走全角表，`{` 这类干脆不在表里，
        // 它们的成对补全（`()`、`{}`）只能在这条路上做。中英一致：只要开了成对补全、且这个键
        // 没被全角表转换，就补（英文 + 西文符号就是这条路）。
        if !keypad && let Some(close) = pairwise_completion(self.config.pairwise_completion, c) {
            return self.insert_pair(c, c, close);
        }
        self.passthrough(c)
    }

    /// 成对补全（`[input] punctuation_marks_pairwise_completion`）：`text` 是开了补全的左半边时，
    /// 连右半边一起上屏、把光标停在中间（`caret_shift = -1`）；左半边就是右半边的那种（引号）不记跳过。
    fn complete_pair(&mut self, typed: char, text: String) -> Effect {
        let mut chars = text.chars();
        let (Some(open), None) = (chars.next(), chars.next()) else {
            return Effect::Changed(Some(text));
        };
        // 转换后是开符号（`（`、`“`）就按它补；转成了收符号或别的（`’`、`》` 这种——引号交替、
        // 半角标点都会给出）就退回按**敲的那个键**补 ASCII 的一对，否则 `''`、`<>` 在中文模式下
        // 永远补不上（真机上报过）。
        let open = match pairwise_completion(self.config.pairwise_completion, open) {
            Some(_) => open,
            None => typed,
        };
        match pairwise_completion(self.config.pairwise_completion, open) {
            Some(close) => self.insert_pair(typed, open, close),
            None => Effect::Changed(Some(text)),
        }
    }

    /// 把 `open` 与配对的 `close` 一起上屏、光标停在中间；记下用户再敲一次右半边时跳过。
    fn insert_pair(&mut self, typed: char, open: char, close: char) -> Effect {
        // 用户下一键会敲的是键盘上那个键（中文全角下敲 `(` 上屏 `（`，配对的还是 `)`）
        let keyboard_close = keyboard_close(close);
        self.pending_close = (keyboard_close != typed).then_some(keyboard_close);
        self.caret_shift = -1;
        Effect::Changed(Some(format!("{open}{close}")))
    }

    /// 英文模式：纯直通，不出候选，字母直接交应用上屏（`eats_key` 那边也放行，应用自己插）。
    /// 缓冲区里还留着上一段拼音时先把它原样上屏，连同这个字母一起由我们插入——Windows 放行是同步的、
    /// 上屏走异步编辑会话，分两步会让应用先插字母再插词。其他键按英文模式那份全角设置转，转不了的交给应用。
    /// 「全角字符」开着时字母也走 [`Self::passthrough`]：DLL 那边会把它们吃掉送来，这里插全角形。
    fn apply_english(&mut self, c: char, event: &KeyEvent) -> Effect {
        let raw = self.composing().then(|| self.engine.take_raw());
        let effect = if c.is_ascii_alphabetic() {
            self.engine.note_passthrough(c);
            self.passthrough(c)
        } else {
            self.apply_punctuation(c, event)
        };
        with_prefix(raw, effect, c)
    }

    /// 本来要直通给应用的一个可见字符：转全角得由我们插（`Changed` 带上转换后的字），不然应用自己插原字符。
    fn passthrough(&self, c: char) -> Effect {
        match self.full_width_char(c) {
            Some(full) => Effect::Changed(Some(full.to_string())),
            None => Effect::Passthrough,
        }
    }

    /// 「全角字符」开着时这个直通字符的全角形；关着或没有全角形为 `None`。
    fn full_width_char(&self, c: char) -> Option<char> {
        if self.config.full_width_chars {
            char_width::full_width(c)
        } else {
            None
        }
    }

    /// 组句中的可打印键：数字选当前页第 N 个（没有这一格就进直输段），翻页键翻页，空格选上高亮候选，
    /// 标点先选高亮候选再把整段补完上屏，其余进英文直输段；已在直输段里就一律追加。
    /// 表达式模式（`v1+2`）里数字和运算符进算式。
    fn apply_printable(&mut self, c: char, event: &KeyEvent) -> Effect {
        let expression = self.engine.expression_mode();
        if expression && shortcut::is_expression_char(c) {
            self.engine.push(c);
            return Effect::Changed(None);
        }
        // 英文直输段（缓冲区里已有 `-` 这类字符）：可见字符一律追加，数字与翻页键也不再选词 / 翻页；
        // 空格整段原样上屏，空格本身也要在（`hello, world`）。
        if self.engine.raw_mode() {
            if c == ' ' {
                let committed = self.finish_composition();
                self.engine.note_passthrough(c);
                return with_prefix(committed, Effect::Passthrough, c);
            }
            if c.is_ascii_graphic() {
                self.engine.push(c);
                return Effect::Changed(None);
            }
        }
        if let Some(digit) = codes::digit(event)
            && let Some(index) = self.slot_index(digit)
        {
            return Effect::Changed(self.commit_index(index));
        }
        if let Some(step) = codes::page_key(event) {
            self.page(step);
            return Effect::Navigated;
        }
        // 空格选中高亮候选：和数字一样只是并进组句，剩下拼音继续出候选；整段转换完才整体上屏。
        // 没有任何候选时（`v`、切不动的串）仍把拼音原样上屏，与以前一致。
        if c == ' ' {
            if self.candidate_count() == 0 {
                let raw = self.engine.take_raw();
                return Effect::Changed((!raw.is_empty()).then_some(raw));
            }
            return Effect::Changed(self.commit_index(self.highlight));
        }
        // 表达式以外的字符不进缓冲区、也不是选词键：先把高亮候选选上、剩余拼音补完整体上屏，
        // 再按没在组句处理这个键（标点按组句外语义转全角）。
        if c != '\'' && expression {
            let committed = self.finish_composition();
            let effect = self.apply_punctuation(c, event);
            return with_prefix(committed, effect, c);
        }
        // 反引号：候选里有英文词时轮换英文候选的大小写（原样 → 全大写 → 首字母大写），不上屏标点
        if c == '`' && self.has_english_candidate() {
            self.engine.cycle_english_case();
            return Effect::Changed(None);
        }
        // 组句里敲标点：先把高亮候选选上、剩余拼音补完整体上屏，再按「没在组句」处理这一键
        // （走符号映射 / 全角标点 / 成对补全）。`-` / `=` 是翻页键（上面已处理），`_` / `+` 与 `'`
        // 由 [`is_commit_punctuation`] 排除，仍进缓冲区。
        if is_commit_punctuation(c) {
            let committed = self.finish_composition();
            let effect = self.apply_punctuation(c, event);
            return with_prefix(committed, effect, c);
        }
        self.engine.push(c);
        Effect::Changed(None)
    }

    /// 数字键在当前页对应的格子下标；这一页没有这一格（`gpt6` 只有三个候选）返回 `None`，数字当内容进缓冲区。
    fn slot_index(&self, digit: usize) -> Option<usize> {
        let page_size = self.config.page_size;
        let index = self.highlight / page_size * page_size + digit - 1;
        (digit <= page_size && index < self.candidate_count()).then_some(index)
    }

    /// 当前候选里有没有英文词（反引号轮换大小写只在有英文候选时生效）。
    fn has_english_candidate(&self) -> bool {
        (0..self.candidate_count()).any(|index| {
            self.layout_candidate(index)
                .is_some_and(|candidate| candidate.kind == CandidateKind::English)
        })
    }

    /// 把当前组句整段交给应用：先选上高亮候选（可能并进组句），再把剩下的未选拼音原样补完一起上屏。
    /// 返回要交给应用的文本；没有候选时就是缓冲原样上屏。调用后组句已清空。
    fn finish_composition(&mut self) -> Option<String> {
        match self.commit_index(self.highlight) {
            Some(text) => Some(text),
            None => {
                let raw = self.engine.take_raw();
                (!raw.is_empty()).then_some(raw)
            }
        }
    }

    fn composing(&self) -> bool {
        !self.engine.composition().is_empty()
    }
}

/// 杀词后候选窗状态行上的提示。
fn kill_notice(text: &str, user_word: bool, learning: bool) -> String {
    let preview: String = text.chars().take(12).collect();
    let preview = if text.chars().count() > 12 {
        format!("{preview}…")
    } else {
        preview
    };
    if user_word {
        format!("已删掉自造词「{preview}」")
    } else if learning {
        format!("已重置「{preview}」的权重")
    } else {
        format!("「{preview}」没有可清除的学习")
    }
}
