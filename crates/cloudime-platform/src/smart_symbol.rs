//! 智能符号的自定义规则：`%APPDATA%\CloudIME\intelligent-symbol.json`。
//!
//! 双击空格（第二次空格）时，用这份文件里的规则按**光标前那一整句**匹配：命中哪条就上屏哪条的
//! `symbol`；一条都不命中走内置规则（`cloudime_core::smart_space_double`），最终兜底是句号。
//! 文件是一组对象：`symbol` 是要上屏的符号，`trigger` 是正则数组（任意一条命中就算，`$` 锚句尾）。

/// 文件名（放在用户目录 `%APPDATA%\CloudIME` 下）。
pub const SMART_SYMBOL_FILE: &str = "intelligent-symbol.json";

/// 用户还没建过这个文件时写进去的示例：与内置规则等价，照着改就行。
///
/// 只列「双击空格」的替换；单个空格（逗号）不受这份文件影响。
pub const SMART_SYMBOL_TEMPLATE: &str = r#"[
  { "symbol": "：", "trigger": ["说$", "說$"] },
  { "symbol": "？", "trigger": ["吗$", "嗎$", "呢$", "嘛$"] },
  {
    "symbol": "！",
    "trigger": ["啊$", "呀$", "哟$", "喲$", "唷$", "哇$", "哦$", "喔$", "噢$"]
  },
  { "symbol": "——", "trigger": ["呜$", "嗚$", "嘶$", "吱$", "嘎$"] }
]
"#;
