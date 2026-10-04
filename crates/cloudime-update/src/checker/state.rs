use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use cloudime_platform::UpdateChannel;

use super::Available;

/// 上次检查的结果，存在数据目录的 `update.json`；Windows 的设置程序直接读这个文件。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateState {
    /// 上次查成功的时间（Unix 秒）；0 = 还没查过。
    pub checked_at: u64,

    /// 那次查的是哪个渠道；换了渠道要立刻重查。
    pub channel: Option<UpdateChannel>,

    /// 那次查到的新版本。
    pub available: Option<Available>,
}

impl UpdateState {
    /// 读不到或读不懂都当没查过。
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// 先写临时文件再改名，读的一方不会看到半截。
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&temporary, path)
    }

    pub(crate) fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs())
    }
}
