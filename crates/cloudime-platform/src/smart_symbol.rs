//! 智能符号的自定义规则：`%APPDATA%\CloudIME\intelligent-symbol.json`。
//!
//! 双击空格（第二次空格）时，用这份文件里的规则按**光标前那一整句**匹配：命中哪条就上屏哪条的
//! `symbol`；一条都不命中走内置规则（`cloudime_core::smart_space_double`），最终兜底是句号。
//! 文件是一组对象：`symbol` 是要上屏的符号，`trigger` 是正则数组（任意一条命中就算，`$` 锚句尾）。

/// 文件名（放在用户目录 `%APPDATA%\CloudIME` 下）。
pub const SMART_SYMBOL_FILE: &str = "intelligent-symbol.json";

/// 缺省规则：安装时由安装包放到用户目录（`onlyifdoesntexist`，不覆盖用户改过的），
/// 设置页「修改配置」在文件缺失时也用它补一份。
///
/// 单一来源是仓库里的 `assets/defaults/intelligent-symbol.json`，这里只为打进二进制。
pub const SMART_SYMBOL_TEMPLATE: &str =
    include_str!("../../../assets/defaults/intelligent-symbol.json");
