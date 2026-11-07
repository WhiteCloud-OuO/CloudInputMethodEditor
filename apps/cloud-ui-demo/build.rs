//! 独立应用示例的清单与图标：PerMonitorV2 DPI 感知 manifest + `resources/cloud-ui-demo.ico`。
//!
//! 产品化时复制这一份 build.rs，改 manifest 名与图标路径即可；`embed_manifest` 缺省会写
//! PerMonitorV2，与 `run_native` 里的 `SetProcessDpiAwarenessContext` 一致。

fn main() {
    println!("cargo:rerun-if-changed=resources/cloud-ui-demo.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::manifest::ExecutionLevel;
        use embed_manifest::{embed_manifest, new_manifest};

        let manifest =
            new_manifest("CloudUI.Demo").requested_execution_level(ExecutionLevel::AsInvoker);
        embed_manifest(manifest).expect("嵌入 Cloud UI Demo manifest 失败");
    }
    embed_icon();
}

#[cfg(windows)]
fn embed_icon() {
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("resources/cloud-ui-demo.ico");
    resource.set("FileDescription", "Cloud UI Demo");
    resource.set("ProductName", "Cloud UI Demo");
    resource.set("OriginalFilename", "cloud-ui-demo.exe");
    if let Err(error) = resource.compile() {
        println!("cargo:warning=嵌入 Cloud UI Demo 图标失败: {error}");
    }
}

#[cfg(not(windows))]
fn embed_icon() {}
