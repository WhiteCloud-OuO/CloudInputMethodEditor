//! 组字查字窗口的清单与图标：PerMonitorV2 DPI 感知 + 云朵输入法图标。

fn main() {
    println!("cargo:rerun-if-changed=resources/cloudime.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::manifest::ExecutionLevel;
        use embed_manifest::{embed_manifest, new_manifest};

        let manifest =
            new_manifest("CloudIME.Ids").requested_execution_level(ExecutionLevel::AsInvoker);
        embed_manifest(manifest).expect("嵌入组字查字 manifest 失败");
    }
    embed_icon();
}

#[cfg(windows)]
fn embed_icon() {
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("resources/cloudime.ico");
    resource.set("FileDescription", "CloudIME 组字查字");
    resource.set("ProductName", "云朵输入法");
    resource.set("OriginalFilename", "cloudime-ids.exe");
    if let Err(error) = resource.compile() {
        println!("cargo:warning=嵌入组字查字图标失败: {error}");
    }
}

#[cfg(not(windows))]
fn embed_icon() {}
