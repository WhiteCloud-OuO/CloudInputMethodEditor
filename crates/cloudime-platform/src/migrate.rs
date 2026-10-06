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
//! - `[fuzzy]` 各开关 → `[input] mo_hu_yin_list`（位图，一位一条规则）
//! - `[model] enabled` → `[candidate] use_local_sentence_organization_model`
//! - `[status_bar] enabled` → 删（状态条常开）
//! - `[general] page_keys`、`[shortcut]`（`expression` / `question` / `question_mark` / `delete_candidate`）→ 删
//!   （翻页键与表达式键已固定，问字与删候选整体下线）
//! - `[[custom_phrases]]` → 短语库的 `user` 表（安装目录 `Phrases\Phrase.db`），`position`（1–9）直接夹到 1–9，停用的丢掉
//! - 老数据目录（`%APPDATA%\CloudIME`）的 `Phrase.db`（单表 `phrases`）→ 新短语库的 `user` 表（位置换算成 1 基），
//!   搬完把老文件改名成 `Phrase.db.migrated`；只在新的 `user` 表为空时才搬
//! - `[dictionaries] domains / disabled` → 删（词库目录里有的都加载，不再要启用清单）

use std::path::Path;

use cloudime_core::CustomPhrase;
use cloudime_core::FuzzyRules;
use cloudime_core::custom_phrase;
use toml_edit::{DocumentMut, Item};

use crate::config::write_with_template;
use crate::phrase::{self, PhraseStore};
use crate::{Config, LayoutMode, MAX_CANDIDATE_COUNT, MIN_CANDIDATE_COUNT, PreeditMode, SimpTrad};

/// 旧版数据迁移的结果。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// 配置里的旧键搬进了新分节。
    pub config: bool,

    /// 从 `[[custom_phrases]]` 搬进短语库的条数。
    pub phrases: usize,

    /// 从老数据目录的 `Phrase.db` 搬进新短语库的条数。
    pub legacy_phrases: usize,
}

impl Migration {
    /// 什么都没做。
    pub fn is_empty(&self) -> bool {
        !self.config && self.phrases == 0 && self.legacy_phrases == 0
    }
}

/// 把旧版配置与数据迁到当前格式；幂等，没有旧内容时什么都不做。
///
/// `config_path` 是 `config.toml`（数据目录由它的父目录推出来）。安装根用 [`crate::resources::bundled_root`]
/// 定位短语库；拿不到就跳过新位置的短语迁移。
pub fn migrate(config_path: &Path) -> Migration {
    migrate_inner(config_path, crate::resources::bundled_root().as_deref())
}

/// [`migrate`] 的可测形式：安装根由调用方给，测试不会写到真实安装目录。
pub(crate) fn migrate_inner(config_path: &Path, bundled_root: Option<&Path>) -> Migration {
    let data_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut migration = Migration::default();
    let mut config_phrases = Vec::new();

    match std::fs::read_to_string(config_path) {
        Ok(source) => match source.parse::<DocumentMut>() {
            Ok(document) if has_legacy_keys(&document) => {
                let backup = config_path.with_extension("toml.bak");
                if let Err(error) = std::fs::copy(config_path, &backup) {
                    tracing::warn!(%error, path = %backup.display(), "备份旧配置失败，继续迁移");
                }
                let legacy = Legacy::read(&document);
                config_phrases = legacy.phrases.clone();
                let mut config = Config::load(config_path).unwrap_or_default();
                legacy.apply(&mut config);
                match write_with_template(config_path, &config) {
                    Ok(()) => {
                        migration.config = true;
                        tracing::info!(path = %config_path.display(), "配置已按新格式重写");
                    }
                    Err(error) => tracing::error!(%error, "重写配置失败，旧配置保持原样"),
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, path = %config_path.display(), "配置语法不对，跳过配置迁移")
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(%error, path = %config_path.display(), "读配置失败，跳过配置迁移")
        }
    }

    let (phrases, legacy_phrases) = migrate_phrases(bundled_root, &data_dir, &config_phrases);
    migration.phrases = phrases;
    migration.legacy_phrases = legacy_phrases;
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

/// 旧 `[fuzzy]` 的开关合成新位图：键名与 Core 的 [`FuzzyRules`] 字段同名，交给它认，再翻成位图。
fn fuzzy_mask(document: &DocumentMut) -> u32 {
    let Some(fuzzy) = table(document, "fuzzy") else {
        return 0;
    };
    let mut rules = FuzzyRules::default();
    for (key, value) in fuzzy.iter() {
        if let Some(on) = value.as_bool() {
            rules.set(key, on);
        }
    }
    crate::fuzzy_bits(&rules)
}

/// 旧 `[[custom_phrases]]`：`position`（1–9，越靠前）直接夹到 1–9，停用的丢掉。
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
            .unwrap_or(i64::from(custom_phrase::DEFAULT_POSITION))
            .clamp(
                i64::from(custom_phrase::MIN_POSITION),
                i64::from(custom_phrase::MAX_POSITION),
            ) as u32;
        phrases.push(CustomPhrase {
            code: code.to_owned(),
            text: text.to_owned(),
            title: None,
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

/// 把配置里搬出的短语与老数据目录的 `Phrase.db` 并进短语库的 `user` 表。
/// 返回 `(配置里的条数, 老库的条数)`；老库只在新的 `user` 表为空时读，读完改名成 `Phrase.db.migrated`。
fn migrate_phrases(
    bundled_root: Option<&Path>,
    data_dir: &Path,
    from_config: &[CustomPhrase],
) -> (usize, usize) {
    let Some(root) = bundled_root else {
        if !from_config.is_empty() {
            tracing::warn!("拿不到安装目录，配置里的旧短语没搬进短语库");
        }
        return (0, 0);
    };
    let store = PhraseStore::locate(root);
    let mut phrases = store.load(false).unwrap_or_default();
    let user_was_empty = phrases.is_empty();

    let mut added_config = 0;
    for phrase in from_config {
        if phrases
            .iter()
            .any(|existing| existing.code == phrase.code && existing.text == phrase.text)
        {
            continue;
        }
        phrases.push(phrase.clone());
        added_config += 1;
    }

    // 老数据目录的 Phrase.db：新 user 表为空时才搬，避免顶掉用户已经在新位置维护的短语。
    let legacy_path = data_dir.join(phrase::PHRASE_FILE);
    let mut read_legacy = false;
    let mut added_legacy = 0;
    if user_was_empty && legacy_path.is_file() && phrase::is_database(&legacy_path) {
        match phrase::read_legacy(&legacy_path) {
            Ok(old) => {
                read_legacy = true;
                for phrase in old {
                    if phrases.iter().any(|existing| {
                        existing.code == phrase.code && existing.text == phrase.text
                    }) {
                        continue;
                    }
                    phrases.push(phrase);
                    added_legacy += 1;
                }
            }
            Err(error) => {
                tracing::warn!(%error, path = %legacy_path.display(), "旧短语库读不出来，跳过");
            }
        }
    }

    if added_config == 0 && added_legacy == 0 {
        return (0, 0);
    }
    match store.save_user(&phrases) {
        Ok(()) => {
            if read_legacy {
                let migrated = legacy_path.with_extension("db.migrated");
                if let Err(error) = std::fs::rename(&legacy_path, &migrated) {
                    tracing::warn!(%error, "旧短语库改名失败，下次启动可能重复搬");
                }
            }
            tracing::info!(
                config = added_config,
                legacy = added_legacy,
                path = %store.path.display(),
                "旧短语已搬进短语库"
            );
            (added_config, added_legacy)
        }
        Err(error) => {
            tracing::warn!(%error, "写短语库失败，旧短语没搬过去");
            (0, 0)
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

        let migration = migrate_inner(&path, Some(&dir));
        assert!(migration.config);
        assert_eq!(migration.phrases, 1);
        assert_eq!(migration.legacy_phrases, 0);

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
        // z_zh + u_v 两位置位（1 = zh/z、64 = u/ü）
        assert_eq!(config.input.mo_hu_yin_list, 1 | 64);
        // 没搬的键原样保留
        assert!(!config.general.learning);
        assert_eq!(config.status_bar.x, Some(10));
        // 下线的翻页键与 [shortcut] 写回后不再出现
        let rewritten = std::fs::read_to_string(&path).unwrap();
        assert!(!rewritten.contains("page_keys"), "{rewritten}");
        assert!(!rewritten.contains("[shortcut]"), "{rewritten}");

        let store = PhraseStore::locate(&dir);
        let phrases = store.load(false).unwrap();
        assert_eq!(phrases.len(), 1);
        assert_eq!(phrases[0].code, "ww");
        assert_eq!(phrases[0].text, "；");
        assert_eq!(phrases[0].position, 1);

        // 旧配置留了备份，再跑一次没有旧键了
        assert!(dir.join("config.toml.bak").is_file());
        assert!(migrate_inner(&path, Some(&dir)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moves_the_legacy_phrase_database_once() {
        let dir = scratch("legacy-db");
        let path = dir.join("config.toml");
        // 没有旧键的配置：只做短语库迁移
        std::fs::write(&path, "[general]\nlearning = true\n").unwrap();

        let legacy = dir.join("Phrase.db");
        let connection = rusqlite::Connection::open(&legacy).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE phrases (id INTEGER PRIMARY KEY, code TEXT NOT NULL, text TEXT NOT NULL, position INTEGER NOT NULL);
                 INSERT INTO phrases (code, text, position) VALUES ('aa', '甲', 0), ('bb', '乙', 3);",
            )
            .unwrap();
        drop(connection);

        let migration = migrate_inner(&path, Some(&dir));
        assert!(!migration.config);
        assert_eq!(migration.phrases, 0);
        assert_eq!(migration.legacy_phrases, 2);

        let store = PhraseStore::locate(&dir);
        let phrases = store.load(false).unwrap();
        assert_eq!(phrases.len(), 2);
        assert_eq!(
            phrases[0],
            CustomPhrase {
                code: "aa".into(),
                text: "甲".into(),
                title: None,
                position: 1,
            }
        );
        assert_eq!(phrases[1].position, 4);
        // 老库改名，第二次不会再搬
        assert!(!legacy.exists());
        assert!(dir.join("Phrase.db.migrated").is_file());
        assert!(migrate_inner(&path, Some(&dir)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
