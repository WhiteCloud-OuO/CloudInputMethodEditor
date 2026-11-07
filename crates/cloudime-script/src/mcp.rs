//! `cloudime.mcp.*` 的结果转换：MCP 的 JSON 与 Lua 表互转。

use cloudime_mcp_client::ResponseBody;
use mlua::{Lua, Table};
use serde_json::Value;

/// 把代理返回的 MCP 结果拼成脚本回调拿到的表。
pub(crate) fn response_payload(
    lua: &Lua,
    server: &str,
    tool: &str,
    body: ResponseBody,
) -> mlua::Result<Table> {
    let payload = lua.create_table()?;
    payload.set("server", server)?;
    match body {
        ResponseBody::Servers { servers } => {
            let list = lua.create_table()?;
            for (index, summary) in servers.iter().enumerate() {
                list.set(index + 1, summary.name.clone())?;
            }
            payload.set("servers", list)?;
        }
        ResponseBody::Started {
            server,
            client_owned,
        } => {
            payload.set("server", server)?;
            payload.set("started", true)?;
            payload.set("client_owned", client_owned)?;
        }
        ResponseBody::Stopped { server } => {
            payload.set("server", server)?;
            payload.set("stopped", true)?;
        }
        ResponseBody::Shutdown => {
            payload.set("shutdown", true)?;
        }
        ResponseBody::Tools { tools } => {
            let list = lua.create_table()?;
            for (index, summary) in tools.iter().enumerate() {
                let item = lua.create_table()?;
                item.set("name", summary.name.clone())?;
                item.set("description", summary.description.clone())?;
                list.set(index + 1, item)?;
            }
            payload.set("tools", list)?;
        }
        ResponseBody::Call(outcome) => {
            payload.set("tool", tool)?;
            payload.set("content", json_to_lua(lua, &Value::Array(outcome.content))?)?;
            if let Some(structured) = outcome.structured {
                payload.set("structured", json_to_lua(lua, &structured)?)?;
            }
            payload.set("is_error", outcome.is_error)?;
            if let Some(error) = outcome.error {
                payload.set("error", error)?;
            }
        }
        ResponseBody::Error { error, .. } => {
            payload.set("tool", tool)?;
            payload.set("error", error)?;
        }
    }
    Ok(payload)
}

/// MCP 的 JSON 转成 Lua：对象 → 字符串键表，数组 → 从 1 开始的序列表，`null` → `nil`。
pub(crate) fn json_to_lua(lua: &Lua, value: &Value) -> mlua::Result<mlua::Value> {
    Ok(match value {
        Value::Null => mlua::Value::Nil,
        Value::Bool(value) => mlua::Value::Boolean(*value),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => mlua::Value::Integer(integer),
            None => mlua::Value::Number(number.as_f64().unwrap_or_default()),
        },
        Value::String(text) => mlua::Value::String(lua.create_string(text)?),
        Value::Array(items) => {
            let table = lua.create_table()?;
            for (index, item) in items.iter().enumerate() {
                table.set(index + 1, json_to_lua(lua, item)?)?;
            }
            mlua::Value::Table(table)
        }
        Value::Object(entries) => {
            let table = lua.create_table()?;
            for (key, item) in entries {
                table.set(key.as_str(), json_to_lua(lua, item)?)?;
            }
            mlua::Value::Table(table)
        }
    })
}

/// Lua 表转成 MCP 的 JSON：序列表 → 数组，字符串键表 → 对象。
///
/// 空表按对象处理（MCP 工具参数基本都是对象）；数组下标必须从 1 连续，不混用字符串键。
pub(crate) fn lua_to_json(value: mlua::Value) -> mlua::Result<Value> {
    Ok(match value {
        mlua::Value::Nil => Value::Null,
        mlua::Value::Boolean(value) => Value::Bool(value),
        mlua::Value::Integer(value) => Value::from(value),
        mlua::Value::Number(value) => serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        mlua::Value::String(text) => Value::String(text.to_str()?.to_string()),
        mlua::Value::Table(table) => table_to_json(table)?,
        other => {
            return Err(mlua::Error::RuntimeError(format!(
                "MCP 参数里不能有 {}",
                other.type_name()
            )));
        }
    })
}

fn table_to_json(table: Table) -> mlua::Result<Value> {
    let mut array = std::collections::BTreeMap::<u64, Value>::new();
    let mut object = serde_json::Map::new();
    let mut count = 0u64;
    for pair in table.pairs::<mlua::Value, mlua::Value>() {
        let (key, value) = pair?;
        count += 1;
        match key {
            mlua::Value::Integer(index) if index >= 1 => {
                array.insert(index as u64, lua_to_json(value)?);
            }
            mlua::Value::String(key) => {
                object.insert(key.to_str()?.to_string(), lua_to_json(value)?);
            }
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "MCP 参数的表键只能是字符串或正整数，给的是 {}",
                    other.type_name()
                )));
            }
        }
    }
    if !array.is_empty() && !object.is_empty() {
        return Err(mlua::Error::RuntimeError(
            "MCP 参数的表不能混用数组下标和字符串键".to_owned(),
        ));
    }
    if !array.is_empty() {
        let contiguous = array.len() as u64 == count
            && array.keys().next() == Some(&1)
            && array.keys().last() == Some(&count);
        if !contiguous {
            return Err(mlua::Error::RuntimeError(
                "MCP 参数的数组下标必须从 1 连续".to_owned(),
            ));
        }
        return Ok(Value::Array(array.into_values().collect()));
    }
    Ok(Value::Object(object))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_nested_json_to_lua_and_back() {
        let lua = Lua::new();
        let source = json!({
            "text": "你好",
            "count": 2,
            "ok": true,
            "items": ["a", "b"],
            "nested": { "value": 1 }
        });
        let value = json_to_lua(&lua, &source).unwrap();
        let back = lua_to_json(value).unwrap();
        assert_eq!(back, source);
    }

    #[test]
    fn json_null_becomes_nil() {
        let lua = Lua::new();
        assert!(matches!(
            json_to_lua(&lua, &serde_json::Value::Null).unwrap(),
            mlua::Value::Nil
        ));
    }

    #[test]
    fn rejects_mixed_and_sparse_tables() {
        let lua = Lua::new();
        let mixed: mlua::Value = lua.load("{ [1] = 'a', name = 'b' }").eval().unwrap();
        assert!(lua_to_json(mixed).is_err());

        let sparse: mlua::Value = lua.load("{ [2] = 'b' }").eval().unwrap();
        assert!(lua_to_json(sparse).is_err());
    }

    #[test]
    fn builds_a_call_payload() {
        let lua = Lua::new();
        let payload = response_payload(
            &lua,
            "search",
            "find",
            ResponseBody::Call(cloudime_mcp_client::CallOutcome {
                content: vec![json!({ "type": "text", "text": "结果" })],
                structured: Some(json!({ "hits": 1 })),
                is_error: false,
                error: None,
            }),
        )
        .unwrap();
        assert_eq!(payload.get::<String>("server").unwrap(), "search");
        assert_eq!(payload.get::<String>("tool").unwrap(), "find");
        let content: Table = payload.get("content").unwrap();
        let first: Table = content.get(1).unwrap();
        assert_eq!(first.get::<String>("text").unwrap(), "结果");
        let structured: Table = payload.get("structured").unwrap();
        assert_eq!(structured.get::<i64>("hits").unwrap(), 1);
    }
}
