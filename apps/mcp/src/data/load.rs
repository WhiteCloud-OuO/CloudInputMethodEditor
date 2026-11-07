//! 主词库、第三方词库装载与只读 Engine 构造。

use std::path::PathBuf;

use cloudime_core::Engine;
use cloudime_dictionary::{DictDb, Dictionary};
use cloudime_platform::{WordBank, resources};

use super::{DictionaryInfo, DictionaryInfoResponse, LoadError, LoadOptions, Loaded};

/// 装载主词库、可选的第三方词库，并构造只读 Engine。
pub fn load(options: &LoadOptions) -> Result<Loaded, LoadError> {
    let bank = resolve_bank(options);
    let main = options
        .dict
        .clone()
        .or_else(|| bank.as_ref().and_then(main_path))
        .ok_or(LoadError::MissingRoot)?;
    if !main.is_file() {
        return Err(LoadError::MissingDictionary(main));
    }

    let data = if options.include_rare {
        DictDb::from_path(&main)
    } else {
        DictDb::from_path_without_rare(&main)
    }
    .map_err(|source| LoadError::Dictionary {
        path: main.clone(),
        source,
    })?;

    let extras = if options.no_extras {
        Vec::new()
    } else {
        bank.as_ref()
            .map(|bank| bank.load_except(Some(main.as_path())))
            .unwrap_or_default()
    };

    let mut dictionaries = Vec::with_capacity(extras.len() + 1);
    let metadata = data.metadata.as_ref();
    dictionaries.push(DictionaryInfo {
        name: metadata
            .map(|metadata| metadata.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                main.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Dict.db")
                    .to_owned()
            }),
        entries: data.len(),
        license: metadata
            .map(|metadata| metadata.license.clone())
            .unwrap_or_default(),
        kind: "main".to_owned(),
    });
    for dictionary in &extras {
        dictionaries.push(info_of(dictionary, "imported"));
    }

    let rare_enabled = options.include_rare && !data.rare.is_empty();
    let DictDb {
        chinese,
        rare,
        english,
        metadata: _,
    } = data;
    let mut engine = Engine::new(chinese);
    if !english.is_empty() {
        engine = engine.with_english(english);
    }
    if rare_enabled {
        engine = engine.with_rare(rare);
        engine.set_rare_enabled(true);
    }
    engine.set_extra_dictionaries(extras);
    engine.set_use_jian_pin(true);

    Ok(Loaded {
        engine,
        info: DictionaryInfoResponse {
            dictionaries,
            rare_enabled,
        },
    })
}

/// 主词库优先 `Dict.*`；绝不能把用户自造词库当成主词库。
fn main_path(bank: &WordBank) -> Option<PathBuf> {
    let (_, path) = bank.main()?;
    let name = path.file_name()?.to_str()?;
    (!name.eq_ignore_ascii_case("UserWordBank.db")).then_some(path)
}

fn resolve_bank(options: &LoadOptions) -> Option<WordBank> {
    if let Some(dir) = &options.word_bank {
        return Some(WordBank { dir: dir.clone() });
    }
    if options.dict.is_some() {
        return None;
    }
    resources::bundled_root().map(|root| WordBank::locate(&root))
}

fn info_of(dictionary: &Dictionary, kind: &str) -> DictionaryInfo {
    let metadata = dictionary.metadata();
    DictionaryInfo {
        name: metadata
            .map(|metadata| metadata.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "未命名词库".to_owned()),
        entries: dictionary.len(),
        license: metadata
            .map(|metadata| metadata.license.clone())
            .unwrap_or_default(),
        kind: kind.to_owned(),
    }
}
