//! `[script]` 分节：用户脚本（安装目录 `Scripts\` 下的 `.lua`）。
//!
//! 规则（每个脚本必须最先声明清单、判无效的脚本不加载、只在此处写开关）见 `docs/design/script.md`。

use serde::{Deserialize, Serialize};

/// `[script]` 分节。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptConfig {
    /// 禁用的脚本文件名（`Scripts\` 下的，大小写不敏感）；空 = 全启用。
    ///
    /// 设置页的「启用 / 禁用」开关写这里。Server **启动时**按它跳过（改完要重启 Server，
    /// 与「改脚本要重启」同一条）。
    pub disabled: Vec<String>,
}
