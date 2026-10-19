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

use self::mapping::{KEY_ROWS, Kana, SymbolCategory};
use super::Router;
use super::key::Effect;

/// u 模式的状态：当前高亮分类、假名种类、制表符/箭头映射套数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct UMode {
    category: usize,
    kana: Kana,
    variant: usize,
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
        KeyMapLayout {
            rows,
            zero_initials: Vec::new(),
            footer: (self.category() == SymbolCategory::Kana)
                .then(|| "按下Tab切换平假名/片假名".to_owned()),
        }
    }

    fn key_cap(&self, key: char) -> KeyCap {
        let label = key.to_string();
        let normal = mapping::symbol(self.category(), self.kana, self.variant, &label, false)
            .unwrap_or_default();
        let shifted = mapping::symbol(self.category(), self.kana, self.variant, &label, true)
            .unwrap_or_default();
        let mapping = if !shifted.is_empty() && shifted != normal {
            format!("{normal}\n{shifted}")
        } else {
            normal.to_owned()
        };
        KeyCap {
            key: label,
            initial: String::new(),
            mapping,
        }
    }

    /// 当前分类里这个键对应的上屏符号（Shift 优先取上档）。
    pub(super) fn symbol(&self, event: &KeyEvent) -> Option<&'static str> {
        let key = key_label(event.virtual_key)?;
        mapping::symbol(
            self.category(),
            self.kana,
            self.variant,
            &key,
            event.modifiers.shift,
        )
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
    /// ȫƴֱ�Ӱ� u / ��ȫƴ Ctrl+Alt+U���ӿ����� u ģʽ��
    pub(super) fn enter_u_mode(&mut self) {
        if self.composing() {
            return;
        }
        self.engine.push('u');
        self.u_mode = Some(UMode::default());
        self.highlight = 0;
        self.update_u_aux();
    }

    /// ��ȫƴ�� `Ctrl+Alt+U`��ȫƴ�� `apply_chinese` ���� u��
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

    /// u ģʽ��İ��������� `None` ��ʾ���� u ģʽ��������ͨ���̡�
    pub(super) fn u_mode_key(&mut self, event: &KeyEvent) -> Effect {
        let Some(mut u) = self.u_mode else {
            return Effect::Passthrough;
        };
        if event.virtual_key == 0x1B {
            // Esc���ط��Ŵ��ڡ��غ�ѡ�������� u ģʽ���ϡ�
            self.u_mode = None;
            self.candidates.set_symbol_aux(None);
            self.engine.clear();
            return Effect::Changed(None);
        }
        if event.modifiers.has_command_key() {
            return Effect::Passthrough;
        }
        if matches!(event.virtual_key, 0x08 | 0x2E) {
            // Backspace / Delete ����Ӧ�����ɾ����Ϊ��u ģʽ����Ŵ����˳���
            return Effect::Passthrough;
        }
        if event.virtual_key == 0x20 {
            // Space �̼�����ѡ���ࡢҲ��������
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
        if let Some(c) = event.character.filter(char::is_ascii_alphabetic) {
            if self.mixture_enabled() {
                // �ڶ���������ĸ���뿪���Ŵ��ڣ��ӻ���ͨƴ�� / Ӣ�Ĵ�ƥ�䡣
                self.u_mode = None;
                self.candidates.set_symbol_aux(None);
                self.engine.push(c);
                return Effect::Changed(None);
            }
            // ��֧����Ӣ���䣺�̵������ĸ�����Ŵ��ڲ�����
            return Effect::Navigated;
        }
        if let Some(symbol) = u.symbol(event) {
            // ����һ�����ţ������� u ģʽ����Ŵ��ڡ�
            self.u_mode = Some(u);
            return Effect::Changed(Some(symbol.to_owned()));
        }
        // ���������س� / ���ܼ��ȣ���u ģʽ��һ���̵�����©��Ӧ�á�
        Effect::Navigated
    }

    /// ���Ŵ����Ƿ�Ӧ��ʾ��������Ӣ�ĵ��ʡ���ʾ��
    pub(super) fn mixture_enabled(&self) -> bool {
        if self.engine.shuangpin().is_some() {
            self.engine.shuangpin_mixture_input()
        } else {
            self.engine.mixture_input()
        }
    }

    pub(super) fn update_u_aux(&mut self) {
        let layout = self.u_mode.map(|u| u.layout());
        self.candidates.set_symbol_aux(layout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_symbol_window_has_four_keyboard_rows() {
        let layout = UMode::default().layout();
        assert_eq!(layout.rows.len(), 4);
        assert_eq!(layout.rows[0].len(), 12);
        assert_eq!(layout.rows[1].len(), 12);
        assert_eq!(layout.rows[2].len(), 11);
        assert_eq!(layout.rows[3].len(), 10);
        assert!(layout.zero_initials.is_empty());
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
        assert!(labels[0].starts_with('①'));
        assert!(labels[5].contains("制表符"));
    }
}
