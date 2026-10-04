use serde::Deserialize;

use super::Asset;

/// 索引里的一个版本。
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub version: String,

    #[serde(default)]
    pub date: String,

    /// `alpha` / `beta` / `rc` / `stable`。
    pub channel: String,

    /// 更新日志，一行一条。
    #[serde(default)]
    pub notes: Vec<String>,

    #[serde(default)]
    pub assets: Vec<Asset>,
}
