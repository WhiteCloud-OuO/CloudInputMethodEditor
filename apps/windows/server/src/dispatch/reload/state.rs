//! 配置热加载记的状态。

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use cloudime_dictionary::Dictionary;
use cloudime_platform::{PhraseStore, UpdateConfig, WordBank};

/// 随包与用户数据目录：启动与热加载用的是同一批（词库）。
/// 分开传参数会越传越长，且热加载与原路径不一致时找不到文件。
#[derive(Debug, Clone, Default)]
pub struct DataDirs {
    /// 用户数据根目录（学习数据在它下面）。
    pub user_root: Option<PathBuf>,

    /// 随包根（词库目录按它定位）。
    pub root: Option<PathBuf>,

    /// 词库目录（WordBank）。
    pub word_bank: Option<WordBank>,

    /// 主词库路径；它不再当附加词库加载一遍。
    pub main_dict: Option<PathBuf>,

    /// 短语库（安装目录下的 `Phrases\Phrase.db`）。
    pub phrase: Option<PhraseStore>,
}

impl DataDirs {
    /// 词库目录的同一份快照；没配目录为空。
    pub(super) fn dict_snapshot(&self) -> Vec<(PathBuf, Option<SystemTime>, u64)> {
        match &self.word_bank {
            Some(bank) => cloudime_platform::word_bank::snapshot(&bank.dir),
            None => Vec::new(),
        }
    }

    /// 短语库文件的快照；没配为空。
    pub(super) fn phrase_snapshot(&self) -> Vec<(PathBuf, Option<SystemTime>, u64)> {
        match &self.phrase {
            Some(store) => cloudime_platform::word_bank::snapshot_files(&[store.path.as_path()]),
            None => Vec::new(),
        }
    }
}

/// 热加载状态。
pub(crate) struct ConfigReload {
    /// `config.toml` 路径。
    pub(super) config_path: PathBuf,

    /// 上次看文件的时间（节流用）。
    pub(super) last_check: Instant,

    /// 随包与用户数据目录。
    pub(super) dirs: DataDirs,

    /// 上次看到的 mtime。
    pub(super) last_mtime: Option<SystemTime>,

    /// 最近加载的词库文件快照（路径、修改时间、长度）。
    pub(super) dictionary_files: Vec<(PathBuf, Option<SystemTime>, u64)>,

    /// 最近加载的短语库快照。
    pub(super) phrase_files: Vec<(PathBuf, Option<SystemTime>, u64)>,

    /// 当前的 `[update]`。
    pub(super) update: UpdateConfig,

    /// 软件自带短语是否参与（`[phrase] use_default_phrases`）。
    pub(super) use_default_phrases: bool,

    /// 检查更新：结果写进用户目录的 `update.json`，设置程序的「关于」页读它；拿不到用户目录时没有。
    pub(super) updates: Option<cloudime_update::Checker>,
}

impl ConfigReload {
    /// 装配附加词库，跳过主词库（它已经装在 Engine 上）。
    pub(super) fn load_dictionaries(&self) -> Vec<Dictionary> {
        let Some(bank) = &self.dirs.word_bank else {
            return Vec::new();
        };
        let dictionaries = bank.load_except(self.dirs.main_dict.as_deref());
        tracing::info!(count = dictionaries.len(), "词库已热重装");
        dictionaries
    }
}
