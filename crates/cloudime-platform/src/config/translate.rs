//! `[translate]` 分节：本地词典的翻译 Tip（候选窗底部那一行左侧的释义）。

use serde::{Deserialize, Serialize};

/// 「学会所需上屏次数」的下限。
pub const MIN_NEED_TIMES: u32 = 3;

/// 「学会所需上屏次数」的上限。
pub const MAX_NEED_TIMES: u32 = 10;

/// `[translate]` 分节。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslateConfig {
    /// 启用翻译 Tip：候选窗底部那一行的左侧显示高亮候选在本地词典里的释义。
    pub enabled: bool,

    /// 选中的本地词典文件名（安装目录 `LocalDictionary\` 下，清单 `dictionaries.list` 里的那一栏）；
    /// 空 = 没选，Tip 不显示。
    pub dictionary: String,

    /// 学会所需上屏次数（3–10）：一个词条的译文上屏这么多次之后算学会，Tip 换颜色。
    pub need_times: u32,

    /// 「重置学习内容」的次数：设置页每点一次加 1，Server 看到它变了就清掉该词典的学习记录。
    /// 设置程序只写配置文件、Server 靠 mtime 热加载，这是两边最省事也最不会打架的通知方式
    /// （学习库只有 Server 一个写入者，不会被内存里的旧值盖回去）。
    pub reset_counter: u64,
}

impl Default for TranslateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dictionary: "glossary-en.db".to_owned(),
            need_times: MIN_NEED_TIMES,
            reset_counter: 0,
        }
    }
}

impl TranslateConfig {
    /// 夹到合法范围的「学会所需上屏次数」。
    pub fn need_times(&self) -> u32 {
        self.need_times.clamp(MIN_NEED_TIMES, MAX_NEED_TIMES)
    }
}
