//! 控件布局文件（JSON）：把前台 UI 文件读成脚本窗口的声明式内容表。
//!
//! 布局文件只描述控件，真正的默认值 / 几何 / 区域生成仍走 [`crate::window`] 的同一套解析器；
//! 这样 JSON 与 Lua `set_content` 的元素字段完全一致，只多了顶层 `controls` 数组。
//! 文件里的路径（`svg_path` / `img.path`）与 Lua 一样按 Server 工作目录（安装目录）解析。

use std::path::Path;

use mlua::{Lua, Table, Value as LuaValue};
use serde_json::Value;

/// 布局文件最大字节数：控件描述是文本，4 MB 已经足够复杂面板。
const MAX_LAYOUT_BYTES: u64 = 4 * 1024 * 1024;

/// 读一个布局文件，转成 `set_content` 能直接消费的 Lua 表。
pub(crate) fn read(lua: &Lua, path: &Path) -> mlua::Result<Table> {
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
    content_table(lua, &value)
        .map_err(|error| mlua::Error::RuntimeError(format!("布局文件 {}：{error}", path.display())))
}

/// JSON 根 → 内容表：`{ width, height, regions, <controls 数组> }`。
///
/// 根可以是对象，也可以是直接写控件数组；对象里 `controls` 与 `elements` 等价。
fn content_table(lua: &Lua, value: &Value) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    match value {
        Value::Array(controls) => append_controls(lua, &table, controls)?,
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
            append_controls(lua, &table, controls)?;
            if let Some(regions) = root.get("regions").filter(|value| !value.is_null()) {
                let Value::Array(_) = regions else {
                    return Err(mlua::Error::RuntimeError("regions 必须是数组".to_owned()));
                };
                table.raw_set("regions", to_lua(lua, regions)?)?;
            }
        }
        _ => {
            return Err(mlua::Error::RuntimeError(
                "布局根要是一个对象（width / height / controls）或控件数组".to_owned(),
            ));
        }
    }
    Ok(table)
}

/// 控件数组 → 内容表的 1 起序列；每个控件必须是对象。
fn append_controls(lua: &Lua, table: &Table, controls: &[Value]) -> mlua::Result<()> {
    for (index, control) in controls.iter().enumerate() {
        if !control.is_object() {
            return Err(mlua::Error::RuntimeError(format!(
                "controls 第 {} 项不是对象",
                index + 1
            )));
        }
        table.raw_set(index as i64 + 1, to_lua(lua, control)?)?;
    }
    Ok(())
}

/// JSON 值 → Lua 值：数组变 1 起序列，对象变字符串键表，`null` 变 `nil`。
fn to_lua(lua: &Lua, value: &Value) -> mlua::Result<LuaValue> {
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
                table.raw_set(index as i64 + 1, to_lua(lua, item)?)?;
            }
            LuaValue::Table(table)
        }
        Value::Object(fields) => {
            let table = lua.create_table()?;
            for (key, field) in fields {
                table.raw_set(key.as_str(), to_lua(lua, field)?)?;
            }
            LuaValue::Table(table)
        }
    })
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
    use super::content_table;
    use mlua::Lua;
    use serde_json::json;

    /// 顶层 `controls` 变序列，`regions` / 控件里的 `events` 原样保留。
    #[test]
    fn controls_and_events_become_lua_tables() {
        let lua = Lua::new();
        let value = json!({
            "width": 320,
            "height": 120,
            "controls": [
                { "type": "rect", "x": 0, "y": 0, "w": 320, "h": 40, "color": "#FFFFFFFF" },
                { "type": "flat_svg_button", "name": "close", "x": 300, "y": 8,
                  "events": { "mouse_clicked": "close" } }
            ],
            "regions": [{ "name": "body", "x": 0, "y": 0, "w": 320, "h": 120 }]
        });
        let table = content_table(&lua, &value).unwrap();
        assert_eq!(table.get::<f32>("width").unwrap(), 320.0);
        assert_eq!(table.get::<f32>("height").unwrap(), 120.0);
        assert_eq!(table.raw_len(), 2);
        let button: mlua::Table = table.get(2).unwrap();
        assert_eq!(button.get::<String>("type").unwrap(), "flat_svg_button");
        let events: mlua::Table = button.get("events").unwrap();
        assert_eq!(events.get::<String>("mouse_clicked").unwrap(), "close");
        let regions: mlua::Table = table.get("regions").unwrap();
        assert_eq!(regions.raw_len(), 1);
    }

    /// 根直接写控件数组也行；缺 `controls` 的对象明确报错。
    #[test]
    fn a_control_array_can_be_the_root() {
        let lua = Lua::new();
        let value = json!([{ "type": "rect", "x": 0, "y": 0, "w": 10, "h": 10,
                             "color": "#FF000000" }]);
        let table = content_table(&lua, &value).unwrap();
        assert_eq!(table.raw_len(), 1);

        let error = content_table(&lua, &json!({ "width": 10 })).unwrap_err();
        assert!(error.to_string().contains("controls"), "{error}");
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
        let table = super::read(&lua, &path).unwrap();
        assert_eq!(table.get::<f32>("width").unwrap(), 10.0);
        assert_eq!(table.raw_len(), 0);
    }
}
