//! i 模式：用候选窗口当脚本启动器。
//!
//! 进入方式：全拼在空组句时按 `i`，或 `Ctrl+Alt+I`。缓冲区只有 `i` 时列脚本；开了中英混输后
//! 继续打字母则只匹配英文单词。具体按键约定见 `docs/design/input-modes.md`。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cloudime_platform::LayoutMode;
use cloudime_platform::protocol::{KeyEvent, OnlineLine, OnlineState};

use super::Router;
use super::key::Effect;

const BACK: u32 = 0x08;
const RETURN: u32 = 0x0D;
const ESCAPE: u32 = 0x1B;
const DELETE: u32 = 0x2E;
const SPACE: u32 = 0x20;
const PRIOR: u32 = 0x21;
const NEXT: u32 = 0x22;
const LEFT: u32 = 0x25;
const UP: u32 = 0x26;
const RIGHT: u32 = 0x27;
const DOWN: u32 = 0x28;
const OEM_4: u32 = 0xDB;
const OEM_6: u32 = 0xDD;

impl Router {
    /// 从空组句进入 i 模式；已经在组句里就不动。
    pub(super) fn enter_i_mode(&mut self) {
        if self.composing() {
            return;
        }
        if self.u_mode.take().is_some() {
            self.candidates.set_symbol_aux(None);
        }
        self.engine.push('i');
        self.i_mode = true;
        self.highlight = 0;
    }

    /// `Ctrl+Alt+I`：全拼 / 双拼 / 形码都能进 i 模式。
    pub(super) fn i_mode_hotkey(&mut self, event: &KeyEvent) -> Option<Effect> {
        if !event.modifiers.ctrl
            || !event.modifiers.alt
            || event.modifiers.win
            || event.virtual_key != u32::from(b'I')
        {
            return None;
        }
        self.enter_i_mode();
        Some(Effect::Changed(None))
    }

    /// i 模式按键。返回 `None` 表示这一键交回普通处理（只有英文候选那一档需要）。
    pub(super) fn i_mode_key(&mut self, event: &KeyEvent) -> Option<Effect> {
        if !self.i_mode {
            return None;
        }
        let text = self.engine.composition().text().to_string();
        if text.chars().count() > 1 {
            return self.i_mode_english_key(event);
        }
        // 缓冲区只剩 `i`：候选是脚本名称。
        let mixture = self.mixture_enabled();
        match event.virtual_key {
            ESCAPE => {
                self.engine.clear();
                self.i_mode = false;
                self.online.clear();
                return Some(Effect::Changed(None));
            }
            RETURN => {
                let raw = self.engine.take_raw();
                self.i_mode = false;
                self.online.clear();
                return Some(Effect::Changed((!raw.is_empty()).then_some(raw)));
            }
            BACK | DELETE if mixture => {
                if event.virtual_key == BACK {
                    self.engine.backspace();
                } else {
                    self.engine.delete_forward();
                }
                if self.engine.composition().is_empty() {
                    self.i_mode = false;
                }
                return Some(Effect::Changed(None));
            }
            // 没开中英混输：字母与退格 / 删除都吞掉，脚本窗口留在原地。
            BACK | DELETE => return Some(Effect::Navigated),
            SPACE => return Some(self.run_highlighted_script()),
            PRIOR => {
                self.page(-1);
                return Some(Effect::Navigated);
            }
            NEXT => {
                self.page(1);
                return Some(Effect::Navigated);
            }
            OEM_4 => {
                self.page(-1);
                return Some(Effect::Navigated);
            }
            OEM_6 => {
                self.page(1);
                return Some(Effect::Navigated);
            }
            LEFT | RIGHT | UP | DOWN => {
                self.i_mode_arrow(event.virtual_key);
                return Some(Effect::Navigated);
            }
            _ => {}
        }
        if let Some(digit) = super::key::codes::digit_virtual_key(event.virtual_key)
            && let Some(effect) = self.i_mode_digit(event, digit)
        {
            return Some(effect);
        }
        // Ctrl / Alt 组合键（数字已单独处理）继续交给应用；单纯字母 / 标点吞掉。
        if event.modifiers.has_command_key() {
            return Some(Effect::Passthrough);
        }
        if let Some(c) = event.character.filter(|c| c.is_ascii_alphabetic()) {
            if mixture {
                self.engine.push(c);
                return Some(Effect::Changed(None));
            }
            return Some(Effect::Navigated);
        }
        Some(Effect::Navigated)
    }

    /// 缓冲区已经不止 `i`：只保留英文单词候选，编辑 / 选择仍走普通按键路径。
    fn i_mode_english_key(&mut self, event: &KeyEvent) -> Option<Effect> {
        match event.virtual_key {
            ESCAPE => {
                self.engine.clear();
                self.i_mode = false;
                self.online.clear();
                return Some(Effect::Changed(None));
            }
            OEM_4 => {
                self.page(-1);
                return Some(Effect::Navigated);
            }
            OEM_6 => {
                self.page(1);
                return Some(Effect::Navigated);
            }
            _ => {}
        }
        // 字母、退格、删除、空格、数字、方向键、PageUp / PageDown 都交给普通候选逻辑。
        // 普通逻辑把组句清空后，`recompose` 会把 i_mode 一起关掉。
        let _ = event;
        None
    }

    fn i_mode_arrow(&mut self, key: u32) {
        let vertical = self.config.layout == LayoutMode::Vertical;
        match key {
            UP if vertical => self.move_highlight(-1),
            DOWN if vertical => self.move_highlight(1),
            LEFT if vertical => self.page(-1),
            RIGHT if vertical => self.page(1),
            LEFT => self.move_highlight(-1),
            RIGHT => self.move_highlight(1),
            UP => self.page(-1),
            DOWN => self.page(1),
            _ => {}
        }
    }

    fn i_mode_digit(&mut self, event: &KeyEvent, digit: usize) -> Option<Effect> {
        let page_size = self.page_size();
        if page_size == 0 {
            return None;
        }
        let index = self.highlight / page_size * page_size + digit - 1;
        let file = self.layout_candidate(index)?.text;
        let info = self
            .scripts
            .script_infos()
            .iter()
            .find(|info| info.file.eq_ignore_ascii_case(&file))
            .cloned();
        if event.modifiers.ctrl {
            let text = info
                .as_ref()
                .and_then(|info| info.description.clone())
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "（这个脚本没有写介绍）".to_owned());
            self.online.set(
                OnlineLine {
                    text,
                    state: OnlineState::Done,
                },
                file,
            );
            return Some(Effect::Navigated);
        }
        if event.modifiers.alt {
            self.open_script_files(&file);
            return Some(Effect::Navigated);
        }
        if event.modifiers.shift {
            self.spawn_manual_script(&file);
            self.notice = Some(format!(
                "已在后台运行脚本：{}",
                info.map_or(file, |info| info.name)
            ));
            return Some(Effect::Navigated);
        }
        Some(self.run_script_file(&file))
    }

    fn run_highlighted_script(&mut self) -> Effect {
        let Some(file) = self.layout_candidate(self.highlight).map(|c| c.text) else {
            return Effect::Navigated;
        };
        self.run_script_file(&file)
    }

    fn run_script_file(&mut self, file: &str) -> Effect {
        match self.run_manual_script(file) {
            Some(text) if !text.is_empty() => {
                self.engine.clear();
                self.i_mode = false;
                self.online.clear();
                Effect::Changed(Some(text))
            }
            _ => Effect::Navigated,
        }
    }

    /// Alt+数字：用记事本打开脚本文件，以及脚本源码里引用到的 JSON 布局。
    fn open_script_files(&self, file: &str) {
        let Some(dir) = &self.config.scripts_dir else {
            tracing::warn!(file, "没有脚本目录，无法编辑脚本");
            return;
        };
        let lua = dir.join(file);
        let mut paths = vec![lua.clone()];
        if let Ok(source) = std::fs::read_to_string(&lua) {
            for reference in json_references(&source) {
                if let Some(path) = resolve_json(dir, &reference) {
                    paths.push(path);
                }
            }
        }
        // 同名的 JSON 也顺手打开：有的脚本把路径拼出来，源码里不一定有完整字符串。
        let stem = Path::new(file).file_stem().unwrap_or_default();
        for candidate in [
            dir.join(stem).with_extension("json"),
            dir.join("lib").join(stem).with_extension("json"),
        ] {
            if candidate.is_file() {
                paths.push(candidate);
            }
        }
        let mut seen = HashSet::new();
        for path in paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            if let Err(error) = std::process::Command::new("notepad.exe").arg(&path).spawn() {
                tracing::warn!(%error, path = %path.display(), "打开脚本文件失败");
            }
        }
    }
}

/// 从 Lua 源码里粗略找出 `.json` 字符串（只看引号包起来的片段，够找布局路径了）。
fn json_references(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for quote in ['"', '\''] {
        let mut rest = source;
        while let Some(start) = rest.find(quote) {
            rest = &rest[start + 1..];
            let Some(end) = rest.find(quote) else {
                break;
            };
            let value = &rest[..end];
            if value.to_ascii_lowercase().ends_with(".json") {
                found.push(value.to_owned());
            }
            rest = &rest[end + 1..];
        }
    }
    found
}

/// 把脚本里写的路径按「安装根目录 → 脚本目录 → 脚本目录/lib」找一遍。
fn resolve_json(scripts_dir: &Path, reference: &str) -> Option<PathBuf> {
    let raw = Path::new(reference);
    if raw.is_absolute() && raw.is_file() {
        return Some(raw.to_path_buf());
    }
    let mut candidates = Vec::new();
    if let Some(root) = scripts_dir.parent() {
        candidates.push(root.join(raw));
    }
    candidates.push(scripts_dir.join(raw));
    if let Some(name) = raw.file_name() {
        candidates.push(scripts_dir.join("lib").join(name));
    }
    candidates.into_iter().find(|path| path.is_file())
}
