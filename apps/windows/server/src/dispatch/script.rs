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
//!
//! `order` / `display` **不绕开引擎**：改的是 `composed` 里那份候选布局本身，之后的方向键、数字键、
//! 鼠标点选都按新顺序走，选中第几个仍然由 Engine 去上屏 / 并进组句；高亮跟着原来那个候选走。
//!
//! 脚本还能读到 `cloudime.context`：光标前文 —— 应用里已经输入、不在候选窗口里的那段文本
//! （DLL 在组句起始时送来，每次派发前刷新）。
//!
//! 还没接的（清单见 `docs/notes/crate-notes.md`）：改「已选文本 / 组句里已经选中的那一段」（要 Engine
//! 侧的接口）、翻译 Tip、词库与短语的权重、主题（主题功能还没做）、启动可执行文件（用标准库的
//! `os.execute` / `io.popen` 已经能做）。

use cloudime_core::{Candidate, CandidateKind, CandidateLayout};
use cloudime_platform::protocol::{InputMode, KeyEvent, OnlineLine, OnlineState};
use cloudime_script::SizeRequest;
use cloudime_script::mlua;

use std::collections::HashMap;

use super::Router;
use super::composed::Composed;

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
}

/// 脚本对候选窗底部那一行（在线翻译）的要求。
#[derive(Debug, Clone)]
pub(super) enum OnlineCommand {
    /// 清掉那一行（`online = false`）。
    Clear,

    /// 写这一行（`online = "文本"` / `online = { text = …, state = … }`）。
    Set(OnlineLine),
}

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

impl Router {
    /// 全部脚本加载完，可以开始派发事件了。没人关心 `startup` 就什么都不做。
    pub(super) fn dispatch_startup_to_scripts(&self) {
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
    }

    /// 把一次按键派给脚本，合并出这一拍要改的东西。
    ///
    /// **没有任何脚本关心 `key` 时直接空手而归**（一次查表就返回）：没脚本就走老路，
    /// 按键路径上不该多花一点（连载荷都不拼）。
    pub(super) fn script_actions(&self, event: &KeyEvent) -> ScriptActions {
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
        ) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "拼脚本能看懂的按键载荷失败");
                return ScriptActions::default();
            }
        };
        self.with_script_actions("key", payload, self.focused_app())
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
        let payload = match candidates_payload(
            self.scripts.lua(),
            self.focused_app(),
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
        if actions.is_empty() {
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

    /// 派发一类事件并合并返回值；顺路把光标前文刷给脚本。
    ///
    /// `app` 是这一刻的宿主应用名：`Some` 时跳过清单里 `apps` 对不上的脚本，`None` 就不过滤。
    fn with_script_actions(
        &self,
        event: &str,
        payload: mlua::Table,
        app: Option<&str>,
    ) -> ScriptActions {
        self.scripts.set_context(&self.surrounding);
        // 候选窗现在多大（点）：脚本 `cloudime.candidate.width()` 折行时用
        self.scripts.set_viewport(self.candidates.viewport());
        let mut actions = ScriptActions::default();
        for response in self.scripts.dispatch(event, payload, app) {
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

    /// 收脚本的 HTTP 结果：调回调，把回调要改的（候选 / 提示 / 加权）落到当前这一屏上再重画。
    ///
    /// 这是「没有按键的那一拍」：`passthrough` / `commit` 无从谈起，给了也忽略（记一条日志）。
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
        if responses.is_empty() && !redraw && size.is_empty() {
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
    }

    /// 现在这一屏候选在脚本那边的样子（没在组句 / 一个候选都没有时 `None`）。
    fn current_candidates_payload(&self) -> Option<mlua::Table> {
        match &self.composed {
            Some(Composed::Candidates { layout, .. }) if !layout.is_empty() => candidates_payload(
                self.scripts.lua(),
                self.focused_app(),
                layout.local(),
                &self.candidate_weights,
            )
            .ok(),
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
/// `highlight` 是这一刻**高亮候选的文本**，脚本想自己翻它时用，没有候选时是空串）。
fn key_payload(
    lua: &mlua::Lua,
    app: Option<&str>,
    mode: InputMode,
    composing: bool,
    highlight: &str,
    event: &KeyEvent,
) -> mlua::Result<mlua::Table> {
    let modifiers = event.modifiers;
    let payload = lua.create_table()?;
    payload.set("app", app)?;
    payload.set("vk", event.virtual_key)?;
    payload.set("char", event.character.map(String::from))?;
    payload.set("ctrl", modifiers.ctrl)?;
    payload.set("alt", modifiers.alt)?;
    payload.set("shift", modifiers.shift)?;
    payload.set("caps", modifiers.caps)?;
    payload.set("english_mode", modifiers.english_mode)?;
    payload.set("composing", composing)?;
    payload.set("mode", mode_key(mode))?;
    payload.set("highlight", highlight)?;
    Ok(payload)
}

/// 一屏候选在脚本那边的样子：1 起的数组，另有 `app` 字段说明这是哪个应用；每项
/// `{ text, display, pinyin, weight, kind }`（`weight` 是 Core 的排名权重，快捷候选没有这一项）。
fn candidates_payload(
    lua: &mlua::Lua,
    app: Option<&str>,
    candidates: &[Candidate],
    weights: &HashMap<String, f64>,
) -> mlua::Result<mlua::Table> {
    let payload = lua.create_table()?;
    payload.set("app", app)?;
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
