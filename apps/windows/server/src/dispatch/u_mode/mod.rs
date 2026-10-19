//! u 模式：符号快速输入。
//!
//! 进入后候选窗口固定六类符号；旁边挂一张四排键盘格子（复用候选窗的辅助窗口资产）。
//! 第二个键是字母时退出符号窗口，接回普通拼音 / 中英混输；其余按键按符号表上屏。
//! 具体映射在 [`mapping`] 里，按键流程不依赖表的内容。

mod mapping;

use cloudime_core::{Candidate, CandidateKind};
use cloudime_platform::LayoutMode;
use cloudime_platform::input_schemes::{KeyCap, KeyMapLayout};
use cloudime_platform::protocol::KeyEvent;

use self::mapping::{GreekState, KEY_ROWS, Kana, SymbolCategory};
use super::Router;
use super::key::Effect;

/// u 模式的状态：当前高亮分类、假名种类、制表符/箭头映射套数、希腊重音/隔音状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct UMode {
    category: usize,
    kana: Kana,
    variant: usize,
    greek: GreekState,
    /// `true` = 符号输入（候选六类 + 符号窗口）；`false` = 默认的字母输入。
    pub(super) symbol_ui: bool,
}

impl UMode {
    /// 当前分类。
    fn category(&self) -> SymbolCategory {
        SymbolCategory::ALL[self.category.min(SymbolCategory::ALL.len() - 1)]
    }

    /// 当前分类下标（跨页高亮用）。
    pub(super) fn category_index(&self) -> usize {
        self.category.min(SymbolCategory::ALL.len() - 1)
    }

    /// 方向键移动分类；越界夹住。
    pub(super) fn move_category(&mut self, delta: isize) {
        self.category = (self.category as isize + delta)
            .clamp(0, SymbolCategory::ALL.len() as isize - 1) as usize;
    }

    /// 数字键 / 鼠标点分类。
    pub(super) fn set_category(&mut self, index: usize) {
        self.category = index.min(SymbolCategory::ALL.len() - 1);
    }

    /// Tab：假名切平/片，制表符|箭头循环映射；别的分类不动。
    pub(super) fn tab(&mut self) {
        match self.category() {
            SymbolCategory::Kana => self.kana = self.kana.toggle(),
            SymbolCategory::TabArrow => self.variant = (self.variant + 1) % 2,
            _ => {}
        }
    }

    /// 候选窗口里的六条候选。
    pub(super) fn candidates(&self) -> Vec<Candidate> {
        SymbolCategory::ALL
            .iter()
            .map(|category| Candidate {
                text: category.label().to_owned(),
                display: Some(category.label().to_owned()),
                kind: CandidateKind::Shortcut,
                syllables: Vec::new(),
                reading: None,
                auxiliary: None,
                auxiliary_typed: None,
            })
            .collect()
    }

    /// 符号输入窗口的四排键位。
    pub(super) fn layout(&self) -> KeyMapLayout {
        let rows = KEY_ROWS
            .iter()
            .map(|row| row.chars().map(|key| self.key_cap(key)).collect())
            .collect();
        let footer = match self.category() {
            SymbolCategory::Kana => "按下Tab切换平假名/片假名".to_owned(),
            SymbolCategory::Cyrillic => "Ё/ё 在前斜杠键".to_owned(),
            // 没有提示也留一行，符号窗口高度不随分类跳。
            _ => String::new(),
        };
        KeyMapLayout {
            rows,
            zero_initials: Vec::new(),
            footer: Some(footer),
            symbol: true,
        }
    }

    fn key_cap(&self, key: char) -> KeyCap {
        let label = key.to_string();
        let normal = mapping::symbol(
            self.category(),
            self.kana,
            self.variant,
            self.greek,
            &label,
            false,
        )
        .unwrap_or_default();
        let shifted = mapping::symbol(
            self.category(),
            self.kana,
            self.variant,
            self.greek,
            &label,
            true,
        )
        .unwrap_or_default();
        KeyCap {
            key: label,
            initial: String::new(),
            mapping: normal.to_owned(),
            shifted: shifted.to_owned(),
        }
    }

    /// 当前分类里这个键对应的上屏符号（Shift 取上档；没有上档就不出符号）。
    pub(super) fn symbol(&self, event: &KeyEvent) -> Option<&'static str> {
        let key = key_label(event.virtual_key)?;
        mapping::symbol(
            self.category(),
            self.kana,
            self.variant,
            self.greek,
            &key,
            event.modifiers.shift,
        )
    }

    /// 希腊字母里 `;` / `'` 是重音 / 隔音状态开关；命中返回 `true`。
    pub(super) fn greek_toggle(&mut self, event: &KeyEvent) -> bool {
        if self.category() != SymbolCategory::Greek {
            return false;
        }
        match event.virtual_key {
            0xBA => {
                self.greek.accent = !self.greek.accent;
                true
            }
            0xDE => {
                self.greek.diaeresis = !self.greek.diaeresis;
                true
            }
            _ => false,
        }
    }
}

/// 虚拟键码 → 四排键盘里的键帽字符（字母用大写）。
fn key_label(virtual_key: u32) -> Option<String> {
    let key = match virtual_key {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(virtual_key)?,
        0xBD => '-',
        0xBB => '=',
        0xDB => '[',
        0xDD => ']',
        0xBA => ';',
        0xDE => '\'',
        0xBC => ',',
        0xBE => '.',
        0xBF => '/',
        _ => return None,
    };
    Some(key.to_ascii_uppercase().to_string())
}

impl Router {
    /// 全拼直接按 u / 非全拼 Ctrl+Alt+U：从空组句进 u 模式。
    pub(super) fn enter_u_mode(&mut self) {
        if self.composing() {
            return;
        }
        self.engine.push('u');
        self.u_mode = Some(UMode {
            // 支持混输时先进字母输入；不支持混输直接进符号输入。
            symbol_ui: !self.mixture_enabled(),
            ..UMode::default()
        });
        self.highlight = 0;
        self.update_u_aux();
    }

    /// 非全拼的 `Ctrl+Alt+U`；全拼走 `apply_chinese` 的裸 u。
    pub(super) fn u_mode_hotkey(&mut self, event: &KeyEvent) -> Option<Effect> {
        if self.config.full_pinyin
            || self.composing()
            || !event.modifiers.ctrl
            || !event.modifiers.alt
            || event.modifiers.win
            || event.virtual_key != u32::from(b'U')
        {
            return None;
        }
        self.enter_u_mode();
        Some(Effect::Changed(None))
    }

    /// 字母输入状态的按键：只拦 Space 切符号、Esc 退出，其余交给普通流程。
    pub(super) fn u_mode_letter_key(&mut self, event: &KeyEvent) -> Option<Effect> {
        let u = self.u_mode?;
        if event.virtual_key == 0x1B {
            self.u_mode = None;
            self.candidates.set_symbol_aux(None);
            self.engine.clear();
            return Some(Effect::Changed(None));
        }
        if event.virtual_key == 0x20 && self.mixture_enabled() {
            let mut next = u;
            next.symbol_ui = true;
            self.u_mode = Some(next);
            self.update_u_aux();
            return Some(Effect::Changed(None));
        }
        None
    }

    /// 符号输入状态的按键。返回 `Effect`；不在 u 模式时调用方不会走到这里。
    pub(super) fn u_mode_key(&mut self, event: &KeyEvent) -> Effect {
        let Some(mut u) = self.u_mode else {
            return Effect::Passthrough;
        };
        if event.virtual_key == 0x1B {
            // Esc：关符号窗口、关候选窗，整段 u 模式作废。
            self.u_mode = None;
            self.candidates.set_symbol_aux(None);
            self.engine.clear();
            return Effect::Changed(None);
        }
        if event.modifiers.has_command_key() {
            return Effect::Passthrough;
        }
        if matches!(event.virtual_key, 0x08 | 0x2E) {
            // Backspace / Delete 保持应用里的删除行为；u 模式与符号窗不退出。
            return Effect::Passthrough;
        }
        if event.virtual_key == 0x20 {
            // 支持混输时 Space 切回字母输入；没有混输时仍只吞键。
            if self.mixture_enabled() {
                u.symbol_ui = false;
                self.u_mode = Some(u);
                self.update_u_aux();
                return Effect::Changed(None);
            }
            return Effect::Navigated;
        }
        if event.virtual_key == 0x09 {
            u.tab();
            self.u_mode = Some(u);
            self.update_u_aux();
            return Effect::Navigated;
        }
        let vertical = self.config.layout == LayoutMode::Vertical;
        let direction = match event.virtual_key {
            0x26 if vertical => Some(-1),
            0x28 if vertical => Some(1),
            0x25 if !vertical => Some(-1),
            0x27 if !vertical => Some(1),
            _ => None,
        };
        if let Some(delta) = direction {
            u.move_category(delta);
            self.u_mode = Some(u);
            self.highlight = u.category_index();
            self.update_u_aux();
            return Effect::Navigated;
        }
        if u.greek_toggle(event) {
            self.u_mode = Some(u);
            self.update_u_aux();
            return Effect::Navigated;
        }
        if let Some(symbol) = u.symbol(event) {
            // 上屏一个符号，但保留 u 模式与符号窗口。
            self.u_mode = Some(u);
            return Effect::Changed(Some(symbol.to_owned()));
        }
        // 其它键（回车 / 功能键等）：u 模式里一律吞掉，别漏进应用。
        Effect::Navigated
    }

    /// 符号窗口是否应显示「可输入英文单词」提示。
    pub(super) fn mixture_enabled(&self) -> bool {
        if self.engine.shuangpin().is_some() {
            self.engine.shuangpin_mixture_input()
        } else {
            self.engine.mixture_input()
        }
    }

    pub(super) fn update_u_aux(&mut self) {
        let layout = self.u_mode.filter(|u| u.symbol_ui).map(|u| u.layout());
        self.candidates.set_symbol_aux(layout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(vk: u32, c: char, shift: bool) -> KeyEvent {
        KeyEvent::new(
            vk,
            Some(c),
            cloudime_platform::protocol::KeyModifiers {
                shift,
                ..Default::default()
            },
        )
    }

    #[test]
    fn the_symbol_window_has_four_keyboard_rows() {
        let layout = UMode::default().layout();
        assert_eq!(layout.rows.len(), 4);
        assert_eq!(layout.rows[0].len(), 12);
        assert_eq!(layout.rows[1].len(), 12);
        assert_eq!(layout.rows[2].len(), 11);
        assert_eq!(layout.rows[3].len(), 10);
        assert!(layout.zero_initials.is_empty());
        assert!(layout.symbol);
    }

    #[test]
    fn kana_tab_toggles_and_shows_the_footer_hint() {
        let mut mode = UMode::default();
        mode.set_category(4);
        assert_eq!(mode.category(), SymbolCategory::Kana);
        assert!(mode.layout().footer.is_some());
        mode.tab();
        assert_eq!(mode.kana, Kana::Katakana);
        mode.tab();
        assert_eq!(mode.kana, Kana::Hiragana);
    }

    #[test]
    fn candidates_are_the_six_fixed_categories() {
        let labels: Vec<String> = UMode::default()
            .candidates()
            .into_iter()
            .map(|candidate| candidate.text)
            .collect();
        assert_eq!(labels.len(), 6);
        assert_eq!(labels[0], "常用符号");
        assert!(labels[5].contains("制表符"));
    }

    #[test]
    fn normal_and_shift_symbols_come_from_the_tables() {
        let mode = UMode::default();
        assert_eq!(mode.symbol(&key(0x31, '1', false)), Some("〈"));
        assert_eq!(mode.symbol(&key(0x31, '!', true)), None, "常用符号没有上档");

        let mut number = UMode::default();
        number.set_category(1);
        assert_eq!(number.symbol(&key(0xBD, '-', false)), Some("₋"));
        assert_eq!(number.symbol(&key(0xBD, '_', true)), Some("⁻"));

        let mut cyrillic = UMode::default();
        cyrillic.set_category(3);
        assert_eq!(cyrillic.symbol(&key(0xBF, '/', false)), Some("ё"));
        assert_eq!(cyrillic.symbol(&key(0xBF, '?', true)), Some("Ё"));
    }

    #[test]
    fn greek_accent_and_diaeresis_states_are_toggled_by_semicolon_and_quote() {
        let mut mode = UMode::default();
        mode.set_category(2);
        assert_eq!(mode.symbol(&key(0x45, 'e', false)), Some("ε"));

        assert!(mode.greek_toggle(&key(0xBA, ';', false)));
        assert_eq!(mode.symbol(&key(0x45, 'e', false)), Some("έ"));
        assert_eq!(
            mode.symbol(&key(0x51, 'q', false)),
            None,
            "重音态只允许指定键"
        );

        assert!(mode.greek_toggle(&key(0xDE, '\'', false)));
        assert_eq!(mode.symbol(&key(0x49, 'i', false)), Some("ΐ"));
        assert_eq!(mode.symbol(&key(0x59, 'y', false)), Some("ΰ"));
    }

    #[test]
    fn cyrillic_has_the_slash_footer_hint() {
        let mut mode = UMode::default();
        mode.set_category(3);
        assert_eq!(mode.layout().footer.as_deref(), Some("Ё/ё 在前斜杠键"));
    }
}
