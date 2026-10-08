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

/// 脚本目录名：**安装目录**（与 `Phrases\`、`WordBank\` 同级）下的这个子目录。
pub const DIRECTORY: &str = "Scripts";

/// 「新建脚本」用的模板：放在脚本目录里，但**不是**真脚本 —— 设置页不列它、加载器不执行它。
pub const TEMPLATE_FILE: &str = "template.lua";

/// 脚本面向的 API 版本：清单里的 `api` 必须是它，否则那个脚本判无效。
const API_VERSION: u32 = 1;

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

    /// 脚本要的候选窗尺寸（`cloudime.candidate.set_*`）：派发/回调结束后 Server 取一次。
    size: Rc<RefCell<SizeRequest>>,

    /// 候选窗最近一次画出来的内容区尺寸（点）；`cloudime.candidate.width()` 读它。
    viewport: Rc<RefCell<Option<(f32, f32)>>>,

    /// 量尺（`cloudime.ui.measure`）：Server 用 [`Runtime::set_measure`] 装进来。
    measure: Rc<RefCell<Option<MeasureHook>>>,

    /// 取输入框文本的接口（`cloudime.text.*`）：Server 用 [`Runtime::set_text_hook`] 装进来。
    text_hook: Rc<RefCell<Option<TextHook>>>,

    /// 剪贴板（`cloudime.clipboard.*`）：Server 用 [`Runtime::set_clipboard`] 装进来。
    clipboard_set: Rc<RefCell<Option<ClipboardSetHook>>>,
    clipboard_get: Rc<RefCell<Option<ClipboardGetHook>>>,

    /// `cloudime` 表本体：脚本只拿到只读代理，运行时自己写 `context` 这类刷新值。
    host: Rc<RefCell<Option<Table>>>,

    /// 加载成功的脚本名（清单里的 `name`，按加载顺序）。
    scripts: Vec<String>,
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
            size: Rc::new(RefCell::new(SizeRequest::default())),
            viewport: Rc::new(RefCell::new(None)),
            measure: Rc::new(RefCell::new(None)),
            text_hook: Rc::new(RefCell::new(None)),
            clipboard_set: Rc::new(RefCell::new(None)),
            clipboard_get: Rc::new(RefCell::new(None)),
            host: Rc::new(RefCell::new(None)),
            scripts: Vec::new(),
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
        // 先把要调的东西 clone 出来再放开借用：处理函数里再 `cloudime.on` 会重入 `handlers`。
        let handlers: Vec<(Rc<Manifest>, Function)> = self
            .handlers
            .borrow()
            .get(event)
            .map(|list| {
                list.iter()
                    .filter(|handler| app.is_none_or(|app| handler.manifest.runs_in(app)))
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

    /// 装一个量尺：脚本调 `cloudime.ui.measure` / `measure_tip` 时用它量文字宽度（单位点）。
    /// 由 Server 在 [`Runtime::load`] / [`Runtime::none`] 之后调（拿不到渲染器时可以一直不装，
    /// 那时脚本调 measure 会报「量不了」）。
    pub fn set_measure(&self, hook: MeasureHook) {
        *self.measure.borrow_mut() = Some(hook);
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
    let size = &runtime.size;
    let viewport = &runtime.viewport;
    let measure = &runtime.measure;
    let text_hook = &runtime.text_hook;
    let clipboard_set = &runtime.clipboard_set;
    let clipboard_get = &runtime.clipboard_get;
    let host = &runtime.host;
    let table = lua.create_table()?;
    // 光标前文：第一次派发前也能读到（空串），不用先判断 nil
    table.set("context", "")?;
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

/// 读一个脚本清单（`cloudime.script{…}`）：缺字段 / 类型不对 / 对不上都报错 —— 那个脚本判无效。
fn parse_manifest(options: &Table, default_name: &str) -> mlua::Result<Manifest> {
    let api = options
        .get::<Option<u32>>("api")?
        .ok_or_else(|| mlua::Error::RuntimeError("清单必须写 api（当前只有 1）".to_owned()))?;
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

    /// 一个合法清单：测试脚本都用它（`write_script` 自动加在最前面）。
    /// **不写 `name`** —— 于是脚本名取文件名，`scripts()` 的断言还能一眼看出是哪个脚本。
    const MANIFEST: &str = "cloudime.script{ api = 1, budget = 1000000, timeout = 10000, \
                             sync = false, handover = 'callback', on_error = false }\n";

    /// 写一个脚本文件（自动在最前面补一份合法清单）。
    fn write_script(dir: &Path, name: &str, source: &str) {
        std::fs::write(dir.join(name), format!("{MANIFEST}{source}")).unwrap();
    }

    /// 写一个**原样**的脚本文件（自己带清单，或者故意不带 —— 测「判无效」用）。
    fn write_raw_script(dir: &Path, name: &str, source: &str) {
        std::fs::write(dir.join(name), source).unwrap();
    }

    /// 载荷：一个空表 + 可选的 `vk`。
    fn payload(runtime: &Runtime, vk: Option<u32>) -> Table {
        let payload = runtime.lua().create_table().unwrap();
        if let Some(vk) = vk {
            payload.set("vk", vk).unwrap();
        }
        payload
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

        runtime.set_viewport(Some((312.0, 140.0)));
        assert_eq!(
            lua.load("return cloudime.candidate.width()")
                .eval::<f32>()
                .unwrap(),
            312.0
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
            "cloudime.script{ api = 1, timeout = 100, sync = true, handover = 'callback', on_error = false }\n\
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
            "cloudime.script{ api = 1, timeout = 100, apps = { 'NotePad.EXE' }, sync = true, handover = 'return', on_error = false }\n\
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
            "cloudime.script{ name = 'a', api = 1, timeout = 100, priority = 10, sync = true, handover = 'return', on_error = false }\n\
             cloudime.on('key', function() return { notice = 'a' } end)\n",
        );
        write_raw_script(
            &dir,
            "b.lua",
            "cloudime.script{ name = 'b', api = 1, timeout = 100, priority = 1, sync = true, handover = 'return', on_error = false }\n\
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
            "cloudime.script{ api = 1, timeout = 100, sync = true, handover = 'return', \
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
            "cloudime.script{ api = 1, budget = 1000, timeout = 2000, sync = true, handover = 'return', on_error = false }\n\
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
            "cloudime.script{ api = 1, timeout = 100, sync = true, handover = 'return', on_error = false }\n\
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
            "cloudime.script{ api = 1, timeout = 100, sync = false, handover = 'callback', on_error = false }\n\
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
}
