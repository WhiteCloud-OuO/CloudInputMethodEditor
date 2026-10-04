use serde::{Deserialize, Serialize};

/// 查到的新版本，够菜单和「关于」页显示用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Available {
    pub version: String,

    /// 索引里的渠道：`alpha` / `beta` / `rc` / `stable`。
    pub channel: String,

    pub date: String,

    /// 更新日志，一行一条。
    pub notes: Vec<String>,
}
