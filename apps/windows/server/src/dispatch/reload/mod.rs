//! 配置热加载：空闲时看 `config.toml` 的 mtime，改了就重读并应用。
//! 便宜的设置无条件重设；附加词库也检查文件增删与更新。热加载状态在 [`ConfigReload`]。

mod state;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use cloudime_platform::{Config, PhraseStore};

pub(super) use self::state::ConfigReload;
pub use self::state::DataDirs;

/// 看配置文件 mtime 的最短间隔；工人循环空闲时按它等，重排的短节拍来得更勤时按这个节流。
pub(super) const CONFIG_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// 检查更新的结果文件名，在用户数据目录下（见 `cloudime-update::UpdateState`）。
const UPDATE_STATE_FILE: &str = "update.json";

use super::{Router, RouterConfig};

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

impl Router {
    /// 检查更新查到了要提示的新版本（开关关着、本地开发包都不算）。
    pub(super) fn update_available(&self) -> bool {
        self.reload.as_ref().is_some_and(|reload| {
            reload
                .updates
                .as_ref()
                .is_some_and(|updates| updates.available(&reload.update).is_some())
        })
    }

    /// `config.toml` 路径；没开热加载（测试）时为 `None`。
    pub(super) fn config_path(&self) -> Option<&Path> {
        self.reload
            .as_ref()
            .map(|reload| reload.config_path.as_path())
    }

    /// 开启热加载：记下路径与当前已应用的词库快照，
    /// 以及启动用的那批数据目录。目录必须与启动同款语义（WordBank），热加载才找得到文件。
    pub fn watch_config(&mut self, config: &Config, config_path: PathBuf, dirs: DataDirs) {
        let last_mtime = mtime(&config_path);
        let dictionary_files = dirs.dict_snapshot();
        let phrase_files = dirs.phrase_snapshot();
        let updates = dirs.user_root.as_deref().map(|dir| {
            cloudime_update::Checker::new(dir.join(UPDATE_STATE_FILE), env!("CARGO_PKG_VERSION"))
        });
        self.reload = Some(ConfigReload {
            config_path,
            last_check: Instant::now(),
            dirs,
            last_mtime,
            dictionary_files,
            phrase_files,
            update: config.update.clone(),
            use_default_phrases: config.phrase.use_default_phrases,
            updates,
        });
    }

    /// 空闲时调；一秒内只真正看一次。配置文件或词库目录变了就重装；
    /// 解析失败保持原配置，mtime 照记（不每秒重试同一个坏文件）。
    pub fn poll_config_reload(&mut self) {
        let Some(reload) = &mut self.reload else {
            return;
        };
        if reload.last_check.elapsed() < CONFIG_POLL_INTERVAL {
            return;
        }
        reload.last_check = Instant::now();
        if let Some(updates) = &reload.updates {
            updates.poll(&reload.update);
        }
        // 词库目录文件增删、同名更新：与配置改动无关，下一拍就生效
        let files = reload.dirs.dict_snapshot();
        if files != reload.dictionary_files {
            // 配置损坏也继续使用上次有效的词库开关；文件变化不触发配置重试。
            self.engine
                .set_extra_dictionaries(reload.load_dictionaries());
            reload.dictionary_files = files;
        }
        // 短语库改了（「设置 → 短语」页存过）：重读短语，配置没动也能生效
        let phrases = reload.dirs.phrase_snapshot();
        let phrases_changed = phrases != reload.phrase_files;
        if phrases_changed {
            reload.phrase_files = phrases;
        }
        let config_changed = {
            let current = mtime(&reload.config_path);
            let changed = current != reload.last_mtime;
            reload.last_mtime = current;
            changed
        };
        let path = reload.config_path.clone();
        if config_changed {
            match Config::load(&path) {
                Ok(config) => {
                    self.apply_config(&config);
                    tracing::info!("配置已热加载");
                }
                Err(error) => tracing::error!(%error, "配置热加载解析失败，保持原配置"),
            }
        }
        if phrases_changed {
            self.reload_phrases();
        }
    }

    /// 应用新配置。
    fn apply_config(&mut self, config: &Config) {
        self.engine.set_fuzzy(config.input.fuzzy_rules());
        self.engine.set_use_jian_pin(config.input.use_jian_pin);
        self.engine.set_mixture_input(config.input.mixture_input);
        self.engine
            .set_punctuation_mapping(config.input.punctuation_mapping());
        self.engine
            .set_half_wide_after_digit(config.input.use_half_wide_punctuation_marks_after_digital);
        self.engine
            .set_association_counts(config.candidate.association_counts());
        self.engine.set_traditional_mode(
            config.input.simp_trad_chinese_chars_toggle == cloudime_platform::SimpTrad::Traditional,
        );
        self.engine.set_learning(config.general.learning);
        self.engine.set_rare_enabled(config.word_bank.rare_items);
        // 全角 / 半角是会话内状态（不进配置文件）：重建配置时沿用当前值，用户刚切的不被热加载冲掉
        let full_width_punctuation = self.config.full_width_punctuation;
        let english_full_width_punctuation = self.config.english_full_width_punctuation;
        let full_width_chars = self.config.full_width_chars;
        // 脚本设的候选窗尺寸（本组句内有效）也一样，别被热加载冲掉
        let script_min_width = self.config.script_min_width;
        let script_page_size = self.config.script_page_size;
        let script_scale = self.config.script_scale;
        self.config = RouterConfig::from(config);
        // 本地词典 / 学习状态跟着配置走：换词典、重置学习内容都在这里落地
        self.translate.configure(
            &self.config.translate_dictionary,
            self.config.translate_reset_counter,
        );
        // 脚本设的候选窗尺寸也是会话内状态（组句结束才清）：热加载时别把它抹掉
        self.config.script_min_width = script_min_width;
        self.config.script_page_size = script_page_size;
        self.config.script_scale = script_scale;
        self.config.full_width_punctuation = full_width_punctuation;
        self.config.english_full_width_punctuation = english_full_width_punctuation;
        self.config.full_width_chars = full_width_chars;
        let settings = self.config.render_settings();
        // 别拿 `settings != previous` 当门：那种 `previous` 是拿**此刻磁盘上的主题文件**读出来的，主题文件
        // 先被「确认保存」改过时它已经是新主题，两边相等就把「重新应用同一个主题」吞掉（只有换主题才生效）。
        // 真正「当前生效的设置」只有 painter 知道，交给 `Painter::configure` 比对（没变它自己会跳过）。
        self.candidates.configure(settings.clone());
        // 脚本的量尺也跟上：字体设置变了它自己会重建渲染器
        self.measure.borrow_mut().0 = settings;
        self.reconcile_status();
        self.apply_model_config(config.candidate.use_local_sentence_organization_model);

        let Some(reload) = &mut self.reload else {
            return;
        };
        reload.update = config.update.clone();
        reload.use_default_phrases = config.phrase.use_default_phrases;
        // 短语库固定在安装目录；自带短语开关可能变了，重新定位并从短语库重读一遍
        if let Some(root) = self
            .reload
            .as_ref()
            .and_then(|reload| reload.dirs.root.clone())
        {
            if let Some(reload) = &mut self.reload {
                reload.dirs.phrase = Some(PhraseStore::locate(&root));
                reload.phrase_files = reload.dirs.phrase_snapshot();
            }
            self.reload_phrases();
        }
    }

    /// 从短语库重装短语；读不出来或内容不合法时保持原短语。
    pub(super) fn reload_phrases(&mut self) {
        let Some(reload) = self.reload.as_ref() else {
            return;
        };
        let Some(store) = reload.dirs.phrase.clone() else {
            return;
        };
        let use_default = reload.use_default_phrases;
        match store.load(use_default) {
            Ok(phrases) => {
                if let Err(error) = self.engine.set_custom_phrases(phrases) {
                    tracing::warn!(%error, "短语库内容不合法，保持原短语");
                }
            }
            Err(error) => tracing::warn!(%error, "短语库读不出来，保持原短语"),
        }
    }
}
