//! 特殊字符输入器：置顶、默认不抢焦点的特殊符号面板。
//!
//! 由悬浮状态条的「特殊字符」按钮启动（Server 按 exe 旁的固定名
//! `SpecialSymbolsInserter.exe` 起它）；不经过 Server / TSF / 协议。

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod data;
mod fancy;
mod inject;
mod theme;
mod ui;

use cloud_ui::{Decorations, RenderSettings, WindowOptions, run_native};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn"))
        .try_init();

    // 先记下启动时的前台窗口：默认不抢焦点，这个就是 SendInput 的第一目标。
    let target = inject::foreground_window();
    let theme = theme::load();
    let render_settings = RenderSettings {
        // 符号优先用 Segoe UI Symbol：多数符号在它是单色字形，
        // 落到 Segoe UI Emoji 会变成彩色（☑ 变蓝框），与原版不一致；缺字再回退中文 / 假名字体。
        candidate_font: ("Segoe UI Symbol".to_owned(), 14.0),
        pinyin_font: ("Microsoft YaHei UI".to_owned(), 11.0),
        // 符号页里 ⑫ ♥ ✓ 🅐 这类字符靠补充字体兜底（cosmic-text 按字回退）。
        symbol_font: "Segoe UI Symbol".to_owned(),
        japanese_font: "MS Gothic".to_owned(),
        ..RenderSettings::default()
    };
    run_native(
        WindowOptions {
            title: "特殊字符输入器".to_owned(),
            size: (600.0, 680.0),
            min_size: Some((520.0, 560.0)),
            decorations: Decorations::None,
            top_most: true,
            no_activate: true,
            shadow: true,
            corner_radius: 12.0,
            caption_height: Some(app::chrome::HEIGHT),
            render_settings,
            ..WindowOptions::default()
        },
        app::SsiApp::new(theme, target),
    )?;
    Ok(())
}
