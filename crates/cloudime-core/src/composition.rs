//! 输入状态机：拼音缓冲区、已选文本与光标。
//!
//! 只维护「用户已经敲了什么、选了哪些、光标在哪」，不理解拼音，也不知道候选。
//! 缓冲区只含未选拼音（ASCII 小写字母和 `'`，表达式模式下还有数字与运算符，见 `shortcut`），
//! 所以字节下标即字符下标。全拼里 Shift+字母不进拼音缓冲区，按英文字面量（[`Literal`]）记在
//! 缓冲区的字节偏移上；双键拼音的辅助码大写仍按小写进缓冲区，记在 `shifted` 里。
//!
//! 中文模式选中一个候选**不立刻上屏**：它并进 [`Composition::select_prefix`] 记下的已选段，
//! 组句显示成「已选文本 + 未选拼音」（`云朵shurufa`），等整段转换完或回车 / 标点时再整体交给应用；
//! 退格先 [`Composition::unselect_last`] 撤回最后一次选择（把中文还原成它的拼音）。

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Composition {
    /// 用户已敲入、尚未选中的拼音，匹配用的小写形式。
    buffer: String,

    /// 与 `buffer` 的字符一一对应：该位是按住 Shift 敲的大写字母（双键拼音的辅助码）。
    shifted: Vec<bool>,

    /// 未选拼音里的英文字面量：`at` 是 `buffer` 的字节偏移，`text` 是连续的大写英文。
    ///
    /// 全拼里大写字母不再进 `buffer`、也不参与拼音匹配；它们按这个位置藏在拼音串里，
    /// 选择候选时随拼音一起并进已选文本（`su'A` 选「苏」得到「苏A」）。
    literals: Vec<Literal>,

    /// 光标位置（字节下标，`0..=buffer.len()`），插入和退格都相对它。只落在未选拼音范围内。
    cursor: usize,

    /// 组句里已经选定、还没整体上屏的段，按选择顺序排列，显示在未选拼音之前。
    selected: Vec<Selected>,
}

/// 一段英文字面量（全拼里的大写字母），`at` 相对未选拼音的缓冲区。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Literal {
    at: usize,
    text: String,
}

/// 一段已选文本：显示文本 + 它消耗掉的原样拼音（退格撤回时还原）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Selected {
    /// 显示文本（可能是繁体）。
    text: String,

    /// 被消耗掉的小写拼音（与 [`Composition::buffer`] 同格式）。
    pinyin: String,

    /// 与 `pinyin` 的字符一一对应：该位是按 Shift 敲的大写字母。
    shifted: Vec<bool>,

    /// 这段选择里夹带的英文字面量（`at` 相对 `pinyin` 开头），撤回选择时原样还原。
    literals: Vec<Literal>,
}

impl Composition {
    pub fn text(&self) -> &str {
        &self.buffer
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty() && self.buffer.is_empty() && self.literals.is_empty()
    }

    /// 缓冲区（未选拼音）的字节长度。
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    /// 组句里有没有已选段。
    pub fn has_selected(&self) -> bool {
        !self.selected.is_empty()
    }

    /// 已选文本按顺序拼起来（含英文字面量）。
    pub fn selected_text(&self) -> String {
        self.selected.iter().map(|s| s.text.as_str()).collect()
    }

    /// 已选段消耗掉的拼音按顺序拼起来（不含英文字面量）。
    pub fn selected_pinyin(&self) -> String {
        self.selected.iter().map(|s| s.pinyin.as_str()).collect()
    }

    /// 完整未上屏组合（已选文本 + 未选拼音原样），回车原样上屏用。
    pub fn raw_text(&self) -> String {
        let mut text = self.selected_text();
        text.push_str(&self.typed(self.buffer.len()));
        text
    }

    /// [`Self::raw_text`] 里的 UTF-8 字节光标：已选文本之后，再加上未选拼音（含英文字面量）的光标。
    pub fn raw_cursor(&self) -> usize {
        let selected: usize = self.selected.iter().map(|s| s.text.len()).sum();
        selected + self.typed_cursor()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 参与候选生成的那段拼音：光标停在中间时只算光标前的（`ni|hao` → `ni`），
    /// 光标在开头或末尾时算整段。搜狗的行为：往回挪光标就是要先把前半段上屏。
    pub fn scope(&self) -> &str {
        if self.cursor == 0 || self.cursor >= self.buffer.len() {
            &self.buffer
        } else {
            &self.buffer[..self.cursor]
        }
    }

    /// 作用域之后剩下的拼音（`ni|hao` → `hao`），只用于显示。
    pub fn rest(&self) -> &str {
        &self.buffer[self.scope().len()..]
    }

    /// 字节位置 `byte`（须落在字符边界上）对应的字符下标。
    fn char_index(&self, byte: usize) -> usize {
        self.buffer[..byte].chars().count()
    }

    /// 在光标处插入一个字符，大小写原样保留。
    pub fn push(&mut self, c: char) {
        // 英文字面量本身就是音节边界，紧挨着它的 `'` 不再进拼音缓冲区（`K'ba` 存成 `K` + `ba`）。
        if c == '\''
            && self
                .literals
                .iter()
                .any(|literal| literal.at == self.cursor)
        {
            return;
        }
        let index = self.char_index(self.cursor);
        let len = c.len_utf8();
        self.buffer.insert(self.cursor, c);
        self.shifted.insert(index, false);
        for literal in &mut self.literals {
            if literal.at > self.cursor {
                literal.at += len;
            }
        }
        self.cursor += len;
    }

    /// 全拼里的大写字母：记在光标处的英文字面量里，不进拼音缓冲区。
    pub fn push_literal(&mut self, c: char) {
        // 大写前后刚敲的 `'` 由这个字面量代替，别让它留在拼音串里
        // （`su'A` 的 `'` 被吃掉，缓冲区是 `su`，选择时 A 排在「苏」后面）。
        let remove_at = if self.cursor > 0 && self.buffer[..self.cursor].ends_with('\'') {
            Some(self.cursor - 1)
        } else if self.buffer[self.cursor..].starts_with('\'') {
            Some(self.cursor)
        } else {
            None
        };
        if let Some(at) = remove_at {
            let index = self.char_index(at);
            self.buffer.remove(at);
            self.shifted.remove(index);
            if at < self.cursor {
                self.cursor -= 1;
            }
            for literal in &mut self.literals {
                if literal.at > at {
                    literal.at -= 1;
                }
            }
        }
        match self
            .literals
            .iter_mut()
            .find(|literal| literal.at == self.cursor)
        {
            Some(literal) => literal.text.push(c),
            None => {
                self.literals.push(Literal {
                    at: self.cursor,
                    text: c.to_string(),
                });
                self.literals.sort_by_key(|literal| literal.at);
            }
        }
    }

    /// 中文模式下按住 Shift 敲的字母：按小写进缓冲区参与匹配，原样上屏时还原大写。
    pub fn push_shifted(&mut self, c: char) {
        let index = self.char_index(self.cursor);
        self.push(c.to_ascii_lowercase());
        if let Some(shifted) = self.shifted.get_mut(index) {
            *shifted = true;
        }
    }

    /// 缓冲区里有没有 Shift 敲的大写字母。
    pub fn has_shifted(&self) -> bool {
        self.shifted.iter().any(|shifted| *shifted)
    }

    /// 未选拼音里有没有英文字面量（全拼的大写字母）。
    pub fn has_literals(&self) -> bool {
        !self.literals.is_empty()
    }

    /// 缓冲区原样（`shifted` 的位置还原大写），回车原样上屏用。
    pub fn typed_text(&self) -> String {
        self.typed(self.buffer.len())
    }

    /// [`Self::scope`] 的原样形式。
    pub fn typed_scope(&self) -> String {
        self.typed(self.scope().len())
    }

    /// 前 `len` 字节的原样形式，英文字面量按位置插回去。
    fn typed(&self, len: usize) -> String {
        let mut text = String::new();
        let mut literal_index = 0;
        let mut byte = 0;
        for (index, c) in self.buffer.chars().enumerate() {
            if byte >= len {
                break;
            }
            while let Some(literal) = self.literals.get(literal_index) {
                if literal.at <= byte {
                    text.push_str(&literal.text);
                    literal_index += 1;
                } else {
                    break;
                }
            }
            text.push(if self.shifted.get(index) == Some(&true) {
                c.to_ascii_uppercase()
            } else {
                c
            });
            byte += c.len_utf8();
        }
        while let Some(literal) = self.literals.get(literal_index) {
            if literal.at <= len {
                text.push_str(&literal.text);
                literal_index += 1;
            } else {
                break;
            }
        }
        text
    }

    /// 未选拼音（含英文字面量）在拼音行里的光标字节位置。
    pub fn typed_cursor(&self) -> usize {
        self.typed(self.cursor).len()
    }

    /// 给查询 / 拼音行用的显示文本：有英文字面量时把它们插回去，否则就是原拼音缓冲区。
    pub fn display_text(&self) -> String {
        if self.has_literals() {
            self.typed_text()
        } else {
            self.buffer.clone()
        }
    }

    /// [`Self::display_text`] 里的光标字节位置。
    pub fn display_cursor(&self) -> usize {
        if self.has_literals() {
            self.typed_cursor()
        } else {
            self.cursor
        }
    }

    /// 删掉光标前一个字符。光标在开头时返回 `false`。
    pub fn backspace(&mut self) -> bool {
        // 光标停在末尾时，先删末尾的英文字面量；没有字面量再删拼音。
        if self.cursor >= self.buffer.len()
            && let Some(index) = self
                .literals
                .iter()
                .position(|literal| literal.at == self.cursor)
        {
            self.literals[index].text.pop();
            if self.literals[index].text.is_empty() {
                self.literals.remove(index);
            }
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        let index = self.char_index(self.cursor) - 1;
        self.cursor -= 1;
        self.buffer.remove(self.cursor);
        self.shifted.remove(index);
        for literal in &mut self.literals {
            if literal.at > self.cursor {
                literal.at -= 1;
            }
        }
        true
    }

    /// 删掉光标后一个字符。光标在末尾时返回 `false`。
    pub fn delete_forward(&mut self) -> bool {
        // 光标后面没有拼音、紧跟着英文字面量时，删字面量。
        if self.cursor >= self.buffer.len()
            && let Some(index) = self
                .literals
                .iter()
                .position(|literal| literal.at == self.cursor)
        {
            self.literals[index].text.remove(0);
            if self.literals[index].text.is_empty() {
                self.literals.remove(index);
            }
            return true;
        }
        if self.cursor >= self.buffer.len() {
            return false;
        }
        let index = self.char_index(self.cursor);
        self.buffer.remove(self.cursor);
        self.shifted.remove(index);
        for literal in &mut self.literals {
            if literal.at > self.cursor {
                literal.at -= 1;
            }
        }
        true
    }

    /// 光标左移一格。已在开头时返回 `false`。
    pub fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    /// 光标右移一格。已在末尾时返回 `false`。
    pub fn move_right(&mut self) -> bool {
        if self.cursor >= self.buffer.len() {
            return false;
        }
        self.cursor += 1;
        true
    }

    /// 删掉光标前 `len` 个字节（不够就删到开头）。光标在开头或 `len` 为零时返回 `false`。
    pub fn delete_before_cursor(&mut self, len: usize) -> bool {
        let len = len.min(self.cursor);
        if len == 0 {
            return false;
        }
        let from = self.char_index(self.cursor - len);
        let to = self.char_index(self.cursor);
        let new_cursor = self.cursor - len;
        self.buffer.drain(self.cursor - len..self.cursor);
        self.shifted.drain(from..to);
        self.literals
            .retain(|literal| literal.at <= new_cursor || literal.at > self.cursor);
        for literal in &mut self.literals {
            if literal.at > self.cursor {
                literal.at -= len;
            }
        }
        self.cursor = new_cursor;
        true
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.buffer.len();
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.shifted.clear();
        self.literals.clear();
        self.cursor = 0;
        self.selected.clear();
    }

    /// 选中作用域前 `consumed` 字节的拼音为文本 `text`：拼音移出缓冲区、记进已选段，
    /// 光标随之落在剩余拼音里。选中只是并进组句，不立刻交给应用（见文件头）。
    pub fn select_prefix(&mut self, text: &str, consumed: usize) {
        let mut len = consumed.min(self.buffer.len());
        // 选中音节后面的 `'` 跟着一起吃掉：`yun'duo` 选 云 之后剩 `duo`，
        // 不留下一个空的 `'`（特殊输入逐音节选字时会卡在最后一个 `'` 上）。
        if consumed > 0 && self.buffer[len..].starts_with('\'') {
            len += 1;
        }
        let removed = self.char_index(len);
        let pinyin = self.buffer[..len].to_owned();
        let shifted: Vec<bool> = self.shifted.drain(..removed).collect();
        // 夹在这段拼音里的英文字面量跟着并进已选文本：开头的排在候选前（`K'ba` 选 吧 → `K吧`），
        // 末尾的排在候选后（`su'A` 选 苏 → `苏A`）。
        let mut literals = Vec::new();
        let mut prefix = String::new();
        let mut suffix = String::new();
        self.literals.retain(|literal| {
            if literal.at <= len {
                if literal.at == 0 {
                    prefix.push_str(&literal.text);
                } else {
                    suffix.push_str(&literal.text);
                }
                literals.push(literal.clone());
                false
            } else {
                true
            }
        });
        for literal in &mut self.literals {
            literal.at -= len;
        }
        self.buffer.drain(..len);
        self.cursor = self.cursor.saturating_sub(len).min(self.buffer.len());
        self.selected.push(Selected {
            text: format!("{prefix}{text}{suffix}"),
            pinyin,
            shifted,
            literals,
        });
    }

    /// 撤回最后一次选择：把它的拼音还原回缓冲区开头，光标落在还原段之后。没有已选段返回 `false`。
    pub fn unselect_last(&mut self) -> bool {
        let Some(segment) = self.selected.pop() else {
            return false;
        };
        let shift = segment.pinyin.len();
        for literal in &mut self.literals {
            literal.at += shift;
        }
        for literal in segment.literals {
            self.add_literal(literal);
        }
        self.buffer.insert_str(0, &segment.pinyin);
        let mut shifted = segment.shifted;
        shifted.append(&mut self.shifted);
        self.shifted = shifted;
        self.cursor = segment.pinyin.len();
        true
    }

    fn add_literal(&mut self, literal: Literal) {
        if literal.text.is_empty() {
            return;
        }
        match self
            .literals
            .iter_mut()
            .find(|existing| existing.at == literal.at)
        {
            Some(existing) => existing.text.push_str(&literal.text),
            None => self.literals.push(literal),
        }
        self.literals.sort_by_key(|literal| literal.at);
    }

    /// 删掉开头 `len` 个字节（上屏消耗掉的拼音），光标随之前移；
    /// 被吃掉的部分盖过了光标（`ni|hao` 上屏了 你）时光标落到末尾，接着组句就是往后打。
    pub fn drain_prefix(&mut self, len: usize) {
        let len = len.min(self.buffer.len());
        let removed = self.char_index(len);
        self.buffer.drain(..len);
        self.shifted.drain(..removed);
        self.literals.retain(|literal| literal.at > len);
        for literal in &mut self.literals {
            literal.at -= len;
        }
        self.cursor = if self.cursor > len {
            self.cursor - len
        } else {
            self.buffer.len()
        };
    }

    /// 吃掉整个作用域（云端词 / 英文词 / 整句对应的是整个作用域）。
    pub fn drain_scope(&mut self) {
        let len = self.scope().len();
        self.drain_prefix(len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Composition {
        let mut composition = Composition::default();
        for c in text.chars() {
            composition.push(c);
        }
        composition
    }

    #[test]
    fn push_and_backspace() {
        let mut composition = Composition::default();
        // 原样保留大小写（英文直输段 `no-Way` 要用），中文模式下壳只会送小写
        composition.push('K');
        composition.push('a');
        assert_eq!(composition.text(), "Ka");
        assert_eq!(composition.cursor(), 2);
        assert!(composition.backspace());
        assert!(composition.backspace());
        assert!(!composition.backspace());
        assert!(composition.is_empty());
    }

    #[test]
    fn shifted_letters_match_lowercase_and_restore_case_on_raw_commit() {
        let mut composition = Composition::default();
        composition.push_shifted('C');
        composition.push('p');
        composition.push('a');
        composition.push('n');
        // 匹配用的小写形式
        assert_eq!(composition.text(), "cpan");
        assert_eq!(composition.scope(), "cpan");
        // 原样上屏用的大小写
        assert_eq!(composition.typed_text(), "Cpan");
        assert_eq!(composition.typed_scope(), "Cpan");
        // 退格把后面的小写删掉，大写那一位还留着
        for _ in 0..3 {
            composition.backspace();
        }
        composition.push('b');
        assert_eq!(composition.text(), "cb");
        assert_eq!(composition.typed_text(), "Cb");
        composition.clear();
        assert_eq!(composition.typed_text(), "");
        assert!(!composition.has_shifted());
    }

    #[test]
    fn edits_happen_at_the_cursor() {
        let mut composition = typed("kaifa");
        assert!(composition.move_left());
        assert!(composition.move_left());
        composition.push('n');
        assert_eq!(composition.text(), "kainfa");
        assert_eq!(composition.cursor(), 4);
        assert!(composition.backspace());
        assert_eq!(composition.text(), "kaifa");
        assert!(composition.delete_forward());
        assert_eq!(composition.text(), "kaia");
        composition.move_home();
        assert!(!composition.move_left());
        assert!(!composition.backspace());
        composition.move_end();
        assert!(!composition.move_right());
        assert!(!composition.delete_forward());
    }

    #[test]
    fn delete_before_cursor_stops_at_the_start() {
        let mut composition = typed("kaifa");
        composition.move_left();
        assert!(composition.delete_before_cursor(2));
        assert_eq!(composition.text(), "kaa");
        assert_eq!(composition.cursor(), 2);
        assert!(composition.delete_before_cursor(5));
        assert_eq!(composition.text(), "a");
        assert_eq!(composition.cursor(), 0);
        assert!(!composition.delete_before_cursor(1));
        assert!(!composition.delete_before_cursor(0));
    }

    #[test]
    fn scope_is_text_before_cursor_unless_cursor_is_at_either_end() {
        let mut composition = typed("nihao");
        assert_eq!(composition.scope(), "nihao");
        assert_eq!(composition.rest(), "");
        composition.move_left();
        composition.move_left();
        composition.move_left();
        assert_eq!(composition.scope(), "ni");
        assert_eq!(composition.rest(), "hao");
        composition.move_home();
        assert_eq!(composition.scope(), "nihao");
        assert_eq!(composition.rest(), "");
    }

    #[test]
    fn draining_past_the_cursor_moves_it_to_the_end() {
        let mut composition = typed("nihao");
        for _ in 0..3 {
            composition.move_left();
        }
        composition.drain_scope();
        assert_eq!(composition.text(), "hao");
        assert_eq!(composition.cursor(), 3);
    }

    #[test]
    fn drain_prefix_moves_cursor_back() {
        let mut composition = typed("kaifazhe");
        composition.move_left();
        composition.drain_prefix(5);
        assert_eq!(composition.text(), "zhe");
        assert_eq!(composition.cursor(), 2);
        composition.drain_prefix(10);
        assert!(composition.is_empty());
        assert_eq!(composition.cursor(), 0);
    }
}
