//! 脚本与输入法的接口：把 Server 这边的事件派给用户脚本（[`cloudime_script::Runtime`]），
//! 再把脚本返回的表读成「这一拍要改什么」。
//!
//! 脚本跑在 Server 进程里（**不在** TSF DLL 里）：DLL 被加载进每个应用，逻辑只留一份在这儿。
//! Server 正对着协议两头 —— DLL 送来的（按键、前景、光标矩形、光标前文）与 DLL 要照做的（上屏、
//! 帧、状态条）都经过 Router，所以「脚本操作输入法」在 Server 侧就能做到，不必给 DLL 也塞一份 Lua。
//!
//! 事件（`cloudime.on(名字, function(载荷) …)`）：
//!
//! - `startup`：全部脚本加载完，可以开始收事件了。载荷是空表。
//! - `key`：一次按键，脚本看到的是**这一键处理之前**的状态。载荷：`app`（宿主 exe 名，用来按应用分支）、
//!   `vk`（虚拟键码）、`char`（这一键产生的字符，没有则 `nil`）、`ctrl` / `alt` / `shift` / `caps` /
//!   `english_mode`（修饰键）、`composing`（是不是在组句）、`mode`（`"chinese"` / `"english"` /
//!   `"disabled"`）。
//! - `candidates`：**引擎排完之后**这一屏候选。载荷是 1 起的数组（另有 `app` 字段），每项
//!   `{ text = 上屏文本, display = 显示的文本, pinyin = 词库读音, weight = Core 的排名权重,
//!   kind = "chinese" / "english" / "shortcut" / "custom" / "sentence" }`（快捷候选没有 `weight`）。
//! - `caret`：脚本用 `cloudime.caret()` 请过一次光标位置，DLL 量到之后送来。载荷是插入点的屏幕矩形
//!   （屏幕坐标 / **点**）：`{ left, top, right, bottom, work_left, work_top, work_right,
//!   work_bottom, scale }`，`work_*` 是它所在显示器的工作区。
//!
//! 处理函数**返回一张表**就是「要改的东西」；多个脚本按登记顺序合并，后面的盖前面的（表里缺的项不动）。
//! 现在认这几项：
//!
//! | 键 | 类型 | 意思 |
//! |---|---|---|
//! | `passthrough` | `true` | 这一键不吃、原样交给应用（游戏里抢键就靠它） |
//! | `commit` | 字符串 | 吃掉这一键、直接上屏这段文本（当前组句作废） |
//! | `notice` | 字符串 | 候选窗里显示一行提示（随这一帧下发，下一次按键清） |
//! | `order` | `{2, 1}` | 候选排序：1 起的下标按这个顺序排到前面，没列到的按原顺序接在后面 |
//! | `display` | `{[1] = "译①"}` | 候选显示：原下标（1 起）→ 显示的文本（上屏的仍是 `text`） |
//! | `adjust` | `{["云朵"] = 2.0}` | 加权 / 降权：词文本 → 系数 |
//! | `online` | 字符串 / 表 / `false` | 候选窗底部那一行（在线翻译）：写 / 清 |
//! | `theme` | 字符串 / `false` | 换主题：写进配置 `[theme] curr_theme` 并重启输入法服务（`false` 回本进程启动时那份） |
//!
//! `order` / `display` **不绕开引擎**：改的是 `composed` 里那份候选布局本身，之后的方向键、数字键、
//! 鼠标点选都按新顺序走，选中第几个仍然由 Engine 去上屏 / 并进组句；高亮跟着原来那个候选走。
//!
//! 脚本还能读到 `cloudime.context`：光标前文 —— 应用里已经输入、不在候选窗口里的那段文本
//! （DLL 在组句起始时送来，每次派发前刷新）。
//!
//! 还没接的（清单见 `docs/notes/crate-notes.md`）：改「已选文本 / 组句里已经选中的那一段」（要 Engine
//! 侧的接口）、翻译 Tip、词库与短语的权重、启动可执行文件（用标准库的
//! `os.execute` / `io.popen` 已经能做）。

use cloudime_core::{Candidate, CandidateKind, CandidateLayout};
use cloudime_platform::Config;
use cloudime_platform::protocol::{InputMode, KeyEvent, OnlineLine, OnlineState, ScreenRect};
use cloudime_script::SizeRequest;
use cloudime_script::mlua;
use windows::Win32::Foundation::POINT;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use super::Router;
use super::composed::Composed;

/// 脚本窗口的输出端（UI 线程）：Router 把 `cloudime.window.*` 产出的请求交出去，窗口在那边建。
///
/// 没有 UI 的测试 / UI 没起来时用 [`NoopScriptWindowSink`]：请求直接丢掉，输入照常。
pub trait ScriptWindowSink: Send {
    /// 执行一次脚本窗口操作（按脚本产出的顺序）。
    fn script_window(&self, request: cloudime_script::WindowRequest);
}

/// 什么都不做的窗口输出端（测试 / UI 没起来）。
pub struct NoopScriptWindowSink;

impl ScriptWindowSink for NoopScriptWindowSink {
    fn script_window(&self, _request: cloudime_script::WindowRequest) {}
}

/// 脚本对**这一拍**的要求：几张开给的表合并出来的结果。
#[derive(Debug, Default)]
pub(super) struct ScriptActions {
    /// 这一键不吃，原样交给应用。
    pub passthrough: bool,

    /// 吃掉这一键、直接上屏这个文本。
    pub commit: Option<String>,

    /// 候选窗里显示的一行提示。
    pub notice: Option<String>,

    /// 候选重排：1 起的下标，按这个顺序排到前面。
    pub order: Vec<u32>,

    /// 候选显示改写：原下标（1 起）→ 显示文本。
    pub display: Vec<(u32, String)>,

    /// 脚本给的加权 / 降权（词文本 → 系数）；`None` = 不动，空的就是全清。
    pub adjust: Option<Vec<(String, f64)>>,

    /// 候选窗底部那一行在线翻译：脚本直接写 / 清（`online = …`）。
    pub online: Option<OnlineCommand>,

    /// 换主题（`theme = "Panic"`；`theme = false` 回本进程启动时配置里那份）。
    pub theme: Option<ThemeCommand>,
}

/// 脚本要换的主题。
#[derive(Debug, Clone)]
pub(super) enum ThemeCommand {
    /// 回本进程启动时配置里的主题（`theme = false`）。
    Clear,

    /// 换到这份主题（`theme = "Panic"`）。
    Set(String),
}

impl ThemeCommand {
    /// `cloudime.apply_theme` 那个请求：`None` = 没提，`Some(None)` = 回本进程启动时那份，`Some(Some(名))` = 换到它。
    pub(super) fn from_request(request: Option<Option<String>>) -> Option<Self> {
        match request? {
            None => Some(Self::Clear),
            Some(name) => Some(Self::Set(name)),
        }
    }
}

/// 脚本对候选窗底部那一行（在线翻译）的要求。
#[derive(Debug, Clone)]
pub(super) enum OnlineCommand {
    /// 清掉那一行（`online = false`）。
    Clear,

    /// 写这一行（`online = "文本"` / `online = { text = …, state = … }`）。
    Set(OnlineLine),
}

/// 一个后台脚本实例跑完之后送回工人线程的东西。
pub(super) struct BackgroundScript {
    /// 脚本返回的动作（`notice` / `online` / `theme` / `commit` 等）。
    actions: ScriptActions,

    /// 脚本在建 / 改 / 收的窗口；编号已抬到高位，避免和主运行时撞。
    windows: Vec<cloudime_script::WindowRequest>,
}

/// 后台脚本窗口编号的下一个起点。
static NEXT_BACKGROUND_WINDOW: AtomicU32 = AtomicU32::new(0x4000_0000);

impl ScriptActions {
    /// 有没有要改的东西。
    fn is_empty(&self) -> bool {
        !self.passthrough
            && self.commit.is_none()
            && self.notice.is_none()
            && self.order.is_empty()
            && self.display.is_empty()
            && self.adjust.is_none()
            && self.online.is_none()
            && self.theme.is_none()
    }

    /// 把一张返回的表并进来：认得的键逐个读，读不动的（类型不对）记一条日志当没写。
    fn merge(&mut self, table: &mlua::Table) {
        if let Some(value) = read_bool(table, "passthrough") {
            self.passthrough = value;
        }
        if let Some(value) = read_string(table, "commit") {
            self.commit = Some(value);
        }
        if let Some(value) = read_string(table, "notice") {
            self.notice = Some(value);
        }
        if let Some(value) = field(table, "order") {
            match value {
                mlua::Value::Table(order) => self.order = read_order(&order),
                value => wrong_type("order", &value),
            }
        }
        if let Some(value) = field(table, "display") {
            match value {
                mlua::Value::Table(display) => self.display = read_display(&display),
                value => wrong_type("display", &value),
            }
        }
        if let Some(value) = field(table, "adjust") {
            match value {
                mlua::Value::Table(adjust) => self.adjust = Some(read_adjustments(&adjust)),
                value => wrong_type("adjust", &value),
            }
        }
        if let Some(value) = field(table, "online") {
            self.online = read_online(&value);
        }
        if let Some(value) = field(table, "theme") {
            self.theme = read_theme(&value);
        }
    }
}

/// 读一个布尔项：表里没写（`nil`）返回 `None`，类型不对记一条日志也当没写。
fn read_bool(table: &mlua::Table, key: &str) -> Option<bool> {
    match field(table, key)? {
        mlua::Value::Boolean(value) => Some(value),
        value => {
            wrong_type(key, &value);
            None
        }
    }
}

/// 读一个字符串项：同上。**不让 Lua 把数字悄悄转成字符串**（`commit = 42` 是写错了，不是想上屏 "42"）。
fn read_string(table: &mlua::Table, key: &str) -> Option<String> {
    match field(table, key)? {
        mlua::Value::String(value) => value.to_str().ok().map(|text| text.to_owned()),
        value => {
            wrong_type(key, &value);
            None
        }
    }
}

/// `online = "文本"` / `online = false` / `online = { text = "…", state = "waiting" }`：
/// 候选窗底部那一行（在线翻译）由脚本直接写；`false` / `nil` 是清掉。
fn read_online(value: &mlua::Value) -> Option<OnlineCommand> {
    match value {
        mlua::Value::Boolean(false) | mlua::Value::Nil => Some(OnlineCommand::Clear),
        mlua::Value::String(text) => match text.to_str() {
            Ok(text) => Some(OnlineCommand::Set(OnlineLine {
                text: text.to_owned(),
                state: OnlineState::Done,
            })),
            Err(_) => {
                wrong_type("online", value);
                None
            }
        },
        mlua::Value::Table(table) => {
            // 表里没写 `text` 就当没写这一项
            let text = read_string(table, "text")?;
            let state = match read_string(table, "state").as_deref() {
                None | Some("result") => OnlineState::Done,
                Some("waiting") => OnlineState::Waiting,
                Some("error") => OnlineState::Failed,
                Some(other) => {
                    tracing::warn!(
                        state = other,
                        "脚本给的 online.state 不认识，按 result 处理"
                    );
                    OnlineState::Done
                }
            };
            Some(OnlineCommand::Set(OnlineLine { text, state }))
        }
        value => {
            wrong_type("online", value);
            None
        }
    }
}

/// `theme = "Panic"`（换到它）/ `theme = false`（回配置里的主题）：脚本换主题。
fn read_theme(value: &mlua::Value) -> Option<ThemeCommand> {
    match value {
        mlua::Value::Boolean(false) | mlua::Value::Nil => Some(ThemeCommand::Clear),
        mlua::Value::String(name) => match name.to_str() {
            Ok(name) => Some(ThemeCommand::Set(name.to_owned())),
            Err(_) => {
                wrong_type("theme", value);
                None
            }
        },
        value => {
            wrong_type("theme", value);
            None
        }
    }
}

/// `order = {2, 1}`：1 起的下标（不是正整数的跳过）。
fn read_order(table: &mlua::Table) -> Vec<u32> {
    let mut order = Vec::new();
    for item in table.sequence_values::<mlua::Value>() {
        let Ok(value) = item else {
            tracing::warn!("脚本返回的候选顺序读不动，后面的丢掉");
            break;
        };
        match value {
            mlua::Value::Integer(index) if index > 0 => order.push(index as u32),
            value => wrong_type("order 里的一项", &value),
        }
    }
    order
}

/// `display = {[1] = "译①"}`：1 起的下标 → 显示文本。
fn read_display(table: &mlua::Table) -> Vec<(u32, String)> {
    let mut display = Vec::new();
    for entry in table.pairs::<mlua::Value, mlua::Value>() {
        match entry {
            Ok((mlua::Value::Integer(index), mlua::Value::String(text))) if index > 0 => match text
                .to_str()
            {
                Ok(text) => display.push((index as u32, text.to_owned())),
                Err(error) => tracing::warn!(%error, index, "脚本给的候选显示不是合法 UTF-8，跳过"),
            },
            Ok((key, _value)) => wrong_type("display 的键", &key),
            Err(error) => {
                tracing::warn!(%error, "脚本返回的候选显示读不动，后面的丢掉");
                break;
            }
        }
    }
    display
}

/// `adjust = {["云朵"] = 2.0, ["今天"] = 0.5}`：词文本 → 系数（大于 1 加权、小于 1 降权）。
fn read_adjustments(table: &mlua::Table) -> Vec<(String, f64)> {
    let mut adjustments = Vec::new();
    for entry in table.pairs::<mlua::Value, mlua::Value>() {
        match entry {
            Ok((mlua::Value::String(word), value)) => {
                let Some(factor) = number(&value) else {
                    wrong_type("adjust 的系数", &value);
                    continue;
                };
                match word.to_str() {
                    Ok(word) => adjustments.push((word.to_owned(), factor)),
                    Err(error) => {
                        tracing::warn!(%error, "脚本给的加权里的词不是合法 UTF-8，跳过")
                    }
                }
            }
            Ok((key, _value)) => wrong_type("adjust 的键", &key),
            Err(error) => {
                tracing::warn!(%error, "脚本返回的加权读不动，后面的丢掉");
                break;
            }
        }
    }
    adjustments
}

/// Lua 的数字：整数与浮点都算。
fn number(value: &mlua::Value) -> Option<f64> {
    match value {
        mlua::Value::Integer(value) => Some(*value as f64),
        mlua::Value::Number(value) => Some(*value),
        _ => None,
    }
}

/// 取原始值（不走元表的 `__index`）：表里没写（`nil`）返回 `None`。
fn field(table: &mlua::Table, key: &str) -> Option<mlua::Value> {
    match table.raw_get::<mlua::Value>(key) {
        Ok(mlua::Value::Nil) => None,
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(%error, key, "脚本返回值里这一项读不动，当没写");
            None
        }
    }
}

fn wrong_type(key: &str, value: &mlua::Value) {
    tracing::warn!(
        key,
        kind = value.type_name(),
        "脚本返回值里这一项类型不对，当没写"
    );
}

/// `caret` 事件的载荷：屏幕坐标一律换算成**点**（与 `win:show{ x, y }` / 内容坐标同一单位）。
fn caret_payload(lua: &mlua::Lua, rect: ScreenRect) -> mlua::Result<mlua::Table> {
    let point = POINT {
        x: rect.left,
        y: rect.top,
    };
    let dpi = crate::ui::dpi_near(point).unwrap_or(96).max(1);
    let scale = dpi as f32 / 96.0;
    let work = crate::ui::work_area_near(point);
    let payload = lua.create_table()?;
    for (key, value) in [
        ("left", rect.left),
        ("top", rect.top),
        ("right", rect.right),
        ("bottom", rect.bottom),
        ("work_left", work.left),
        ("work_top", work.top),
        ("work_right", work.right),
        ("work_bottom", work.bottom),
    ] {
        payload.set(key, value as f32 / scale)?;
    }
    payload.set("scale", scale)?;
    Ok(payload)
}

impl Router {
    /// 脚本要的光标位置到了（DLL 量到的插入点，`ClientMessage::Caret`）：派给 `caret` 处理函数，
    /// 再把它们建 / 改 / 收窗口的请求转给 UI 线程。
    pub(super) fn dispatch_caret_to_scripts(&mut self, rect: ScreenRect) {
        if !self.scripts.has_handlers("caret") {
            return;
        }
        let payload = match caret_payload(self.scripts.lua(), rect) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "拼光标位置载荷给脚本失败");
                return;
            }
        };
        let actions = self.with_script_actions("caret", payload, self.focused_app());
        self.apply_common_actions(&actions);
        self.flush_script_windows();
    }

    /// 全部脚本加载完，可以开始派发事件了。没人关心 `startup` 就什么都不做。
    pub(super) fn dispatch_startup_to_scripts(&mut self) {
        if !self.scripts.has_handlers("startup") {
            return;
        }
        let payload = match self.scripts.lua().create_table() {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "建脚本事件载荷失败");
                return;
            }
        };
        // `startup` 的返回值还没有约定，读了也没处用，丢掉；它早于任何应用，不按应用过滤。
        self.scripts.set_viewport(self.candidates.viewport());
        self.scripts.dispatch("startup", payload, None);
        self.flush_script_windows();
    }

    /// 把一次按键派给脚本，合并出这一拍要改的东西。
    ///
    /// **没有任何脚本关心 `key` 时直接空手而归**（一次查表就返回）：没脚本就走老路，
    /// 按键路径上不该多花一点（连载荷都不拼）。
    pub(super) fn script_actions(&mut self, event: &KeyEvent) -> ScriptActions {
        if !self.scripts.has_handlers("key") {
            return ScriptActions::default();
        }
        let payload = match key_payload(
            self.scripts.lua(),
            self.focused_app(),
            self.mode,
            self.composed.is_some(),
            &self.highlighted_text(),
            event,
            self.focused_script_window.as_ref(),
        ) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "拼脚本能看懂的按键载荷失败");
                return ScriptActions::default();
            }
        };
        let actions = self.with_script_actions("key", payload, self.focused_app());
        // 脚本在 key 处理函数里建 / 改 / 收窗口：这一拍结束前转给 UI 线程
        self.flush_script_windows();
        actions
    }

    /// i 模式运行一个脚本的 `manual` 处理函数；返回值里的 `commit` 交给调用方上屏。
    ///
    /// 只在主脚本运行时上同步执行；脚本期间产生的窗口 / 主题 / 提示等动作按普通规则落地。
    pub(super) fn run_manual_script(&mut self, file: &str) -> Option<String> {
        let payload = match self.manual_payload(file) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, file, "拼 i 模式脚本载荷失败");
                return None;
            }
        };
        let app = self.focused_app().map(str::to_owned);
        let actions = self.with_script_actions_for("manual", payload, app.as_deref(), Some(file));
        self.apply_manual_actions(&actions);
        self.flush_script_windows();
        actions.commit
    }

    /// Shift+数字：在新的线程里用新的脚本运行时跑这个脚本，允许多开。
    ///
    /// 独立线程起一个新的 Lua 运行时；跑完把动作和窗口请求送回工人线程，窗口编号抬到高位。
    pub(super) fn spawn_manual_script(&self, file: &str) {
        let Some(dir) = &self.config.scripts_dir else {
            tracing::warn!(file, "没有脚本目录，i 模式无法后台运行");
            return;
        };
        let path = dir.join(file);
        let file = file.to_owned();
        let app = self.focused_app().map(str::to_owned);
        let surrounding = self.surrounding.clone();
        let sender = self.background_sender.clone();
        let window_base = NEXT_BACKGROUND_WINDOW.fetch_add(1000, Ordering::Relaxed);
        std::thread::spawn(move || {
            let runtime = cloudime_script::Runtime::load_file(&path);
            runtime.set_context(&surrounding);
            runtime.set_window_id_base(window_base);
            let payload = match runtime.lua().create_table() {
                Ok(table) => table,
                Err(error) => {
                    tracing::warn!(%error, file, "后台脚本载荷创建失败");
                    return;
                }
            };
            let _ = payload.set("app", app.as_deref());
            let _ = payload.set("file", file.as_str());
            let _ = payload.set("source", "i_mode");
            let responses = runtime.dispatch_to_script(&file, "manual", payload, app.as_deref());
            let mut actions = ScriptActions::default();
            for response in &responses {
                actions.merge(response);
            }
            let windows = runtime.take_window_requests();
            tracing::info!(
                script = %file,
                responses = responses.len(),
                windows = windows.len(),
                "i 模式后台脚本结束"
            );
            let _ = sender.send(BackgroundScript { actions, windows });
        });
    }

    /// i 模式 `manual` 事件的载荷：应用名 / 文件名 / 来源。
    fn manual_payload(&self, file: &str) -> mlua::Result<mlua::Table> {
        let payload = self.scripts.lua().create_table()?;
        payload.set("app", self.focused_app())?;
        payload.set("file", file)?;
        payload.set("source", "i_mode")?;
        Ok(payload)
    }

    /// 运行手动脚本后的动作：没有按键语义，`passthrough` / `commit` 由调用方处理。
    fn apply_manual_actions(&mut self, actions: &ScriptActions) {
        if let Some(adjustments) = &actions.adjust {
            self.engine
                .set_word_adjustments(adjustments.iter().cloned());
        }
        if actions.notice.is_some() {
            self.notice = actions.notice.clone();
        }
        self.apply_online_actions(actions);
        if let Some(command) = &actions.theme {
            self.apply_script_theme(Some(command.clone()));
        }
    }

    /// 引擎排完候选之后把这一屏交给脚本：`order` 改顺序、`display` 改显示。
    ///
    /// 改的是 `composed` 里那份布局本身（**不绕开引擎**）：之后的方向键、数字键、鼠标点选都按新顺序走，
    /// 选中第几个仍然由 Engine 去上屏 / 并进组句。高亮跟着原来那个候选走。
    pub(super) fn let_scripts_reorder_candidates(&mut self) {
        let Some(Composed::Candidates { layout, .. }) = self.composed.as_ref() else {
            return;
        };
        let page_size = layout.page_size();
        let candidates = layout.local().to_vec();
        let pinyin = self.engine.raw_preedit().text;
        let payload = match candidates_payload(
            self.scripts.lua(),
            self.focused_app(),
            &pinyin,
            &candidates,
            &self.candidate_weights,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "拼候选载荷给脚本失败");
                return;
            }
        };
        let actions = self.with_script_actions("candidates", payload, self.focused_app());
        if actions.is_empty() && !self.scripts.has_window_requests() {
            return;
        }
        // 这一拍也认 `notice` / `adjust`（HTTP 回调那一拍同理）：`adjust` 影响下一次查询，
        // `passthrough` / `commit` 得有按键才行，这里没有，记一条日志当没写。
        self.apply_common_actions(&actions);
        self.apply_candidate_actions(&actions, candidates, page_size);
        // 这一拍本来就是「重算」来的：`candidates` 里再要 `redraw()` / 改尺寸都没意义
        // （已经在算了），清掉标记，免得下一拍白白再算一遍。
        self.scripts.take_redraw_request();
        self.scripts.take_size_request();
        // 换主题不是「重算」的事：脚本在这一拍里要换就换（`cloudime.apply_theme`）
        self.apply_script_theme(ThemeCommand::from_request(
            self.scripts.take_theme_request(),
        ));
        self.flush_script_windows();
    }

    /// 两个「非按键」时机（`candidates` 事件、HTTP 回调）共用的那几项：提示与加权 / 降权。
    ///
    /// `passthrough` / `commit` 要有按键才谈得上，给了就记一条日志忽略。
    fn apply_common_actions(&mut self, actions: &ScriptActions) {
        if actions.passthrough || actions.commit.is_some() {
            tracing::warn!("脚本在没有按键的那一拍给了 passthrough / commit：忽略");
        }
        if let Some(adjustments) = &actions.adjust {
            self.engine
                .set_word_adjustments(adjustments.iter().cloned());
        }
        if actions.notice.is_some() {
            self.notice = actions.notice.clone();
        }
        if let Some(command) = &actions.theme {
            self.apply_script_theme(Some(command.clone()));
        }
        self.apply_online_actions(actions);
    }

    /// 脚本给的「在线翻译那一行」：`online = "文本"` / `{ text = …, state = … }` / `false`（清掉）。
    pub(super) fn apply_online_actions(&mut self, actions: &ScriptActions) {
        let Some(command) = &actions.online else {
            return;
        };
        match command {
            OnlineCommand::Clear => self.online.clear(),
            OnlineCommand::Set(line) => {
                // 记下「这是给哪个候选写的」：`Ctrl + 反引号` 只认还对应着当前高亮候选的那一行
                let word = self.highlighted_text();
                self.online.set(line.clone(), word);
            }
        }
    }

    /// 脚本设的候选窗尺寸回到配置值（组句结束时调）：它只活在一次组句里，与 `Ctrl + 滚轮` 同级。
    pub(super) fn clear_script_size(&mut self) {
        if self.config.script_min_width.is_none()
            && self.config.script_page_size.is_none()
            && self.config.script_scale.is_none()
        {
            return;
        }
        self.config.script_min_width = None;
        self.config.script_page_size = None;
        self.config.script_scale = None;
        let settings = self.config.render_settings();
        self.candidates.configure(settings);
    }

    /// 脚本要的候选窗尺寸（`cloudime.candidate.set_min_width` / `set_page_size` / `set_scale`）：
    /// **只在本次组句内有效**（`Router::reset_composition` 回配置值）。
    ///
    /// 最小宽度与缩放折进 [`RenderSettings`]（变了才重新下发，窗口自己按新设置重画）；
    /// 一页候选数要重排（`recompose`）才看得见。
    pub(super) fn apply_size_request(&mut self, request: SizeRequest) {
        if request.is_empty() {
            return;
        }
        let before = self.config.render_settings();
        if let Some(width) = request.min_width {
            self.config.script_min_width = width;
        }
        if let Some(scale) = request.scale {
            self.config.script_scale = scale;
        }
        if let Some(count) = request.page_size {
            self.config.script_page_size = count;
        }
        let settings = self.config.render_settings();
        if settings != before {
            self.candidates.configure(settings);
        }
        if request.page_size.is_some() {
            self.recompose();
        }
    }

    /// 当前主题名变了（`script_theme` / `curr_theme`）：`cloudime.get_theme()` 的钩子按它现读文件。
    pub(super) fn refresh_theme_name(&self) {
        *self.script_theme_name.borrow_mut() = self
            .config
            .script_theme
            .clone()
            .unwrap_or_else(|| self.config.curr_theme.clone());
    }

    /// 脚本换主题（`cloudime.apply_theme(...)` 与动作表的 `theme`）：**写进配置 `[theme] curr_theme`
    /// 并立刻重启输入法服务**（与设置页「应用主题」同一套语义 —— 重启才一定换上、选择也留了下来）。
    /// `None` = 没提，`Clear` = 回本进程启动时配置里那份。名字找不到时不写配置、不重启，只本次按缺省画。
    ///
    /// **候选窗开着时先挂起**（`pending_theme`）：换主题要重建 painter、整屏重画，正在看候选的人会觉得闪；
    /// 等这一段组句结束、候选窗关掉再补上（[`flush_pending_theme`](Self::flush_pending_theme)）。
    pub(super) fn apply_script_theme(&mut self, command: Option<ThemeCommand>) {
        let Some(command) = command else {
            return;
        };
        if self.composing() {
            self.pending_theme = Some(command);
            return;
        }
        self.switch_script_theme(command);
    }

    /// 候选窗关掉了（组句结束）：把组句里挂着的换主题补上；没有挂着的就什么也不做。
    pub(super) fn flush_pending_theme(&mut self) {
        let Some(command) = self.pending_theme.take() else {
            return;
        };
        self.switch_script_theme(command);
    }

    /// 真的换：先改 `RouterConfig.script_theme`（本次运行立刻换上）、重建 painter；要落到配置的主题名
    /// 和配置里那份不一样时，再写 `[theme] curr_theme` 并**重启输入法服务**（新实例按配置换上它）——
    /// 配置里已经是这份就不折腾，免得同一主题反复重启。
    fn switch_script_theme(&mut self, command: ThemeCommand) {
        let next = match &command {
            ThemeCommand::Clear => None,
            ThemeCommand::Set(name) => Some(name.clone()),
        };
        if self.config.script_theme == next {
            return;
        }
        // 配置里该写哪一份（`Clear` = 本进程启动时那份）
        let target = match &command {
            ThemeCommand::Clear => self.theme_at_startup.clone(),
            ThemeCommand::Set(name) => {
                if cloudime_platform::find_theme(
                    cloudime_platform::resources::bundled_root().as_deref(),
                    name,
                )
                .is_none()
                {
                    // 名字找不到：不写配置、不重启（免得下次启动按缺省画），只本次按缺省画并记一条
                    tracing::warn!(%name, "脚本要的主题不存在，按缺省主题画");
                    self.config.script_theme = next;
                    self.refresh_theme_name();
                    let settings = self.config.render_settings();
                    self.candidates.configure(settings);
                    self.reconcile_status();
                    return;
                }
                format!("{name}.json")
            }
        };
        self.config.script_theme = next;
        // 只有真跑（有配置路径）才写配置 + 重启；测试里没开热加载，`config_path` 是 `None`，别去拉新进程
        let can_persist = self.config_path().is_some();
        if can_persist && self.config.curr_theme != target {
            self.persist("theme", "curr_theme", target.clone());
            self.config.curr_theme = target;
            super::status::spawn_replacement_server();
            self.restart_pending = true;
        }
        self.refresh_theme_name();
        let settings = self.config.render_settings();
        self.candidates.configure(settings);
        self.reconcile_status();
    }

    /// 派发一类事件并合并返回值；顺路把光标前文刷给脚本。
    ///
    /// `app` 是这一刻的宿主应用名：`Some` 时跳过清单里 `apps` 对不上的脚本，`None` 就不过滤。
    fn with_script_actions(
        &self,
        event: &str,
        payload: mlua::Table,
        app: Option<&str>,
    ) -> ScriptActions {
        self.with_script_actions_for(event, payload, app, None)
    }

    /// 与 [`Self::with_script_actions`] 相同，但只派给 `only_file` 这个脚本（i 模式按数字运行）。
    fn with_script_actions_for(
        &self,
        event: &str,
        payload: mlua::Table,
        app: Option<&str>,
        only_file: Option<&str>,
    ) -> ScriptActions {
        self.scripts.set_context(&self.surrounding);
        // 候选窗现在多大（点）：脚本 `cloudime.candidate.width()` 折行时用
        self.scripts.set_viewport(self.candidates.viewport());
        let mut actions = ScriptActions::default();
        let responses = match only_file {
            Some(file) => self.scripts.dispatch_to_script(file, event, payload, app),
            None => self.scripts.dispatch(event, payload, app),
        };
        for response in responses {
            actions.merge(&response);
        }
        actions
    }

    /// 把脚本要的重排 / 改显示落到 `composed` 的布局上。
    fn apply_candidate_actions(
        &mut self,
        actions: &ScriptActions,
        mut candidates: Vec<Candidate>,
        page_size: usize,
    ) {
        // 先按**原下标**改显示（脚本看到的是原顺序）
        for (index, display) in &actions.display {
            match position(*index, candidates.len()) {
                Some(index) => candidates[index].display = Some(display.clone()),
                None => tracing::warn!(
                    index,
                    count = candidates.len(),
                    "脚本给的候选显示下标越界，跳过"
                ),
            }
        }
        // 再重排：`order` 列到的按它的顺序排前面，没列到的按原顺序接在后面
        let mut taken = vec![false; candidates.len()];
        let mut items = Vec::with_capacity(candidates.len());
        // 原下标 → 新下标：高亮要留在原来那个候选上（脚本把第 3 个提到最前，高亮也跟着过去）
        let mut moved = vec![0usize; candidates.len()];
        for index in &actions.order {
            let Some(source) = position(*index, candidates.len()) else {
                tracing::warn!(
                    index,
                    count = candidates.len(),
                    "脚本给的候选顺序越界，跳过"
                );
                continue;
            };
            if taken[source] {
                tracing::warn!(index, "脚本给的候选顺序里这一项重复，跳过");
                continue;
            }
            taken[source] = true;
            moved[source] = items.len();
            items.push(candidates[source].clone());
        }
        for (source, candidate) in candidates.iter().enumerate() {
            if !taken[source] {
                moved[source] = items.len();
                items.push(candidate.clone());
            }
        }
        let highlight = self.highlight.min(moved.len().saturating_sub(1));
        self.highlight = moved.get(highlight).copied().unwrap_or(0);
        if let Some(Composed::Candidates { layout, .. }) = self.composed.as_mut() {
            *layout = CandidateLayout::new(items, page_size);
        }
    }

    /// `time_range` 脚本到点了：派一次 `time`（载荷与 `cloudime.get_time()` 同一张表），把脚本要改的
    /// 落到当前这一屏上再重画。
    ///
    /// 「到点」由 `Runtime::time_due` 按**时间段窗口**去重（每个窗口每个脚本只跑一次；
    /// `Manifest::accepts` 再逐脚本核对当前时间在不在它的时间段里）。一个 `time_range` 脚本都没有时一次都不花。
    pub(super) fn poll_script_time(&mut self) {
        if !self.scripts.time_due() {
            return;
        }
        let payload = match self.scripts.time_payload(self.focused_app()) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "建 `time` 事件载荷失败");
                return;
            }
        };
        let responses = self.scripts.dispatch("time", payload, self.focused_app());
        let redraw = self.scripts.take_redraw_request();
        let size = self.scripts.take_size_request();
        let theme = ThemeCommand::from_request(self.scripts.take_theme_request());
        if responses.is_empty()
            && !redraw
            && size.is_empty()
            && theme.is_none()
            && !self.scripts.has_window_requests()
        {
            return;
        }
        let mut actions = ScriptActions::default();
        for response in responses {
            actions.merge(&response);
        }
        // 脚本在 `time` 里调过 `cloudime.candidate.redraw()`：先按它最新的状态重算这一屏
        if redraw {
            self.recompose();
        }
        // 没有按键的那一拍：`passthrough` / `commit` 给了由 `apply_common_actions` 记日志忽略
        self.apply_size_request(size);
        self.apply_common_actions(&actions);
        self.apply_script_theme(theme);
        // 换主题 / 改提示 / 在线那一行要立刻看得见
        self.reconcile_candidates(&self.self_drawn_frame());
        self.flush_script_windows();
    }

    /// 收脚本的 HTTP 结果：调回调，把回调要改的（候选 / 提示 / 加权）落到当前这一屏上再重画。
    ///
    /// 这是「没有按键的那一拍」：`passthrough` / `commit` 无从谈起，给了也忽略（记一条日志）。
    /// 收后台脚本的结果：动作落到当前状态，窗口请求转给 UI 线程。
    pub(super) fn poll_background_scripts(&mut self) {
        while let Ok(result) = self.background_scripts.try_recv() {
            if result.actions.commit.is_some() {
                tracing::warn!("后台脚本返回了 commit：没有对应会话，忽略");
            }
            self.apply_manual_actions(&result.actions);
            for request in result.windows {
                if self.script_windows_ready {
                    self.script_windows.script_window(request);
                } else {
                    self.pending_script_windows.push(request);
                }
            }
        }
    }

    pub(super) fn poll_scripts_requests(&mut self) {
        if !self.scripts.has_pending_requests() {
            return;
        }
        let candidates = match self.current_candidates_payload() {
            Some(payload) => payload,
            None => match self.scripts.lua().create_table() {
                Ok(table) => {
                    // 没在组句也要能看出这是哪个应用
                    let _ = table.set("app", self.focused_app());
                    table
                }
                Err(error) => {
                    tracing::warn!(%error, "建脚本事件载荷失败");
                    return;
                }
            },
        };
        self.scripts.set_viewport(self.candidates.viewport());
        let responses = self.scripts.poll_requests(candidates);
        let redraw = self.scripts.take_redraw_request();
        let size = self.scripts.take_size_request();
        let theme = ThemeCommand::from_request(self.scripts.take_theme_request());
        if responses.is_empty()
            && !redraw
            && size.is_empty()
            && theme.is_none()
            && !self.scripts.has_window_requests()
        {
            return;
        }
        let mut actions = ScriptActions::default();
        for response in responses {
            actions.merge(&response);
        }
        // 脚本在回调里调过 `cloudime.candidate.redraw()`：先按它最新的状态把这一屏重算一遍，
        // 再把回调要的改动（顺序 / 显示 / 提示 / 在线那一行）叠上去。
        if redraw {
            self.recompose();
        }
        // 脚本在回调里设过候选窗尺寸（本组句内有效）
        self.apply_size_request(size);
        // 回调里换的主题（`cloudime.apply_theme`）
        self.apply_script_theme(theme);
        self.apply_common_actions(&actions);
        if (!actions.order.is_empty() || !actions.display.is_empty())
            && let Some(Composed::Candidates { layout, .. }) = self.composed.as_ref()
        {
            let page_size = layout.page_size();
            let current = layout.local().to_vec();
            self.apply_candidate_actions(&actions, current, page_size);
        }
        // 重画：回调改的顺序 / 显示 / 提示要立刻看得见
        self.reconcile_candidates(&self.self_drawn_frame());
        self.flush_script_windows();
    }

    /// UI 线程发来的一次脚本窗口鼠标事件：派给 `window` 处理函数。
    ///
    /// 与 `startup` / HTTP 回调一样，**不受清单 `trigger_condition` 的限制**（窗口是脚本自己建的，
    /// 事件当然归它）。处理函数里通常调 `win:set_content{…}`，攒下的操作由 [`Self::flush_script_windows`]
    /// 在同一拍转回 UI 线程。
    pub fn handle_script_window_event(&mut self, event: cloudime_script::ScriptWindowEvent) {
        // 焦点先记下来：`key` 载荷要用（`window` / `window_id`）
        match event.action {
            cloudime_script::WindowAction::Focus => {
                self.focused_script_window = Some((event.window, event.name.clone()));
            }
            cloudime_script::WindowAction::Blur
                if self
                    .focused_script_window
                    .as_ref()
                    .is_some_and(|(id, _)| *id == event.window) =>
            {
                self.focused_script_window = None;
            }
            _ => {}
        }
        if !self.scripts.has_handlers("window") {
            return;
        }
        let payload = match cloudime_script::window_event_payload(self.scripts.lua(), &event) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "建 `window` 事件载荷失败");
                return;
            }
        };
        let actions = self.with_script_actions("window", payload, self.focused_app());
        // 没有按键的那一拍：`passthrough` / `commit` 由 `apply_common_actions` 记日志忽略
        self.apply_common_actions(&actions);
        self.flush_script_windows();
    }

    /// 焦点切进某个程序了：给 `focus_app` 脚本派一次 `focus`（载荷 `{ app }`）。
    ///
    /// 应用没变就不派（同一程序里换输入框不算）；`apps` 对不上的脚本由派发那一层过滤。
    pub(super) fn dispatch_focus_scripts(&mut self) {
        let app = self.focused_app().map(str::to_owned);
        if app == self.last_script_focus_app {
            return;
        }
        self.last_script_focus_app = app.clone();
        let Some(app) = app else {
            return;
        };
        if !self.scripts.has_handlers("focus") {
            return;
        }
        let payload = match self.scripts.lua().create_table() {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "建 `focus` 事件载荷失败");
                return;
            }
        };
        if let Err(error) = payload.set("app", app.as_str()) {
            tracing::warn!(%error, "写 `focus` 事件载荷失败");
            return;
        }
        let actions = self.with_script_actions("focus", payload, Some(&app));
        // 没有按键的那一拍：`passthrough` / `commit` 由 `apply_common_actions` 记日志忽略
        self.apply_common_actions(&actions);
        // 焦点这一拍没有「重算」的事：脚本在 focus 里要的 redraw 清掉（下一次按键自然会重排）
        self.scripts.take_redraw_request();
        self.flush_script_windows();
    }

    /// 把脚本这一拍攒下的窗口操作按顺序转给 UI 线程（`cloudime.window.*`）。
    ///
    /// 脚本在工人线程产出请求、窗口在 UI 线程上建：这条队伍是单向的，不会等回话。
    /// 输出端还没装（`Router::new` 里的 `startup` 早于 `set_script_window_sink`）就先攒着。
    fn flush_script_windows(&mut self) {
        for request in self.scripts.take_window_requests() {
            if self.script_windows_ready {
                self.script_windows.script_window(request);
            } else {
                self.pending_script_windows.push(request);
            }
        }
    }

    /// 现在这一屏候选在脚本那边的样子（没在组句 / 一个候选都没有时 `None`）。
    fn current_candidates_payload(&self) -> Option<mlua::Table> {
        match &self.composed {
            Some(Composed::Candidates { layout, .. }) if !layout.is_empty() => {
                let pinyin = self.engine.raw_preedit().text;
                candidates_payload(
                    self.scripts.lua(),
                    self.focused_app(),
                    &pinyin,
                    layout.local(),
                    &self.candidate_weights,
                )
                .ok()
            }
            _ => None,
        }
    }
}

/// 1 起下标 → 0 起下标；越界 / 不是正数返回 `None`。
fn position(index: u32, count: usize) -> Option<usize> {
    usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .filter(|index| *index < count)
}

/// 一次按键在脚本那边的样子（`app` 是宿主应用名 —— 脚本据此按应用分支，比如游戏里放行按键；
/// `highlight` 是这一刻**高亮候选的文本**，脚本想自己翻它时用，没有候选时是空串；
/// `script_window` 是这会儿拿着键盘焦点的脚本窗口 —— 有它时载荷里多带 `window` / `window_id`）。
fn key_payload(
    lua: &mlua::Lua,
    app: Option<&str>,
    mode: InputMode,
    composing: bool,
    highlight: &str,
    event: &KeyEvent,
    script_window: Option<&(cloudime_script::WindowId, String)>,
) -> mlua::Result<mlua::Table> {
    let modifiers = event.modifiers;
    let payload = lua.create_table()?;
    payload.set("app", app)?;
    payload.set("vk", event.virtual_key)?;
    payload.set("char", event.character.map(String::from))?;
    payload.set("ctrl", modifiers.ctrl)?;
    payload.set("alt", modifiers.alt)?;
    payload.set("shift", modifiers.shift)?;
    payload.set("win", modifiers.win)?;
    payload.set("caps", modifiers.caps)?;
    payload.set("english_mode", modifiers.english_mode)?;
    payload.set("composing", composing)?;
    payload.set("mode", mode_key(mode))?;
    payload.set("highlight", highlight)?;
    if let Some((id, name)) = script_window {
        payload.set("window", name.clone())?;
        payload.set("window_id", *id)?;
    }
    Ok(payload)
}

/// 脚本 `cloudime.get_curr_config()` 看到的那张表：**整份配置**（键名与 `config.toml` 一致，
/// 分节同名），再给 `theme` 补一个 `available`（可选主题名列表 —— 这项不在配置文件里）。
pub(super) fn theme_table(
    lua: &mlua::Lua,
    theme: &cloudime_platform::ThemeFile,
) -> mlua::Result<mlua::Table> {
    let candidate = lua.create_table()?;
    for (name, color) in [
        ("background", theme.candidate.background),
        ("highlight", theme.candidate.highlight),
        ("shadow", theme.candidate.shadow),
        ("pinyin", theme.candidate.pinyin),
        ("pinyin_caret", theme.candidate.pinyin_caret),
        ("highlight_text", theme.candidate.highlight_text),
        ("text", theme.candidate.text),
        ("highlight_index", theme.candidate.highlight_index),
        ("index", theme.candidate.index),
        ("page_number", theme.candidate.page_number),
        ("badge", theme.candidate.badge),
        ("translate_meta", theme.candidate.translate_meta),
        ("translate_learned", theme.candidate.translate_learned),
        ("translate_fresh", theme.candidate.translate_fresh),
        ("extra", theme.candidate.extra),
    ] {
        candidate.set(name, color.hex())?;
    }

    let bar = lua.create_table()?;
    for (name, color) in [
        ("background", theme.bar.background),
        ("icon", theme.bar.icon),
        ("shadow", theme.bar.shadow),
    ] {
        bar.set(name, color.hex())?;
    }

    let tip = lua.create_table()?;
    for (name, color) in [
        ("background", theme.tip.background),
        ("icon", theme.tip.icon),
        ("shadow", theme.tip.shadow),
    ] {
        tip.set(name, color.hex())?;
    }

    let table = lua.create_table()?;
    table.set("candidate", candidate)?;
    table.set("bar", bar)?;
    table.set("tip", tip)?;
    Ok(table)
}

pub(super) fn config_table(lua: &mlua::Lua, config: &Config) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    if let serde_json::Value::Object(sections) =
        serde_json::to_value(config).unwrap_or(serde_json::Value::Null)
    {
        for (name, value) in sections {
            table.set(name.as_str(), json_value(lua, &value)?)?;
        }
    }
    // 主题那一节：`curr_theme` 配置里就有，可选列表要读主题目录
    let theme = match table.get::<Option<mlua::Table>>("theme")? {
        Some(theme) => theme,
        None => {
            let theme = lua.create_table()?;
            table.set("theme", theme.clone())?;
            theme
        }
    };
    let root = cloudime_platform::resources::bundled_root();
    let names: Vec<String> = cloudime_platform::list_themes(root.as_deref())
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    theme.set("available", names)?;
    Ok(table)
}

/// `serde_json` 的值 → Lua 值：对象 → 表（键是字符串）、数组 → 1 起的表、其余标量照搬。
fn json_value(lua: &mlua::Lua, value: &serde_json::Value) -> mlua::Result<mlua::Value> {
    Ok(match value {
        serde_json::Value::Null => mlua::Value::Nil,
        serde_json::Value::Bool(flag) => mlua::Value::Boolean(*flag),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(int) => mlua::Value::Integer(int),
            None => mlua::Value::Number(number.as_f64().unwrap_or(0.0)),
        },
        serde_json::Value::String(text) => mlua::Value::String(lua.create_string(text)?),
        serde_json::Value::Array(items) => {
            let table = lua.create_table()?;
            for (index, item) in items.iter().enumerate() {
                table.set(index + 1, json_value(lua, item)?)?;
            }
            mlua::Value::Table(table)
        }
        serde_json::Value::Object(fields) => {
            let table = lua.create_table()?;
            for (key, item) in fields {
                table.set(key.as_str(), json_value(lua, item)?)?;
            }
            mlua::Value::Table(table)
        }
    })
}

/// 一屏候选在脚本那边的样子：1 起的数组，另有 `app`（哪个应用）与 `pinyin`（这一次组句的拼音串，
/// 候选窗触发按它比对）两个字段；每项 `{ text, display, pinyin, weight, kind }`
///（`weight` 是 Core 的排名权重，快捷候选没有这一项）。
fn candidates_payload(
    lua: &mlua::Lua,
    app: Option<&str>,
    pinyin: &str,
    candidates: &[Candidate],
    weights: &HashMap<String, f64>,
) -> mlua::Result<mlua::Table> {
    let payload = lua.create_table()?;
    payload.set("app", app)?;
    payload.set("pinyin", pinyin)?;
    for (index, candidate) in candidates.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("text", candidate.text.clone())?;
        entry.set("display", candidate.display_text().to_owned())?;
        entry.set("pinyin", candidate.syllables.join(" "))?;
        if let Some(weight) = weights.get(&candidate.text) {
            entry.set("weight", *weight)?;
        }
        entry.set("kind", kind_key(candidate.kind))?;
        payload.set(index + 1, entry)?;
    }
    Ok(payload)
}

/// 模式在脚本那边的写法。
fn mode_key(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Chinese => "chinese",
        InputMode::English => "english",
        InputMode::Disabled => "disabled",
    }
}

/// 候选来源在脚本那边的写法。
fn kind_key(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Chinese => "chinese",
        CandidateKind::English => "english",
        CandidateKind::Shortcut => "shortcut",
        CandidateKind::Custom => "custom",
        CandidateKind::Sentence => "sentence",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloudime_platform::protocol::KeyModifiers;

    /// `cloudime.get_theme()` 的表就是主题文件的三组色，颜色统一 `#AARRGGBB`。
    #[test]
    fn the_theme_table_mirrors_the_theme_file_colors() {
        let lua = mlua::Lua::new();
        let table = theme_table(&lua, &cloudime_platform::ThemeFile::default()).unwrap();
        let candidate: mlua::Table = table.get("candidate").unwrap();
        assert_eq!(candidate.get::<String>("background").unwrap(), "#FFFFFFFF");
        assert_eq!(candidate.get::<String>("page_number").unwrap(), "#FF888888");
        assert_eq!(candidate.get::<String>("extra").unwrap(), "#FF0F6CBD");
        let bar: mlua::Table = table.get("bar").unwrap();
        assert_eq!(bar.get::<String>("icon").unwrap(), "#FF231F20");
        let tip: mlua::Table = table.get("tip").unwrap();
        assert_eq!(tip.get::<String>("background").unwrap(), "#FFFFFFFF");
    }

    /// 有脚本窗口拿着键盘焦点时，`key` 载荷多带 `window` / `window_id`；没有就不带（`nil`）。
    #[test]
    fn the_key_payload_carries_the_focused_script_window() {
        let lua = mlua::Lua::new();
        let event = KeyEvent::new(0x1B, None, KeyModifiers::default());
        let payload = key_payload(
            &lua,
            Some("notepad.exe"),
            InputMode::Chinese,
            false,
            "",
            &event,
            Some(&(7, "面板".to_owned())),
        )
        .unwrap();
        assert_eq!(payload.get::<String>("window").unwrap(), "面板");
        assert_eq!(payload.get::<u32>("window_id").unwrap(), 7);

        let payload = key_payload(&lua, None, InputMode::Chinese, false, "", &event, None).unwrap();
        assert!(
            payload.get::<Option<String>>("window").unwrap().is_none(),
            "没有焦点窗口时是 nil"
        );
    }
}
