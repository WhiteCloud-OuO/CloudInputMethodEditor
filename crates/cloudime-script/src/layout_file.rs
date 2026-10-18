//! 控件布局文件（JSON）：把前台 UI 文件读成脚本窗口的声明式内容表。
//!
//! 布局文件只描述控件，真正的默认值 / 几何 / 区域生成仍走 [`crate::window`] 的同一套解析器；
//! 这样 JSON 与 Lua `set_content` 的元素字段完全一致，只多了顶层 `controls` 数组。
//! 文件里的路径（`svg_path` / `img.path`）先按布局文件目录找，再逐级向上找；因此放在
//! `Scripts/lib/` 里的布局可以直接引用安装根目录的 `Images/...`。
//!
//! **每个控件必须写 `name`**：它既是脚本认控件的标识符，也参与布局内唯一性校验。

use std::collections::HashSet;
use std::path::Path;

use cloudime_render::ControlName;
use mlua::{Lua, Table, Value as LuaValue};
use serde_json::Value;

/// 布局文件最大字节数：控件描述是文本，4 MB 已经足够复杂面板。
const MAX_LAYOUT_BYTES: u64 = 4 * 1024 * 1024;

/// 控件名最长 255 个字符（规则里字符集全是 ASCII，所以字节数也一样）。
const MAX_NAME_LEN: usize = 255;

/// 不能用作控件名的 Rust 关键字（严格关键字 + 保留关键字 + 弱关键字）。
const RUST_KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "raw",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "union", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// 一份解析后的布局：可直接交给内容解析器的 Lua 表 + 控件名索引。
#[derive(Debug)]
pub(crate) struct Layout {
    pub table: Table,
    pub control_names: Vec<ControlName>,
}

/// 读一个布局文件，转成 `set_content` 能直接消费的 Lua 表。
pub(crate) fn read(lua: &Lua, path: &Path) -> mlua::Result<Layout> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        mlua::Error::RuntimeError(format!("读不了布局文件 {}：{error}", path.display()))
    })?;
    if metadata.len() > MAX_LAYOUT_BYTES {
        return Err(mlua::Error::RuntimeError(format!(
            "布局文件 {} 太大（{} 字节，上限 {MAX_LAYOUT_BYTES}）",
            path.display(),
            metadata.len()
        )));
    }
    let text = std::fs::read_to_string(path).map_err(|error| {
        mlua::Error::RuntimeError(format!("读不了布局文件 {}：{error}", path.display()))
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let value: Value = serde_json::from_str(text).map_err(|error| {
        mlua::Error::RuntimeError(format!(
            "布局文件 {} 不是合法 JSON：{error}",
            path.display()
        ))
    })?;
    content(lua, &value, path.parent().unwrap_or_else(|| Path::new(".")))
        .map_err(|error| mlua::Error::RuntimeError(format!("布局文件 {}：{error}", path.display())))
}

/// JSON 根 → 内容表 + 控件名：`{ width, height, regions, <controls 数组> }`。
///
/// 根可以是对象，也可以是直接写控件数组；对象里 `controls` 与 `elements` 等价。
fn content(lua: &Lua, value: &Value, base: &Path) -> mlua::Result<Layout> {
    let table = lua.create_table()?;
    let mut control_names = Vec::new();
    let mut seen = HashSet::new();
    let mut path = Vec::new();
    match value {
        Value::Array(controls) => {
            append_controls(
                lua,
                &table,
                controls,
                &mut control_names,
                &mut seen,
                &mut path,
                base,
            )?;
        }
        Value::Object(root) => {
            if let Some(width) = root.get("width").filter(|value| !value.is_null()) {
                table.raw_set("width", json_number(width, "width")?)?;
            }
            if let Some(height) = root.get("height").filter(|value| !value.is_null()) {
                table.raw_set("height", json_number(height, "height")?)?;
            }
            let controls = root
                .get("controls")
                .or_else(|| root.get("elements"))
                .ok_or_else(|| {
                    mlua::Error::RuntimeError(
                        "根对象缺 controls（控件数组；也可以用 elements）".to_owned(),
                    )
                })?;
            let Value::Array(controls) = controls else {
                return Err(mlua::Error::RuntimeError("controls 必须是数组".to_owned()));
            };
            append_controls(
                lua,
                &table,
                controls,
                &mut control_names,
                &mut seen,
                &mut path,
                base,
            )?;
            if let Some(regions) = root.get("regions").filter(|value| !value.is_null()) {
                let Value::Array(_) = regions else {
                    return Err(mlua::Error::RuntimeError("regions 必须是数组".to_owned()));
                };
                table.raw_set("regions", to_lua(lua, regions, base)?)?;
            }
        }
        _ => {
            return Err(mlua::Error::RuntimeError(
                "布局根要是一个对象（width / height / controls）或控件数组".to_owned(),
            ));
        }
    }
    Ok(Layout {
        table,
        control_names,
    })
}

/// 控件数组 → 内容表的 1 起序列；每个控件必须是对象，并在这里校验 / 收集 `name`。
fn append_controls(
    lua: &Lua,
    table: &Table,
    controls: &[Value],
    names: &mut Vec<ControlName>,
    seen: &mut HashSet<String>,
    path: &mut Vec<usize>,
    base: &Path,
) -> mlua::Result<()> {
    for (index, control) in controls.iter().enumerate() {
        path.push(index);
        collect_control(control, path, names, seen)?;
        table.raw_set(index as i64 + 1, to_lua(lua, control, base)?)?;
        path.pop();
    }
    Ok(())
}

/// 读取并校验一个控件的 `name`，再递归校验 `children`；名字在同一份布局里区分大小写。
fn collect_control(
    value: &Value,
    path: &[usize],
    names: &mut Vec<ControlName>,
    seen: &mut HashSet<String>,
) -> mlua::Result<()> {
    let Some(control) = value.as_object() else {
        return Err(mlua::Error::RuntimeError(format!(
            "{} 不是对象",
            path_label(path)
        )));
    };
    let name = control
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| mlua::Error::RuntimeError(format!("{} 缺 name", path_label(path))))?;
    validate_name(name).map_err(|error| {
        mlua::Error::RuntimeError(format!("{} 的 name 不合法：{error}", path_label(path)))
    })?;
    if !seen.insert(name.to_owned()) {
        return Err(mlua::Error::RuntimeError(format!(
            "控件名重复：{name}（同一份布局里区分大小写）"
        )));
    }
    names.push(ControlName {
        name: name.to_owned(),
        path: path.to_vec(),
    });

    if let Some(children) = control.get("children").filter(|value| !value.is_null()) {
        let Value::Array(children) = children else {
            return Err(mlua::Error::RuntimeError(format!(
                "{} 的 children 必须是数组",
                path_label(path)
            )));
        };
        for (index, child) in children.iter().enumerate() {
            let mut child_path = path.to_vec();
            child_path.push(index);
            collect_control(child, &child_path, names, seen)?;
        }
    }
    Ok(())
}

/// 控件名规则：ASCII 字母开头，后续只能是 ASCII 字母 / 数字 / 下划线，最长 255，且不能是 Rust 关键字。
fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("不能为空".to_owned());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("最长 {MAX_NAME_LEN} 个字符"));
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err("不能为空".to_owned());
    };
    if !first.is_ascii_alphabetic() {
        return Err("必须以英文字母开头".to_owned());
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("只能包含英文字母、数字和下划线".to_owned());
    }
    if RUST_KEYWORDS.contains(&name) {
        return Err(format!("{name} 是 Rust 关键字"));
    }
    Ok(())
}

fn path_label(path: &[usize]) -> String {
    let mut label = String::from("controls");
    for index in path {
        label.push('[');
        label.push_str(&index.to_string());
        label.push(']');
    }
    label
}

/// JSON 值 → Lua 值：数组变 1 起序列，对象变字符串键表，`null` 变 `nil`。
fn to_lua(lua: &Lua, value: &Value, base: &Path) -> mlua::Result<LuaValue> {
    Ok(match value {
        Value::Null => LuaValue::Nil,
        Value::Bool(value) => LuaValue::Boolean(*value),
        Value::Number(number) => LuaValue::Number(number.as_f64().ok_or_else(|| {
            mlua::Error::RuntimeError(format!("JSON 数字 {number} 放不进 Lua number"))
        })?),
        Value::String(text) => LuaValue::String(lua.create_string(text)?),
        Value::Array(items) => {
            let table = lua.create_table()?;
            for (index, item) in items.iter().enumerate() {
                table.raw_set(index as i64 + 1, to_lua(lua, item, base)?)?;
            }
            LuaValue::Table(table)
        }
        Value::Object(fields) => {
            let table = lua.create_table()?;
            for (key, field) in fields {
                let value = if matches!(key.as_str(), "svg_path" | "path") {
                    match field {
                        Value::String(text) => {
                            LuaValue::String(lua.create_string(resolve_path(base, text))?)
                        }
                        _ => to_lua(lua, field, base)?,
                    }
                } else {
                    to_lua(lua, field, base)?
                };
                table.raw_set(key.as_str(), value)?;
            }
            LuaValue::Table(table)
        }
    })
}

/// JSON 里的文件路径：绝对路径原样；相对路径先按布局文件目录找，再逐级向上找
///（这样 `Scripts/lib/ui.json` 里的 `Images/...` 能落到安装根目录的图片）。
fn resolve_path(base: &Path, value: &str) -> String {
    let path = Path::new(value);
    if path.is_absolute() {
        return value.to_owned();
    }
    let mut directory = Some(base);
    while let Some(current) = directory {
        let candidate = current.join(path);
        if candidate.exists() {
            return candidate.to_string_lossy().into_owned();
        }
        directory = current.parent();
    }
    base.join(path).to_string_lossy().into_owned()
}

fn json_number(value: &Value, field: &str) -> mlua::Result<f64> {
    value
        .as_f64()
        .ok_or_else(|| mlua::Error::RuntimeError(format!("{field} 要是数字，不是 {}", kind(value))))
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "布尔值",
        Value::Number(_) => "数字",
        Value::String(_) => "字符串",
        Value::Array(_) => "数组",
        Value::Object(_) => "对象",
    }
}

#[cfg(test)]
mod tests {
    use super::{content, validate_name};
    use mlua::Lua;
    use serde_json::json;

    /// 顶层 `controls` 变序列，`regions` / 控件里的 `events` 原样保留，同时收集名字。
    #[test]
    fn controls_and_events_become_lua_tables() {
        let lua = Lua::new();
        let value = json!({
            "width": 320,
            "height": 120,
            "controls": [
                { "name": "background", "type": "rect", "x": 0, "y": 0, "w": 320, "h": 40,
                  "color": "#FFFFFFFF" },
                { "name": "close", "type": "flat_svg_button", "x": 300, "y": 8,
                  "events": { "mouse_clicked": "close" } }
            ],
            "regions": [{ "name": "body", "x": 0, "y": 0, "w": 320, "h": 120 }]
        });
        let layout = content(&lua, &value, std::path::Path::new(".")).unwrap();
        let table = layout.table;
        assert_eq!(table.get::<f32>("width").unwrap(), 320.0);
        assert_eq!(table.get::<f32>("height").unwrap(), 120.0);
        assert_eq!(table.raw_len(), 2);
        let button: mlua::Table = table.get(2).unwrap();
        assert_eq!(button.get::<String>("type").unwrap(), "flat_svg_button");
        let events: mlua::Table = button.get("events").unwrap();
        assert_eq!(events.get::<String>("mouse_clicked").unwrap(), "close");
        let regions: mlua::Table = table.get("regions").unwrap();
        assert_eq!(regions.raw_len(), 1);
        assert_eq!(layout.control_names.len(), 2);
        assert_eq!(layout.control_names[1].name, "close");
        assert_eq!(layout.control_names[1].path, vec![1]);
    }

    /// 根直接写控件数组也行；缺 `controls` 的对象明确报错。
    #[test]
    fn a_control_array_can_be_the_root() {
        let lua = Lua::new();
        let value = json!([{ "name": "only", "type": "rect", "x": 0, "y": 0, "w": 10, "h": 10,
                             "color": "#FF000000" }]);
        let layout = content(&lua, &value, std::path::Path::new(".")).unwrap();
        assert_eq!(layout.table.raw_len(), 1);

        let error = content(&lua, &json!({ "width": 10 }), std::path::Path::new(".")).unwrap_err();
        assert!(error.to_string().contains("controls"), "{error}");
    }

    /// 每个控件都必须有合法名字；大小写不同算两个名字，重复 / 关键字 / 非法字符都拒绝。
    #[test]
    fn control_names_are_required_valid_and_unique() {
        for name in ["A1", "a1", "A_1", "abcDEF_09"] {
            assert_eq!(validate_name(name), Ok(()), "{name}");
        }
        for (name, expected) in [
            ("", "不能为空"),
            ("1abc", "英文字母开头"),
            ("_abc", "英文字母开头"),
            ("a-b", "只能包含"),
            ("中文", "英文字母开头"),
            ("fn", "Rust 关键字"),
            ("Self", "Rust 关键字"),
            ("async", "Rust 关键字"),
        ] {
            let error = validate_name(name).unwrap_err();
            assert!(error.contains(expected), "{name}: {error}");
        }
        assert!(validate_name(&"a".repeat(256)).is_err());
        assert!(validate_name(&"a".repeat(255)).is_ok());

        let lua = Lua::new();
        let case_sensitive = content(
            &lua,
            &json!({ "controls": [
                { "name": "A1", "type": "rect", "x": 0, "y": 0, "w": 1, "h": 1,
                  "color": "#FF000000" },
                { "name": "a1", "type": "rect", "x": 0, "y": 0, "w": 1, "h": 1,
                  "color": "#FF000000" }
            ] }),
            std::path::Path::new("."),
        )
        .unwrap();
        assert_eq!(case_sensitive.control_names.len(), 2);

        let missing = content(
            &lua,
            &json!({ "controls": [{ "type": "rect", "x": 0, "y": 0, "w": 1, "h": 1,
                                    "color": "#FF000000" }] }),
            std::path::Path::new("."),
        )
        .unwrap_err();
        assert!(missing.to_string().contains("缺 name"), "{missing}");

        let duplicate = content(
            &lua,
            &json!({ "controls": [
                { "name": "a1", "type": "rect", "x": 0, "y": 0, "w": 1, "h": 1,
                  "color": "#FF000000" },
                { "name": "a1", "type": "rect", "x": 0, "y": 0, "w": 1, "h": 1,
                  "color": "#FF000000" }
            ] }),
            std::path::Path::new("."),
        )
        .unwrap_err();
        assert!(duplicate.to_string().contains("控件名重复"), "{duplicate}");
    }

    /// 嵌套的 panel 子控件也要求名字，并收集完整路径。
    #[test]
    fn nested_children_are_named_and_indexed() {
        let lua = Lua::new();
        let layout = content(
            &lua,
            &json!({ "controls": [
                { "name": "panel", "type": "panel", "x": 0, "y": 0, "w": 10, "h": 10,
                  "children": [
                    { "name": "label", "type": "label", "x": 0, "y": 0,
                      "runs": [{ "text": "x", "color": "#FF000000" }] }
                  ] }
            ] }),
            std::path::Path::new("."),
        )
        .unwrap();
        assert_eq!(layout.control_names[0].path, vec![0]);
        assert_eq!(layout.control_names[1].name, "label");
        assert_eq!(layout.control_names[1].path, vec![0, 0]);
    }

    /// 随包测试布局本身能解析，且名字都收集到了。
    #[test]
    fn the_bundled_test_layout_parses() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../Scripts/lib/test-caret-window.json");
        let lua = Lua::new();
        let layout =
            super::read(&lua, &path).unwrap_or_else(|error| panic!("{}：{error}", path.display()));
        assert!(
            layout
                .control_names
                .iter()
                .any(|control| control.name == "search")
        );
        assert!(
            layout
                .control_names
                .iter()
                .any(|control| control.name == "close")
        );
    }

    /// 带 UTF-8 BOM 的布局文件（Windows 记事本常见）也能读。
    #[test]
    fn layout_files_with_a_utf8_bom_are_accepted() {
        let dir = std::env::temp_dir().join("cloudime-script-layout-bom");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ui.json");
        std::fs::write(
            &path,
            "\u{feff}{\"width\":10,\"height\":20,\"controls\":[]}",
        )
        .unwrap();
        let lua = Lua::new();
        let layout = super::read(&lua, &path).unwrap();
        assert_eq!(layout.table.get::<f32>("width").unwrap(), 10.0);
        assert_eq!(layout.table.raw_len(), 0);
    }
}
