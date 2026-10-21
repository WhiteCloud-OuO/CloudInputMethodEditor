//! Server 进程入口：读配置、装配 Engine、在命名管道上服务 TSF DLL。逻辑在库部分，这里只装配与启动。
//! release 编成 GUI 子系统（登录自启静默跑，日志走文件）；debug 保留控制台看 stderr。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cloudime_core::Engine;
use cloudime_platform::{Config, ConfigError, LogLevel, PhraseStore, WordBank, resources};
use cloudime_windows_server::{
    AssemblySpec, LanguageModelFiles, Router, RouterConfig, ServerError, assembly, dispatch,
};

/// 用户数据目录 `%APPDATA%\CloudIME`。非 Windows 拿不到。
fn user_dir() -> Option<PathBuf> {
    cloudime_platform::dirs::user_dir()
}

fn config_path() -> Option<PathBuf> {
    cloudime_platform::dirs::config_path()
}

/// 首次启动把带说明的配置模板写到 `%APPDATA%\CloudIME\config.toml`；
/// 这时日志还没装好，结果交给 `main` 记。已有文件返回 `Ok(false)`。
fn write_config_template() -> Option<Result<bool, ConfigError>> {
    Some(Config::write_template_if_missing(&config_path()?))
}

/// 文件不存在按默认值；解析失败记错误退回默认。
fn load_config() -> Config {
    match config_path() {
        Some(path) => Config::load(&path).unwrap_or_else(|error| {
            tracing::error!(%error, path = %path.display(), "配置解析失败，用默认值");
            Config::default()
        }),
        None => Config::default(),
    }
}

fn sample_dict(root: &Path) -> PathBuf {
    root.join("assets/sample/dict.tsv")
}

/// 解析 `--wait-pid <pid>`：新起的 Server 用它等旧进程退出再占命名管道。没有这个参数、缺值或值不是
/// 数字都返回 `None`（照常启动，不阻塞）。做成纯函数便于单测。
fn restart_wait_pid(args: impl Iterator<Item = OsString>) -> Option<u32> {
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg.to_str() == Some("--wait-pid") {
            return args.next()?.to_str()?.parse().ok();
        }
    }
    None
}

/// 新起的 Server 等旧进程退出再占命名管道：`OpenProcess` 拿同步句柄，最多等 15 秒兜底。
/// 拿不到句柄（进程已退 / 权限）就跳过、照常启动。
#[cfg(windows)]
fn wait_for_process_exit(pid: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    const TIMEOUT_MS: u32 = 15_000;
    let handle = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(handle) if !handle.is_invalid() => handle,
        _ => {
            tracing::warn!(pid, "拿不到旧 Server 的进程句柄，直接启动");
            return;
        }
    };
    let waited = unsafe { WaitForSingleObject(handle, TIMEOUT_MS) };
    let _ = unsafe { CloseHandle(handle) };
    tracing::info!(pid, ?waited, "新 Server 已等旧进程退出（或超时），继续启动");
}

#[cfg(not(windows))]
fn wait_for_process_exit(_pid: u32) {}

/// 正式词库装配失败回落样例词库，连样例都装不起来才报错。
/// 语言模型坏掉在 [`assembly::assemble`] 里就地降级、不走这里——数据坏了只该掉效果，不该让 Server 起不来
///（装完打不出候选、按键没反应就是这么来的）。
/// 双键拼音方案里 `WordBank` 指的词库（相对方案目录）；没配 / 文件不在 → `None`（照旧用随包主词库）。
fn scheme_word_bank(config: &Config) -> Option<PathBuf> {
    if config.input.input_schemes != cloudime_platform::InputScheme::DualKeyPinyin {
        return None;
    }
    let dir = cloudime_platform::input_schemes::double_keys_dir()?;
    let entries = cloudime_platform::input_schemes::list(&dir);
    let current = cloudime_platform::input_schemes::resolve_current(&dir, &entries)?;
    let entry = entries
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(&current))?;
    let scheme = entry.scheme.as_ref().ok()?;
    let path = dir.join(scheme.word_bank.as_ref()?);
    if !path.is_file() {
        tracing::warn!(path = %path.display(), "方案指定的词库不在，仍用随包主词库");
        return None;
    }
    Some(path)
}

fn assemble_with_fallback(mut spec: AssemblySpec, root: &Path) -> Result<Engine, ServerError> {
    assembly::assemble(&spec).or_else(|error| {
        tracing::error!(%error, dict = %spec.dict.display(), "正式词库装配失败，回落样例词库");
        spec.dict = sample_dict(root);
        assembly::assemble(&spec)
    })
}

/// 三个进程共用的日志目录 `%LOCALAPPDATA%\CloudIME\logs`（见 `cloudime_platform::dirs`），这里顺手建出来。
fn log_dir() -> Option<PathBuf> {
    let dir = cloudime_platform::dirs::log_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// 级别按 `[general] log_level`（`RUST_LOG` 可覆盖），同时写 stderr 与按天滚动的文件（留 7 天）。
/// 返回的 guard 要活到进程结束，否则缓冲的日志不落盘。
fn init_logging(config: &Config) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    let level = if config.general.log_level == LogLevel::Debug {
        "debug"
    } else {
        "info"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    match log_dir() {
        Some(dir) => {
            let appender = tracing_appender::rolling::RollingFileAppender::builder()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("server")
                .filename_suffix("log")
                .max_log_files(7)
                .build(&dir)
                .expect("构建滚动日志文件");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(writer.and(std::io::stderr))
                .init();
            Some(guard)
        }
        None => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
            None
        }
    }
}

fn main() {
    // 日志级别取自配置，所以先写模板、迁移旧配置、读配置，再装日志。
    let template = write_config_template();
    // 装机布局与 exe 同级，开发布局是仓库根；都找不到回落工作目录。
    let root = resources::bundled_root().unwrap_or_else(|| PathBuf::from("."));
    let migration = config_path().map(|path| cloudime_platform::migrate::migrate(&path));
    let config = load_config();
    let _log_guard = init_logging(&config);
    match template {
        Some(Ok(true)) => tracing::info!("已写出配置模板"),
        Some(Err(error)) => tracing::warn!(%error, "写配置模板失败"),
        _ => {}
    }
    if let Some(migration) = migration
        && !migration.is_empty()
    {
        tracing::info!(
            config = migration.config,
            phrases = migration.phrases,
            legacy_phrases = migration.legacy_phrases,
            "旧版配置与短语已迁移"
        );
    }
    // 托盘菜单点了「重启输入法服务」：本进程是先起的新实例，等旧进程退出再占管道（超时 15 秒兜底）
    if let Some(pid) = restart_wait_pid(std::env::args_os().skip(1)) {
        wait_for_process_exit(pid);
    }
    // 词库在随包根的 WordBank\ 下（主词库 Dict.db），用户导入的附加词库也一起加载；一个都没有时回落样例
    let word_bank = WordBank::locate(&root);
    let mut dict = std::env::var_os("CLOUDIME_DICT")
        .map(PathBuf::from)
        .or_else(|| word_bank.main().map(|(_, path)| path))
        .unwrap_or_else(|| sample_dict(&root));
    // 双键拼音方案里 `WordBank` 指了词库：用它顶掉随包主词库（第三方词库与用户自造词库照旧）
    if let Some(path) = scheme_word_bank(&config) {
        dict = path;
    }
    // 用户自造词库按 [word_bank] user_file 定位（相对安装目录，也可绝对路径）
    let user_word_bank = word_bank.user_file(&config.word_bank);
    let spec = AssemblySpec {
        language_model: LanguageModelFiles::find(&root.join("data/generated")),
        word_bank: Some(word_bank.clone()),
        user_dir: user_dir(),
        user_word_bank: Some(user_word_bank),
        input_log: config.general.input_log,
        ..AssemblySpec::new(&dict)
    };
    let mut engine = match assemble_with_fallback(spec, &root) {
        Ok(engine) => engine,
        Err(error) => {
            tracing::error!(%error, "样例词库也装配失败");
            std::process::exit(1);
        }
    };
    engine.set_fuzzy(config.input.fuzzy_rules());
    engine.set_use_jian_pin(config.input.use_jian_pin);
    // 双键拼音：配置里选了它就把当前方案（`use_input_scheme.cfg` 指的那份）的键位表挂上；
    // 方案目录空 / 方案读不出来 → `None`，这一轮退回全键拼音
    engine.set_shuangpin(
        (config.input.input_schemes == cloudime_platform::InputScheme::DualKeyPinyin)
            .then(cloudime_platform::input_schemes::current_table)
            .flatten(),
    );
    // 辅助码：方案开着才挂（只筛选候选，不排序）
    engine.set_auxiliary_codes(
        (config.input.input_schemes == cloudime_platform::InputScheme::DualKeyPinyin)
            .then(cloudime_platform::input_schemes::current_auxiliary_boxed)
            .flatten(),
    );
    // 辅助码怎么对（`use_input_scheme.cfg` 的 `auxiliary_mode`，缺省逐字）
    engine.set_auxiliary_mode(cloudime_platform::input_schemes::current_auxiliary_mode());
    // 双键拼音下显示原始输入按键（配置项，与方案文件无关）
    engine.set_show_raw_input(config.input.show_raw_input);
    // 双键拼音自己的「中英混合输入」（`use_input_scheme.cfg` 的 `mixture_input`，不进 `config.toml`）
    engine.set_shuangpin_mixture_input(cloudime_platform::input_schemes::mixture_input());
    engine.set_mixture_input(config.input.mixture_input);
    engine.set_punctuation_mapping(config.input.punctuation_mapping());
    engine.set_half_wide_after_digit(config.input.use_half_wide_punctuation_marks_after_digital);
    engine.set_association_counts(config.candidate.association_counts());
    engine.set_traditional_mode(
        config.input.simp_trad_chinese_chars_toggle == cloudime_platform::SimpTrad::Traditional,
    );
    engine.set_learning(config.general.learning);
    // 生僻项（词库稀有组）缺省不查，由 [word_bank] rare_items 决定；热加载同款
    engine.set_rare_enabled(config.word_bank.rare_items);
    // 短语库固定在安装目录的 Phrases\Phrase.db；软件自带短语是否参与看 [phrase] use_default_phrases。
    // 读不出来只按没有短语处理。
    let phrase_store = PhraseStore::locate(&root);
    // 内置短语随升级更新：安装包只覆盖同步源 data\phrase-default.db（保住 Phrases\Phrase.db 里的用户短语），
    // 这里启动时把它的 cloudime_default 同步进去（与当前相同就不动）。
    match phrase_store.sync_defaults(&root.join(cloudime_platform::phrase::DEFAULT_SOURCE_FILE)) {
        Ok(true) => tracing::info!("内置短语已按随包同步源更新"),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "内置短语同步失败，用当前已有的"),
    }
    match phrase_store.load(config.phrase.use_default_phrases) {
        Ok(phrases) => {
            if let Err(error) = engine.set_custom_phrases(phrases) {
                tracing::warn!(%error, "短语库内容不合法，本次不启用短语");
            }
        }
        Err(error) => tracing::warn!(%error, "短语库读不出来，本次不启用短语"),
    }
    engine.log_session(env!("CARGO_PKG_VERSION"), "windows");
    let router_config = RouterConfig::from(&config).with_bundled_scripts();
    let mut router = Router::new(engine, router_config.clone());
    // 脚本的 `cloudime.get_curr_config()` 从这里拿到第一份配置（之后热加载会刷新）
    router.set_script_config(&config);
    let model_path = dispatch::find_model(user_dir().as_deref(), &root);
    router.configure_local_model(
        model_path.clone(),
        config.candidate.use_local_sentence_organization_model,
    );
    if let Some(path) = config_path() {
        let user = user_dir();
        router.watch_config(
            &config,
            path,
            dispatch::DataDirs {
                user_root: user.clone(),
                root: Some(root.clone()),
                word_bank: Some(word_bank.clone()),
                main_dict: Some(dict.clone()),
                phrase: Some(phrase_store.clone()),
            },
        );
    }
    tracing::info!(
        dict = %dict.display(),
        page_size = router_config.page_size,
        layout = router_config.layout.key(),
        fuzzy = config.input.fuzzy_rules().any(),
        model = model_path.as_deref().map(|p| p.display().to_string()).unwrap_or_default(),
        model_enabled = config.candidate.use_local_sentence_organization_model,
        sessions = router.session_count(),
        "云朵 Windows Server 就绪"
    );

    serve(router);
}

/// 日志目录 `%LOCALAPPDATA%\CloudIME\logs` 给 AppContainer 应用（任务栏搜索 / 设置）写权限：
/// 那些进程里的 DLL 默认写不了用户目录，出了问题连日志都没有。失败只记警告。
#[cfg(windows)]
fn grant_appcontainer_log_access() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Some(dir) = log_dir() else {
        return;
    };
    // S-1-15-2-1 = ALL APPLICATION PACKAGES，S-1-15-2-2 = ALL RESTRICTED APPLICATION PACKAGES。
    let status = std::process::Command::new("icacls")
        .arg(&dir)
        .args(["/grant", "*S-1-15-2-1:(OI)(CI)M"])
        .args(["/grant", "*S-1-15-2-2:(OI)(CI)M"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!(%status, "给 AppContainer 授权日志目录失败"),
        Err(error) => tracing::warn!(%error, "跑 icacls 失败"),
    }
}

/// 起 UI 线程作为候选窗口 / 状态条的输出端（失败退化为不画），再在命名管道上服务到进程结束。
#[cfg(windows)]
fn serve(mut router: Router) {
    use cloudime_windows_server::ipc::{Work, pipe};
    use cloudime_windows_server::ui::UiHandle;
    grant_appcontainer_log_access();
    // 工人循环的活：各连接的消息 + UI 线程发来的状态条操作与候选窗鼠标操作。
    let (work_tx, work_rx) = std::sync::mpsc::channel::<Work>();
    let status_events = work_tx.clone();
    let on_status = Box::new(move |event| {
        let _ = status_events.send(Work::Status(event));
    });
    let candidate_events = work_tx.clone();
    let on_candidates = Box::new(move |event| {
        let _ = candidate_events.send(Work::Candidate(event));
    });
    // 脚本窗口的鼠标事件：UI 线程（渲染窗口那条线程）→ 工人线程（派给脚本的 `window` 事件）。
    // 多个脚本窗口共享同一份出口，所以用 `Arc`；`Sender` 不是 `Sync`，拿锁包一层。
    let script_window_events = {
        let sender = std::sync::Mutex::new(work_tx.clone());
        std::sync::Arc::new(move |event| {
            if let Ok(sender) = sender.lock() {
                let _ = sender.send(Work::ScriptWindow(Box::new(event)));
            }
        })
    };
    match UiHandle::spawn(on_status, on_candidates, script_window_events) {
        Ok(ui) => {
            router.set_candidate_sink(Box::new(ui.clone()));
            router.set_status_sink(Box::new(ui.clone()));
            router.set_script_window_sink(Box::new(ui));
        }
        Err(error) => tracing::error!(%error, "UI 线程启动失败，将不显示候选框 / 状态条"),
    }
    if let Err(error) = pipe::serve_pipe(pipe::DEFAULT_PIPE_NAME, &mut router, work_tx, work_rx) {
        tracing::error!(%error, "命名管道服务退出");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn serve(_router: Router) {
    tracing::warn!("命名管道传输仅 Windows 提供；本平台只装配 Engine 供测试");
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::restart_wait_pid;

    fn args(items: &[&str]) -> impl Iterator<Item = OsString> {
        items
            .iter()
            .map(|item| OsString::from(*item))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parses_wait_pid_flag() {
        assert_eq!(restart_wait_pid(args(&["--wait-pid", "4321"])), Some(4321));
        // 前面还有别的参数
        assert_eq!(
            restart_wait_pid(args(&["--foo", "--wait-pid", "7"])),
            Some(7)
        );
        // 有参数缺值、值不是数字、根本没有这个参数
        assert_eq!(restart_wait_pid(args(&["--wait-pid"])), None);
        assert_eq!(restart_wait_pid(args(&["--wait-pid", "abc"])), None);
        assert_eq!(restart_wait_pid(args(&[])), None);
    }
}
