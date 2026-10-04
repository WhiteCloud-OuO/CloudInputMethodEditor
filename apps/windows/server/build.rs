//! 嵌 uiAccess manifest：候选窗口要盖过商店 / 任务栏搜索这些高 z-band 宿主，`SetWindowPos(HWND_TOPMOST)`
//! 才能升进 UIAccess 高带。系统只对签名且装在 Program Files 的 exe 授予，光有 manifest 不够；
//! **没签名的 exe 带 uiAccess=true 会直接起不来**，所以没有证书的构建（CI 内测包）要设 `CLOUDIME_UIACCESS=0`
//! 关掉它，代价是候选窗在 UWP 宿主里可能被盖住（用户文档已列为已知问题）。
//! manifest 缺省含 PerMonitorV2 DPI 感知，与运行时那次 `SetProcessDpiAwarenessContext` 一致。
//! 另把云朵输入法图标嵌进 exe（任务管理器 / 启动项里显示），并把悬浮状态条的图标按钮拷到 exe 旁的 `data\`。

use std::path::{Path, PathBuf};

use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    // build.rs 跑在宿主机上，只有目标是 Windows 时才嵌。
    println!("cargo:rerun-if-env-changed=CLOUDIME_UIACCESS");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let ui_access = std::env::var("CLOUDIME_UIACCESS").map_or(true, |v| v != "0");
        if !ui_access {
            println!(
                "cargo:warning=CLOUDIME_UIACCESS=0：Server 不带 uiAccess，候选窗在 UWP 宿主里可能被盖住"
            );
        }
        let manifest = new_manifest("CloudIME.Server")
            .requested_execution_level(ExecutionLevel::AsInvoker)
            .ui_access(ui_access);
        embed_manifest(manifest).expect("嵌入 Server manifest 失败");
    }
    embed_icon();
    stage_status_icons();
    println!("cargo:rerun-if-changed=build.rs");
}

/// 悬浮状态条的图标按钮：把 `src\ui\status\` 下的排布表与 `icons\` 拷到 exe 旁的 `data\`。
///
/// 装机包里由 `cloudime.iss` 装到 `{app}\data\`，Server 按 exe 位置找同一份；开发时就得让 `cargo build`
/// 也拷一份到 `target\{profile}\data\`，不然跑起来的 Server 找不到排布表、状态条一个按钮都不画。
/// 失败只警告，别让编译挂掉（状态条不显示，输入照常）。
fn stage_status_icons() {
    let status = Path::new("src/ui/status");
    let icons = status.join("icons");
    let arrangement = status.join("icons-arrangement.cfg");
    println!("cargo:rerun-if-changed={}", arrangement.display());
    println!("cargo:rerun-if-changed={}", icons.display());
    // OUT_DIR 是 `{target}\{profile}\build\{包名}-{哈希}\out`，往上三层就是 exe 所在的 `{target}\{profile}`
    let Some(profile) = std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .and_then(|out| out.ancestors().nth(3).map(Path::to_path_buf))
    else {
        return;
    };
    let data = profile.join("data");
    if let Err(error) = stage(&arrangement, &data.join("icons-arrangement.cfg")) {
        println!("cargo:warning=拷贝状态条图标排布表失败: {error}");
    }
    match std::fs::read_dir(&icons) {
        Ok(entries) => {
            for entry in entries.flatten() {
                if let Err(error) =
                    stage(&entry.path(), &data.join("icons").join(entry.file_name()))
                {
                    println!("cargo:warning=拷贝状态条图标失败: {error}");
                }
            }
        }
        Err(error) => println!("cargo:warning=读状态条图标目录失败: {error}"),
    }
}

/// 拷一个文件过去，需要时先建目录。
fn stage(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to).map(|_| ())
}

/// 图标资源要 `rc.exe`（MSVC）编，只在 Windows 宿主上做；失败只警告，别让编译挂掉。
/// winresource 缺省不带 manifest，与上面链接器嵌的那份不冲突。
#[cfg(windows)]
fn embed_icon() {
    const ICON: &str = "../tsf/resources/cloudime.ico";
    println!("cargo:rerun-if-changed={ICON}");
    if let Err(error) = winresource::WindowsResource::new().set_icon(ICON).compile() {
        println!("cargo:warning=嵌入 Server 图标失败: {error}");
    }
}

#[cfg(not(windows))]
fn embed_icon() {}
