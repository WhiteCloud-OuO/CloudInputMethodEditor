//! 组字查字窗口：IDS 组字查询 + 互动画布。
//!
//! 数据从 `ids.qj` 读取（mmap），窗口只是一个普通 Win32 进程；不经过 Server / TSF。

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod canvas;
mod clipboard;
mod composer;
mod config;
mod data;
mod drawer;
mod glyph;
mod preview;
mod results;
mod ui;

use std::path::PathBuf;

use cloud_ui::{Decorations, RenderSettings, WindowOptions, run_native};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn"))
        .try_init();

    let database = data::resolve(data_argument().as_deref()).map_err(|error| error.to_string());
    let config = config::load();
    // 系统 UI 字体的 GDI 链接不覆盖 IDS 运算符（U+2FF0..），缺省用微软雅黑；
    // 扩展区部件缺字时由字形探测退回缩写。
    let render_settings = RenderSettings {
        candidate_font: (config.font_family.clone(), 13.0),
        pinyin_font: (config.font_family.clone(), 11.0),
        ..RenderSettings::default()
    };
    run_native(
        WindowOptions {
            title: "组字查字".to_owned(),
            size: (980.0, 660.0),
            min_size: Some((760.0, 520.0)),
            decorations: Decorations::None,
            shadow: true,
            corner_radius: 12.0,
            caption_height: Some(36.0),
            render_settings,
            ..WindowOptions::default()
        },
        app::IdsApp::new(database, config),
    )?;
    Ok(())
}

/// `--data <ids.qj>`：调试时指定数据文件；不做完整命令行解析。
fn data_argument() -> Option<PathBuf> {
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--data" {
            return arguments.next().map(PathBuf::from);
        }
    }
    None
}
