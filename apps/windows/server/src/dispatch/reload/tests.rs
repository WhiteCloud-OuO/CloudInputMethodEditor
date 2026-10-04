//! 词库文件变化在配置不变时仍应进入正在运行的引擎。

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use cloudime_core::Engine;
use cloudime_dictionary::Dictionary;
use cloudime_platform::{Config, WordBank};

use super::CONFIG_POLL_INTERVAL;
use crate::dispatch::{DataDirs, Router, RouterConfig};

fn poll(router: &mut Router) {
    router.reload.as_mut().unwrap().last_check = Instant::now() - CONFIG_POLL_INTERVAL;
    router.poll_config_reload();
}

fn modified_at(path: &Path, seconds: u64) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
        .unwrap();
}

#[test]
fn import_replace_and_remove_without_config_changes() {
    let dir = std::env::temp_dir().join(format!("cloudime-reload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("config.toml");
    std::fs::write(&config_path, "").unwrap();
    let config = Config::load(&config_path).unwrap();
    let original_config = std::fs::read(&config_path).unwrap();
    let mut router = Router::new(Engine::new(Dictionary::default()), RouterConfig::default());
    router.watch_config(
        &config,
        config_path.clone(),
        DataDirs {
            user_root: Some(dir.clone()),
            word_bank: Some(WordBank {
                dir: dir.join("WordBank"),
            }),
            ..DataDirs::default()
        },
    );

    let bank = dir.join("WordBank");
    std::fs::create_dir_all(&bank).unwrap();
    let source = bank.join("law.tsv");
    std::fs::write(&source, "合同法\the tong fa\t120\n").unwrap();
    modified_at(&source, 100);
    poll(&mut router);
    let dictionaries = router.engine.extra_dictionaries();
    assert_eq!(dictionaries.len(), 1);
    assert_eq!(
        dictionaries[0].lookup(&["he", "tong", "fa"], false)[0].text,
        "合同法"
    );

    // 只改词频：文件内容变了，靠 mtime 发现并重装。
    std::fs::write(&source, "合同法\the tong fa\t121\n").unwrap();
    modified_at(&source, 200);
    poll(&mut router);
    assert_eq!(
        router.engine.extra_dictionaries()[0].lookup(&["he", "tong", "fa"], false)[0].frequency,
        121
    );

    // 保持旧词库映射打开，模拟运行中的 Server 收到同名替换。
    std::fs::write(&source, "民法典\tmin fa dian\t100\n法律\tfa lv\t30\n").unwrap();
    modified_at(&source, 300);
    poll(&mut router);
    let dictionary = &router.engine.extra_dictionaries()[0];
    assert!(dictionary.lookup(&["he", "tong", "fa"], false).is_empty());
    assert_eq!(
        dictionary.lookup(&["min", "fa", "dian"], false)[0].text,
        "民法典"
    );

    let removed = bank.join("removed");
    std::fs::create_dir_all(&removed).unwrap();
    std::fs::rename(&source, removed.join("law.tsv")).unwrap();
    poll(&mut router);
    assert!(router.engine.extra_dictionaries().is_empty());
    assert_eq!(std::fs::read(&config_path).unwrap(), original_config);
    drop(router);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dictionary_changes_do_not_retry_broken_config() {
    let dir = std::env::temp_dir().join(format!("cloudime-reload-broken-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("config.toml");
    std::fs::write(&config_path, "[candidate]\ncandidate_count = 5\n").unwrap();
    modified_at(&config_path, 100);
    let config = Config::load(&config_path).unwrap();
    let mut router = Router::new(
        Engine::new(Dictionary::default()),
        RouterConfig::from(&config),
    );
    router.watch_config(
        &config,
        config_path.clone(),
        DataDirs {
            user_root: Some(dir.clone()),
            word_bank: Some(WordBank {
                dir: dir.join("WordBank"),
            }),
            ..DataDirs::default()
        },
    );

    std::fs::write(&config_path, "[broken").unwrap();
    modified_at(&config_path, 200);
    poll(&mut router);
    let bank = dir.join("WordBank");
    std::fs::create_dir_all(&bank).unwrap();
    let source = bank.join("law.tsv");
    std::fs::write(&source, "合同法\the tong fa\t120\n").unwrap();
    poll(&mut router);
    assert_eq!(router.config.page_size, 5);
    assert_eq!(router.engine.extra_dictionaries().len(), 1);

    // 保持坏配置的 mtime，以可观察的配置值确认后续轮询不会重新读它。
    std::fs::write(&config_path, "[candidate]\ncandidate_count = 9\n").unwrap();
    modified_at(&config_path, 200);
    poll(&mut router);
    assert_eq!(router.config.page_size, 5);
    let removed = bank.join("removed");
    std::fs::create_dir_all(&removed).unwrap();
    std::fs::rename(&source, removed.join("law.tsv")).unwrap();
    poll(&mut router);
    assert_eq!(router.config.page_size, 5);
    assert!(router.engine.extra_dictionaries().is_empty());

    modified_at(&config_path, 300);
    poll(&mut router);
    assert_eq!(router.config.page_size, 9);
    drop(router);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `[word_bank] rare_items` 改动走配置热加载：装配时挂着的稀有组跟着开关参与 / 退出查询。
#[test]
fn rare_items_toggle_follows_config() {
    let dir = std::env::temp_dir().join(format!("cloudime-reload-rare-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("config.toml");
    std::fs::write(&config_path, "[word_bank]\nrare_items = false\n").unwrap();
    let config = Config::load(&config_path).unwrap();

    let mut engine =
        Engine::new(Dictionary::default()).with_rare(Dictionary::parse("龘\tda\t500\n").unwrap());
    engine.set_rare_enabled(config.word_bank.rare_items);
    let mut router = Router::new(engine, RouterConfig::from(&config));
    router.watch_config(
        &config,
        config_path.clone(),
        DataDirs {
            user_root: Some(dir.clone()),
            ..DataDirs::default()
        },
    );
    poll(&mut router);
    assert!(!router.engine.rare_enabled());

    // 打开开关：热加载后稀有组参与查询。
    std::fs::write(&config_path, "[word_bank]\nrare_items = true\n").unwrap();
    modified_at(&config_path, 200);
    poll(&mut router);
    assert!(router.engine.rare_enabled());

    // 再关掉。
    std::fs::write(&config_path, "[word_bank]\nrare_items = false\n").unwrap();
    modified_at(&config_path, 300);
    poll(&mut router);
    assert!(!router.engine.rare_enabled());

    drop(router);
    let _ = std::fs::remove_dir_all(&dir);
}
