//! 旧版配置的迁移：四节重排之前写的键搬进新分节，用户短语挪进短语库。
//!
//! 只在升级后第一次运行时做一遍（旧键都不在了就什么都不做），写回配置前先留一份 `config.toml.bak`。
//! 认不出的旧键（`[apps]`、`[general] english_mode` 这类已经下线的功能）直接丢掉。
//!
//! 映射表（旧 → 新）：
//! - `[general] page_size` → `[candidate] candidate_count`（夹到 5–9）
//! - `[general] layout` → `[candidate] candidate_arrangement_direction`
//! - `[general] font` → 三个字体的 `family`
//! - `[general] preedit` → `[candidate] preedit`
//! - `[general] traditional` → `[input] simp_trad_chinese_chars_toggle`
//! - `[fuzzy]` 各开关 → `[input] mo_hu_yin_list`（位图，规则被合并的组只要有一项开着就置位）
//! - `[model] enabled` → `[candidate] use_local_sentence_organization_model`
//! - `[status_bar] enabled` → 删（状态条常开）
//! - `[general] page_keys`、`[shortcut]`（`expression` / `question` / `question_mark` / `delete_candidate`）→ 删
//!   （翻页键与表达式键已固定，问字与删候选整体下线）
//! - `[[custom_phrases]]` → 短语库（`Phrase.db`），`position`（1–9）换成 0–8 的候选位置，停用的丢掉
//! - `[dictionaries] domains / disabled` → 删（词库目录里有的都加载，不再要启用清单）

use std::path::Path;

use cloudime_core::CustomPhrase;
use cloudime_core::custom_phrase;
use toml_edit::{DocumentMut, Item};

use crate::config::write_with_template;
use crate::{
    Config, LayoutMode, MAX_CANDIDATE_COUNT, MIN_CANDIDATE_COUNT, PhraseStore, PreeditMode,
    SimpTrad,
};

/// 旧版数据迁移的结果。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// 配置里的旧键搬进了新分节。
    pub config: bool,

    /// 从 `[[custom_phrases]]` 搬进短语库的条数。
    pub phrases: usize,
}

impl Migration {
    /// 什么都没做。
    pub fn is_empty(&self) -> bool {
        !self.config && self.phrases == 0
    }
}

/// 把旧版配置与数据迁到当前格式；幂等，没有旧内容时什么都不做。
///
/// `config_path` 是 `config.toml`（数据目录由它的父目录推出来）。
pub fn migrate(config_path: &Path) -> Migration {
    let Ok(source) = std::fs::read_to_string(config_path) else {
        return Migration::default();
    };
    let Ok(document) = source.parse::<DocumentMut>() else {
        tracing::warn!(path = %config_path.display(), "配置语法不对，跳过迁移");
        return Migration::default();
    };
    if !has_legacy_keys(&document) {
        return Migration::default();
    }
    let data_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let legacy = Legacy::read(&document);

    let backup = config_path.with_extension("toml.bak");
    if let Err(error) = std::fs::copy(config_path, &backup) {
        tracing::warn!(%error, path = %backup.display(), "备份旧配置失败，继续迁移");
    }

    let mut config = Config::load(config_path).unwrap_or_default();
    legacy.apply(&mut config);
    let mut migration = Migration::default();
    match write_with_template(config_path, &config) {
        Ok(()) => {
            migration.config = true;
            tracing::info!(path = %config_path.display(), "配置已按新格式重写");
        }
        Err(error) => {
            tracing::error!(%error, "重写配置失败，旧配置保持原样");
            return migration;
        }
    }

    migration.phrases = migrate_phrases(&data_dir, &config, &legacy.phrases);
    migration
}

/// 文档里还有没有旧键。
fn has_legacy_keys(document: &DocumentMut) -> bool {
    ["fuzzy", "model", "dictionaries", "apps", "shortcut"]
        .iter()
        .any(|name| document.get(name).is_some())
        || document.get("custom_phrases").is_some()
        || [
            "page_size",
            "layout",
            "font",
            "preedit",
            "traditional",
            "page_keys",
        ]
        .iter()
        .any(|key| {
            table(document, "general")
                .and_then(|table| table.get(key))
                .is_some()
        })
}

fn table<'a>(document: &'a DocumentMut, name: &str) -> Option<&'a toml_edit::Table> {
    document.get(name).and_then(Item::as_table)
}

fn boolean(document: &DocumentMut, section: &str, key: &str) -> Option<bool> {
    table(document, section)?.get(key).and_then(Item::as_bool)
}

fn integer(document: &DocumentMut, section: &str, key: &str) -> Option<i64> {
    table(document, section)?
        .get(key)
        .and_then(Item::as_integer)
}

fn text(document: &DocumentMut, section: &str, key: &str) -> Option<String> {
    table(document, section)?
        .get(key)
        .and_then(Item::as_str)
        .map(str::to_owned)
}

/// 旧 `[fuzzy]` 的开关合成新位图：组里只要有一项开着就置位。
fn fuzzy_mask(document: &DocumentMut) -> u32 {
    let Some(fuzzy) = table(document, "fuzzy") else {
        return 0;
    };
    let on = |key: &str| fuzzy.get(key).and_then(Item::as_bool).unwrap_or(false);
    let mut mask = 0;
    if on("z_zh") || on("c_ch") || on("s_sh") {
        mask |= 1;
    }
    if on("n_l") || on("l_r") {
        mask |= 2;
    }
    if on("an_ang") || on("en_eng") || on("in_ing") {
        mask |= 4;
    }
    if on("u_v") {
        mask |= 8;
    }
    if on("f_h") {
        mask |= 16;
    }
    mask
}

/// 旧 `[[custom_phrases]]`：`position`（1–9，越靠前）换成 0–8 的候选位置（0 = 第一位），停用的丢掉。
fn old_phrases(document: &DocumentMut) -> Vec<CustomPhrase> {
    let mut phrases = Vec::new();
    let Some(tables) = document
        .get("custom_phrases")
        .and_then(Item::as_array_of_tables)
    else {
        return phrases;
    };
    for table in tables.iter() {
        if !table.get("enabled").and_then(Item::as_bool).unwrap_or(true) {
            continue;
        }
        let (Some(code), Some(text)) = (
            table.get("code").and_then(Item::as_str),
            table.get("text").and_then(Item::as_str),
        ) else {
            continue;
        };
        let position = table
            .get("position")
            .and_then(Item::as_integer)
            .unwrap_or(1);
        let position = (position - 1).clamp(0, i64::from(custom_phrase::MAX_POSITION)) as u32;
        phrases.push(CustomPhrase {
            code: code.to_owned(),
            text: text.to_owned(),
            position,
        });
    }
    phrases
}

/// 旧配置里认得出的值。
struct Legacy {
    page_size: Option<usize>,
    layout: Option<String>,
    font: Option<String>,
    preedit: Option<String>,
    traditional: Option<bool>,
    fuzzy: u32,
    model_enabled: Option<bool>,
    phrases: Vec<CustomPhrase>,
}

impl Legacy {
    fn read(document: &DocumentMut) -> Self {
        Self {
            page_size: integer(document, "general", "page_size")
                .and_then(|value| usize::try_from(value).ok()),
            layout: text(document, "general", "layout"),
            font: text(document, "general", "font"),
            preedit: text(document, "general", "preedit"),
            traditional: boolean(document, "general", "traditional"),
            fuzzy: fuzzy_mask(document),
            model_enabled: boolean(document, "model", "enabled"),
            phrases: old_phrases(document),
        }
    }

    /// 把旧值覆盖到新配置上。
    fn apply(&self, config: &mut Config) {
        if let Some(page_size) = self.page_size {
            config.candidate.candidate_count =
                page_size.clamp(MIN_CANDIDATE_COUNT, MAX_CANDIDATE_COUNT);
        }
        if let Some(layout) = self.layout.as_deref().and_then(parse_layout) {
            config.candidate.candidate_arrangement_direction = layout;
        }
        if let Some(family) = self
            .font
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            for font in [
                &mut config.candidate.pinyin_font,
                &mut config.candidate.candidate_font,
                &mut config.candidate.item_number_font,
            ] {
                font.family = family.to_owned();
            }
        }
        if let Some(preedit) = self.preedit.as_deref().and_then(parse_preedit) {
            config.candidate.preedit = preedit;
        }
        if let Some(traditional) = self.traditional {
            config.input.simp_trad_chinese_chars_toggle = if traditional {
                SimpTrad::Traditional
            } else {
                SimpTrad::Simplified
            };
        }
        config.input.mo_hu_yin_list = self.fuzzy;
        if let Some(enabled) = self.model_enabled {
            config.candidate.use_local_sentence_organization_model = enabled;
        }
    }
}

fn parse_layout(value: &str) -> Option<LayoutMode> {
    LayoutMode::ALL
        .into_iter()
        .find(|mode| mode.key() == value.trim())
}

fn parse_preedit(value: &str) -> Option<PreeditMode> {
    PreeditMode::ALL
        .into_iter()
        .find(|mode| mode.key() == value.trim())
}

/// 把旧短语并进短语库（已有的同码同文本不重复加）。
fn migrate_phrases(data_dir: &Path, config: &Config, migrated: &[CustomPhrase]) -> usize {
    if migrated.is_empty() {
        return 0;
    }
    let store = PhraseStore::locate(data_dir, &config.phrase);
    let mut phrases = store.load().unwrap_or_default();
    let mut added = 0;
    for phrase in migrated {
        if phrases
            .iter()
            .any(|existing| existing.code == phrase.code && existing.text == phrase.text)
        {
            continue;
        }
        phrases.push(phrase.clone());
        added += 1;
    }
    if added == 0 {
        return 0;
    }
    match store.save(&phrases) {
        Ok(()) => {
            tracing::info!(count = added, path = %store.path.display(), "旧短语已搬进短语库");
            added
        }
        Err(error) => {
            tracing::warn!(%error, "写短语库失败，旧短语没搬过去");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cloudime-migrate-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const OLD_CONFIG: &str = r#"[general]
page_size = 3
layout = "horizontal"
font = "LXGW WenKai"
preedit = "inline"
traditional = true
page_keys = ",."
learning = false

[fuzzy]
z_zh = true
u_v = true

[model]
enabled = false

[status_bar]
enabled = true
x = 10

[shortcut]
expression = "i"
question = "u"
question_mark = true
delete_candidate = "shift"

[[custom_phrases]]
code = "ww"
text = "；"
position = 1
enabled = true

[[custom_phrases]]
code = "xx"
text = "停用"
position = 2
enabled = false
"#;

    #[test]
    fn migrates_old_config_keys_and_phrases() {
        let dir = scratch("config");
        let path = dir.join("config.toml");
        std::fs::write(&path, OLD_CONFIG).unwrap();

        let migration = migrate(&path);
        assert!(migration.config);
        assert_eq!(migration.phrases, 1);

        let config = Config::load(&path).unwrap();
        // page_size = 3 夹到下限 5
        assert_eq!(config.candidate.candidate_count, 5);
        assert_eq!(
            config.candidate.candidate_arrangement_direction,
            LayoutMode::Horizontal
        );
        assert_eq!(config.candidate.preedit, PreeditMode::Inline);
        assert!(!config.candidate.use_local_sentence_organization_model);
        assert_eq!(config.candidate.pinyin_font.family, "LXGW WenKai");
        assert_eq!(config.candidate.candidate_font.family, "LXGW WenKai");
        assert_eq!(
            config.input.simp_trad_chinese_chars_toggle,
            SimpTrad::Traditional
        );
        // z_zh + u_v 两组置位
        assert_eq!(config.input.mo_hu_yin_list, 1 | 8);
        // 没搬的键原样保留
        assert!(!config.general.learning);
        assert_eq!(config.status_bar.x, Some(10));
        // 下线的翻页键与 [shortcut] 写回后不再出现
        let rewritten = std::fs::read_to_string(&path).unwrap();
        assert!(!rewritten.contains("page_keys"), "{rewritten}");
        assert!(!rewritten.contains("[shortcut]"), "{rewritten}");

        let store = PhraseStore::locate(&dir, &config.phrase);
        let phrases = store.load().unwrap();
        assert_eq!(phrases.len(), 1);
        assert_eq!(phrases[0].code, "ww");
        assert_eq!(phrases[0].text, "；");
        assert_eq!(phrases[0].position, 0);

        // 旧配置留了备份，再跑一次没有旧键了
        assert!(dir.join("config.toml.bak").is_file());
        assert!(migrate(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
