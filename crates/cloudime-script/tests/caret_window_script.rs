//! 随包的测试脚本（`Scripts/lib/test-caret-window.lua`）的行为：
//! 按键请求光标 → `caret` 事件建窗口 / 读 SVG → 点扁平按钮的自动区域销毁。

use std::path::{Path, PathBuf};

use cloudime_render::{ButtonState, Color, Document, Element};
use cloudime_script::{Runtime, WindowRequest, mlua};

/// 一个干净的临时脚本目录。
fn script_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cloudime-script-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn event(runtime: &Runtime, fields: &[(&str, mlua::Value)]) -> mlua::Table {
    let table = runtime.lua().create_table().unwrap();
    for (name, value) in fields {
        table.set(*name, value.clone()).unwrap();
    }
    table
}

fn content_of(requests: &[WindowRequest], id: u32) -> &Document {
    requests
        .iter()
        .find_map(|request| match request {
            WindowRequest::SetContent(shown, document) if *shown == id => Some(document),
            _ => None,
        })
        .expect("送了内容")
}

#[test]
fn the_caret_window_script_draws_and_closes_itself() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Scripts/lib/test-caret-window.lua");
    let dir = script_dir("caret-window");
    std::fs::copy(&source, dir.join("test-caret-window.lua")).unwrap();
    let runtime = Runtime::load(&dir, &[]);
    runtime.set_install_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    // 当前主题：脚本用 `cloudime.get_theme()` 取色（这里只给用到的几项）。
    runtime.set_theme_hook(std::rc::Rc::new(
        |lua: &mlua::Lua| -> mlua::Result<mlua::Table> {
            let candidate = lua.create_table()?;
            candidate.set("background", "#FF123456")?;
            candidate.set("text", "#FFABCDEF")?;
            candidate.set("page_number", "#FF888888")?;
            candidate.set("extra", "#FF0000FF")?;
            let theme = lua.create_table()?;
            theme.set("candidate", candidate)?;
            Ok(theme)
        },
    ));
    assert!(runtime.scripts().iter().any(|name| name == "光标窗口测试"));

    // Ctrl+Alt+F3：脚本请求光标位置。
    let key = event(
        &runtime,
        &[
            ("vk", mlua::Value::Integer(0x72)),
            ("ctrl", mlua::Value::Boolean(true)),
            ("alt", mlua::Value::Boolean(true)),
            ("shift", mlua::Value::Boolean(false)),
            ("win", mlua::Value::Boolean(false)),
            ("composing", mlua::Value::Boolean(false)),
        ],
    );
    runtime.dispatch("key", key.clone(), None);
    assert!(runtime.take_caret_request());

    // DLL 回来光标矩形（屏幕点）：窗口摆在光标上方 30 点。
    let caret = event(
        &runtime,
        &[
            ("left", mlua::Value::Number(100.0)),
            ("top", mlua::Value::Number(200.0)),
            ("right", mlua::Value::Number(101.0)),
            ("bottom", mlua::Value::Number(216.0)),
            ("work_left", mlua::Value::Number(0.0)),
            ("work_top", mlua::Value::Number(0.0)),
            ("work_right", mlua::Value::Number(1920.0)),
            ("work_bottom", mlua::Value::Number(1080.0)),
            ("scale", mlua::Value::Number(1.0)),
        ],
    );
    runtime.dispatch("caret", caret, None);

    let requests = runtime.take_window_requests();
    let id = requests
        .iter()
        .find_map(|request| match request {
            WindowRequest::Create(id, spec) => {
                assert!(!spec.background, "面板底色 / 圆角由脚本自己画");
                assert!(spec.shadow, "阴影与候选窗同一套（缺省开启）");
                Some(*id)
            }
            _ => None,
        })
        .expect("建了窗口");
    let show = requests
        .iter()
        .find_map(|request| match request {
            WindowRequest::Show(shown, Some((x, y))) => (*shown == id).then_some((*x, *y)),
            _ => None,
        })
        .expect("显示窗口");
    assert_eq!(show, (100.0, 246.0), "上方放不下，改到光标下方 30 点");

    let document = content_of(&requests, id);
    // 背景由脚本自己画：第一块是整窗圆角矩形，颜色来自 `cloudime.get_theme()`。
    assert!(matches!(
        &document.elements[0],
        Element::Rect { x, y, width, height, color, .. }
            if x.abs() < 0.01 && y.abs() < 0.01
                && (*width - 500.0).abs() < 0.01
                && (*height - 360.0).abs() < 0.01
                && *color == Color::rgba(0x12, 0x34, 0x56, 0xFF)
    ));
    assert!(
        document.elements.iter().any(|element| matches!(
            element,
            Element::FlatSvgButton(button)
                if button.svg.contains("<svg")
                    && button.state == ButtonState::Normal
                    && (button.width - 20.0).abs() < 0.01
                    && (button.height - 20.0).abs() < 0.01
                    && button.region_name == "close"
        )),
        "关闭按钮是扁平 SVG 按钮（画板 20×20，自动区域 close）"
    );
    assert!(document.regions.iter().any(|region| region.name == "close"));

    // 开着窗口再按 F3：不创建、不关闭。
    runtime.dispatch("key", key, None);
    assert!(
        !runtime
            .take_window_requests()
            .iter()
            .any(|request| matches!(
                request,
                WindowRequest::Close(_) | WindowRequest::Create(_, _)
            ))
    );

    // 点关闭区域：销毁窗口。
    runtime.dispatch(
        "window",
        event(
            &runtime,
            &[
                ("id", mlua::Value::Integer(i64::from(id))),
                (
                    "action",
                    mlua::Value::String(runtime.lua().create_string("mouse_clicked").unwrap()),
                ),
                (
                    "region",
                    mlua::Value::String(runtime.lua().create_string("close").unwrap()),
                ),
            ],
        ),
        None,
    );
    assert!(
        runtime
            .take_window_requests()
            .iter()
            .any(|request| matches!(request, WindowRequest::Close(closed) if *closed == id))
    );
}
