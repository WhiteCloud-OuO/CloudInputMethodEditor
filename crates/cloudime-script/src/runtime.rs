//! 脚本的加载与事件派发。
//!
//! 脚本文件放数据目录的 `scripts\` 下（[`DIRECTORY`]），按文件名排序依次执行一遍。**每个脚本必须
//! 最先**调一次 `cloudime.script{…}` 声明清单（字段语义见 `docs/design/script.md`）：缺清单、字段
//! 缺失 / 类型不对、`api` 不认识、`sync` 与 `handover` 对不上 —— 那个脚本**判无效**（跳过 + 日志写明原因）。
//!
//! 给脚本的宿主 API 是全局表 `cloudime`（**锁住**：只读，写一律报错）：
//!
//! - `cloudime.script{…}`：清单，必须最先、只一次。
//! - `cloudime.on(事件名, function(载荷) … end)`：登记某类事件的处理函数，可以登记多个。
//! - `cloudime.log(文本)`：写进 Server 的日志（`%LOCALAPPDATA%\CloudIME\logs\server.*.log`）。
//! - `cloudime.context`：光标前文 —— 应用里已经输入、不在候选窗口里的那段文本（派发前刷新）。
//! - `cloudime.http_get(url, timeout_ms, callback)`：异步发一次 GET。**响应时间限制必给**，
//!   结果在派发线程下一次 [`Runtime::poll_requests`] 时交给 `callback(结果, 当前这一屏候选)`；
//!   `timeout_ms` 之后才回来的结果一律丢掉。请求在自己的线程里跑，不阻塞输入。
//! - `cloudime.http_post(url, body, options, callback)`：异步发一次 POST，`options` 是
//!   `{ timeout_ms = …, headers = { ["Content-Type"] = "application/json" } }`（表头可选）。
//!   结果表与 `http_get` 完全一样。
//! - `cloudime.candidate.redraw()`：请 Server 把当前这一屏**重算一遍再重画**
//!   （会重新派发 `candidates` 事件）—— 脚本在异步回调里改了自己的状态之后用它。
//! - `cloudime.run(命令)`：启动一个外部程序（**启动就返回、不等它**）；`os.execute` 转发到它。
//!
//! 事件名与载荷由派发方（Server）定，本 crate 不认识它们 —— 加一种事件只要在 Server 那边多派发一次。
//!
//! **脚本出错不影响输入法**：加载期出错 = 整个脚本无效；运行期出错 / 超预算 / 超时 = 只中止这一次
//!（先调脚本清单里的 `on_error`，再跳过），按键照旧走引擎。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mlua::{Function, Lua, Table};

use crate::http;
use crate::install;
use crate::window;

/// 脚本目录名：**安装目录**（与 `Phrases\`、`WordBank\` 同级）下的这个子目录。
pub const DIRECTORY: &str = "Scripts";

/// 「新建脚本」用的模板：放在脚本目录里，但**不是**真脚本 —— 设置页不列它、加载器不执行它。
pub const TEMPLATE_FILE: &str = "template.lua";

/// 脚本面向的 API 版本：清单里的 `api` 必须是它，否则那个脚本判无效。
const API_VERSION: u32 = 2;

/// 一次「连着跑」允许用掉多少条指令的**全局上限**，超过就中止这一次。
///
/// 1 亿条在 LuaJIT 的解释器里约 **1–2 秒**：正常脚本远远用不到（一次按键的脚本工作通常几千条），
/// 写错的死循环最多卡这么久就自己回来。卡住的是**整条工人线程 —— 所有应用都会打不了字**，
/// 所以这道闸必须有；宁可 1–2 秒后回来，也不能无限卡住（见 `docs/design/script.md`）。
/// 脚本清单里的 `budget` 只能把它**收窄**。
const INSTRUCTION_BUDGET: u32 = 100_000_000;

/// 预算钩子每这么多条指令醒一次（钩子本身有开销，不能设得太密）。
const INSTRUCTION_STEP: u32 = 10_000;

/// 加载一个脚本文件时允许的墙钟上限（那会儿脚本的清单还没读出来，先用它兜住）。
const LOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// 清单里 `timeout` 允许的最大值（毫秒）：再大也不该让一次调用占住工人线程那么久。
const MAX_TIMEOUT_MS: u64 = 60_000;

/// 宿主 API 的表名。
const HOST_TABLE: &str = "cloudime";

/// 控制权怎么交回 Server（清单的 `handover`）：必须与 `sync` 一致，对不上判无效。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handover {
    /// 同步：处理函数算完返回就交回（配 `sync = true`）。
    Return,

    /// 异步：结果在 `cloudime.http_get` / `cloudime.http_post` 的回调里给，
    /// 回调返回时交回（配 `sync = false`）。
    Callback,
}

/// 清单里的 `trigger_condition`：这个脚本**由什么触发**；Server 只按它派对应的事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerCondition {
    /// **组合键**（`key` 事件）：清单声明修饰键（`combination_modifiers`，可写 `"none"`）+
    /// 具体键（`keys`）。不论候选窗口开没开都收 —— DLL 走 TSF 的按键路径把它们送上来
    /// （命中清单的修饰键、或命中了声明的键）。脚本不吃就由 DLL 重放回应用。
    CombinationKey,

    /// **时间段**（`time` 事件）：本地时间落在清单 `time_range` 里；**每个时间段只触发一次**
    ///（跨夜的时间段按「昨晚 20:00 到今天 6:00」算同一个窗口）。
    TimeRange,

    /// **候选窗口里的拼音**（`candidates` 事件）：候选窗口出现、且拼音串正好等于清单的
    /// `candidate_pinyin`（比较时忽略分隔符与大小写）。
    CandidateContext,

    /// **编辑光标所在的程序**（`focus` 事件）：焦点切进清单 `apps` 里的某个程序时派一次。
    FocusApp,

    /// **手动触发**（预留给「i 模式」）：永不自动派发；脚本照样加载、登记处理函数。
    Manual,
}

/// 清单 `time_range` 的一个时间段（本地时间的分钟数 0–1439；跨夜时 `from > to`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimeRange {
    from: u16,
    to: u16,
}

impl TimeRange {
    /// 这个分钟在不在时间段里（左闭右开）。
    fn contains(&self, minute: u16) -> bool {
        if self.from < self.to {
            self.from <= minute && minute < self.to
        } else {
            // 跨夜：20:00–06:00 是 [20:00, 24:00) ∪ [00:00, 06:00)
            minute >= self.from || minute < self.to
        }
    }

    /// 当前这一刻属于哪个「时间段窗口」：返回窗口开始那天的 `(年, 月, 日)`。
    /// `None` = 不在时间段里。跨夜时凌晨那半段算**昨天**开的窗口。
    fn window_start(&self, date: jiff::civil::Date, minute: u16) -> Option<(i16, i8, i8)> {
        if !self.contains(minute) {
            return None;
        }
        let start = if minute < self.from {
            date.yesterday().ok()?
        } else {
            date
        };
        Some((start.year(), start.month(), start.day()))
    }
}

/// 清单 `keys` 认的键名 → 虚拟键码。**不产生字符的键**（Enter / Tab / 方向键 / F1–F12）列在
/// [`KEY_NAMES`] 里；单个字母 / 数字 / 标点由 [`parse_key_name`] 现算。
const KEY_NAMES: [(u32, &str); 26] = [
    (0x0D, "enter"),
    (0x09, "tab"),
    (0x20, "space"),
    (0x08, "backspace"),
    (0x2E, "delete"),
    (0x1B, "esc"),
    (0x25, "left"),
    (0x26, "up"),
    (0x27, "right"),
    (0x28, "down"),
    (0x24, "home"),
    (0x23, "end"),
    (0x21, "pageup"),
    (0x22, "pagedown"),
    (0x70, "f1"),
    (0x71, "f2"),
    (0x72, "f3"),
    (0x73, "f4"),
    (0x74, "f5"),
    (0x75, "f6"),
    (0x76, "f7"),
    (0x77, "f8"),
    (0x78, "f9"),
    (0x79, "f10"),
    (0x7A, "f11"),
    (0x7B, "f12"),
];

/// 键名 → 虚拟键码：先查 [`KEY_NAMES`]，再认单个字母 / 数字 / 标点。
fn parse_key_name(name: &str) -> Option<u32> {
    let name = name.trim();
    if let Some((vk, _)) = KEY_NAMES
        .iter()
        .find(|(_, known)| known.eq_ignore_ascii_case(name))
    {
        return Some(*vk);
    }
    let mut chars = name.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    match c {
        'a'..='z' => Some(c.to_ascii_uppercase() as u32),
        'A'..='Z' => Some(c as u32),
        '0'..='9' => Some(c as u32),
        '`' => Some(0xC0),
        '-' => Some(0xBD),
        '=' => Some(0xBB),
        '[' => Some(0xDB),
        ']' => Some(0xDD),
        '\\' => Some(0xDC),
        ';' => Some(0xBA),
        '\'' => Some(0xDE),
        ',' => Some(0xBC),
        '.' => Some(0xBE),
        '/' => Some(0xBF),
        _ => None,
    }
}

/// 键名的说明（报错时用）。
const KEY_NAME_HELP: &str = "enter / tab / space / backspace / delete / esc / left / up / right / down / home / end / pageup / pagedown / f1..f12，或单个字母、数字、标点（a–z、0–9、` - = [ ] \\ ; ' , . /）";

/// 清单里 `combination_modifiers` 认的那几套修饰键（位图：bit0 ctrl / bit1 alt / bit2 shift / bit3 win）。
/// `"none"` = 不按修饰键（单键，走 DLL 的 `script_keys` 那条路）。
/// 别的组合（单独的 `shift` / `win`、以及没列到的多键组合）一概不接受。
const COMBINATION_MODIFIERS: [(u8, &str); 11] = [
    (0b0000, "none"),
    (0b0001, "ctrl"),
    (0b0011, "ctrl+alt"),
    (0b0101, "ctrl+shift"),
    (0b0110, "alt+shift"),
    (0b0010, "alt"),
    (0b1001, "win+ctrl"),
    (0b1010, "win+alt"),
    (0b1100, "win+shift"),
    (0b1011, "win+alt+ctrl"),
    (0b1101, "win+ctrl+shift"),
];

/// 解析清单里的 `combination_modifiers`（`"ctrl"` / `"ctrl+alt"` / …；`+` 连接的顺序随意，
/// 大小写不敏感）。不在 [`COMBINATION_MODIFIERS`] 里的一律报错。
fn parse_combination_modifiers(text: &str) -> mlua::Result<u8> {
    let invalid = || {
        mlua::Error::RuntimeError(format!(
            "清单的 combination_modifiers = \"{text}\" 不认识（只有这几种：{}）",
            COMBINATION_MODIFIERS
                .iter()
                .map(|(_, name)| *name)
                .collect::<Vec<_>>()
                .join(" / ")
        ))
    };
    // `"none"`（不按修饰键）单独一种写法：不能与别的修饰键混着写
    if text.trim().eq_ignore_ascii_case("none") {
        return Ok(0);
    }
    let mut mask = 0u8;
    for part in text.split('+') {
        let bit = match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" => 0b0001,
            "alt" => 0b0010,
            "shift" => 0b0100,
            "win" => 0b1000,
            _ => return Err(invalid()),
        };
        if mask & bit != 0 {
            return Err(invalid());
        }
        mask |= bit;
    }
    if !COMBINATION_MODIFIERS
        .iter()
        .any(|(allowed, _)| *allowed == mask)
    {
        return Err(invalid());
    }
    Ok(mask)
}

/// 输入法自己占用的组合键（2026-10-17）：**清单里声明这些组合的脚本判无效，Server 也不派发给脚本**。
///
/// 分三类：
/// - 现有的输入法操作：`Ctrl+Enter`（原样上屏）、`Ctrl+反引号`（翻译 Tip）/ `Shift+反引号`（发音）、
///   `Ctrl+数字`（主键盘 / 小键盘，杀词 / 选词）、`Ctrl+Alt+Space`（中英切换的可选组合）、
///   `Ctrl+Alt+,` / `Ctrl+Alt+.`（简繁与标点，TSF 保留键）；
/// - **预留给非全拼模式**：`Ctrl+Alt+V` / `Ctrl+Alt+U` / `Ctrl+Alt+I`（v/u/i 模式，2026-10-17 起）；
/// - 按「精确匹配」：`Ctrl+Alt+V/U/I` 只要 Ctrl+Alt 按着（带不带 Shift 都算），别的组合键按清单那一套。
///
/// `modifiers` 是物理修饰键位图（bit0 ctrl / bit1 alt / bit2 shift / bit3 win），与清单同一位数。
pub fn is_reserved_combination(modifiers: u8, virtual_key: u32) -> bool {
    const CTRL: u8 = 0b0001;
    const ALT: u8 = 0b0010;
    const SHIFT: u8 = 0b0100;
    const WIN: u8 = 0b1000;

    // 现有的输入法操作 ------------------------------------------------------
    if modifiers & CTRL != 0 && modifiers & (ALT | WIN) == 0 {
        // Ctrl + Enter / 反引号；Ctrl + 主键盘 / 小键盘数字
        if matches!(virtual_key, 0x0D | 0xC0) {
            return true;
        }
        if (0x30..=0x39).contains(&virtual_key) || (0x60..=0x69).contains(&virtual_key) {
            return true;
        }
    }
    // Shift + 反引号：发音（不带 Ctrl / Alt / Win）
    if modifiers == SHIFT && virtual_key == 0xC0 {
        return true;
    }
    // 输入法自己的开关 / TSF 保留键
    // 中英切换（可选组合）、简繁（Ctrl+Alt+,）、中英标点（Ctrl+Alt+.）
    if modifiers == CTRL | ALT && matches!(virtual_key, 0x20 | 0xBC | 0xBE) {
        return true;
    }
    // 预留给非全拼模式：v / u / i 模式 --------------------------------
    // Ctrl+Alt 按着就保留（再带 Shift 也不给脚本）—— 免得以后模式改了才发现撞键。
    if modifiers & (CTRL | ALT) == CTRL | ALT
        && modifiers & WIN == 0
        && matches!(virtual_key, 0x56 | 0x55 | 0x49)
    {
        return true;
    }
    false
}

/// 组合键在报错 / 文档里的写法（`ctrl+alt+v`）。
pub fn combination_name(modifiers: u8, virtual_key: u32) -> String {
    let mask_name = COMBINATION_MODIFIERS
        .iter()
        .find(|(mask, _)| *mask == modifiers)
        .map(|(_, name)| (*name).to_owned())
        .unwrap_or_else(|| format!("mask={modifiers:#06b}"));
    let key = key_display_name(virtual_key);
    if modifiers == 0 {
        key
    } else {
        format!("{mask_name}+{key}")
    }
}

/// 虚拟键码在报错里怎么说（字母 / 数字 / 标点 / 功能键）。
fn key_display_name(virtual_key: u32) -> String {
    if let Some((_, name)) = KEY_NAMES.iter().find(|(code, _)| *code == virtual_key) {
        return (*name).to_owned();
    }
    match virtual_key {
        0x41..=0x5A => ((virtual_key as u8 as char).to_ascii_lowercase()).to_string(),
        0x30..=0x39 => (virtual_key as u8 as char).to_string(),
        0xBA => ";".to_owned(),
        0xBB => "=".to_owned(),
        0xBC => ",".to_owned(),
        0xBD => "-".to_owned(),
        0xBE => ".".to_owned(),
        0xBF => "/".to_owned(),
        0xC0 => "`".to_owned(),
        0xDB => "[".to_owned(),
        0xDC => "\\".to_owned(),
        0xDD => "]".to_owned(),
        0xDE => "'".to_owned(),
        other => format!("vk={other}"),
    }
}

/// 解析清单的 `keys`（键名数组）→ 虚拟键码（去重；至少一个）。
///
/// 命中输入法自己占用的组合（见 [`is_reserved_combination`]）直接报错 —— 这个脚本判无效。
fn parse_keys(options: &Table, modifiers: u8) -> mlua::Result<Vec<u32>> {
    let names = options.get::<Option<Table>>("keys")?.ok_or_else(|| {
        mlua::Error::RuntimeError(
            "trigger_condition = \"combination_key\" 时清单必须写 keys（键名数组，如 { \"w\" }；可用键名见 lua.md）"
                .to_owned(),
        )
    })?;
    let mut keys = Vec::new();
    for name in names.sequence_values::<String>() {
        let name = name?;
        let key = parse_key_name(&name).ok_or_else(|| {
            mlua::Error::RuntimeError(format!(
                "清单 keys 里的 \"{name}\" 不认识（可用：{KEY_NAME_HELP}）"
            ))
        })?;
        if is_reserved_combination(modifiers, key) {
            return Err(mlua::Error::RuntimeError(format!(
                "清单 keys 里的 \"{name}\"（{}）是输入法自己占用的组合键，脚本不能绑定 \
                 （Ctrl+Alt+V/U/I 预留给 v/u/i 模式；现有操作见 lua.md「触发条件」）",
                combination_name(modifiers, key)
            )));
        }
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if keys.is_empty() {
        return Err(mlua::Error::RuntimeError(
            "清单的 keys 是空的：至少写一个键".to_owned(),
        ));
    }
    Ok(keys)
}

/// 解析 `time_range = { from = "20:00", to = "06:00" }` → 分钟数（跨夜时 `from > to`）。
fn parse_time_range(range: &Table) -> mlua::Result<TimeRange> {
    let clock = |key: &str| -> mlua::Result<u16> {
        let text = range.get::<Option<String>>(key)?.ok_or_else(|| {
            mlua::Error::RuntimeError(format!("time_range 缺 {key}（要 \"HH:MM\"，本地时间）"))
        })?;
        let (hour, minute) = parse_clock(&text)?;
        Ok(u16::try_from(hour).unwrap_or(0) * 60 + u16::try_from(minute).unwrap_or(0))
    };
    let from = clock("from")?;
    let to = clock("to")?;
    if from == to {
        return Err(mlua::Error::RuntimeError(
            "time_range 的 from 与 to 不能相同".to_owned(),
        ));
    }
    Ok(TimeRange { from, to })
}

/// 一个脚本的清单（`cloudime.script{…}`）：每个脚本必须**最先**声明一次。
struct Manifest {
    /// 日志里点名用；缺省 = 文件名。
    name: String,

    /// 设置页列表里的一句话介绍；缺省 `None`（那边取文件名）。
    description: Option<String>,

    /// 单次调用的指令数上限；`None` = 用全局缺省。只能收窄（钳到 [`INSTRUCTION_BUDGET`]）。
    budget: Option<u32>,

    /// 单次调用的墙钟上限。
    timeout: Duration,

    /// 只在哪些应用里跑（exe 文件名，大小写不敏感）；空 = 所有应用。
    apps: Vec<String>,

    /// 出错回调；`None` = 脚本写的是 `false`（不处理，运行时直接中止这一次）。
    on_error: Option<Function>,

    /// 合并顺序：大的排在后面，于是盖住前面的（同值按登记顺序）。
    priority: i32,

    /// 由什么触发（清单**必填**）：只按它收对应的事件。
    trigger: TriggerCondition,

    /// `time_range` 的时间段；其余触发条件是 `None`。
    time_range: Option<TimeRange>,

    /// 这个时间段窗口（开始那天的年月日）已经触发过没有：每个窗口只派一次。
    time_fired: Cell<Option<(i16, i8, i8)>>,

    /// `combination_key` 要的修饰键位图（`0` = 不按修饰键，单键）；其余触发条件是 `0`。
    combination_modifiers: u8,

    /// `combination_key` 要的那些键（虚拟键码）；其余触发条件是空。
    wanted_keys: Vec<u32>,

    /// `candidate_context` 要比对的拼音串（已规范化：去掉分隔符 / 空白、转小写）。
    candidate_pinyin: Option<String>,

    /// 要不要每次派发前刷新 `cloudime.context`（按键路径上 DLL 会多读一次光标前文）。
    needs_context: bool,
}

impl Manifest {
    /// 这个应用跑不跑这个脚本（`apps` 空 = 所有应用都跑）。
    fn runs_in(&self, app: &str) -> bool {
        self.apps.is_empty()
            || self
                .apps
                .iter()
                .any(|name| name.trim().eq_ignore_ascii_case(app.trim()))
    }

    /// 这个事件该不该派给这个脚本。
    ///
    /// `key` 是 `(虚拟键码, 修饰键位图)`、`pinyin` 是这一次组句的拼音串（已规范化）——
    /// 只有当前触发条件对得上的脚本才收。`startup` / `window` 谁都收（窗口是脚本自己建的）。
    fn accepts(&self, event: &str, key: Option<(u32, u8)>, pinyin: Option<&str>) -> bool {
        match event {
            "startup" | "window" => true,
            "key" => key.is_some_and(|(vk, mask)| {
                self.trigger == TriggerCondition::CombinationKey
                    && mask == self.combination_modifiers
                    && self.wanted_keys.contains(&vk)
            }),
            "candidates" => {
                self.trigger == TriggerCondition::CandidateContext
                    && pinyin.is_some_and(|pinyin| self.candidate_pinyin.as_deref() == Some(pinyin))
            }
            "time" => {
                self.trigger == TriggerCondition::TimeRange
                    && current_minute().is_some_and(|minute| {
                        self.time_range.is_some_and(|range| range.contains(minute))
                    })
            }
            "focus" => self.trigger == TriggerCondition::FocusApp,
            _ => true,
        }
    }

    /// 到点了没有：本地时间落在这个脚本的时间段里、且这个窗口还没触发过。
    /// 触发过一次就记下窗口开始那天 —— 同一晚的 20:00–06:00 只派一次。
    fn mark_time_due(&self, now: &jiff::Zoned) -> bool {
        let Some(range) = self.time_range else {
            return false;
        };
        let minute = minute_of(now);
        let Some(start) = range.window_start(now.date(), minute) else {
            return false;
        };
        if self.time_fired.get() == Some(start) {
            return false;
        }
        self.time_fired.set(Some(start));
        true
    }

    /// 这一次调用用多少条指令（清单给了就收窄，不超全局上限）。
    fn instruction_budget(&self) -> u32 {
        self.budget
            .unwrap_or(INSTRUCTION_BUDGET)
            .min(INSTRUCTION_BUDGET)
    }
}

/// 一个已登记的处理函数：函数本体 + 它属于哪个脚本（清单里带着名字与限额）。
/// 按 `(priority, 登记顺序)` 派发 —— priority 大的在后面，于是合并时盖住前面的。
struct Handler {
    manifest: Rc<Manifest>,
    function: Function,

    /// 登记时的序号，用于同 priority 之间保持登记顺序。
    seq: u64,
}

/// 一次挂着的异步请求（`cloudime.http_get` 与 `cloudime.http_post` 共用这套管线）。
struct Pending {
    /// 请求地址（日志与结果表里用）。
    url: String,

    /// 脚本给的响应时间限制：超过它才回来的结果一律丢掉。
    deadline: Instant,

    /// 结果回来时调的处理函数。
    callback: Function,

    /// 发请求那个脚本的清单（回调的限额与 `on_error` 按它来）。
    manifest: Rc<Manifest>,
}

/// 量一段文字要用哪个字体（`cloudime.ui.measure` 的第二个参数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureFont {
    /// 拼音串那一行。
    Pinyin,

    /// 候选词（缺省）。
    Candidate,

    /// 候选项序号。
    ItemNumber,

    /// 翻译 Tip / 在线翻译那一行。
    Translate,
}

impl MeasureFont {
    /// 脚本里写的那几个名字。
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "pinyin" => Some(Self::Pinyin),
            "candidate" => Some(Self::Candidate),
            "item_number" => Some(Self::ItemNumber),
            "translate" => Some(Self::Translate),
            _ => None,
        }
    }

    /// 报错时列出来的合法名字。
    const NAMES: &'static str = "pinyin / candidate / item_number / translate";
}

/// `cloudime.ui.measure` 背后的量尺：由 Server 装（[`Runtime::set_measure`]），单位是**点**。
pub type MeasureHook = Rc<dyn Fn(&str, MeasureFont) -> Option<(f32, f32)>>;

/// `cloudime.get_theme()` 背后的主题：由 Server 装（[`Runtime::set_theme_hook`]），每次调用现读当前主题。
/// 表形如 `{ candidate = { … }, bar = { … }, tip = { … } }`，颜色是 `#AARRGGBB`。
pub type ThemeHook = Rc<dyn Fn(&Lua) -> mlua::Result<Table>>;

/// `cloudime.text.*` 要哪一段文本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRange {
    /// 整篇（超上限时留光标附近那一段）。
    All,

    /// 光标之前那一段。
    Before,

    /// 光标之后那一段。
    After,
}

/// `cloudime.text.*` 背后的取文本接口：由 Server 装（[`Runtime::set_text_hook`]）。
/// 返回 `(文本, 有没有被截过)`；**这一拍还拿不到就给 `None`** —— 脚本那边是 `nil`
/// （取文本要 DLL 读、走一个来回，第一次调用经常是 `nil`，下一次调用就有了）。
pub type TextHook = Rc<dyn Fn(TextRange, u64) -> Option<(String, bool)>>;

/// `cloudime.clipboard.settext` 背后的实现：由 Server 装（[`Runtime::set_clipboard`]）。
pub type ClipboardSetHook = Rc<dyn Fn(&str) -> Result<(), String>>;

/// `cloudime.clipboard.gettext` 背后的实现：由 Server 装（[`Runtime::set_clipboard`]）。
/// `Ok(None)` = 剪贴板里没有文本（图片 / 文件 / 空）。
pub type ClipboardGetHook = Rc<dyn Fn() -> Result<Option<String>, String>>;

/// `cloudime.keyboard.press` 背后的实现：由 Server 装（[`Runtime::set_keyboard`]）。
/// 参数是一组 `ctrl+alt+key` 形式的组合键；返回失败原因。
pub type KeyboardHook = Rc<dyn Fn(&str) -> Result<(), String>>;

/// `cloudime.get_curr_config` 背后的实现：由 Server 装（[`Runtime::set_config_hook`]），
/// 把当前配置拼成一张 Lua 表（键名与 `config.toml` 一致，另加 `theme.available`；见 `lua.md`）。
pub type ConfigHook = Rc<dyn Fn(&Lua) -> mlua::Result<Table>>;

/// `cloudime.foreground_app` 背后的实现：由 Server 装（[`Runtime::set_app_hook`]），
/// 返回当前聚焦的宿主程序 exe 名；没聚焦任何程序时给 `None`（脚本那边是 `nil`）。
pub type AppHook = Rc<dyn Fn() -> Option<String>>;

/// 脚本要的候选窗尺寸（`cloudime.candidate.set_*`）。
///
/// 每一项都是「这次有没有提 / 提了什么」：`None` = 没提，`Some(None)` = 恢复默认（配置 / 滚轮），
/// `Some(Some(v))` = 设成 `v`。Server 取一次就清（见 [`Runtime::take_size_request`]）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SizeRequest {
    /// 候选窗口的最小宽度（像素）：竖排、横排都生效。
    pub min_width: Option<Option<f32>>,

    /// 一页几个候选（5–9）。
    pub page_size: Option<Option<usize>>,

    /// 缩放倍数（`1.0` = 100%）。
    pub scale: Option<Option<f32>>,
}

impl SizeRequest {
    /// 这次一项都没提（Server 据此可以跳过整段）。
    pub fn is_empty(&self) -> bool {
        self.min_width.is_none() && self.page_size.is_none() && self.scale.is_none()
    }
}

/// 脚本能给的最小宽度上限（与设置页那一项的夹取一致）；再大就是没有意义的巨窗。
const MAX_SCRIPT_MIN_WIDTH: f32 = 2000.0;

/// 一页候选数的范围（与 `cloudime_platform` 的 `MIN/MAX_CANDIDATE_COUNT` 一致）。
const SCRIPT_PAGE_SIZE_RANGE: (usize, usize) = (5, 9);

/// 脚本能给的最小 / 最大缩放（与候选窗 `Ctrl + 滚轮` 的级数范围一致：`1.2^-3` … `1.2^6`）。
const SCRIPT_SCALE_RANGE: (f32, f32) = (0.578_703_7, 2.985_984);

/// 异步请求那套共享状态：挂着的请求、请求号、工作线程发回结果的通道。
struct Requests {
    /// 请求号 → 挂着的请求。
    pending: RefCell<HashMap<u64, Pending>>,

    /// 下一个请求号。
    next_request: Cell<u64>,

    /// 工作线程发回的结果；[`Runtime::poll_requests`] 收。
    results: RefCell<mpsc::Receiver<(u64, http::Outcome)>>,

    /// 上面那个通道的发送端（clone 进工作线程）。
    tx: mpsc::Sender<(u64, http::Outcome)>,
}

impl Requests {
    fn new() -> Self {
        let (tx, results) = mpsc::channel();
        Self {
            pending: RefCell::new(HashMap::new()),
            next_request: Cell::new(0),
            results: RefCell::new(results),
            tx,
        }
    }
}

/// 脚本运行时：一个 Lua 状态 + 从脚本目录里加载来的处理函数。
pub struct Runtime {
    /// Lua 状态：脚本的全局变量、`cloudime` 表都住在这里。
    lua: Lua,

    /// 事件名 → 处理函数。与 `cloudime.on` 的闭包共享。
    handlers: Rc<RefCell<HashMap<String, Vec<Handler>>>>,

    /// 正在执行 / 正在登记的那个脚本（`on` 记名、加载完取清单用）。
    current: Rc<RefCell<String>>,

    /// 正在加载的那个脚本的清单（`cloudime.script` 里设、`cloudime.on` 里要求已有；
    /// 派发时按「正在跑的那个脚本」重设 —— 脚本里的 `log` / `http_get` 靠它认脚本）。
    manifest: Rc<RefCell<Option<Rc<Manifest>>>>,

    /// 是不是在**加载期**（`cloudime.on` 只允许这会儿调；运行期再登记就报错）。
    loading: Rc<Cell<bool>>,

    /// 这一次调用的限额：`(已用指令, 指令上限, 墙钟截止)`。钩子读它，每次调用前重设。
    limits: Rc<Cell<(u32, u32, Instant)>>,

    /// 处理函数登记序号（同 priority 之间保持登记顺序）。
    seq: Rc<Cell<u64>>,

    /// 异步请求那套共享状态（`http_get` / `http_post`）。
    requests: Rc<Requests>,

    /// 脚本调过 `cloudime.candidate.redraw()`：请 Server 把当前这一屏重算一遍再重画。
    /// 就是一个「脏」标记 —— 派发/回调结束后 Server 取一次（取了就清）。
    redraw: Rc<Cell<bool>>,

    /// 脚本调过 `cloudime.caret()`：请 Server 让 DLL 量一次光标位置（下一次按键时）。
    /// 派发/回调结束后 Server 取一次（取了就清）。
    caret: Rc<Cell<bool>>,

    /// 安装目录（`Images\` / `Scripts\` 的上一级）：`cloudime.install_dir()` / `install_path()`
    /// 用它。不是正式安装（测试 / CLI）时是 `None`。
    install: Rc<install::InstallDir>,

    /// 脚本要的候选窗尺寸（`cloudime.candidate.set_*`）：派发/回调结束后 Server 取一次。
    size: Rc<RefCell<SizeRequest>>,

    /// 脚本要换的主题（`cloudime.apply_theme`）：`None` = 没提，`Some(None)` = 去掉这次设的主题，
    /// `Some(Some(名))` = 换到它。派发/回调结束后 Server 取一次。
    theme: Rc<RefCell<Option<Option<String>>>>,

    /// 候选窗最近一次画出来的内容区尺寸（点）；`cloudime.candidate.width()` 读它。
    viewport: Rc<RefCell<Option<(f32, f32)>>>,

    /// 量尺（`cloudime.ui.measure`）：Server 用 [`Runtime::set_measure`] 装进来。
    measure: Rc<RefCell<Option<MeasureHook>>>,

    /// 当前主题（`cloudime.get_theme()`）：Server 用 [`Runtime::set_theme_hook`] 装进来。
    theme_hook: Rc<RefCell<Option<ThemeHook>>>,

    /// 取输入框文本的接口（`cloudime.text.*`）：Server 用 [`Runtime::set_text_hook`] 装进来。
    text_hook: Rc<RefCell<Option<TextHook>>>,

    /// 剪贴板（`cloudime.clipboard.*`）：Server 用 [`Runtime::set_clipboard`] 装进来。
    clipboard_set: Rc<RefCell<Option<ClipboardSetHook>>>,
    clipboard_get: Rc<RefCell<Option<ClipboardGetHook>>>,
    /// 模拟组合键（`cloudime.keyboard.press`）；Server 用 Windows `SendInput` 实现。
    keyboard: Rc<RefCell<Option<KeyboardHook>>>,

    /// 当前配置（`cloudime.get_curr_config`）：Server 用 [`Runtime::set_config_hook`] 装进来。
    config_hook: Rc<RefCell<Option<ConfigHook>>>,

    /// 当前前台程序名（`cloudime.foreground_app`）：Server 用 [`Runtime::set_app_hook`] 装进来。
    app_hook: Rc<RefCell<Option<AppHook>>>,

    /// `cloudime` 表本体：脚本只拿到只读代理，运行时自己写 `context` 这类刷新值。
    host: Rc<RefCell<Option<Table>>>,

    /// 加载成功的脚本名（清单里的 `name`，按加载顺序）。
    scripts: Vec<String>,

    /// 各脚本清单里的 `trigger_condition`（与 `scripts` 一一对应）：Server 据此决定
    /// 「要不要让 DLL 在没有组句时也送 Ctrl 组合」「要不要跑定时检查」。
    triggers: Vec<TriggerCondition>,

    /// 各 `combination_key` 脚本要的修饰键位图（清单 `combination_modifiers`）。
    combination_modifiers: Vec<u8>,

    /// 各 `combination_key` 脚本声明的键（虚拟键码，清单 `keys`）。
    wanted_keys: Vec<u32>,

    /// 有没有脚本要求每次派发前刷新 `cloudime.context`（清单 `needs_context`）。
    needs_context: bool,

    /// 脚本要建的窗口 / 对窗口的操作（`cloudime.window.*`）：派发 / 回调结束后 Server 取一次。
    window_requests: Rc<RefCell<Vec<window::Request>>>,

    /// 窗口编号（脚本句柄上的 `id`；UI 线程按它认窗口）。
    window_seq: Rc<Cell<u32>>,

    /// 脚本窗口内容里的图像缓存（按路径判新旧）：同一张图不用每次 `set_content` 都重新解码。
    /// 脚本侧看到的编辑框文本缓存（UI 线程的 `text_changed` 事件刷新）。
    window_texts: window::TextCache,

    /// 脚本窗口要的图像缓存（按路径 + 修改时间；同一张图不会每次 `set_content` 都重新解码）。
    image_cache: Rc<RefCell<window::ImageCache>>,
}

impl Runtime {
    /// 建一个不读盘、也没有脚本的运行时：`RouterConfig::scripts_dir` 为空（测试、或以后「脚本开关
    /// 关掉」）时用它，Lua 状态照建，派发照样安全地什么都不做。
    pub fn none() -> Self {
        let runtime = Self {
            lua: Lua::new(),
            handlers: Rc::new(RefCell::new(HashMap::new())),
            current: Rc::new(RefCell::new(String::new())),
            manifest: Rc::new(RefCell::new(None)),
            loading: Rc::new(Cell::new(false)),
            limits: Rc::new(Cell::new((0, INSTRUCTION_BUDGET, Instant::now()))),
            seq: Rc::new(Cell::new(0)),
            requests: Rc::new(Requests::new()),
            redraw: Rc::new(Cell::new(false)),
            caret: Rc::new(Cell::new(false)),
            install: Rc::new(install::InstallDir::default()),
            size: Rc::new(RefCell::new(SizeRequest::default())),
            theme: Rc::new(RefCell::new(None)),
            viewport: Rc::new(RefCell::new(None)),
            measure: Rc::new(RefCell::new(None)),
            theme_hook: Rc::new(RefCell::new(None)),
            text_hook: Rc::new(RefCell::new(None)),
            clipboard_set: Rc::new(RefCell::new(None)),
            clipboard_get: Rc::new(RefCell::new(None)),
            keyboard: Rc::new(RefCell::new(None)),
            config_hook: Rc::new(RefCell::new(None)),
            app_hook: Rc::new(RefCell::new(None)),
            host: Rc::new(RefCell::new(None)),
            scripts: Vec::new(),
            triggers: Vec::new(),
            combination_modifiers: Vec::new(),
            wanted_keys: Vec::new(),
            needs_context: false,
            window_requests: Rc::new(RefCell::new(Vec::new())),
            window_seq: Rc::new(Cell::new(0)),
            image_cache: Rc::new(RefCell::new(window::ImageCache::default())),
            window_texts: Rc::new(RefCell::new(HashMap::new())),
        };
        if let Err(error) = install_host_api(&runtime) {
            // 装不上就少了 `cloudime` 表，脚本会报 nil：记一条，别让 Server 起不来。
            tracing::warn!(%error, "装脚本宿主 API 失败，脚本将没有 cloudime 表");
        }
        runtime
    }

    /// 按清单给这一次调用上紧限额（`limits` 就是钩子读的那一份）。
    fn arm(&self, manifest: &Manifest) {
        self.limits.set((
            0,
            manifest.instruction_budget(),
            Instant::now() + manifest.timeout,
        ));
    }

    /// 读 `dir` 下的 `*.lua`（按文件名排序）依次执行。目录不在 / 读不了就什么都不加载。
    ///
    /// 跳过 [`TEMPLATE_FILE`]（「新建脚本」的模板，不是真脚本）与 `disabled` 里列到的文件名
    ///（大小写不敏感；来自 `[script] disabled` —— 设置页的「启用 / 禁用」开关写它）。
    pub fn load(dir: &Path, disabled: &[String]) -> Self {
        let mut runtime = Self::none();
        // 脚本目录的上一级就是安装目录：加载期就能用 `cloudime.install_dir()` / `install_path()`
        //（`Scripts\` 与 `Images\` 同级）。相对路径 / 没有上一级时留 `None`。
        if let Some(root) = dir.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            runtime.set_install_dir(root.to_path_buf());
        }
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                let mut files: Vec<PathBuf> = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("lua"))
                    })
                    .collect();
                files.sort();
                for file in files {
                    if skipped(&file, disabled) {
                        tracing::debug!(file = %file.display(), "跳过（模板或已禁用）");
                        continue;
                    }
                    runtime.run(&file);
                }
            }
            Err(error) => {
                tracing::debug!(%error, dir = %dir.display(), "脚本目录读不了，这次不加载脚本");
            }
        }
        tracing::info!(
            dir = %dir.display(),
            scripts = runtime.scripts.len(),
            handlers = runtime.handler_count(),
            "脚本已加载"
        );
        runtime
    }

    /// 执行一个脚本文件。**加载期出错（含清单不合法）= 这个脚本无效**：记一条日志、继续下一个；
    /// 它注册过一半的处理函数也一并撤掉（不然「无效」的脚本还会被派发到）。
    fn run(&mut self, path: &Path) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                tracing::warn!(%error, script = %name, "脚本读不了，跳过");
                return;
            }
        };
        let first_new_seq = self.seq.get() + 1;
        *self.current.borrow_mut() = name.clone();
        *self.manifest.borrow_mut() = None;
        self.loading.set(true);
        // 清单还没读出来，先用全局上限与 `LOAD_TIMEOUT` 兜住加载期
        self.limits
            .set((0, INSTRUCTION_BUDGET, Instant::now() + LOAD_TIMEOUT));
        let executed = self
            .lua
            .load(&source)
            .set_name(path.to_string_lossy().as_ref())
            .exec();
        self.loading.set(false);
        self.current.borrow_mut().clear();
        let (executed_ok, manifest) = match executed {
            Ok(()) => (true, self.manifest.borrow_mut().take()),
            Err(error) => {
                tracing::warn!(%error, script = %name, "脚本执行出错，判无效，跳过");
                (false, None)
            }
        };
        let Some(manifest) = manifest else {
            if executed_ok {
                tracing::warn!(
                    script = %name,
                    "脚本没有在最前面声明清单（cloudime.script{{…}}），判无效，跳过"
                );
            }
            self.retract_handlers(first_new_seq);
            return;
        };
        tracing::info!(
            script = %manifest.name,
            description = manifest.description.as_deref().unwrap_or("-"),
            file = %name,
            "脚本已加载"
        );
        self.scripts.push(manifest.name.clone());
        self.triggers.push(manifest.trigger);
        self.needs_context |= manifest.needs_context;
        if manifest.trigger == TriggerCondition::CombinationKey {
            self.wanted_keys
                .extend(manifest.wanted_keys.iter().copied());
            self.combination_modifiers
                .push(manifest.combination_modifiers);
        }
    }

    /// 撤掉这次加载里登记的处理函数（序号 `>= first_new_seq` 的），别的脚本的原样留着。
    fn retract_handlers(&self, first_new_seq: u64) {
        let mut handlers = self.handlers.borrow_mut();
        for list in handlers.values_mut() {
            list.retain(|handler| handler.seq < first_new_seq);
        }
        handlers.retain(|_, list| !list.is_empty());
    }

    /// 把一类事件派给登记过的处理函数，`payload` 是它们收到的表。
    ///
    /// `app` 是这一刻的宿主应用名（exe 文件名）：给 `Some` 时**跳过清单里 `apps` 对不上的脚本**；
    /// 给 `None`（`startup`、HTTP 回调）就不按应用过滤。
    ///
    /// 返回处理函数们**返回的表**（按 `priority` + 登记顺序）：返回 `nil` 或别的东西的当没说话，跳过。
    /// 某个处理函数报错 / 超限额只中止这一次（先调它清单里的 `on_error`），其余照常。
    pub fn dispatch(&self, event: &str, payload: Table, app: Option<&str>) -> Vec<Table> {
        // UI 线程的编辑框文本变化先落进脚本侧缓存，`win:get_text` 不用等下一个来回。
        if event == "window" {
            let id = payload.get::<u32>("id").ok();
            let control = payload.get::<String>("control").ok();
            let text = payload.get::<String>("text").ok();
            if let (Some(id), Some(control), Some(text)) = (id, control, text) {
                self.window_texts.borrow_mut().insert((id, control), text);
            }
        }
        // `key`：组句里 Server 会派发**所有**按键，组合键脚本只该收它声明的那一组（修饰键 + 键）。
        let key = (event == "key")
            .then(|| {
                let vk = payload.get::<u32>("vk").ok()?;
                let mask = u8::from(payload.get::<bool>("ctrl").unwrap_or(false))
                    | (u8::from(payload.get::<bool>("alt").unwrap_or(false)) << 1)
                    | (u8::from(payload.get::<bool>("shift").unwrap_or(false)) << 2)
                    | (u8::from(payload.get::<bool>("win").unwrap_or(false)) << 3);
                Some((vk, mask))
            })
            .flatten();
        // `candidates`：只派给「拼音串正好等于清单里那一段」的脚本（比较前去掉分隔符 / 空白）。
        let pinyin = (event == "candidates")
            .then(|| payload.get::<String>("pinyin").ok())
            .flatten()
            .map(|pinyin| normalize_pinyin(&pinyin));
        // 先把要调的东西 clone 出来再放开借用：处理函数里再 `cloudime.on` 会重入 `handlers`。
        let handlers: Vec<(Rc<Manifest>, Function)> = self
            .handlers
            .borrow()
            .get(event)
            .map(|list| {
                list.iter()
                    .filter(|handler| app.is_none_or(|app| handler.manifest.runs_in(app)))
                    .filter(|handler| handler.manifest.accepts(event, key, pinyin.as_deref()))
                    .map(|handler| (handler.manifest.clone(), handler.function.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let mut responses = Vec::new();
        for (manifest, function) in &handlers {
            // 让脚本里的 `log` / `http_get` 认得出「现在跑的是哪个脚本」（限额与出错回调也按它）
            *self.current.borrow_mut() = manifest.name.clone();
            *self.manifest.borrow_mut() = Some(manifest.clone());
            // 这一次调用按这个脚本的清单上紧限额（budget / timeout）
            self.arm(manifest);
            match function.call::<mlua::Value>(payload.clone()) {
                Ok(mlua::Value::Table(table)) => responses.push(table),
                // 返回 nil / 数字 / 字符串…：当它没说话
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        %error,
                        event,
                        script = %manifest.name,
                        "脚本处理事件出错，中止这一次"
                    );
                    self.report_error(manifest, event, &error.to_string());
                }
            }
        }
        responses
    }

    /// 出错时先调脚本清单里的 `on_error(event, message)`（它自己也按同一个清单限额）；
    /// 脚本写的是 `false`（不处理）或回调又出错，就只记日志。
    fn report_error(&self, manifest: &Manifest, event: &str, message: &str) {
        let Some(on_error) = &manifest.on_error else {
            return;
        };
        self.arm(manifest);
        if let Err(error) = on_error.call::<()>((event, message)) {
            tracing::warn!(%error, script = %manifest.name, "脚本的 on_error 回调也出错，忽略");
        }
    }

    /// Lua 状态：派发方拿它拼事件载荷，以后要加宿主能力也从这里进。
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// 加载成功的脚本名（清单里的 `name`；日志 / 设置页显示用）。
    pub fn scripts(&self) -> &[String] {
        &self.scripts
    }

    /// 有没有脚本关心这类事件。没人登记时派发方连载荷都不用拼。
    pub fn has_handlers(&self, event: &str) -> bool {
        self.handlers
            .borrow()
            .get(event)
            .is_some_and(|list| !list.is_empty())
    }

    /// 取一次「脚本请求重画候选窗」的标记（`cloudime.candidate.redraw()`）：取了就清。
    /// 派发完按键、或收完异步回调之后由 Server 调；为真时它会把当前这一屏重算一遍再重画。
    pub fn take_redraw_request(&self) -> bool {
        self.redraw.replace(false)
    }

    /// 取一次「脚本要的候选窗尺寸」（`cloudime.candidate.set_min_width` / `set_page_size` / `set_scale`）：
    /// 取了就清。Server 据此在本组句内改候选窗的大小（组句结束它自己回配置值，不归这里管）。
    pub fn take_size_request(&self) -> SizeRequest {
        std::mem::take(&mut *self.size.borrow_mut())
    }

    /// 取一次「脚本要换的主题」（`cloudime.apply_theme`）：取了就清。
    /// `None` = 没提；`Some(None)` = 回配置里的主题；`Some(Some(名))` = 换到它。
    pub fn take_theme_request(&self) -> Option<Option<String>> {
        self.theme.borrow_mut().take()
    }

    /// 脚本要光标位置（`cloudime.caret()`）：取一次（取了就清），Server 在 `KeyResult.want_caret`
    /// 里请 DLL 量。
    pub fn take_caret_request(&self) -> bool {
        self.caret.replace(false)
    }

    /// 装安装目录（`cloudime.install_dir()` / `install_path()` 用）。Server 建 Router 时调一次。
    pub fn set_install_dir(&self, dir: PathBuf) {
        self.install.set(dir);
    }

    /// 取一次脚本对窗口的操作（`cloudime.window.*`），取了就清。
    ///
    /// 脚本在**工人线程**上产出这些请求，窗口在 **UI 线程**上建 / 改 / 收：Server 每次派发
    ///（按键、候选、`time`、HTTP 回调、窗口事件）结束后取一次，按顺序转给 UI 线程即可。
    pub fn take_window_requests(&self) -> Vec<window::Request> {
        std::mem::take(&mut *self.window_requests.borrow_mut())
    }

    /// 有没有攒着的窗口操作（`take_window_requests` 取走之前先看它 —— 别让「响应为空」把
    /// 只动窗口的那一拍提前返回掉）。
    pub fn has_window_requests(&self) -> bool {
        !self.window_requests.borrow().is_empty()
    }

    /// 有没有清单声明的触发条件是 `trigger` 的脚本：Server 据此决定要不要让 DLL 在没有组句时
    /// 也送 Ctrl 组合（`CombinationKey`）、要不要每次 tick 检查到点（`SysTime`）。
    pub fn uses_trigger(&self, trigger: TriggerCondition) -> bool {
        self.triggers.contains(&trigger)
    }

    /// 有没有脚本要「按键那一刻的光标前文」（清单 `needs_context = true`）：
    /// Server 据此让 DLL 在每个按键**之前**现读一份光标前文送上来（`cloudime.text.*` 读到的就是它）。
    /// 没有这类脚本时为 `false`，按键路径上一分钱不花。
    pub fn wants_surrounding_text(&self) -> bool {
        self.needs_context
    }

    /// 脚本声明的那些键（清单 `keys`）的并集（虚拟键码，去重升序）：DLL 据此在没组句时也把这些键
    /// 送来问一趟（脚本不吃就重放回应用）。组合键的键也在这里 —— 修饰键那一条由
    /// [`Self::combination_key_masks`] 另外送。
    pub fn wanted_keys(&self) -> Vec<u32> {
        let mut keys = self.wanted_keys.clone();
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    /// 组合键脚本们要的那几套修饰键（清单 `combination_modifiers` 的并集）：
    /// 返回值的第 N 位置位 = 「修饰键位图 N」这套组合键要送上来（bit0 ctrl / bit1 alt / bit2 shift / bit3 win）。
    /// `0` = 一个带修饰键的组合键脚本都没有，DLL 那边按键路径与以前逐字节一致。
    pub fn combination_key_masks(&self) -> u16 {
        self.combination_modifiers.iter().fold(0u16, |masks, mask| {
            if *mask == 0 {
                // `combination_modifiers = "none"`：单键走 `script_keys`，不占修饰键位
                masks
            } else {
                masks | (1u16 << *mask)
            }
        })
    }

    /// 有没有脚本在时间段里到点了（**每个时间段窗口每个脚本只算一次**）。
    /// 没有 `time_range` 脚本时永远 `false`（一秒都不花）。到点由 `dispatch("time", …)` 里的
    /// `Manifest::accepts` 逐脚本核对。
    pub fn time_due(&self) -> bool {
        if !self.uses_trigger(TriggerCondition::TimeRange) {
            return false;
        }
        let now = jiff::Zoned::now();
        let handlers = self.handlers.borrow();
        let Some(list) = handlers.get("time") else {
            return false;
        };
        let mut due = false;
        // 同一个脚本可能登记了多个 `time` 处理函数：清单只有一份，只判一次
        let mut seen: Vec<*const Manifest> = Vec::new();
        for handler in list {
            let manifest = Rc::as_ptr(&handler.manifest);
            if seen.contains(&manifest) {
                continue;
            }
            seen.push(manifest);
            if handler.manifest.mark_time_due(&now) {
                due = true;
            }
        }
        due
    }

    /// `time` 事件的载荷：与 `cloudime.get_time()` 同一张表，另加 `app`
    /// （这一刻聚焦的宿主程序 exe 名，没有则 `nil`）。
    pub fn time_payload(&self, app: Option<&str>) -> mlua::Result<Table> {
        let fields = time_table(&self.lua)?;
        fields.set("app", app)?;
        Ok(fields)
    }

    /// 装一个「当前配置」接口：脚本调 `cloudime.get_curr_config` 时用它拼一张表。
    /// 由 Server 在建好 Router 时与每次配置热加载后调（没装时脚本调它会报错）。
    pub fn set_config_hook(&self, hook: ConfigHook) {
        *self.config_hook.borrow_mut() = Some(hook);
    }

    /// 装一个「当前前台程序」接口：脚本调 `cloudime.foreground_app` 时用它问（`None` = 没聚焦任何程序）。
    /// 由 Server 在建好 Router 时调（没装时脚本调它会报错）。
    pub fn set_app_hook(&self, hook: AppHook) {
        *self.app_hook.borrow_mut() = Some(hook);
    }

    /// 装一个量尺：脚本调 `cloudime.ui.measure` / `measure_tip` 时用它量文字宽度（单位点）。
    /// 由 Server 在 [`Runtime::load`] / [`Runtime::none`] 之后调（拿不到渲染器时可以一直不装，
    /// 那时脚本调 measure 会报「量不了」）。
    pub fn set_measure(&self, hook: MeasureHook) {
        *self.measure.borrow_mut() = Some(hook);
    }

    /// 装「当前主题」（`cloudime.get_theme()`）：Server 每次调用现读当前生效的主题，
    /// 返回 `{ candidate = { … }, bar = { … }, tip = { … } }`（颜色是 `#AARRGGBB`）。
    pub fn set_theme_hook(&self, hook: ThemeHook) {
        *self.theme_hook.borrow_mut() = Some(hook);
    }

    /// 有没有挂着的异步请求（HTTP）。调用方据此决定要不要收结果、要不要拼候选载荷。
    pub fn has_pending_requests(&self) -> bool {
        !self.requests.pending.borrow().is_empty()
    }

    /// 收脚本发起的异步请求结果：**没超过脚本给的时间限制**的调它的回调，回调返回的表按事件那套语义
    /// 由调用方合并；超过时限才回来的**直接丢掉**（记一条日志）。
    ///
    /// `candidates` 是这一刻那一屏候选，回调的第二个参数就是它 —— 这样回调不必依赖发请求时那份
    /// 已经可能过期的候选下标。
    pub fn poll_requests(&self, candidates: Table) -> Vec<Table> {
        // 先把结果摘出来（回调里可能再发请求，不能握着 `pending` 的借用）
        let mut ready = Vec::new();
        {
            let mut pending = self.requests.pending.borrow_mut();
            loop {
                let Ok((id, outcome)) = self.requests.results.borrow_mut().try_recv() else {
                    break;
                };
                let Some(request) = pending.remove(&id) else {
                    continue;
                };
                if Instant::now() > request.deadline {
                    tracing::warn!(url = %request.url, "脚本的 HTTP 结果超过了它给的时限，丢掉");
                    continue;
                }
                ready.push((request, outcome));
            }
        }
        let mut responses = Vec::new();
        for (request, outcome) in ready {
            let payload = match http_payload(self.lua(), &request.url, outcome) {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::warn!(%error, "拼 HTTP 结果给脚本失败");
                    continue;
                }
            };
            self.arm(&request.manifest);
            *self.current.borrow_mut() = request.manifest.name.clone();
            *self.manifest.borrow_mut() = Some(request.manifest.clone());
            match request
                .callback
                .call::<mlua::Value>((payload, candidates.clone()))
            {
                Ok(mlua::Value::Table(table)) => responses.push(table),
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        %error,
                        url = %request.url,
                        script = %request.manifest.name,
                        "脚本的 HTTP 回调出错，中止这一次"
                    );
                    self.report_error(&request.manifest, "http", &error.to_string());
                }
            }
        }
        responses
    }

    /// 把「光标前文」（应用里已经输入、不在候选窗口里的那段文本）放进 `cloudime.context`。
    /// 派发前调用：脚本每次读到的都是这一刻的文本。写的是 `cloudime` 表**本体**（脚本只拿到只读代理）。
    pub fn set_context(&self, text: &str) {
        let Some(host) = self.host.borrow().clone() else {
            return;
        };
        if let Err(error) = host.set("context", text) {
            tracing::warn!(%error, "写 cloudime.context 失败");
        }
    }

    /// 记下候选窗最近画出来的内容区尺寸（点）：脚本的 `cloudime.candidate.width()` 读它。
    /// 与 [`Runtime::set_context`] 同一时机（每次派发 / 收回调之前）由 Server 刷一次。
    pub fn set_viewport(&self, size: Option<(f32, f32)>) {
        *self.viewport.borrow_mut() = size;
    }

    /// 装取输入框文本的接口：脚本调 `cloudime.text.all` / `before` 时用它要一份快照。
    /// 由 Server 在 [`Runtime::load`] / [`Runtime::none`] 之后调（没装时脚本调这两个会报错）。
    pub fn set_text_hook(&self, hook: TextHook) {
        *self.text_hook.borrow_mut() = Some(hook);
    }

    /// 装剪贴板的两条路：脚本的 `cloudime.clipboard.settext` / `gettext` 走它们。
    /// Lua 标准库里没有剪贴板，实现在 Server 那边（Windows 剪贴板 API）。
    pub fn set_clipboard(&self, set: ClipboardSetHook, get: ClipboardGetHook) {
        *self.clipboard_set.borrow_mut() = Some(set);
        *self.clipboard_get.borrow_mut() = Some(get);
    }

    /// 装“模拟组合键”接口（`cloudime.keyboard.press`），Server 用 Windows `SendInput` 实现。
    pub fn set_keyboard(&self, press: KeyboardHook) {
        *self.keyboard.borrow_mut() = Some(press);
    }

    /// 登记过任何处理函数没有（Server 据此决定要不要去读整篇文本 —— 没脚本就不读）。
    pub fn has_any_handlers(&self) -> bool {
        self.handlers
            .borrow()
            .values()
            .any(|handlers| !handlers.is_empty())
    }

    /// 登记过的处理函数总数。
    fn handler_count(&self) -> usize {
        self.handlers.borrow().values().map(Vec::len).sum()
    }
}

/// 装宿主 API：`cloudime` 表（`script` / `on` / `log` / `context` / `http_get` / `run`）+ 几处刻意改动
/// （`os.exit` / `os.execute` / `io.popen` / `io.stdin` / `coroutine`）+ 指令预算钩子 + 锁住 `cloudime`。
fn install_host_api(runtime: &Runtime) -> mlua::Result<()> {
    let lua = &runtime.lua;
    let handlers = &runtime.handlers;
    let current = &runtime.current;
    let manifest = &runtime.manifest;
    let loading = &runtime.loading;
    let limits = &runtime.limits;
    let seq = &runtime.seq;
    let requests = &runtime.requests;
    let redraw = &runtime.redraw;
    let caret = &runtime.caret;
    let install = &runtime.install;
    let size = &runtime.size;
    let theme = &runtime.theme;
    let viewport = &runtime.viewport;
    let measure = &runtime.measure;
    let text_hook = &runtime.text_hook;
    let clipboard_set = &runtime.clipboard_set;
    let clipboard_get = &runtime.clipboard_get;
    let keyboard = &runtime.keyboard;
    let config_hook = &runtime.config_hook;
    let app_hook = &runtime.app_hook;
    let host = &runtime.host;
    let table = lua.create_table()?;
    // 光标前文：第一次派发前也能读到（空串），不用先判断 nil
    table.set("context", "")?;
    // 光标位置：`cloudime.caret()` 只是**请求**一次，结果以 `caret` 事件送来
    //（`{ left, top, right, bottom, work_left, …, scale }`，屏幕坐标 / 点）。
    let caret_flag = caret.clone();
    let caret_current = current.clone();
    table.set(
        "caret",
        lua.create_function(move |_, ()| {
            caret_flag.set(true);
            tracing::debug!(script = %caret_current.borrow(), "脚本请求光标位置");
            Ok(())
        })?,
    )?;
    // 安装目录：`cloudime.install_dir()` → 绝对路径（测试 / 没装时是 nil）；
    // `cloudime.install_path("Images/SVG/打叉.svg")` → 安装目录里的文件绝对路径（不存在给 nil / 越界报错）。
    let dir_state = install.clone();
    table.set(
        "install_dir",
        lua.create_function(move |_, ()| {
            Ok(dir_state
                .root()
                .map(|dir| dir.to_string_lossy().into_owned()))
        })?,
    )?;
    let path_state = install.clone();
    table.set(
        "install_path",
        lua.create_function(move |_, relative: String| -> mlua::Result<Option<String>> {
            path_state
                .resolve(&relative)
                .map_err(mlua::Error::RuntimeError)
        })?,
    )?;
    let log_current = current.clone();
    table.set(
        "log",
        lua.create_function(move |_, message: String| {
            let script = log_current.borrow().clone();
            tracing::info!(target: "cloudime_script", script = %script, "{message}");
            Ok(())
        })?,
    )?;
    // 清单：每个脚本必须**最先**调一次。缺字段 / 类型不对 / `api` 不认识 / `sync` 与 `handover`
    // 对不上 → 这里直接报错 → 加载失败 → 这个脚本判无效（见 `Runtime::run`）。
    let script_current = current.clone();
    let script_manifest = manifest.clone();
    table.set(
        "script",
        lua.create_function(move |_, options: Table| {
            if script_manifest.borrow().is_some() {
                return Err(mlua::Error::RuntimeError(
                    "清单（cloudime.script{…}）只能声明一次".to_owned(),
                ));
            }
            let parsed = parse_manifest(&options, &script_current.borrow())?;
            *script_manifest.borrow_mut() = Some(Rc::new(parsed));
            Ok(())
        })?,
    )?;
    let on_handlers = handlers.clone();
    let on_manifest = manifest.clone();
    let on_loading = loading.clone();
    let on_seq = seq.clone();
    table.set(
        "on",
        lua.create_function(move |_, (event, function): (String, Function)| {
            if !on_loading.get() {
                return Err(mlua::Error::RuntimeError(
                    "cloudime.on 只能在加载脚本的时候调（运行期不能再登记处理函数）".to_owned(),
                ));
            }
            let Some(manifest) = on_manifest.borrow().clone() else {
                return Err(mlua::Error::RuntimeError(
                    "清单（cloudime.script{…}）必须写在最前面：它给这个脚本定限额与能跑的应用"
                        .to_owned(),
                ));
            };
            let seq = on_seq.get().wrapping_add(1);
            on_seq.set(seq);
            tracing::debug!(script = %manifest.name, event, "脚本登记了处理函数");
            let mut handlers = on_handlers.borrow_mut();
            let list = handlers.entry(event).or_default();
            list.push(Handler {
                manifest,
                function,
                seq,
            });
            // priority 大的排在后面（合并时盖住前面的）；同 priority 保持登记顺序
            list.sort_by_key(|handler| (handler.manifest.priority, handler.seq));
            Ok(())
        })?,
    )?;
    // 候选窗口的命名空间：`cloudime.candidate.redraw()` —— 请求 Server 把当前这一屏重算一遍
    // （会重新派发 `candidates` 事件）再重画。脚本在异步回调里改了自己的状态之后用它；
    // 它只是置一个「脏」标记，真正的重算/重画在 Server 那一拍结束前做。
    let redraw_flag = redraw.clone();
    let redraw_current = current.clone();
    let size_state = size.clone();
    let size_current = current.clone();
    let candidate = lua.create_table()?;
    // 候选窗现在的宽度（点）：折行就按窗口宽度来，不用在脚本里写死一个数。
    // 没画过（不在组句 / 已经收起）时给 `nil`，脚本自己回退。
    let width_state = viewport.clone();
    let width_current = current.clone();
    candidate.set(
        "width",
        lua.create_function(move |_, ()| {
            let width = width_state.borrow().map(|(width, _)| width);
            tracing::debug!(script = %width_current.borrow(), ?width, "脚本问候选窗宽度");
            Ok(width)
        })?,
    )?;
    let height_state = viewport.clone();
    let height_current = current.clone();
    candidate.set(
        "height",
        lua.create_function(move |_, ()| {
            let height = height_state.borrow().map(|(_, height)| height);
            tracing::debug!(script = %height_current.borrow(), ?height, "脚本问候选窗高度");
            Ok(height)
        })?,
    )?;
    candidate.set(
        "redraw",
        lua.create_function(move |_, ()| {
            redraw_flag.set(true);
            tracing::debug!(script = %redraw_current.borrow(), "脚本请求重画候选窗");
            Ok(())
        })?,
    )?;
    // 尺寸三项：只在**本次组句**内有效（组句结束 Server 自己回配置值）；不带参数就是恢复默认。
    candidate.set(
        "set_min_width",
        lua.create_function(move |_, width: Option<f32>| {
            let script = size_current.borrow().clone();
            size_state.borrow_mut().min_width = Some(width.map(|width| {
                let width = width.clamp(0.0, MAX_SCRIPT_MIN_WIDTH);
                tracing::debug!(script = %script, width, "脚本设候选窗最小宽度");
                width
            }));
            Ok(())
        })?,
    )?;
    let page_size_state = size.clone();
    let page_size_current = current.clone();
    candidate.set(
        "set_page_size",
        lua.create_function(move |_, count: Option<u32>| {
            let script = page_size_current.borrow().clone();
            let (min, max) = SCRIPT_PAGE_SIZE_RANGE;
            page_size_state.borrow_mut().page_size = Some(count.map(|count| {
                let count = (count as usize).clamp(min, max);
                tracing::debug!(script = %script, count, "脚本设一页候选数");
                count
            }));
            Ok(())
        })?,
    )?;
    let scale_state = size.clone();
    let scale_current = current.clone();
    candidate.set(
        "set_scale",
        lua.create_function(move |_, factor: Option<f32>| {
            let script = scale_current.borrow().clone();
            let (min, max) = SCRIPT_SCALE_RANGE;
            scale_state.borrow_mut().scale = Some(factor.map(|factor| {
                let factor = if factor.is_finite() {
                    factor.clamp(min, max)
                } else {
                    1.0
                };
                tracing::debug!(script = %script, factor, "脚本设候选窗缩放");
                factor
            }));
            Ok(())
        })?,
    )?;
    table.set("candidate", candidate)?;
    // 量文字：`cloudime.ui.measure(文本, 字体名?)` 返回 `{ width, height }`（单位点，
    // 直接喂 `cloudime.candidate.set_min_width` 就装得下）；`measure_tip` 是「底部那一行（翻译 / 在线）」
    // 那个字体的捷径。量尺由 Server 装（`Runtime::set_measure`）—— 没装就报错。
    let ui = lua.create_table()?;
    let measure_state = measure.clone();
    let measure_current = current.clone();
    ui.set(
        "measure",
        lua.create_function(move |lua, (text, font): (String, Option<String>)| {
            let font = match font.as_deref() {
                None => MeasureFont::Candidate,
                Some(name) => parse_font(name)?,
            };
            measure_table(lua, &measure_state, &measure_current, &text, font)
        })?,
    )?;
    let tip_state = measure.clone();
    let tip_current = current.clone();
    ui.set(
        "measure_tip",
        lua.create_function(move |lua, text: String| {
            measure_table(lua, &tip_state, &tip_current, &text, MeasureFont::Translate)
        })?,
    )?;
    // 截断 / 折行：`cloudime.ui.truncate(文本, 行宽 [, 选项])`，选项表
    // `{ mode = "ellipsis"（缺省，单行补「…」）| "wrap"（切到下一行）, max_lines = 5, font = "candidate" }`。
    // 返回 `{ text, lines, truncated, width }`；`"wrap"` 时 `text` 里带换行，直接写进 `online` 就显示成多行。
    let truncate_state = measure.clone();
    let truncate_current = current.clone();
    ui.set(
        "truncate",
        lua.create_function(
            move |lua, (text, max_width, options): (String, f32, Option<Table>)| {
                let options = TruncateOptions::parse(options)?;
                truncate_table(
                    lua,
                    &truncate_state,
                    &truncate_current,
                    &text,
                    max_width,
                    options,
                )
            },
        )?,
    )?;
    table.set("ui", ui)?;
    // 当前主题（`cloudime.get_theme()`）：Server 装进来的钩子每次现读，颜色都是 `#AARRGGBB`。
    // 没有装（测试 / 加载期）给 `nil`，脚本自己回退。
    let theme_state = runtime.theme_hook.clone();
    table.set(
        "get_theme",
        lua.create_function(move |lua, ()| {
            let hook = theme_state.borrow().clone();
            match hook {
                Some(hook) => Ok(mlua::Value::Table(hook(lua)?)),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    // 脚本窗口：`cloudime.window.create{…}` 建一块独立窗口，`create_auxiliary{ owner = …, … }`
    // 在它名下挂一件辅助窗口（资产）。这里只产出请求（见 `window` 模块），窗口在 UI 线程上建；
    // 句柄上的方法把后续操作推进同一条队列，Server 每次派发结束后 `take_window_requests` 取走。
    let window_table = lua.create_table()?;
    let create_requests = runtime.window_requests.clone();
    let create_seq = runtime.window_seq.clone();
    let create_images = runtime.image_cache.clone();
    let create_texts = runtime.window_texts.clone();
    window_table.set(
        "create",
        lua.create_function(move |lua, options: mlua::Table| {
            let mut spec = window::parse_window(&options)?;
            let layout = options
                .get::<Option<String>>("layout")?
                .filter(|path| !path.trim().is_empty());
            let mut document = None;
            if let Some(path) = layout {
                let parsed = window::load_layout_document(
                    lua,
                    Path::new(&path),
                    (Some(spec.width), Some(spec.height)),
                    &mut create_images.borrow_mut(),
                )?;
                spec.width = parsed.width;
                spec.height = parsed.height;
                document = Some(parsed);
            }
            let id = next_window_id(&create_seq);
            create_requests
                .borrow_mut()
                .push(window::Request::Create(id, spec.clone()));
            if let Some(document) = document {
                window::cache_document_texts(&document, id, &create_texts);
                create_requests
                    .borrow_mut()
                    .push(window::Request::SetContent(id, document));
            }
            window::handle(
                lua,
                window::Kind::Independent,
                id,
                &spec.name,
                (Some(spec.width), Some(spec.height)),
                create_requests.clone(),
                create_images.clone(),
                create_texts.clone(),
            )
        })?,
    )?;
    let auxiliary_requests = runtime.window_requests.clone();
    let auxiliary_seq = runtime.window_seq.clone();
    let auxiliary_images = runtime.image_cache.clone();
    let auxiliary_texts = runtime.window_texts.clone();
    window_table.set(
        "create_auxiliary",
        lua.create_function(move |lua, options: mlua::Table| {
            let id = next_window_id(&auxiliary_seq);
            let mut spec = window::parse_auxiliary(&options, id)?;
            let layout = options
                .get::<Option<String>>("layout")?
                .filter(|path| !path.trim().is_empty());
            let mut document = None;
            if let Some(path) = layout {
                let parsed = window::load_layout_document(
                    lua,
                    Path::new(&path),
                    (spec.width, spec.height),
                    &mut auxiliary_images.borrow_mut(),
                )?;
                spec.width = Some(parsed.width);
                spec.height = Some(parsed.height);
                document = Some(parsed);
            }
            auxiliary_requests
                .borrow_mut()
                .push(window::Request::CreateAuxiliary(id, spec.clone()));
            if let Some(document) = document {
                window::cache_document_texts(&document, id, &auxiliary_texts);
                auxiliary_requests
                    .borrow_mut()
                    .push(window::Request::SetContent(id, document));
            }
            window::handle(
                lua,
                window::Kind::Auxiliary,
                id,
                &spec.name,
                (spec.width, spec.height),
                auxiliary_requests.clone(),
                auxiliary_images.clone(),
                auxiliary_texts.clone(),
            )
        })?,
    )?;
    table.set("window", window_table)?;
    // 图像句柄：`cloudime.image.load(路径)` → 用户数据（`Arc<Pixmap>`），可以反复放进
    // `static_image` / `img` 的 `image` 字段；Lua 丢掉句柄后，已经放进内容的图仍由元素持有。
    let image = lua.create_table()?;
    let image_cache = runtime.image_cache.clone();
    image.set(
        "load",
        lua.create_function(move |_, path: String| {
            let pixmap = image_cache.borrow_mut().load(Path::new(&path))?;
            Ok(window::ImageHandle::new(pixmap))
        })?,
    )?;
    table.set("image", image)?;
    // 输入框文本：`cloudime.text.all(上限?)` / `cloudime.text.before(上限?)` → `{ text, truncated }`；
    // 这一拍还拿不到（DLL 要读一趟、走个来回）给 `nil`，脚本下一次调用再试。
    // 上限按**显示宽度**算：西文 / 半角 1、中文 / 全角 2；缺省 2^64-1（等于不限）
    //（`all` 超上限留光标附近那一段，`before` 取光标之前那一段）。
    let text = lua.create_table()?;
    let all_state = text_hook.clone();
    text.set(
        "all",
        lua.create_function(move |lua, limit: Option<u64>| {
            text_table(lua, &all_state, TextRange::All, limit)
        })?,
    )?;
    let before_state = text_hook.clone();
    text.set(
        "before",
        lua.create_function(move |lua, limit: Option<u64>| {
            text_table(lua, &before_state, TextRange::Before, limit)
        })?,
    )?;
    let after_state = text_hook.clone();
    text.set(
        "after",
        lua.create_function(move |lua, limit: Option<u64>| {
            text_table(lua, &after_state, TextRange::After, limit)
        })?,
    )?;
    table.set("text", text)?;
    // 剪贴板：`cloudime.clipboard.settext(文本)` / `cloudime.clipboard.gettext()` → 字符串 / nil。
    // Lua 标准库里没有剪贴板，实现在 Server 那边（Windows 剪贴板 API，见 `dispatch/clipboard.rs`）；
    // 没装接口报错，「打不开剪贴板」也报错，「剪贴板里没有文本」给 nil（见 `lua.md`）。
    let clipboard = lua.create_table()?;
    let set_state = clipboard_set.clone();
    clipboard.set(
        "settext",
        lua.create_function(move |_, text: String| {
            let hook = set_state.borrow().clone().ok_or_else(|| {
                mlua::Error::RuntimeError(
                    "cloudime.clipboard 用不了：这次运行没有装这个接口".to_owned(),
                )
            })?;
            hook(&text).map_err(mlua::Error::RuntimeError)
        })?,
    )?;
    let get_state = clipboard_get.clone();
    clipboard.set(
        "gettext",
        lua.create_function(move |_, ()| {
            let hook = get_state.borrow().clone().ok_or_else(|| {
                mlua::Error::RuntimeError(
                    "cloudime.clipboard 用不了：这次运行没有装这个接口".to_owned(),
                )
            })?;
            hook().map_err(mlua::Error::RuntimeError)
        })?,
    )?;
    table.set("clipboard", clipboard)?;
    // 模拟组合键：`cloudime.keyboard.press("ctrl+c")`。真实现由 Server 用 SendInput 装。
    let keyboard_table = lua.create_table()?;
    let keyboard_state = keyboard.clone();
    keyboard_table.set(
        "press",
        lua.create_function(move |_, chord: String| {
            let hook = keyboard_state.borrow().clone().ok_or_else(|| {
                mlua::Error::RuntimeError(
                    "cloudime.keyboard 用不了：服务器没有安装键盘接口".to_owned(),
                )
            })?;
            hook(&chord).map_err(mlua::Error::RuntimeError)
        })?,
    )?;
    table.set("keyboard", keyboard_table)?;
    // 当前时间：`cloudime.get_time()` → `{ year, month, day, hour, minute, second, weekday, unix }`。
    // 前七个字段是**本地时间**；`weekday` 1 = 周一 … 7 = 周日；`unix` 是 Unix 秒。
    // （脚本本来也能用标准库的 `os.date` / `os.time`，这一份是不依赖标准库的稳定形状。）
    let config_state = config_hook.clone();
    table.set(
        "get_curr_config",
        lua.create_function(move |lua, ()| {
            let hook = config_state.borrow().clone().ok_or_else(|| {
                mlua::Error::RuntimeError(
                    "cloudime.get_curr_config 用不了：这次运行没有装这个接口".to_owned(),
                )
            })?;
            hook(lua)
        })?,
    )?;
    let app_state = app_hook.clone();
    table.set(
        "foreground_app",
        lua.create_function(move |_, ()| {
            let hook = app_state.borrow().clone().ok_or_else(|| {
                mlua::Error::RuntimeError(
                    "cloudime.foreground_app 用不了：这次运行没有装这个接口".to_owned(),
                )
            })?;
            Ok(hook())
        })?,
    )?;
    table.set("get_time", lua.create_function(|lua, ()| time_table(lua))?)?;
    // 换主题：`cloudime.apply_theme("Panic")` 换上（按名字找，可带 / 不带 `.json`）；
    // `cloudime.apply_theme(false)` 去掉这次设的主题。具体的落法（写配置 + 重启输入法服务）由 Server 定，
    // 这里只留一个请求；Server 在派发 / 回调结束后取一次（见 `Runtime::take_theme_request`）。
    let theme_state = theme.clone();
    table.set(
        "apply_theme",
        lua.create_function(move |_, name: mlua::Value| {
            let request = match name {
                mlua::Value::String(name) => Some(
                    name.to_str()
                        .map_err(|_| {
                            mlua::Error::RuntimeError(
                                "cloudime.apply_theme 的主题名不是合法 UTF-8".to_owned(),
                            )
                        })?
                        .to_string(),
                ),
                mlua::Value::Boolean(false) | mlua::Value::Nil => None,
                value => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "cloudime.apply_theme 要主题名字符串（想去掉这次设的主题就传 false），给的是 {}",
                        value.type_name()
                    )));
                }
            };
            *theme_state.borrow_mut() = Some(request);
            Ok(())
        })?,
    )?;
    let requests_state = requests.clone();
    let requests_current = current.clone();
    let requests_manifest = manifest.clone();
    table.set(
        "http_get",
        lua.create_function(
            move |_, (url, timeout_ms, callback): (String, u64, Function)| {
                if url.trim().is_empty() {
                    return Err(mlua::Error::RuntimeError(
                        "cloudime.http_get 的地址是空的".to_owned(),
                    ));
                }
                let (timeout, manifest) =
                    check_timeout("cloudime.http_get", timeout_ms, &requests_manifest)?;
                let id = start_request(
                    &requests_state,
                    Pending {
                        url: url.clone(),
                        deadline: Instant::now() + timeout,
                        callback,
                        manifest,
                    },
                );
                tracing::debug!(
                    script = %requests_current.borrow(),
                    url = %url,
                    timeout_ms,
                    "脚本发起 HTTP GET"
                );
                // 请求在自己的线程里跑，派发线程立刻回去处理输入（见 `http`）
                let tx = requests_state.tx.clone();
                std::thread::spawn(move || {
                    let outcome = http::request(http::Method::Get, &url, "", &[], timeout);
                    // 收不到（Runtime 已经没了）就算了
                    let _ = tx.send((id, outcome));
                });
                Ok(())
            },
        )?,
    )?;
    // POST：体 + 表头（`options = { timeout_ms = …, headers = { … } }`）。
    // 表头名字 / 值在这儿先验一遍，报一条清楚的 Lua 错 —— 别丢给后台线程 panic。
    let post_state = requests.clone();
    let post_current = current.clone();
    let post_manifest = manifest.clone();
    table.set(
        "http_post",
        lua.create_function(
            move |_, (url, body, options, callback): (String, String, Table, Function)| {
                if url.trim().is_empty() {
                    return Err(mlua::Error::RuntimeError(
                        "cloudime.http_post 的地址是空的".to_owned(),
                    ));
                }
                let timeout_ms = options.get::<Option<u64>>("timeout_ms")?.ok_or_else(|| {
                    mlua::Error::RuntimeError(
                        "cloudime.http_post 的 options 里必须给 timeout_ms（毫秒，> 0）".to_owned(),
                    )
                })?;
                let mut headers = Vec::new();
                if let Some(table) = options.get::<Option<Table>>("headers")? {
                    for entry in table.pairs::<String, String>() {
                        headers.push(entry?);
                    }
                }
                if let Err(error) = http::check_headers(&headers) {
                    return Err(mlua::Error::RuntimeError(format!(
                        "cloudime.http_post：{error}"
                    )));
                }
                let (timeout, manifest) =
                    check_timeout("cloudime.http_post", timeout_ms, &post_manifest)?;
                let id = start_request(
                    &post_state,
                    Pending {
                        url: url.clone(),
                        deadline: Instant::now() + timeout,
                        callback,
                        manifest,
                    },
                );
                tracing::debug!(
                    script = %post_current.borrow(),
                    url = %url,
                    bytes = body.len(),
                    timeout_ms,
                    "脚本发起 HTTP POST"
                );
                let tx = post_state.tx.clone();
                std::thread::spawn(move || {
                    let outcome = http::request(http::Method::Post, &url, &body, &headers, timeout);
                    let _ = tx.send((id, outcome));
                });
                Ok(())
            },
        )?,
    )?;
    // 启动外部程序：**启动就返回、不等**子进程（同步等一样会堵住工人线程）。
    // 脚本那边是 `os.execute(...)`，转发到这里；`cmd /c start ""` 让子进程自己独立在窗口 / 控制台里跑。
    let run_current = current.clone();
    table.set(
        "run",
        lua.create_function(move |_, command: String| {
            let script = run_current.borrow().clone();
            tracing::info!(script = %script, command = %command, "脚本启动外部程序");
            let spawned = {
                let mut command_line = std::process::Command::new("cmd");
                command_line.args(["/c", "start", "", &command]);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                    command_line.creation_flags(CREATE_NO_WINDOW);
                }
                command_line.spawn()
            };
            match spawned {
                Ok(_) => Ok(()),
                Err(error) => Err(mlua::Error::RuntimeError(format!(
                    "启动外部程序失败：{error}"
                ))),
            }
        })?,
    )?;

    // 先把 `cloudime` 表本体留给运行时（写 `context` 这类刷新值用），脚本只拿到**只读代理**：
    // 读走本体，写一律报错（清单规则里「不许改 cloudime 表」）。
    *host.borrow_mut() = Some(table.clone());
    let proxy = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__index", table.clone())?;
    meta.set(
        "__newindex",
        lua.create_function(|_, (_, key): (Table, mlua::Value)| -> mlua::Result<()> {
            Err(mlua::Error::RuntimeError(format!(
                "不许改 cloudime 表（想写 {}）",
                key.type_name()
            )))
        })?,
    )?;
    proxy.set_metatable(Some(meta))?;
    lua.globals().set(HOST_TABLE, proxy)?;

    // 会退掉 Server 或堵死工人线程的那几样，就地换掉（放在 Rust 里，不靠 Lua 片段 —— 便于确认生效）。
    let os = lua.globals().get::<Table>("os")?;
    let exit_current = current.clone();
    os.set(
        "exit",
        lua.create_function(move |_, code: Option<mlua::Value>| {
            let script = exit_current.borrow().clone();
            tracing::warn!(
                %script,
                "脚本调用了 os.exit：已忽略 —— 那会退掉整个输入法服务（所有应用都会断线）"
            );
            let _ = code;
            Ok(())
        })?,
    )?;
    let execute_current = current.clone();
    let run: Function = table.get("run")?;
    os.set(
        "execute",
        lua.create_function(move |_, command: String| {
            let script = execute_current.borrow().clone();
            tracing::info!(%script, %command, "脚本启动外部程序（不等它）");
            run.call::<()>(command)?;
            Ok(0)
        })?,
    )?;
    let io = lua.globals().get::<Table>("io")?;
    io.set(
        "popen",
        lua.create_function(|_, _: mlua::MultiValue| -> mlua::Result<()> {
            Err(mlua::Error::RuntimeError(
                "io.popen 是同步的，会把输入法卡住（所有应用都打不了字）；要发网络请求用 cloudime.http_get"
                    .to_owned(),
            ))
        })?,
    )?;
    // 标准输入关掉：Server 由控制台启动时（开发机上自己跑）`io.read()` 会一直等输入，把工人线程堵死。
    io.set("stdin", mlua::Value::Nil)?;

    // 不给协程：Lua 的调试钩子是**每个线程**一份，脚本在协程里转圈预算拦不住（会卡住所有应用）。
    // 输入法脚本用不到协程，直接不给建 —— 比「看门狗重启进程」干净（用户不会看到输入法闪一下、丢句子）。
    let coroutine = lua.globals().get::<Table>("coroutine")?;
    let no_coroutine = lua.create_function(|_, _: mlua::MultiValue| -> mlua::Result<()> {
        Err(mlua::Error::RuntimeError(
            "脚本里不提供协程（coroutine）：Lua 的调试钩子每个线程一份，协程里的死循环会把输入法卡住"
                .to_owned(),
        ))
    })?;
    coroutine.set("create", no_coroutine.clone())?;
    coroutine.set("wrap", no_coroutine)?;

    // 指令预算 + 墙钟：mlua 的钩子每 `INSTRUCTION_STEP` 条指令醒一次，超了就从钩子里报错中止这一次。
    // 限额由 `Runtime::arm` 在每次调用前按**该脚本清单**里的 `budget` / `timeout` 设好
    //（加载期用全局上限与 `LOAD_TIMEOUT`）。为什么必须有：脚本卡住堵的是**整条工人线程** ——
    // 所有应用都会打不了字，不是一个应用。见 `docs/design/script.md`。
    let hook_limits = limits.clone();
    lua.set_hook(
        mlua::HookTriggers::new().every_nth_instruction(INSTRUCTION_STEP),
        move |_, _| {
            let (used, budget, deadline) = hook_limits.get();
            if Instant::now() > deadline {
                return Err(mlua::Error::RuntimeError(
                    "脚本这一次超过了清单里给的 timeout，本次中止".to_owned(),
                ));
            }
            let used = used.saturating_add(INSTRUCTION_STEP);
            hook_limits.set((used, budget, deadline));
            if used > budget {
                return Err(mlua::Error::RuntimeError(format!(
                    "脚本这一次跑得太久（超过 {budget} 条指令），本次中止"
                )));
            }
            Ok(mlua::VmState::Continue)
        },
    )?;
    // LuaJIT 编成 trace 之后不再检查调试钩子（实测 `while true do end` 会真的跑飞），所以脚本的 JIT 关掉；
    // `jit.on` 也收掉，免得脚本再打开。mlua 的 `Lua::new()` 只开**安全子集**：没有 `debug` 库，
    // 所以脚本本来就摘不掉钩子（`debug` 为 nil 时这行也不该报错）。
    lua.load(
        "if jit and jit.off then jit.off() jit.on = nil end\n\
         if debug then debug.sethook = nil end\n",
    )
    .exec()?;
    Ok(())
}

/// 下一个窗口编号（从 1 开始）：脚本句柄上的 `id`，UI 线程按它认窗口。
fn next_window_id(seq: &Cell<u32>) -> u32 {
    let id = seq.get().wrapping_add(1).max(1);
    seq.set(id);
    id
}

/// 解析清单里的 `HH:MM`（本地时间，24 小时制）→ `(小时, 分钟)`。
fn parse_clock(text: &str) -> mlua::Result<(i8, i8)> {
    let invalid = || {
        mlua::Error::RuntimeError(format!(
            "清单的 time_range 里的 \"{text}\" 读不出时间（要 \"HH:MM\"，本地时间，24 小时制）"
        ))
    };
    let (hour, minute) = text.trim().split_once(':').ok_or_else(invalid)?;
    let hour: i8 = hour.trim().parse().map_err(|_| invalid())?;
    let minute: i8 = minute.trim().parse().map_err(|_| invalid())?;
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) {
        return Err(invalid());
    }
    Ok((hour, minute))
}

/// 本地时间换算成分钟（0–1439）。
fn minute_of(now: &jiff::Zoned) -> u16 {
    u16::try_from(now.hour()).unwrap_or(0) * 60 + u16::try_from(now.minute()).unwrap_or(0)
}

/// 当前的本地分钟数；拿不到时间给 `None`（那会儿任何 `time_range` 都不命中）。
fn current_minute() -> Option<u16> {
    Some(minute_of(&jiff::Zoned::now()))
}

/// 拼音串规范化：去掉分隔符（`'` / 空白）并转小写 —— `Ni'Hao` 与 `nihao` 算同一段。
fn normalize_pinyin(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != '\'')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 当前本地时间的一张表：`cloudime.get_time()` 与 `time` 事件的载荷共用。
fn time_table(lua: &Lua) -> mlua::Result<Table> {
    let now = jiff::Zoned::now();
    let fields = lua.create_table()?;
    fields.set("year", now.year())?;
    fields.set("month", now.month())?;
    fields.set("day", now.day())?;
    fields.set("hour", now.hour())?;
    fields.set("minute", now.minute())?;
    fields.set("second", now.second())?;
    fields.set("weekday", now.date().weekday().to_monday_one_offset())?;
    fields.set("unix", now.timestamp().as_second())?;
    Ok(fields)
}

fn parse_manifest(options: &Table, default_name: &str) -> mlua::Result<Manifest> {
    let api = options.get::<Option<u32>>("api")?.ok_or_else(|| {
        mlua::Error::RuntimeError(format!("清单必须写 api（当前只有 {API_VERSION}）"))
    })?;
    if api != API_VERSION {
        return Err(mlua::Error::RuntimeError(format!(
            "清单的 api = {api} 不认识（当前只有 {API_VERSION}）"
        )));
    }
    let sync = options.get::<Option<bool>>("sync")?.ok_or_else(|| {
        mlua::Error::RuntimeError("清单必须写 sync（true 同步 / false 异步）".to_owned())
    })?;
    let handover = match options.get::<Option<String>>("handover")?.as_deref() {
        Some("return") => Handover::Return,
        Some("callback") => Handover::Callback,
        Some(other) => {
            return Err(mlua::Error::RuntimeError(format!(
                "清单的 handover = \"{other}\" 不认识（只有 \"return\" / \"callback\"）"
            )));
        }
        None => {
            return Err(mlua::Error::RuntimeError(
                "清单必须写 handover（\"return\" / \"callback\"）".to_owned(),
            ));
        }
    };
    let expected = if sync {
        Handover::Return
    } else {
        Handover::Callback
    };
    if handover != expected {
        return Err(mlua::Error::RuntimeError(format!(
            "清单里 sync = {sync} 与 handover 对不上（true 配 \"return\"、false 配 \"callback\"）"
        )));
    }
    let timeout_ms = options.get::<Option<u64>>("timeout")?.ok_or_else(|| {
        mlua::Error::RuntimeError("清单必须写 timeout（单次调用的墙钟上限，毫秒）".to_owned())
    })?;
    if timeout_ms == 0 || timeout_ms > MAX_TIMEOUT_MS {
        return Err(mlua::Error::RuntimeError(format!(
            "清单的 timeout = {timeout_ms} 超出范围（1–{MAX_TIMEOUT_MS} 毫秒）"
        )));
    }
    // 触发条件（**必填**）：Server 只按它派对应的事件。
    let trigger = match options
        .get::<Option<String>>("trigger_condition")?
        .as_deref()
    {
        Some("combination_key") => TriggerCondition::CombinationKey,
        Some("time_range") => TriggerCondition::TimeRange,
        Some("candidate_context") => TriggerCondition::CandidateContext,
        Some("focus_app") => TriggerCondition::FocusApp,
        Some("manual") => TriggerCondition::Manual,
        Some(other) => {
            return Err(mlua::Error::RuntimeError(format!(
                "清单的 trigger_condition = \"{other}\" 不认识（只有 \"combination_key\" / \"time_range\" / \"candidate_context\" / \"focus_app\" / \"manual\"）"
            )));
        }
        None => {
            return Err(mlua::Error::RuntimeError(
                "清单必须写 trigger_condition（combination_key 组合键 / time_range 时间段 / candidate_context 候选窗拼音 / focus_app 光标所在程序 / manual 手动触发）".to_owned(),
            ));
        }
    };
    // `combination_key`：修饰键（可 "none"）+ 具体键（`keys`）。
    let (combination_modifiers, wanted_keys) = if trigger == TriggerCondition::CombinationKey {
        let text = options
            .get::<Option<String>>("combination_modifiers")?
            .ok_or_else(|| {
                mlua::Error::RuntimeError(
                    "trigger_condition = \"combination_key\" 时清单必须写 combination_modifiers（\"none\" / \"ctrl\" / \"ctrl+alt\" / \"ctrl+shift\" / \"alt+shift\" / \"alt\" / \"win+ctrl\" / \"win+alt\" / \"win+shift\" / \"win+alt+ctrl\" / \"win+ctrl+shift\"）"
                        .to_owned(),
                )
            })?;
        let modifiers = parse_combination_modifiers(&text)?;
        (modifiers, parse_keys(options, modifiers)?)
    } else {
        (0, Vec::new())
    };
    // `time_range`：`{ from = "20:00", to = "06:00" }`（跨夜也行，只触发一次）。
    let time_range = if trigger == TriggerCondition::TimeRange {
        let range = options.get::<Option<Table>>("time_range")?.ok_or_else(|| {
            mlua::Error::RuntimeError(
                "trigger_condition = \"time_range\" 时清单必须写 time_range = { from = \"20:00\", to = \"06:00\" }"
                    .to_owned(),
            )
        })?;
        Some(parse_time_range(&range)?)
    } else {
        None
    };
    // `candidate_context`：候选窗口出现且拼音串等于它（比较时忽略分隔符与大小写）。
    let candidate_pinyin = if trigger == TriggerCondition::CandidateContext {
        let text = options
            .get::<Option<String>>("candidate_pinyin")?
            .ok_or_else(|| {
                mlua::Error::RuntimeError(
                    "trigger_condition = \"candidate_context\" 时清单必须写 candidate_pinyin（比如 \"nihao\"）"
                        .to_owned(),
                )
            })?;
        let normalized = normalize_pinyin(&text);
        if normalized.is_empty() {
            return Err(mlua::Error::RuntimeError(
                "清单的 candidate_pinyin 是空的".to_owned(),
            ));
        }
        Some(normalized)
    } else {
        None
    };
    let budget = match options.get::<Option<u32>>("budget")? {
        Some(0) => {
            return Err(mlua::Error::RuntimeError(
                "清单的 budget 不能是 0（不写就用全局上限）".to_owned(),
            ));
        }
        Some(budget) => {
            if budget > INSTRUCTION_BUDGET {
                tracing::warn!(budget, "清单的 budget 超过全局上限，按全局上限走");
            }
            Some(budget)
        }
        None => None,
    };
    let on_error = match options.get::<Option<mlua::Value>>("on_error")? {
        Some(mlua::Value::Function(function)) => Some(function),
        Some(mlua::Value::Boolean(false)) => None,
        Some(value) => {
            return Err(mlua::Error::RuntimeError(format!(
                "清单的 on_error 必须是函数或 false，实际是 {}",
                value.type_name()
            )));
        }
        None => {
            return Err(mlua::Error::RuntimeError(
                "清单必须写 on_error（出错回调函数，或 false 表示不处理、直接中止这一次）"
                    .to_owned(),
            ));
        }
    };
    let apps = match options.get::<Option<Table>>("apps")? {
        Some(apps) => apps
            .sequence_values::<String>()
            .collect::<mlua::Result<Vec<String>>>()?,
        None => Vec::new(),
    };
    if trigger == TriggerCondition::FocusApp && apps.is_empty() {
        return Err(mlua::Error::RuntimeError(
            "trigger_condition = \"focus_app\" 时清单必须写 apps（光标在哪些程序里触发，如 { \"Code.exe\" }）"
                .to_owned(),
        ));
    }
    let needs_context = options
        .get::<Option<bool>>("needs_context")?
        .unwrap_or(false);
    let priority = options.get::<Option<i32>>("priority")?.unwrap_or(0);
    let name = options
        .get::<Option<String>>("name")?
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| default_name.to_owned());
    let description = options
        .get::<Option<String>>("description")?
        .filter(|description| !description.trim().is_empty());
    Ok(Manifest {
        name,
        description,
        budget,
        timeout: Duration::from_millis(timeout_ms),
        apps,
        on_error,
        trigger,
        time_range,
        time_fired: Cell::new(None),
        combination_modifiers,
        wanted_keys,
        candidate_pinyin,
        needs_context,
        priority,
    })
}

/// 这个文件要不要跳过：模板，或在禁用名单里（按文件名，大小写不敏感）。
fn skipped(path: &Path, disabled: &[String]) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    name.eq_ignore_ascii_case(TEMPLATE_FILE)
        || disabled
            .iter()
            .any(|disabled| disabled.trim().eq_ignore_ascii_case(name))
}

/// 量一段文字，拼成脚本要的表 `{ width, height }`（单位点）；没装量尺 / 量不了就报错。
fn measure_table(
    lua: &Lua,
    state: &Rc<RefCell<Option<MeasureHook>>>,
    current: &Rc<RefCell<String>>,
    text: &str,
    font: MeasureFont,
) -> mlua::Result<Table> {
    let hook = measure_hook(state, "cloudime.ui.measure")?;
    let (width, height) = hook(text, font).ok_or_else(|| {
        mlua::Error::RuntimeError("cloudime.ui.measure 量不了：字体库没建起来".to_owned())
    })?;
    tracing::debug!(script = %current.borrow(), text, width, height, "脚本量文字宽度");
    let table = lua.create_table()?;
    table.set("width", width)?;
    table.set("height", height)?;
    Ok(table)
}

/// `measure` / `truncate` 里那个字体名：不认识就报错并列出可用的。
fn parse_font(name: &str) -> mlua::Result<MeasureFont> {
    MeasureFont::from_name(name).ok_or_else(|| {
        mlua::Error::RuntimeError(format!(
            "字体名不认识：{name}（可用：{}）",
            MeasureFont::NAMES
        ))
    })
}

/// 取一次量尺（`measure` / `truncate` 共用）；没装就报错。
fn measure_hook(state: &Rc<RefCell<Option<MeasureHook>>>, api: &str) -> mlua::Result<MeasureHook> {
    state.borrow().clone().ok_or_else(|| {
        mlua::Error::RuntimeError(format!(
            "{api} 量不了：这次运行没有装量尺（一般是没走渲染器）"
        ))
    })
}

/// 截断时补上的省略号（与渲染器那边同一颗）。
const ELLIPSIS: &str = "…";

/// `cloudime.ui.truncate` 的选项表。
struct TruncateOptions {
    /// 用哪个字体量。
    font: MeasureFont,

    /// `true` 折行（`"wrap"`）；`false` 单行补「…」（`"ellipsis"`）。
    wrap: bool,

    /// 折行最多几行。
    max_lines: usize,
}

impl TruncateOptions {
    /// 从脚本给的表里读；缺省：候选词字体、补「…」、最多 5 行。
    fn parse(options: Option<Table>) -> mlua::Result<Self> {
        let mut parsed = Self {
            font: MeasureFont::Candidate,
            wrap: false,
            max_lines: 5,
        };
        let Some(options) = options else {
            return Ok(parsed);
        };
        if let Some(name) = options.get::<Option<String>>("font")? {
            parsed.font = parse_font(&name)?;
        }
        match options.get::<Option<String>>("mode")?.as_deref() {
            None | Some("ellipsis") => parsed.wrap = false,
            Some("wrap") => parsed.wrap = true,
            Some(mode) => {
                return Err(mlua::Error::RuntimeError(format!(
                    "cloudime.ui.truncate 的 mode 不认识：{mode}（可用：ellipsis / wrap）"
                )));
            }
        }
        if let Some(lines) = options.get::<Option<f64>>("max_lines")? {
            parsed.max_lines = (lines.round() as i64).clamp(1, 20) as usize;
        }
        Ok(parsed)
    }
}

/// 截断 / 折行，拼成脚本要的表 `{ text, lines, truncated, width }`（宽度单位点）。
fn truncate_table(
    lua: &Lua,
    state: &Rc<RefCell<Option<MeasureHook>>>,
    current: &Rc<RefCell<String>>,
    text: &str,
    max_width: f32,
    options: TruncateOptions,
) -> mlua::Result<Table> {
    if !max_width.is_finite() || max_width <= 0.0 {
        return Err(mlua::Error::RuntimeError(
            "cloudime.ui.truncate 的行宽要是正数（单位点）".to_owned(),
        ));
    }
    let hook = measure_hook(state, "cloudime.ui.truncate")?;
    let (text, lines, truncated, width) = truncate_text(
        &hook,
        text,
        options.font,
        max_width,
        options.wrap,
        options.max_lines,
    )
    .ok_or_else(|| {
        mlua::Error::RuntimeError("cloudime.ui.truncate 量不了：字体库没建起来".to_owned())
    })?;
    tracing::debug!(
        script = %current.borrow(),
        lines,
        truncated,
        width,
        "脚本截断 / 折行文字"
    );
    let table = lua.create_table()?;
    table.set("text", text)?;
    table.set("lines", lines)?;
    table.set("truncated", truncated)?;
    table.set("width", width)?;
    Ok(table)
}

/// 截断 / 折行的本体：`wrap = false` 单行补「…」，`wrap = true` 折成最多 `max_lines` 行。
///
/// 返回 `(最终文本, 行数, 是否截断过, 最宽一行的宽度)`；量不了返回 `None`。
fn truncate_text(
    hook: &MeasureHook,
    text: &str,
    font: MeasureFont,
    max_width: f32,
    wrap: bool,
    max_lines: usize,
) -> Option<(String, usize, bool, f32)> {
    if text.is_empty() {
        return Some((String::new(), 1, false, 0.0));
    }
    if !wrap {
        let (shown, truncated, width) = ellipsize(hook, text, font, max_width)?;
        return Some((shown, 1, truncated, width));
    }

    let mut rest: Vec<char> = text.chars().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut truncated = false;
    while !rest.is_empty() {
        if lines.len() + 1 >= max_lines {
            // 最后一行：剩下的全塞进去，放不下就从尾部去字补「…」
            let remaining: String = rest.iter().collect();
            let (shown, cut, _) = ellipsize(hook, &remaining, font, max_width)?;
            lines.push(shown);
            truncated |= cut;
            break;
        }
        let (line, used) = take_line(hook, &rest, font, max_width)?;
        lines.push(line);
        rest.drain(..used);
    }
    let mut width = 0.0_f32;
    for line in &lines {
        width = width.max(hook(line, font)?.0);
    }
    Some((lines.join("\n"), lines.len(), truncated, width))
}

/// 从尾部去字直到「文字 + …」放得下。返回 `(显示文本, 是否截断过, 宽度)`。
fn ellipsize(
    hook: &MeasureHook,
    text: &str,
    font: MeasureFont,
    max_width: f32,
) -> Option<(String, bool, f32)> {
    let width = hook(text, font)?.0;
    if width <= max_width {
        return Some((text.to_owned(), false, width));
    }
    let mut kept: Vec<char> = text.chars().collect();
    while kept.pop().is_some() {
        let mut candidate: String = kept.iter().collect();
        candidate.push_str(ELLIPSIS);
        let width = hook(&candidate, font)?.0;
        if kept.is_empty() || width <= max_width {
            return Some((candidate, true, width));
        }
    }
    None
}

/// 贪心取一行：放得下的最长前缀；优先在空白处断（那个空白甩掉、不带进下一行）。
/// 返回 `(这一行的文字, 吃掉几个字)`。
fn take_line(
    hook: &MeasureHook,
    chars: &[char],
    font: MeasureFont,
    max_width: f32,
) -> Option<(String, usize)> {
    let mut fits = 0;
    for end in 1..=chars.len() {
        let probe: String = chars[..end].iter().collect();
        if hook(&probe, font)?.0 > max_width {
            break;
        }
        fits = end;
    }
    if fits == 0 {
        // 一个字都放不下也得吃一个，免得原地打转
        fits = 1;
    }
    if fits >= chars.len() {
        return Some((chars.iter().collect(), chars.len()));
    }
    if let Some(space) = chars[..fits].iter().rposition(|c| c.is_whitespace())
        && space > 0
    {
        return Some((chars[..space].iter().collect(), space + 1));
    }
    Some((chars[..fits].iter().collect(), fits))
}

/// `cloudime.text.*`：问 Server 要一份文本快照，拼成脚本要的表 `{ text, truncated }`；
/// 这一拍拿不到给 `nil`（Server 那头会顺手请 DLL 下次带上）。
fn text_table(
    lua: &Lua,
    state: &Rc<RefCell<Option<TextHook>>>,
    range: TextRange,
    limit: Option<u64>,
) -> mlua::Result<mlua::Value> {
    let hook = state.borrow().clone().ok_or_else(|| {
        mlua::Error::RuntimeError(
            "cloudime.text 拿不到文本：这次运行没有装这个接口（一般是没走 Server）".to_owned(),
        )
    })?;
    let Some((text, truncated)) = hook(range, limit.unwrap_or(u64::MAX)) else {
        return Ok(mlua::Value::Nil);
    };
    let table = lua.create_table()?;
    table.set("text", text)?;
    table.set("truncated", truncated)?;
    Ok(mlua::Value::Table(table))
}

/// `http_get` / `http_post` 共用的参数检查：**响应时间限制必给**（不然不知道该等到什么时候，
/// 宁可不发），而且不能超过脚本清单里声明的最长等候时间（规则一：开销得声明清楚）。
///
/// 返回「时限 + 发请求那个脚本的清单」：回调属于发请求那个脚本，限额与 `on_error` 都按它的清单来
///（这两个 API 只能在处理函数里调，那会儿清单必然在）。
fn check_timeout(
    tool: &str,
    timeout_ms: u64,
    manifest: &Rc<RefCell<Option<Rc<Manifest>>>>,
) -> mlua::Result<(Duration, Rc<Manifest>)> {
    if timeout_ms == 0 {
        return Err(mlua::Error::RuntimeError(format!(
            "{tool} 的第二个参数必须是响应时间限制（毫秒，> 0）"
        )));
    }
    let Some(manifest) = manifest.borrow().clone() else {
        return Err(mlua::Error::RuntimeError(format!(
            "{tool} 只能在处理函数里调（那会儿清单已经在了）"
        )));
    };
    let allowed = manifest.timeout.as_millis() as u64;
    if timeout_ms > allowed {
        return Err(mlua::Error::RuntimeError(format!(
            "{tool} 的响应时间限制 {timeout_ms} 毫秒超过了清单里的 timeout（{allowed} 毫秒）"
        )));
    }
    Ok((Duration::from_millis(timeout_ms), manifest))
}

/// 挂上一次异步请求，返回它的请求号（工作线程回来时按这个号找回调）。
fn start_request(requests: &Rc<Requests>, pending: Pending) -> u64 {
    let id = requests.next_request.get().wrapping_add(1);
    requests.next_request.set(id);
    requests.pending.borrow_mut().insert(id, pending);
    id
}

fn http_payload(lua: &Lua, url: &str, outcome: http::Outcome) -> mlua::Result<Table> {
    let payload = lua.create_table()?;
    payload.set("url", url)?;
    match outcome.error {
        Some(error) => payload.set("error", error)?,
        None => {
            payload.set("status", outcome.status.unwrap_or(0))?;
            payload.set("body", outcome.body)?;
        }
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个空的临时脚本目录；同名目录先清掉。
    fn script_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cloudime-script-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `cloudime.caret()` 只是置标记，Server 取一次就清。
    #[test]
    fn caret_requests_are_taken_once() {
        let runtime = Runtime::none();
        assert!(!runtime.take_caret_request(), "一开始没有请求");
        runtime.lua().load("cloudime.caret()").exec().unwrap();
        assert!(runtime.take_caret_request(), "脚本请求过");
        assert!(!runtime.take_caret_request(), "取过就清");
    }

    /// `install_path` 只认安装目录里的文件：存在的给绝对路径，越界的报错，不存在的给 nil。
    #[test]
    fn install_paths_stay_inside_the_install_directory() {
        let dir = script_dir("install-path");
        let images = dir.join("Images");
        std::fs::create_dir_all(&images).unwrap();
        std::fs::write(images.join("icon.svg"), "<svg/>").unwrap();
        let outside = std::env::temp_dir().join("cloudime-script-install-outside.txt");
        std::fs::write(&outside, "x").unwrap();

        let runtime = Runtime::none();
        runtime.set_install_dir(dir.clone());
        let root: String = runtime
            .lua()
            .load("return cloudime.install_dir()")
            .eval()
            .unwrap();
        assert_eq!(Path::new(&root), dir.as_path());
        let found: Option<String> = runtime
            .lua()
            .load(r#"return cloudime.install_path("Images/icon.svg")"#)
            .eval()
            .unwrap();
        let found = PathBuf::from(found.expect("安装目录里的文件该找得到"));
        assert_eq!(found.file_name().unwrap(), "icon.svg");
        assert!(found.is_absolute());

        let missing: Option<String> = runtime
            .lua()
            .load(r#"return cloudime.install_path("Images/missing.svg")"#)
            .eval()
            .unwrap();
        assert!(missing.is_none());
        assert!(
            runtime
                .lua()
                .load(r#"return cloudime.install_path("../cloudime-script-install-outside.txt")"#)
                .eval::<Option<String>>()
                .is_err(),
            "跑出安装目录该报错"
        );
        let _ = std::fs::remove_file(outside);
    }

    /// 一个合法清单：测试脚本都用它（`write_script` 自动加在最前面）。
    /// **不写 `name`** —— 于是脚本名取文件名，`scripts()` 的断言还能一眼看出是哪个脚本。
    const MANIFEST: &str = "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'a', 'b' }, budget = 1000000, timeout = 10000, \
                             sync = false, handover = 'callback', on_error = false }\n";

    /// 写一个脚本文件（自动在最前面补一份合法清单）。
    fn write_script(dir: &Path, name: &str, source: &str) {
        std::fs::write(dir.join(name), format!("{MANIFEST}{source}")).unwrap();
    }

    /// 写一个**原样**的脚本文件（自己带清单，或者故意不带 —— 测「判无效」用）。
    fn write_raw_script(dir: &Path, name: &str, source: &str) {
        std::fs::write(dir.join(name), source).unwrap();
    }

    /// 载荷：一个空表 + 可选的 `vk`；不给 `vk` 时用测试清单里声明的那个键（`0x41` = `a`，
    /// 修饰键全没按 —— 与清单的 `combination_modifiers = 'none'` 对上）。
    fn payload(runtime: &Runtime, vk: Option<u32>) -> Table {
        let payload = runtime.lua().create_table().unwrap();
        payload.set("vk", vk.unwrap_or(0x41)).unwrap();
        payload
    }

    /// 一个**没有** `vk` 的载荷（正常派发一定带 `vk`，这条测的是「缺了就当没命中」）。
    fn payload_without_vk(runtime: &Runtime) -> Table {
        runtime.lua().create_table().unwrap()
    }

    #[test]
    fn the_standard_library_is_available() {
        // 定调：给脚本完整的标准库（io / os / package 都在），脚本是用户自己的本机程序。
        let runtime = Runtime::none();
        for name in ["io", "os", "string", "table", "math", "package"] {
            let is_table: bool = runtime
                .lua()
                .load(format!("type({name}) == 'table'"))
                .eval()
                .unwrap();
            assert!(is_table, "{name} 不在");
        }
    }

    #[test]
    fn scripts_register_handlers_and_receive_events() {
        let dir = script_dir("handlers");
        write_script(
            &dir,
            "counter.lua",
            "hits = 0\n\
             cloudime.on('key', function(event)\n\
                 hits = hits + 1\n\
                 if event.vk == 65 then hits = hits + 10 end\n\
                 return {}\n\
             end)\n\
             cloudime.on('key', function() hits = hits + 100 return {} end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        assert_eq!(runtime.scripts().to_vec(), ["counter.lua"]);

        // 没人登记的事件：一个都不调。
        assert_eq!(
            runtime
                .dispatch("mode", payload(&runtime, None), None)
                .len(),
            0
        );
        assert_eq!(runtime.lua().globals().get::<i64>("hits").unwrap(), 0);

        // 登记过的：两个处理函数都收到，收到的载荷就是派发出去的那张表。
        assert_eq!(
            runtime
                .dispatch("key", payload(&runtime, Some(65)), None)
                .len(),
            2
        );
        assert_eq!(runtime.lua().globals().get::<i64>("hits").unwrap(), 111);
        assert_eq!(
            runtime
                .dispatch("key", payload(&runtime, Some(66)), None)
                .len(),
            2
        );
        assert_eq!(runtime.lua().globals().get::<i64>("hits").unwrap(), 212);
    }

    /// 处理函数返回的表按登记顺序原样递回来（约定之外的东西跳过）。
    #[test]
    fn handler_return_values_come_back_in_order() {
        let dir = script_dir("returns");
        write_script(
            &dir,
            "answer.lua",
            "cloudime.on('key', function() return { commit = '一' } end)\n\
             cloudime.on('key', function() return nil end)\n\
             cloudime.on('key', function() return 42 end)\n\
             cloudime.on('key', function() return { commit = '二', passthrough = true } end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        let responses = runtime.dispatch("key", payload(&runtime, None), None);
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0].get::<String>("commit").unwrap(), "一");
        assert_eq!(responses[1].get::<String>("commit").unwrap(), "二");
        assert!(responses[1].get::<bool>("passthrough").unwrap());
    }

    #[test]
    fn a_broken_script_is_skipped_without_stopping_the_others() {
        let dir = script_dir("broken");
        write_script(&dir, "a-broken.lua", "error('加载期就炸')\n");
        write_script(&dir, "b-fine.lua", "loaded = 1\n");
        let runtime = Runtime::load(&dir, &[]);
        // 只有后面那个进了脚本表，前面那个跳过了
        assert_eq!(runtime.scripts().to_vec(), ["b-fine.lua"]);
        assert_eq!(runtime.lua().globals().get::<i64>("loaded").unwrap(), 1);
    }

    #[test]
    fn a_handler_that_throws_does_not_stop_the_others() {
        let dir = script_dir("handler-throws");
        write_script(
            &dir,
            "boom.lua",
            "cloudime.on('key', function() error('派发期炸') end)\n",
        );
        write_script(
            &dir,
            "fine.lua",
            "cloudime.on('key', function() seen = (seen or 0) + 1 return {} end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        // 只收到一张返回的表：炸的那个报错跳过了，后面的照收
        assert_eq!(
            runtime.dispatch("key", payload(&runtime, None), None).len(),
            1
        );
        // 炸的那个跳过，后面的照收
        assert_eq!(runtime.lua().globals().get::<i64>("seen").unwrap(), 1);
    }

    #[test]
    fn a_missing_directory_loads_nothing() {
        let dir = std::env::temp_dir().join("cloudime-script-does-not-exist");
        let _ = std::fs::remove_dir_all(&dir);
        let runtime = Runtime::load(&dir, &[]);
        assert!(runtime.scripts().is_empty());
        assert_eq!(
            runtime.dispatch("key", payload(&runtime, None), None).len(),
            0
        );
    }

    #[test]
    fn only_lua_files_are_loaded() {
        let dir = script_dir("only-lua");
        write_script(&dir, "keep.lua", "kept = 1\n");
        write_script(&dir, "notes.txt", "不是脚本\n");
        let runtime = Runtime::load(&dir, &[]);
        assert_eq!(runtime.scripts().to_vec(), ["keep.lua"]);
    }

    /// 派发方能先问「有没有人关心这类事件」，也不用拼载荷；`cloudime.context` 是每次都刷新的光标前文。
    #[test]
    fn handlers_can_be_queried_and_context_is_visible() {
        let dir = script_dir("context");
        write_script(
            &dir,
            "read.lua",
            "cloudime.on('key', function() seen = cloudime.context return {} end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        assert!(runtime.has_handlers("key"));
        assert!(!runtime.has_handlers("candidates"));

        // 一次都没派发过时是空串，不是 nil：脚本不用先判断
        let cloudime: Table = runtime.lua().globals().get("cloudime").unwrap();
        assert_eq!(cloudime.get::<String>("context").unwrap(), "");

        runtime.set_context("已经输入的");
        runtime.dispatch("key", payload(&runtime, None), None);
        assert_eq!(
            runtime.lua().globals().get::<String>("seen").unwrap(),
            "已经输入的"
        );
    }

    /// 一次只回一次的最小本机 HTTP 服务（回环，不联网），`times` 次请求后关掉。
    fn one_shot_server(body: &'static str, times: usize) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..times {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = [0u8; 512];
                let _ = std::io::Read::read(&mut stream, &mut request);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
            }
        });
        format!("http://{address}/")
    }

    /// HTTP 结果在下次 `poll_requests` 时交给回调（第二个参数是那一刻的候选）；
    /// **超过脚本给的时间限制才回来的收信一律丢掉**。
    #[test]
    fn http_results_reach_the_callback_and_late_ones_are_dropped() {
        let dir = script_dir("http");
        let address = one_shot_server("你好，脚本", 2);
        write_script(
            &dir,
            "fetch.lua",
            &format!(
                "cloudime.on('startup', function()\n\
                     cloudime.http_get([[{address}]], 5000, function(result, list)\n\
                         got = result.status .. '/' .. result.body\n\
                         return {{}}\n\
                     end)\n\
                     cloudime.http_get([[{address}]], 1, function()\n\
                         late = 'called'\n\
                     end)\n\
                 end)\n"
            ),
        );
        let runtime = Runtime::load(&dir, &[]);
        let payload = runtime.lua().create_table().unwrap();
        runtime.dispatch("startup", payload, None);

        // 先等一会儿再收：时限 1ms 的那个早过期了
        std::thread::sleep(Duration::from_millis(200));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let candidates = runtime.lua().create_table().unwrap();
            if !runtime.poll_requests(candidates).is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            runtime.lua().globals().get::<String>("got").unwrap(),
            "200/你好，脚本"
        );
        assert!(
            runtime
                .lua()
                .globals()
                .get::<Option<String>>("late")
                .unwrap()
                .is_none(),
            "超时的结果不该调回调"
        );
    }

    /// 响应时间限制**必给**：不给 / 给 0 报错（脚本那边看得见），请求不发。
    #[test]
    fn http_get_requires_a_timeout() {
        let runtime = Runtime::none();
        let error = runtime
            .lua()
            .load("cloudime.http_get('http://127.0.0.1:1/', 0, function() end)")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("响应时间限制"), "{error}");
        assert!(!runtime.has_pending_requests());
    }

    /// 回一次固定内容的本机服务，并且把收到的整个请求记下来（验 POST 的体到没到）。
    fn one_shot_server_capturing(
        body: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<String>>) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let recorder = seen.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            // 先把请求头读掉，再按 Content-Length 读体（POST 的体就在后面）
            let mut head = [0u8; 2048];
            let read = std::io::Read::read(&mut stream, &mut head).unwrap_or(0);
            let bytes = &head[..read];
            let text = String::from_utf8_lossy(bytes);
            let length: usize = text
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse().unwrap_or(0))
                })
                .unwrap_or(0);
            // 请求头结束的位置（查的是 ASCII 那一段，偏移与字节一一对应；找不到就当整段都读完了）
            let header_end = text
                .find("\r\n\r\n")
                .map(|at| at + 4)
                .unwrap_or(read)
                .min(read);
            let mut rest = bytes[header_end..].to_vec();
            while rest.len() < length {
                let mut buffer = [0u8; 512];
                match std::io::Read::read(&mut stream, &mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => rest.extend_from_slice(&buffer[..count]),
                }
            }
            *recorder.lock().unwrap() = format!("{text}\n{}", String::from_utf8_lossy(&rest));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        });
        (format!("http://{address}/"), seen)
    }

    /// `http_post`：options 里必须给时限、表头名字不合法当场报错、真发一次时体和表头都到得了服务端。
    #[test]
    fn http_post_needs_options_and_delivers_the_result() {
        // 1) options 里没给 timeout_ms：报错、请求不发
        let runtime = Runtime::none();
        let error = runtime
            .lua()
            .load("cloudime.http_post('http://127.0.0.1:1/', '{}', {}, function() end)")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("timeout_ms"), "{error}");

        // 2) 表头名字不合法：当场报错（不给后台线程 panic 的机会）
        let error = runtime
            .lua()
            .load(
                "cloudime.http_post('http://127.0.0.1:1/', '{}', { timeout_ms = 1000, \
                 headers = { ['坏 名字'] = 'x' } }, function() end)",
            )
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("表头名字不合法"), "{error}");
        assert!(!runtime.has_pending_requests());

        // 3) 真发一次（打本机那个一次性服务，不联网）：POST 体与表头到得了服务端，结果经回调回来
        let (address, seen) = one_shot_server_capturing("pong");
        let dir = script_dir("http-post");
        write_script(
            &dir,
            "post.lua",
            &format!(
                "local address = [[{address}]]\n\
                 local body = [[{body}]]\n\
                 cloudime.on('startup', function()\n\
                     cloudime.http_post(address, body, {{ timeout_ms = 5000, headers = {{ ['Content-Type'] = 'application/json' }} }}, function(result, list)\n\
                         got = tostring(result.status) .. '/' .. result.body\n\
                         return {{ notice = got }}\n\
                     end)\n\
                 end)\n",
                address = address,
                body = r#"{"a":1}"#,
            ),
        );
        let runtime = Runtime::load(&dir, &[]);
        let payload = runtime.lua().create_table().unwrap();
        runtime.dispatch("startup", payload, None);
        assert!(runtime.has_pending_requests());

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut responses = Vec::new();
        while Instant::now() < deadline {
            let candidates = runtime.lua().create_table().unwrap();
            responses = runtime.poll_requests(candidates);
            if !responses.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(responses.len(), 1, "回调返回的动作表要带回来");
        assert_eq!(
            runtime.lua().globals().get::<String>("got").unwrap(),
            "200/pong"
        );
        assert!(!runtime.has_pending_requests(), "收过一次就不再挂着");

        let request = seen.lock().unwrap().clone();
        assert!(request.starts_with("POST "), "{request}");
        assert!(
            request.contains("content-type: application/json"),
            "{request}"
        );
        assert!(request.contains(r#"{"a":1}"#), "POST 体要发出去：{request}");
    }

    /// 随包的 `Scripts/lib/md5.lua`（用户脚本拿它算签名）：拿 RFC 1321 的向量与小牛翻译文档里的例子回归。
    /// 这份文件随包发布、用户直接 `dofile` 它 —— 写错了签名就全错，所以放在这儿当用例。
    #[test]
    fn the_shipped_lua_md5_matches_the_known_vectors() {
        let dir = script_dir("md5-lib");
        // 装到用户机器上时它在安装目录的 `Scripts\lib\`；这里指仓库里那份随包文件
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Scripts/lib/md5.lua");
        // 80 位数字（RFC 1321 的向量之一，跨 55 / 56 / 64 字节的填充边界）
        const DIGITS: &str =
            "12345678901234567890123456789012345678901234567890123456789012345678901234567890";
        // 小牛翻译文档里那一组参数拼出来的签名串
        const SIGNED: &str = "apikey=a3a5c0e35bd5382fd85e9efb30c6d218&appId=YcW1740708102939&from=zh&srcText=欢迎使用我们的翻译系统。&timestamp=1689564909&to=en";
        write_script(
            &dir,
            "check.lua",
            &format!(
                "local md5 = dofile([[{path}]]).hex\n\
                 cloudime.on('startup', function()\n\
                     empty = md5('')\n\
                     abc = md5('abc')\n\
                     digest = md5('message digest')\n\
                     digits = md5('{DIGITS}')\n\
                     niu = md5('{SIGNED}')\n\
                 end)\n"
            ),
        );
        let runtime = Runtime::load(&dir, &[]);
        let payload = runtime.lua().create_table().unwrap();
        runtime.dispatch("startup", payload, None);
        let got = |name: &str| runtime.lua().globals().get::<String>(name).unwrap();

        assert_eq!(got("empty"), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(got("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(got("digest"), "f96b697d7cb7938d525a2f31aaf161d0");
        assert_eq!(got("digits"), "57edf4a22be3c955ac49da2e2107b67a");
        // 与文档标着「最终的传参」的那个 authStr 一致（也是我们 Rust 侧那份实现算出来的值）
        assert_eq!(got("niu"), "3b116d5dc2eb50b0de4ff043b4bddd1d");
    }

    /// `cloudime.candidate.redraw()`：置一次「请求重画」标记，取一次就清（Server 据此重算这一屏）。
    #[test]
    fn redraw_requests_are_taken_once() {
        let dir = script_dir("redraw");
        write_script(
            &dir,
            "ask.lua",
            "cloudime.on('startup', function()\n\
                 cloudime.candidate.redraw()\n\
                 cloudime.candidate.redraw()\n\
             end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        assert!(!runtime.take_redraw_request(), "还没派发，不该有请求");

        let payload = runtime.lua().create_table().unwrap();
        runtime.dispatch("startup", payload, None);
        assert!(runtime.take_redraw_request(), "脚本要过重画");
        assert!(!runtime.take_redraw_request(), "取过一次就清");
    }

    /// `cloudime.candidate.set_*`：三项记下来（取一次就清）、越界夹取、`nil` 是恢复默认。
    #[test]
    fn candidate_size_requests_are_collected() {
        let dir = script_dir("candidate-size");
        write_script(
            &dir,
            "ask.lua",
            "cloudime.on('startup', function()\n\
                 cloudime.candidate.set_min_width(320)\n\
                 cloudime.candidate.set_page_size(99)\n\
                 cloudime.candidate.set_scale(0.1)\n\
             end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        let payload = runtime.lua().create_table().unwrap();
        runtime.dispatch("startup", payload, None);
        let request = runtime.take_size_request();
        assert_eq!(request.min_width, Some(Some(320.0)));
        assert_eq!(request.page_size, Some(Some(9)), "99 该夹到上界 9");
        assert!(
            request.scale.unwrap().unwrap() > 0.5,
            "0.1 该夹到下界（1.2^-3 左右）"
        );
        assert!(runtime.take_size_request().is_empty(), "取过一次就清");

        // 不带参数（`nil`）= 恢复默认
        let dir = script_dir("candidate-size-reset");
        write_script(
            &dir,
            "ask.lua",
            "cloudime.on('startup', function()\n\
                 cloudime.candidate.set_min_width()\n\
                 cloudime.candidate.set_page_size()\n\
                 cloudime.candidate.set_scale()\n\
             end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        let payload = runtime.lua().create_table().unwrap();
        runtime.dispatch("startup", payload, None);
        let request = runtime.take_size_request();
        assert_eq!(request.min_width, Some(None));
        assert_eq!(request.page_size, Some(None));
        assert_eq!(request.scale, Some(None));
    }

    /// `cloudime.ui.measure` / `measure_tip`：调 Server 装进来的量尺，返回 `{ width, height }`；
    /// 字体名不认识、或这次运行没装量尺，都报错。
    #[test]
    fn ui_measure_uses_the_installed_ruler() {
        let dir = script_dir("ui-measure");
        write_script(
            &dir,
            "ask.lua",
            "cloudime.on('startup', function()\n\
                 local box = cloudime.ui.measure('hello')\n\
                 got = box.width .. 'x' .. box.height\n\
                 tip_font = tostring(cloudime.ui.measure_tip('译文').width)\n\
                 local ok, err = pcall(function() cloudime.ui.measure('x', 'nope') end)\n\
                 bad_font = tostring(err)\n\
             end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        // 装一个假量尺：每个字符 10 点宽、20 点高；顺便记下都用到了哪个字体
        let seen: Rc<RefCell<Vec<(String, MeasureFont)>>> = Rc::new(RefCell::new(Vec::new()));
        let recorder = seen.clone();
        runtime.set_measure(Rc::new(move |text: &str, font: MeasureFont| {
            recorder.borrow_mut().push((text.to_owned(), font));
            Some((text.chars().count() as f32 * 10.0, 20.0))
        }));
        let payload = runtime.lua().create_table().unwrap();
        runtime.dispatch("startup", payload, None);

        assert_eq!(
            runtime.lua().globals().get::<String>("got").unwrap(),
            "50x20"
        );
        assert_eq!(
            runtime.lua().globals().get::<String>("tip_font").unwrap(),
            "20"
        );
        assert!(
            seen.borrow()
                .contains(&("译文".to_owned(), MeasureFont::Translate)),
            "measure_tip 该用「翻译 Tip / 在线那一行」的字体"
        );
        assert!(
            runtime
                .lua()
                .globals()
                .get::<String>("bad_font")
                .unwrap()
                .contains("字体名不认识")
        );

        // 没装量尺（比如没走渲染器）：报「量不了」
        let bare = Runtime::none();
        let error = bare
            .lua()
            .load("cloudime.ui.measure('x')")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("量不了"), "{error}");
    }

    /// `cloudime.get_time()`：一张本地时间的表，字段齐全、取值在合理范围内。
    #[test]
    fn get_time_reports_the_local_clock() {
        let runtime = Runtime::none();
        let lua = runtime.lua();
        let (year, month, day, hour, minute, second, weekday, unix): (
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = lua
            .load(
                "local t = cloudime.get_time()\n\
                 return t.year, t.month, t.day, t.hour, t.minute, t.second, t.weekday, t.unix",
            )
            .eval()
            .unwrap();
        // 不写死时间：只校验形状与范围（什么时候跑都成立）
        assert!((2020..2100).contains(&year), "year={year}");
        assert!((1..=12).contains(&month), "month={month}");
        assert!((1..=31).contains(&day), "day={day}");
        assert!((0..=23).contains(&hour), "hour={hour}");
        assert!((0..=59).contains(&minute), "minute={minute}");
        assert!((0..=60).contains(&second), "second={second}");
        assert!((1..=7).contains(&weekday), "weekday={weekday}");
        // Unix 秒：晚于 2020-01-01
        assert!(unix > 1_577_836_800, "unix={unix}");
    }

    /// `cloudime.apply_theme`：留一个请求给 Server，取一次就清；`false` 是「去掉这次设的主题」。
    #[test]
    fn apply_theme_leaves_one_request_for_the_host() {
        let runtime = Runtime::none();
        let lua = runtime.lua();
        assert!(runtime.take_theme_request().is_none(), "还没调过就没有请求");
        lua.load("cloudime.apply_theme('Panic.json')")
            .exec()
            .unwrap();
        assert_eq!(
            runtime.take_theme_request(),
            Some(Some("Panic.json".to_owned()))
        );
        assert!(runtime.take_theme_request().is_none(), "取过就清");
        // `false` = 回配置里的主题
        lua.load("cloudime.apply_theme(false)").exec().unwrap();
        assert_eq!(runtime.take_theme_request(), Some(None));
        // 类型不对当场报错（别静默什么都不做）
        let error = lua
            .load("cloudime.apply_theme(42)")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("apply_theme"), "{error}");
    }

    /// `combination_key` + `keys`：解析成虚拟键码、汇总出 `wanted_keys`；`needs_context = true`
    /// 才让 DLL 在按键前现读光标前文（`wants_surrounding_text`）。
    #[test]
    fn combination_keys_are_collected_for_the_dll() {
        let dir = script_dir("keys-trigger");
        std::fs::write(
            dir.join("auto-number.lua"),
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'Enter', 'tab' }, needs_context = true, \
             budget = 1000000, timeout = 10000, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function(event) return {} end)\n",
        )
        .unwrap();
        let runtime = Runtime::load(&dir, &[]);
        assert_eq!(runtime.wanted_keys(), vec![0x09, 0x0D], "键名 → 虚拟键码");
        assert!(
            runtime.wants_surrounding_text(),
            "声明了 needs_context 就要现读光标前文"
        );
        assert!(runtime.has_handlers("key"));

        // 键名不认识 → 判无效
        let dir = script_dir("keys-bad");
        std::fs::write(
            dir.join("bad.lua"),
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'nope' }, \
             budget = 1000000, timeout = 10000, sync = true, handover = 'return', on_error = false }\n",
        )
        .unwrap();
        let runtime = Runtime::load(&dir, &[]);
        assert!(runtime.scripts().is_empty(), "键名不认识应该判无效");
        assert!(runtime.wanted_keys().is_empty());

        // 没声明 needs_context 的脚本不读前文
        assert!(!Runtime::none().wants_surrounding_text());
    }

    /// 组合键脚本只收清单声明的那一组（修饰键 + 键）：组句里 Server 会派发**所有**按键，
    /// 别的键不该打扰它。
    #[test]
    fn a_combination_trigger_only_receives_its_own_keys() {
        let dir = script_dir("key-filter");
        std::fs::write(
            dir.join("only-enter.lua"),
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'enter' }, \
             budget = 1000000, timeout = 10000, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return {} end)\n",
        )
        .unwrap();
        let runtime = Runtime::load(&dir, &[]);

        // Enter 派得到（`payload` 的第一个参数就是 `vk`）
        assert_eq!(
            runtime
                .dispatch("key", payload(&runtime, Some(0x0D)), None)
                .len(),
            1
        );
        // 别的键（组句里 Server 也会派发）派不到
        assert_eq!(
            runtime
                .dispatch("key", payload(&runtime, Some(0x41)), None)
                .len(),
            0
        );
        // 载荷里没有 `vk` 的更派不到（正常派发一定有）
        assert_eq!(
            runtime
                .dispatch("key", payload_without_vk(&runtime), None)
                .len(),
            0
        );
    }

    /// `cloudime.foreground_app`：走 Server 装的接口；没装时报错。
    #[test]
    fn foreground_app_goes_through_the_host() {
        let runtime = Runtime::none();
        let error = runtime
            .lua()
            .load("cloudime.foreground_app()")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("foreground_app"), "{error}");

        let app: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        runtime.set_app_hook({
            let app = app.clone();
            Rc::new(move || app.borrow().clone())
        });
        // 没聚焦任何程序 → nil
        assert!(
            runtime
                .lua()
                .load("return cloudime.foreground_app() == nil")
                .eval::<bool>()
                .unwrap()
        );
        *app.borrow_mut() = Some("notepad.exe".to_owned());
        assert_eq!(
            runtime
                .lua()
                .load("return cloudime.foreground_app()")
                .eval::<String>()
                .unwrap(),
            "notepad.exe"
        );
    }

    /// `cloudime.get_curr_config`：走 Server 装的接口；没装时报错。
    #[test]
    fn get_curr_config_goes_through_the_host() {
        let runtime = Runtime::none();
        let error = runtime
            .lua()
            .load("cloudime.get_curr_config()")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("get_curr_config"), "{error}");

        runtime.set_config_hook(Rc::new(|lua| {
            let table = lua.create_table()?;
            table.set("候选数", 7)?;
            Ok(table)
        }));
        let count: i64 = runtime
            .lua()
            .load("return cloudime.get_curr_config()['候选数']")
            .eval()
            .unwrap();
        assert_eq!(count, 7);
    }

    /// `cloudime.clipboard.settext` / `gettext`：走 Server 装的两条路；没装接口报错。
    #[test]
    fn clipboard_goes_through_the_host() {
        let runtime = Runtime::none();
        let slot: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        let setter = slot.clone();
        runtime.set_clipboard(
            Rc::new(move |text: &str| {
                *setter.borrow_mut() = text.to_owned();
                Ok(())
            }),
            {
                let slot = slot.clone();
                Rc::new(move || Ok((!slot.borrow().is_empty()).then(|| slot.borrow().clone())))
            },
        );
        let lua = runtime.lua();
        lua.load("cloudime.clipboard.settext('云朵 abc')")
            .exec()
            .unwrap();
        assert_eq!(*slot.borrow(), "云朵 abc");
        assert_eq!(
            lua.load("return cloudime.clipboard.gettext()")
                .eval::<String>()
                .unwrap(),
            "云朵 abc"
        );
        // 剪贴板里没有文本 → nil
        slot.borrow_mut().clear();
        assert!(
            lua.load("return cloudime.clipboard.gettext() == nil")
                .eval::<bool>()
                .unwrap()
        );

        // 没装接口（比如没走 Server）：报错
        let bare = Runtime::none();
        let error = bare
            .lua()
            .load("cloudime.clipboard.gettext()")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("用不了"), "{error}");
    }

    /// `cloudime.keyboard.press`：把组合键交给 Server；没装接口报错。
    #[test]
    fn keyboard_presses_go_through_the_host() {
        let runtime = Runtime::none();
        let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let recorder = seen.clone();
        runtime.set_keyboard(Rc::new(move |chord| {
            recorder.borrow_mut().push(chord.to_owned());
            Ok(())
        }));
        runtime
            .lua()
            .load("cloudime.keyboard.press('ctrl+c')")
            .exec()
            .unwrap();
        assert_eq!(*seen.borrow(), ["ctrl+c"]);

        let bare = Runtime::none();
        let error = bare
            .lua()
            .load("cloudime.keyboard.press('ctrl+c')")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("用不了"), "{error}");
    }

    /// `cloudime.text.all` / `before`：问 Server 要快照；没装接口报错、这一拍拿不到给 `nil`。
    #[test]
    fn text_all_and_before_ask_the_host() {
        let runtime = Runtime::none();
        let seen: Rc<RefCell<Vec<(TextRange, u64)>>> = Rc::new(RefCell::new(Vec::new()));
        let recorder = seen.clone();
        runtime.set_text_hook(Rc::new(move |range, limit| {
            recorder.borrow_mut().push((range, limit));
            match range {
                TextRange::All => Some(("整篇".to_owned(), true)),
                TextRange::Before => None,
                TextRange::After => Some(("之后".to_owned(), false)),
            }
        }));
        let lua = runtime.lua();

        let all = lua
            .load("local box = cloudime.text.all()\nreturn box.text .. '|' .. tostring(box.truncated)")
            .eval::<String>()
            .unwrap();
        assert_eq!(all, "整篇|true");
        assert_eq!(
            seen.borrow()[0],
            (TextRange::All, u64::MAX),
            "缺省上限是 2^64-1"
        );

        assert!(
            lua.load("return cloudime.text.before(20) == nil")
                .eval::<bool>()
                .unwrap(),
            "拿不到该给 nil"
        );
        assert_eq!(seen.borrow()[1], (TextRange::Before, 20));

        let after = lua
            .load("local box = cloudime.text.after()\nreturn box.text .. '|' .. tostring(box.truncated)")
            .eval::<String>()
            .unwrap();
        assert_eq!(after, "之后|false");
        assert_eq!(seen.borrow()[2], (TextRange::After, u64::MAX));

        // 没装接口（比如没走 Server）：报错
        let bare = Runtime::none();
        let error = bare
            .lua()
            .load("cloudime.text.all()")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("拿不到文本"), "{error}");
    }

    /// `cloudime.ui.truncate`：补「…」与折行两种模式，都按点宽（这里用假量尺：每个字 10 点）。
    #[test]
    fn ui_truncate_ellipsizes_or_wraps() {
        let runtime = Runtime::none();
        runtime.set_measure(Rc::new(|text: &str, _font: MeasureFont| {
            Some((text.chars().count() as f32 * 10.0, 20.0))
        }));
        let lua = runtime.lua();

        // 放得下：原样给回来
        let fits = lua
            .load("return cloudime.ui.truncate('abc', 50).text")
            .eval::<String>()
            .unwrap();
        assert_eq!(fits, "abc");

        // 放不下：从尾部去字补「…」（10 点一个字，50 点正好放 "abcd…"）
        let cut = lua
            .load(
                "local box = cloudime.ui.truncate('abcdefgh', 50)\n\
                 return box.text .. '|' .. box.lines .. '|' .. tostring(box.truncated)",
            )
            .eval::<String>()
            .unwrap();
        assert_eq!(cut, "abcd…|1|true");

        // 折行：一行 3 个字、最多 2 行，剩下的在末行补「…」
        let wrapped = lua
            .load(
                "local box = cloudime.ui.truncate('abcdefghij', 30, { mode = 'wrap', max_lines = 2 })\n\
                 return box.text .. '|' .. box.lines .. '|' .. tostring(box.truncated)",
            )
            .eval::<String>()
            .unwrap();
        assert_eq!(wrapped, "abc\nde…|2|true");

        // mode / 字体名写错：当场报错，脚本自己看得出来
        let bad_mode = lua
            .load("return cloudime.ui.truncate('x', 50, { mode = 'nope' })")
            .eval::<mlua::Value>()
            .unwrap_err()
            .to_string();
        assert!(bad_mode.contains("mode 不认识"), "{bad_mode}");
        let bad_font = lua
            .load("return cloudime.ui.truncate('x', 50, { font = 'nope' })")
            .eval::<mlua::Value>()
            .unwrap_err()
            .to_string();
        assert!(bad_font.contains("字体名不认识"), "{bad_font}");
    }

    /// `cloudime.candidate.width()`：读到 Server 刷进来的候选窗宽度（点）；没刷过就是 `nil`。
    #[test]
    fn candidate_width_reports_the_viewport() {
        let runtime = Runtime::none();
        let lua = runtime.lua();
        assert!(
            lua.load("return cloudime.candidate.width() == nil")
                .eval::<bool>()
                .unwrap(),
            "没刷过该给 nil"
        );

        assert!(
            lua.load("return cloudime.candidate.height() == nil")
                .eval::<bool>()
                .unwrap(),
            "没刷过该给 nil"
        );

        runtime.set_viewport(Some((312.0, 140.0)));
        assert_eq!(
            lua.load("return cloudime.candidate.width()")
                .eval::<f32>()
                .unwrap(),
            312.0
        );
        assert_eq!(
            lua.load("return cloudime.candidate.height()")
                .eval::<f32>()
                .unwrap(),
            140.0
        );
    }

    /// 指令预算：处理函数里死循环被中止，其余照常、状态还能接着用。
    /// 这一道闸防的是「脚本把整条工人线程堵死 → 所有应用都打不了字」。
    #[test]
    fn a_runaway_handler_is_aborted() {
        let dir = script_dir("runaway");
        write_script(
            &dir,
            "loop.lua",
            "cloudime.on('key', function() while true do end end)\n\
             cloudime.on('key', function() fine = 'ok' return {} end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        // 死循环被中止（记日志），下一个处理函数照常返回
        let responses = runtime.dispatch("key", payload(&runtime, None), None);
        assert_eq!(responses.len(), 1);
        assert_eq!(runtime.lua().globals().get::<String>("fine").unwrap(), "ok");
    }

    /// 会退掉 Server 或堵死线程的那几样被换掉了：`os.exit` 不退、`os.execute` 只启动、
    /// `io.popen` 报错，脚本摘不掉预算钩子。
    #[test]
    fn the_dangerous_stdlib_entries_are_replaced() {
        let runtime = Runtime::none();
        // os.exit 被换掉：真让它执行，这个测试进程就没了
        runtime.lua().load("os.exit(0)").exec().unwrap();
        assert!(runtime.lua().load("after = 1").exec().is_ok());
        assert_eq!(runtime.lua().globals().get::<i64>("after").unwrap(), 1);

        // io.popen 直接报错，并指向 http_get
        let error = runtime
            .lua()
            .load("io.popen('curl example.com')")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("http_get"), "{error}");

        // 摘不掉钩子：`debug` 库根本没开（mlua 的 `Lua::new()` 只开安全子集），
        // 脚本连 `debug.sethook` 都没有。钩子本身由 `a_runaway_handler_is_aborted` 行为验证。
        let no_debug: bool = runtime.lua().load("debug == nil").eval().unwrap();
        assert!(no_debug, "debug 库不该开着（脚本因此摘不掉预算钩子）");

        // 不给协程：`create` / `wrap` 直接报错，脚本没法绕过预算（见 `docs/design/script.md`）
        for snippet in [
            "coroutine.create(function() end)",
            "coroutine.wrap(function() end)",
        ] {
            let error = runtime.lua().load(snippet).exec().unwrap_err().to_string();
            assert!(error.contains("协程"), "{snippet} → {error}");
        }

        // 标准输入关掉：`io.read()` 不会去等输入（Server 由控制台启动时那会堵住工人线程）
        let no_stdin: bool = runtime.lua().load("io.stdin == nil").eval().unwrap();
        assert!(no_stdin, "io.stdin 该关掉");
    }

    /// 没有清单 = 无效：脚本不加载、处理函数也不注册。
    #[test]
    fn a_script_without_a_manifest_is_invalid() {
        let dir = script_dir("no-manifest");
        write_raw_script(
            &dir,
            "bare.lua",
            "cloudime.on('key', function() ran = true end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        assert!(runtime.scripts().is_empty(), "缺清单该判无效");
        assert_eq!(
            runtime.dispatch("key", payload(&runtime, None), None).len(),
            0
        );
        assert!(
            runtime
                .lua()
                .globals()
                .get::<Option<bool>>("ran")
                .unwrap()
                .is_none()
        );
    }

    /// 清单字段不合法 = 无效：`sync` 与 `handover` 必须一致。
    #[test]
    fn a_mismatched_manifest_is_invalid() {
        let dir = script_dir("mismatch");
        write_raw_script(
            &dir,
            "bad.lua",
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'a' }, timeout = 100, sync = true, handover = 'callback', on_error = false }\n\
             cloudime.on('key', function() ran = true end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        assert!(
            runtime.scripts().is_empty(),
            "sync 与 handover 对不上该判无效"
        );
        assert_eq!(
            runtime.dispatch("key", payload(&runtime, None), None).len(),
            0
        );
    }

    /// `apps`：只有清单里列到的应用才跑（大小写不敏感）；给 `None`（`startup` / HTTP 回调）不过滤。
    #[test]
    fn a_script_only_runs_in_the_apps_it_declares() {
        let dir = script_dir("apps");
        write_raw_script(
            &dir,
            "notepad.lua",
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'a' }, timeout = 100, apps = { 'NotePad.EXE' }, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return {} end)\n\
             cloudime.on('startup', function() return {} end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        assert_eq!(
            runtime
                .dispatch("key", payload(&runtime, None), Some("notepad.exe"))
                .len(),
            1
        );
        assert_eq!(
            runtime
                .dispatch("key", payload(&runtime, None), Some("other.exe"))
                .len(),
            0
        );
        assert_eq!(
            runtime
                .dispatch("startup", payload(&runtime, None), None)
                .len(),
            1
        );
    }

    /// `priority`：大的排在后面，于是合并时盖住前面的。
    #[test]
    fn priority_decides_the_dispatch_order() {
        let dir = script_dir("priority");
        write_raw_script(
            &dir,
            "a.lua",
            "cloudime.script{ name = 'a', api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'a' }, timeout = 100, priority = 10, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return { notice = 'a' } end)\n",
        );
        write_raw_script(
            &dir,
            "b.lua",
            "cloudime.script{ name = 'b', api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'a' }, timeout = 100, priority = 1, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return { notice = 'b' } end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        let responses = runtime.dispatch("key", payload(&runtime, None), None);
        assert_eq!(responses.len(), 2);
        // 小的（priority = 1 的 b）在前，大的（10 的 a）在后 —— 调用方按顺序合并，后面的赢
        assert_eq!(responses[0].get::<String>("notice").unwrap(), "b");
        assert_eq!(responses[1].get::<String>("notice").unwrap(), "a");
    }

    /// 出错时先调清单里的 `on_error(event, message)`。
    #[test]
    fn the_error_callback_is_called() {
        let dir = script_dir("on-error");
        write_raw_script(
            &dir,
            "careful.lua",
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'a' }, timeout = 100, sync = true, handover = 'return', \
             on_error = function(event, message) seen = event .. '/' .. message end }\n\
             cloudime.on('key', function() error('故意炸') end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        runtime.dispatch("key", payload(&runtime, None), None);
        let seen = runtime.lua().globals().get::<String>("seen").unwrap();
        assert!(seen.starts_with("key/"), "{seen}");
        assert!(seen.contains("故意炸"), "{seen}");
    }

    /// 清单里的 `budget` 把限额收窄：小预算的脚本照样会被中止（全局上限拦不住它）。
    #[test]
    fn the_manifest_budget_narrows_the_limit() {
        let dir = script_dir("budget");
        write_raw_script(
            &dir,
            "greedy.lua",
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'a' }, budget = 1000, timeout = 2000, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() local n = 0 for _ = 1, 100000 do n = n + 1 end end)\n\
             cloudime.on('key', function() return {} end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        let responses = runtime.dispatch("key", payload(&runtime, None), None);
        // 第一个被中止，第二个照常（每次调用都重新上紧限额）
        assert_eq!(responses.len(), 1);
    }

    /// 模板与禁用的脚本都不加载：`template.lua` 按文件名跳过，`disabled` 里的按名跳过（大小写不敏感）。
    #[test]
    fn template_and_disabled_scripts_are_skipped() {
        let dir = script_dir("disabled");
        write_script(
            &dir,
            "keep.lua",
            "cloudime.on('key', function() return {} end)\n",
        );
        write_script(
            &dir,
            "off.lua",
            "cloudime.on('key', function() return {} end)\n",
        );
        // 模板：内容合法（带清单）但按文件名跳过
        std::fs::write(
            dir.join(crate::TEMPLATE_FILE),
            format!("{MANIFEST}cloudime.on('key', function() return {{}} end)\n"),
        )
        .unwrap();

        let runtime = Runtime::load(&dir, &["OFF.LUA".to_owned()]);
        assert_eq!(runtime.scripts().to_vec(), ["keep.lua"]);
        assert_eq!(
            runtime.dispatch("key", payload(&runtime, None), None).len(),
            1
        );
    }

    /// `cloudime` 表只读：脚本改它（加字段 / 换函数）直接报错。
    #[test]
    fn the_cloudime_table_is_read_only() {
        let runtime = Runtime::none();
        for snippet in ["cloudime.whatever = 1", "cloudime.log = function() end"] {
            let error = runtime.lua().load(snippet).exec().unwrap_err().to_string();
            assert!(error.contains("不许改 cloudime"), "{snippet} → {error}");
        }
    }

    /// 一个脚本判无效时，撤掉的只是**它自己**登记的函数 —— 前一个脚本的不能跟着被撤。
    #[test]
    fn an_invalid_script_does_not_retract_the_previous_script() {
        let dir = script_dir("retract");
        write_script(
            &dir,
            "a-good.lua",
            "cloudime.on('key', function() return {} end)\n",
        );
        write_raw_script(
            &dir,
            "b-bad.lua",
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'a' }, timeout = 100, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return {} end)\n\
             error('加载到一半就炸')\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        assert_eq!(runtime.scripts().to_vec(), ["a-good.lua"]);
        // a-good 的处理函数还在（b-bad 自己那个被撤掉了）
        assert_eq!(
            runtime.dispatch("key", payload(&runtime, None), None).len(),
            1
        );
    }

    /// `http_get` 的响应时间限制不能超过清单里声明的最长等候时间（规则一：开销得声明清楚）。
    #[test]
    fn http_get_cannot_wait_longer_than_the_manifest_allows() {
        let dir = script_dir("http-too-long");
        write_raw_script(
            &dir,
            "greedy.lua",
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', keys = { 'a' }, timeout = 100, sync = false, handover = 'callback', on_error = false }\n\
             cloudime.on('startup', function()\n\
                 cloudime.http_get('http://127.0.0.1:1/', 5000, function() end)\n\
             end)\n",
        );
        let runtime = Runtime::load(&dir, &[]);
        let payload = runtime.lua().create_table().unwrap();
        runtime.dispatch("startup", payload, None);
        // 请求没发出去（`http_get` 报错了），所以没有挂着的请求
        assert!(!runtime.has_pending_requests());
    }

    /// UI 层的 `text_changed` 事件先刷新脚本侧文本缓存，再进脚本处理函数。
    #[test]
    fn text_changed_events_update_the_runtime_cache() {
        let runtime = Runtime::none();
        let payload = runtime.lua().create_table().unwrap();
        payload.set("id", 7).unwrap();
        payload.set("action", "text_changed").unwrap();
        payload.set("control", "search").unwrap();
        payload.set("text", "hello").unwrap();
        runtime.dispatch("window", payload, None);
        assert_eq!(
            runtime
                .window_texts
                .borrow()
                .get(&(7, "search".to_owned()))
                .map(String::as_str),
            Some("hello")
        );
    }

    /// `cloudime.window.*`：脚本只产出请求（窗口在 UI 线程上建），句柄上的方法按调用顺序排队，
    /// `take_window_requests` 取一次就清。内容里的七类元素都读得出来。
    #[test]
    fn window_requests_are_queued_in_order_and_taken_once() {
        let dir = script_dir("window-api");
        write_script(
            &dir,
            "win.lua",
            r##"
            local win = cloudime.window.create{ name = "面板", width = 200, height = 80, interactive = true }
            win:set_content{
                { type = "rect", x = 0, y = 0, w = 200, h = 24, radius = 6, color = "#33111111" },
                { type = "text", x = 8, y = 4, text = "你好", size = 14, color = "#FF222222" },
                { type = "line", from = { 0, 24 }, to = { 200, 24 }, thickness = 1, color = "#22000000" },
                { type = "circle", cx = 40, cy = 60, radius = 10, color = "#FF00FF00" },
                { type = "path", color = "#FF0000FF", commands = { { "M", 0, 0 }, { "L", 10, 0 }, { "Z" } } },
            }
            win:show{ x = 10, y = 20 }
            local aux = cloudime.window.create_auxiliary{ owner = win, width = 120, height = 60 }
            aux:set_content{ { type = "rect", x = 0, y = 0, w = 120, h = 60, color = "#FFFFFFFF" } }
            aux:show()
            aux:focus(true)
            win:hide()
            win:close()
            "##,
        );
        let runtime = Runtime::load(&dir, &[]);
        assert!(runtime.has_window_requests());
        let requests = runtime.take_window_requests();
        assert!(!runtime.has_window_requests(), "取了就清");
        assert_eq!(
            requests.len(),
            9,
            "建 / 内容 / 显示 / 建辅助 / 内容 / 显示 / 焦点 / 收起 / 关"
        );
        match &requests[0] {
            window::Request::Create(id, spec) => {
                assert_eq!(*id, 1);
                assert_eq!(spec.name, "面板");
                assert_eq!((spec.width, spec.height), (200.0, 80.0));
                assert!(spec.interactive);
            }
            other => panic!("第一条该是建独立窗口：{other:?}"),
        }
        match &requests[1] {
            window::Request::SetContent(id, document) => {
                assert_eq!(*id, 1);
                assert_eq!(document.elements.len(), 5);
                assert_eq!((document.width, document.height), (200.0, 80.0));
            }
            other => panic!("第二条该是换内容：{other:?}"),
        }
        assert!(matches!(
            &requests[2],
            window::Request::Show(1, Some((10.0, 20.0)))
        ));
        match &requests[3] {
            window::Request::CreateAuxiliary(id, spec) => {
                assert_eq!(*id, 2);
                assert_eq!(spec.owner, 1, "owner 读的是句柄上的 id");
            }
            other => panic!("第四条该是建辅助窗口：{other:?}"),
        }
        assert!(matches!(&requests[4], window::Request::SetContent(2, _)));
        assert!(matches!(&requests[5], window::Request::Show(2, None)));
        assert!(matches!(&requests[6], window::Request::Focus(2, true)));
        assert!(matches!(&requests[7], window::Request::Hide(1)));
        assert!(matches!(&requests[8], window::Request::Close(1)));
    }

    /// `win:load_layout(path)`：运行中换布局，句柄尺寸跟着文件更新。
    #[test]
    fn load_layout_replaces_the_window_content() {
        let dir = script_dir("window-load-layout");
        let layout = dir.join("ui.json");
        std::fs::write(
            &layout,
            r##"{
                "width": 180,
                "height": 70,
                "controls": [
                    { "name": "background", "type": "rect", "x": 0, "y": 0, "w": 180, "h": 70,
                      "color": "#FF000000" }
                ]
            }"##,
        )
        .unwrap();
        let path = layout.to_string_lossy();
        write_script(
            &dir,
            "load.lua",
            &format!(
                "local win = cloudime.window.create{{ name = \"布局\", width = 100, height = 50 }}\n\
                 win:load_layout([[{path}]])\n\
                 after_width = win.width\n\
                 after_height = win.height\n"
            ),
        );
        let runtime = Runtime::load(&dir, &[]);
        let requests = runtime.take_window_requests();
        assert_eq!(requests.len(), 2, "建 / 换内容");
        match &requests[0] {
            window::Request::Create(id, spec) => {
                assert_eq!(*id, 1);
                assert_eq!((spec.width, spec.height), (100.0, 50.0));
            }
            other => panic!("第一条该是建独立窗口：{other:?}"),
        }
        match &requests[1] {
            window::Request::SetContent(id, document) => {
                assert_eq!(*id, 1);
                assert_eq!((document.width, document.height), (180.0, 70.0));
                assert_eq!(document.elements.len(), 1);
            }
            other => panic!("第二条该是换内容：{other:?}"),
        }
        let globals = runtime.lua().globals();
        assert_eq!(globals.get::<f32>("after_width").unwrap(), 180.0);
        assert_eq!(globals.get::<f32>("after_height").unwrap(), 70.0);
    }

    /// `create{ layout = path }`：JSON 布局直接成为初始内容，尺寸取布局文件。
    #[test]
    fn a_window_can_be_created_from_a_json_layout() {
        let dir = script_dir("window-layout");
        let layout = dir.join("ui.json");
        std::fs::write(
            &layout,
            r##"{
                "width": 260,
                "height": 90,
                "controls": [
                    { "name": "background", "type": "rect", "x": 0, "y": 0, "w": 260, "h": 90,
                      "radius": 8, "color": "#FFFFFFFF" },
                    { "type": "flat_svg_button", "name": "close", "x": 232, "y": 8,
                      "w": 20, "h": 20,
                      "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 20 20\"><rect width=\"20\" height=\"20\"/></svg>",
                      "events": { "mouse_clicked": "close_window" } }
                ]
            }"##,
        )
        .unwrap();
        let path = layout.to_string_lossy();
        write_script(
            &dir,
            "layout.lua",
            &format!(
                "local win = cloudime.window.create{{ name = \"布局\", layout = [[{path}]] }}\n\
                 win:show{{ x = 1, y = 2 }}\n"
            ),
        );
        let runtime = Runtime::load(&dir, &[]);
        let requests = runtime.take_window_requests();
        assert_eq!(requests.len(), 3, "建 / 内容 / 显示");
        match &requests[0] {
            window::Request::Create(id, spec) => {
                assert_eq!(*id, 1);
                assert_eq!((spec.width, spec.height), (260.0, 90.0));
            }
            other => panic!("第一条该是建独立窗口：{other:?}"),
        }
        match &requests[1] {
            window::Request::SetContent(id, document) => {
                assert_eq!(*id, 1);
                assert_eq!(document.elements.len(), 2);
                assert_eq!(document.regions.len(), 1);
                assert_eq!(
                    document.regions[0].handler("mouse_clicked"),
                    Some("close_window")
                );
            }
            other => panic!("第二条该是换内容：{other:?}"),
        }
        assert!(matches!(
            &requests[2],
            window::Request::Show(1, Some((1.0, 2.0)))
        ));
    }

    /// 内容写错（缺颜色 / 元素类型不认识）当场报错：这个脚本判无效，别的照常。
    #[test]
    fn a_broken_window_content_invalidates_only_that_script() {
        let dir = script_dir("window-broken");
        write_script(
            &dir,
            "broken.lua",
            "local win = cloudime.window.create{}\n\
             win:set_content{ { type = 'rect', x = 0, y = 0, w = 10, h = 10 } }\n",
        );
        write_script(&dir, "fine.lua", "loaded = 1\n");
        let runtime = Runtime::load(&dir, &[]);
        assert_eq!(runtime.scripts().to_vec(), ["fine.lua"]);
    }

    /// `combine_region` / `prune_region`：各排一条 `EditRegion` 请求（合并 / 剪除）。
    #[test]
    fn region_edit_methods_queue_requests() {
        let dir = script_dir("region-edit");
        write_script(
            &dir,
            "edit.lua",
            r##"
            local win = cloudime.window.create{ name = "面板", width = 120, height = 60 }
            win:combine_region("box", { x = 10, y = 10, w = 20, h = 20 })
            win:prune_region("box", { x = 5, y = 5, w = 8, h = 8, radius = 2 })
            "##,
        );
        let runtime = Runtime::load(&dir, &[]);
        let requests = runtime.take_window_requests();
        assert_eq!(requests.len(), 3, "create + combine + prune");
        match &requests[1] {
            window::Request::EditRegion {
                id: 1,
                name,
                shape,
                combine,
            } => {
                assert_eq!(name, "box");
                assert!(combine);
                assert!(matches!(shape, cloudime_render::HitShape::Rect(_)));
            }
            other => panic!("第二条该是合并：{other:?}"),
        }
        match &requests[2] {
            window::Request::EditRegion {
                combine: false,
                shape: cloudime_render::HitShape::RoundRect { .. },
                ..
            } => {}
            other => panic!("第三条该是剪除（圆角）：{other:?}"),
        }
    }

    /// `time_range`：同一天 / 跨夜两套时间段，左闭右开；窗口开始那天跨夜算昨天。
    #[test]
    fn time_ranges_cover_same_day_and_overnight() {
        let day = TimeRange {
            from: 8 * 60,
            to: 18 * 60,
        };
        assert!(day.contains(8 * 60));
        assert!(day.contains(17 * 60 + 59));
        assert!(!day.contains(18 * 60), "右端开");
        assert!(!day.contains(7 * 60 + 59));

        let night = TimeRange {
            from: 20 * 60,
            to: 6 * 60,
        };
        assert!(night.contains(20 * 60));
        assert!(night.contains(23 * 60 + 59));
        assert!(night.contains(0));
        assert!(night.contains(5 * 60 + 59));
        assert!(!night.contains(6 * 60), "右端开");
        assert!(!night.contains(19 * 60 + 59));

        let date = jiff::civil::date(2026, 10, 17);
        assert_eq!(night.window_start(date, 21 * 60), Some((2026, 10, 17)));
        assert_eq!(
            night.window_start(date, 3 * 60),
            Some((2026, 10, 16)),
            "凌晨算昨晚开的窗口"
        );
        assert_eq!(night.window_start(date, 12 * 60), None);
    }

    /// `candidate_context`：候选窗出现时，拼音串（忽略分隔符 / 大小写）相等才派。
    #[test]
    fn candidate_context_needs_the_declared_pinyin() {
        let dir = script_dir("candidate-pinyin");
        std::fs::write(
            dir.join("pinyin.lua"),
            "cloudime.script{ api = 2, trigger_condition = 'candidate_context', candidate_pinyin = 'Ni Hao', \
             budget = 1000000, timeout = 10000, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('candidates', function() return {} end)\n",
        )
        .unwrap();
        let runtime = Runtime::load(&dir, &[]);
        let with_pinyin = |text: &str| {
            let payload = runtime.lua().create_table().unwrap();
            payload.set("pinyin", text).unwrap();
            payload
        };
        assert_eq!(
            runtime
                .dispatch("candidates", with_pinyin("ni'hao"), None)
                .len(),
            1
        );
        assert_eq!(
            runtime
                .dispatch("candidates", with_pinyin("NIHAO"), None)
                .len(),
            1
        );
        assert_eq!(
            runtime
                .dispatch("candidates", with_pinyin("nihaoa"), None)
                .len(),
            0
        );
        let empty = runtime.lua().create_table().unwrap();
        assert_eq!(runtime.dispatch("candidates", empty, None).len(), 0);
    }

    /// `focus_app`：光标切进清单里的程序才派 `focus`；别的程序不派。
    #[test]
    fn focus_app_matches_the_declared_programs() {
        let dir = script_dir("focus-app");
        std::fs::write(
            dir.join("focus.lua"),
            "cloudime.script{ api = 2, trigger_condition = 'focus_app', apps = { 'NotePad.EXE' }, \
             budget = 1000000, timeout = 10000, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('focus', function(event) return { notice = event.app } end)\n",
        )
        .unwrap();
        let runtime = Runtime::load(&dir, &[]);
        let payload = runtime.lua().create_table().unwrap();
        payload.set("app", "notepad.exe").unwrap();
        let responses = runtime.dispatch("focus", payload, Some("notepad.exe"));
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].get::<String>("notice").unwrap(), "notepad.exe");
        let payload = runtime.lua().create_table().unwrap();
        payload.set("app", "other.exe").unwrap();
        assert_eq!(
            runtime.dispatch("focus", payload, Some("other.exe")).len(),
            0
        );
    }

    /// `manual`（i 模式预留）：不自动派发任何事件，但 `startup` 照旧。
    #[test]
    fn a_manual_trigger_never_fires_on_its_own() {
        let dir = script_dir("manual");
        std::fs::write(
            dir.join("manual.lua"),
            "cloudime.script{ api = 2, trigger_condition = 'manual', budget = 1000000, timeout = 10000, \
             sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return {} end)\n\
             cloudime.on('startup', function() return {} end)\n",
        )
        .unwrap();
        let runtime = Runtime::load(&dir, &[]);
        assert_eq!(
            runtime
                .dispatch("key", payload(&runtime, Some(0x41)), None)
                .len(),
            0
        );
        assert_eq!(
            runtime
                .dispatch("startup", payload(&runtime, None), None)
                .len(),
            1
        );
        assert!(!runtime.time_due());
    }

    /// 组合键的修饰键要**精确**对上：声明 `ctrl` 的脚本收不到 `ctrl+shift` 那一拍。
    #[test]
    fn a_combination_requires_the_exact_modifiers() {
        let dir = script_dir("combo-mask");
        std::fs::write(
            dir.join("ctrl-a.lua"),
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'ctrl', keys = { 'a' }, \
             budget = 1000000, timeout = 10000, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return {} end)\n",
        )
        .unwrap();
        let runtime = Runtime::load(&dir, &[]);
        let press = |ctrl: bool, shift: bool| {
            let payload = runtime.lua().create_table().unwrap();
            payload.set("vk", 0x41).unwrap();
            payload.set("ctrl", ctrl).unwrap();
            payload.set("shift", shift).unwrap();
            payload
        };
        assert_eq!(runtime.dispatch("key", press(true, false), None).len(), 1);
        assert_eq!(
            runtime.dispatch("key", press(true, true), None).len(),
            0,
            "多按一个 Shift 就是另一套"
        );
        assert_eq!(runtime.dispatch("key", press(false, false), None).len(), 0);
        assert_eq!(
            runtime.combination_key_masks(),
            1u16 << 0b0001,
            "ctrl 那一套要告诉 DLL"
        );
    }

    /// 新触发条件的必填项：缺了 / 旧 API 版本 / 旧的触发名，一律判无效。
    #[test]
    fn the_new_triggers_require_their_fields() {
        for source in [
            "cloudime.script{ api = 2, trigger_condition = 'time_range', timeout = 100, sync = true, handover = 'return', on_error = false }\n",
            "cloudime.script{ api = 2, trigger_condition = 'candidate_context', timeout = 100, sync = true, handover = 'return', on_error = false }\n",
            "cloudime.script{ api = 2, trigger_condition = 'focus_app', timeout = 100, sync = true, handover = 'return', on_error = false }\n",
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'none', timeout = 100, sync = true, handover = 'return', on_error = false }\n",
            "cloudime.script{ api = 2, trigger_condition = 'sys_time', trigger_time = '08:00', timeout = 100, sync = true, handover = 'return', on_error = false }\n",
            "cloudime.script{ api = 1, trigger_condition = 'manual', timeout = 100, sync = true, handover = 'return', on_error = false }\n",
        ] {
            let dir = script_dir("trigger-required");
            std::fs::write(dir.join("bad.lua"), source).unwrap();
            let runtime = Runtime::load(&dir, &[]);
            assert!(runtime.scripts().is_empty(), "应判无效：{source}");
        }
    }

    /// 输入法自己占用的组合键（含预留给 v/u/i 模式的 `Ctrl+Alt+V/U/I`）：函数判得出来，
    /// 清单里声明它的脚本判无效；换成没占用的组合就有效。
    #[test]
    fn the_imes_own_combos_cannot_be_bound_by_scripts() {
        for (mask, vk) in [
            (0b0001, 0x0D), // Ctrl+Enter
            (0b0001, 0xC0), // Ctrl+`
            (0b0001, 0x31), // Ctrl+1
            (0b0001, 0x61), // Ctrl+小键盘 1
            (0b0100, 0xC0), // Shift+`
            (0b0011, 0x20), // Ctrl+Alt+Space
            (0b0011, 0xBC), // Ctrl+Alt+,
            (0b0011, 0x56), // Ctrl+Alt+V
            (0b0011, 0x55), // Ctrl+Alt+U
            (0b0011, 0x49), // Ctrl+Alt+I
            (0b0111, 0x56), // Ctrl+Alt+Shift+V 也保留
        ] {
            assert!(is_reserved_combination(mask, vk), "{mask:#06b}/{vk:#x}");
        }
        for (mask, vk) in [
            (0b0000, 0x0D), // 光 Enter
            (0b0011, 0x57), // Ctrl+Alt+W 可以用
            (0b0101, 0x56), // Ctrl+Shift+V 可以用
            (0b0010, 0x56), // Alt+V
            (0b0001, 0x56), // Ctrl+V
        ] {
            assert!(!is_reserved_combination(mask, vk), "{mask:#06b}/{vk:#x}");
        }

        // 清单里写 Ctrl+Alt+V → 判无效
        let dir = script_dir("reserved-combo");
        std::fs::write(
            dir.join("bad.lua"),
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'ctrl+alt', keys = { 'v' }, \
             budget = 1000000, timeout = 10000, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return {} end)\n",
        )
        .unwrap();
        let runtime = Runtime::load(&dir, &[]);
        assert!(runtime.scripts().is_empty(), "Ctrl+Alt+V 预留给 v 模式");

        // 换成 Ctrl+Alt+W 就有效
        let dir = script_dir("reserved-combo-ok");
        std::fs::write(
            dir.join("good.lua"),
            "cloudime.script{ api = 2, trigger_condition = 'combination_key', combination_modifiers = 'ctrl+alt', keys = { 'w' }, \
             budget = 1000000, timeout = 10000, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return {} end)\n",
        )
        .unwrap();
        let runtime = Runtime::load(&dir, &[]);
        assert_eq!(runtime.scripts().to_vec(), ["good.lua"]);
    }
}
