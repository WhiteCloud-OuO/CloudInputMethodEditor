//! 随包资源（词库 / 语言模型 / 样例）的定位。
//!
//! 两套布局，装机优先、回落开发：
//! - **装机**：资源与可执行文件同级（安装程序把 `data\` / `assets\` 装在 exe 旁）。
//! - **开发**：仓库 `ime/` 目录，exe 在 `ime\target\{debug,release}\` 下，往上三层。
//!
//! 相对写法两套布局一致（如 `data/generated/dict.qj`），只有根不同。
//! Server 进程与设置窗口共用的资源路径。

use std::path::{Path, PathBuf};

/// 随包资源的根目录：其下有 `data/` 与 `assets/`。装机布局与 exe 同级，否则回落开发布局的仓库根；两处都没有为 `None`。
pub fn bundled_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    if has_resources(exe_dir) {
        return Some(exe_dir.to_path_buf());
    }
    // exe → {debug,release} → target → ime
    let dev_root = exe.ancestors().nth(3)?;
    has_resources(dev_root).then(|| dev_root.to_path_buf())
}

/// 一个随包资源的完整路径（相对随包根，如 `data/generated/dict.qj`）；根找不到或该路径不存在为 `None`。
pub fn bundled_resource(rel: &str) -> Option<PathBuf> {
    let path = bundled_root()?.join(rel);
    path.exists().then_some(path)
}

/// 一个目录是不是随包根：`data` 与 `assets` **都**在才算。
///
/// 只认两者都有是为了开发布局：`cargo build` 会把状态条图标拷进 `target\{debug,release}\data\`，
/// 只看 `data` 的话那个目录会被当成装机根，开发时 Server 反而找不到仓库根的词库（装机包两者都有）。
fn has_resources(dir: &Path) -> bool {
    dir.join("data").is_dir() && dir.join("assets").is_dir()
}
