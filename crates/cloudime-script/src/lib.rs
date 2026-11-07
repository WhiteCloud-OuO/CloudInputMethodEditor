//! 脚本运行时：内嵌 LuaJIT（[mlua]），给「设置 → 脚本」里的用户脚本用。
//!
//! LuaJIT 的源码随 crate 一起编译（`mlua` 的 `vendored` 特性），装机包不依赖机器上装的 Lua ——
//! 与词库存档自带 SQLite（`rusqlite` 的 `bundled`）是同一个路子。`mlua` 只在本 crate 声明一次，
//! 别处用 [`mlua`] 这个再导出，后端与特性开关就只此一处。
//!
//! 脚本由 Server 加载与派发（[`Runtime`]）：脚本放安装目录的 `Scripts\<脚本名>\`（[`DIRECTORY`]），
//! 每个文件夹一个脚本、根部正好一个入口 `.lua`；加载时按文件夹名排序执行一遍，用
//! `cloudime.on(事件名, 处理函数)` 登记处理函数。
//! 用的是**完整的 Lua 标准库**（`io` / `os` / `package` 都在）：脚本就是用户自己写的本机程序，
//! 能读写文件、起进程，风险由用户自担。

mod http;
mod install;
mod layout_file;
mod mcp;
mod page_layout;
mod runtime;
mod window;

pub use cloud_ui_core as layout_core;
pub use mlua;
pub use runtime::{
    AppHook, ClipboardGetHook, ClipboardSetHook, ConfigHook, DIRECTORY, KeyboardHook, MeasureFont,
    MessageDialogHook, Runtime, ScriptInfo, SizeRequest, TEMPLATE_FILE, TextHook,
    TextInputDialogHook, TextRange, ThemeHook, TriggerCondition, combination_name,
    is_reserved_combination,
};
pub use window::{
    AuxiliarySpec, DialogSpec, DialogStyle, MessageDialogRequest, Request as WindowRequest,
    ScriptWindowEvent, TextInputDialogRequest, WindowAction, WindowId, WindowSpec,
    event_payload as window_event_payload,
};

#[cfg(test)]
mod tests {
    use super::mlua;

    /// 运行时起得来，而且是 LuaJIT：`_VERSION` 报 5.1 兼容版，另有一张 `jit` 表。
    #[test]
    fn the_runtime_is_luajit() {
        let lua = mlua::Lua::new();
        let version: String = lua.load("_VERSION").eval().unwrap();
        assert_eq!(version, "Lua 5.1");
        let jit: String = lua.load("jit.version").eval().unwrap();
        assert!(jit.starts_with("LuaJIT"), "{jit}");
    }
}
