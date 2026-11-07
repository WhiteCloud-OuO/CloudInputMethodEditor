//! MCP Client sidecar 的装配：读 `mcp.toml`、懒启动 sidecar、把 Lua 请求转成命名管道帧。
//!
//! 输入工人线程只做「把请求塞进 mpsc」；真正的子进程、管道读写和超时都在这里自己的线程里。
//! 每条请求一个转发线程，响应由独立的读线程按请求号分发，所以不同外部 MCP Server 可以并行。

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cloudime_mcp_client::{
    IPC_PIPE, McpRequest, McpResponse, RequestAction, ResponseBody, ServerSummary, load_config,
    read_message, write_message,
};
use cloudime_script::Runtime;

/// sidecar 名字（安装目录 / 调试时的 exe 旁边）。
const SIDECAR_EXE: &str = "cloudime-mcp-client.exe";

/// 启动 sidecar 后等它把管道建好的最长时间。
const PIPE_TIMEOUT: Duration = Duration::from_secs(5);

/// 单条代理请求在 Server 侧的最高等待时间（真正时限由 Lua 与 sidecar 管）。
const REQUEST_BACKSTOP: Duration = Duration::from_secs(60);

/// 有 `mcp.toml` 就把 sidecar 代理接上；没有则脚本的 `cloudime.mcp` 保持不可用。
pub fn attach(scripts: &Runtime) {
    let Some(path) = config_path() else {
        return;
    };
    if !path.is_file() {
        return;
    }
    let config = match load_config(&path) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "读取 mcp.toml 失败，MCP Client 不启用");
            return;
        }
    };
    if config.servers.is_empty() {
        return;
    }
    let Some(exe) = sidecar_path() else {
        tracing::warn!(
            exe = SIDECAR_EXE,
            "找不到 MCP Client sidecar，MCP Client 不启用"
        );
        return;
    };
    let summaries: Vec<ServerSummary> = config
        .servers
        .iter()
        .map(|server| ServerSummary {
            name: server.name.clone(),
            allowed_tools: server.allowed_tools.clone(),
        })
        .collect();
    let (requests, incoming) = mpsc::channel::<McpRequest>();
    let results = scripts.attach_mcp(summaries, requests);
    let spawned = std::thread::Builder::new()
        .name("cloudime-mcp-proxy".to_owned())
        .spawn(move || proxy_loop(path, exe, incoming, results));
    if let Err(error) = spawned {
        tracing::warn!(%error, "启动 MCP Client 代理线程失败");
    }
}

fn config_path() -> Option<PathBuf> {
    cloudime_platform::dirs::config_path().map(|path| path.with_file_name("mcp.toml"))
}

fn sidecar_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let path = dir.join(SIDECAR_EXE);
        if path.is_file() {
            return Some(path);
        }
    }
    let root = cloudime_platform::resources::bundled_root()?;
    let path = root.join(SIDECAR_EXE);
    path.is_file().then_some(path)
}

fn proxy_loop(
    config: PathBuf,
    exe: PathBuf,
    requests: Receiver<McpRequest>,
    results: Sender<(u64, ResponseBody)>,
) {
    let shared = Arc::new(Mutex::new(None::<Arc<Link>>));
    let config = Arc::new(config);
    let exe = Arc::new(exe);
    while let Ok(request) = requests.recv() {
        let shared = shared.clone();
        let config = config.clone();
        let exe = exe.clone();
        let results = results.clone();
        let id = request.id;
        let worker_results = results.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("cloudime-mcp-request-{id}"))
            .spawn(move || {
                let body = handle_request(&shared, &config, &exe, &request);
                let _ = worker_results.send((request.id, body));
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "启动 MCP 请求转发线程失败");
            let _ = results.send((
                id,
                ResponseBody::Error {
                    error: "启动 MCP 请求转发线程失败".to_owned(),
                    client_owned: false,
                },
            ));
        }
    }
}

fn handle_request(
    shared: &Mutex<Option<Arc<Link>>>,
    config: &Path,
    exe: &Path,
    request: &McpRequest,
) -> ResponseBody {
    // 停止 / 关闭在没有 sidecar 时也算成功：链尾清理不该再报错。
    if let RequestAction::StopServer { server } = &request.action
        && link(shared).is_none()
    {
        return ResponseBody::Stopped {
            server: server.clone(),
        };
    }
    if matches!(request.action, RequestAction::Shutdown) && link(shared).is_none() {
        return ResponseBody::Shutdown;
    }

    let allow_spawn = matches!(request.action, RequestAction::StartServer { .. });
    let link = match link_for(shared, config, exe, allow_spawn) {
        Ok(Some(link)) => link,
        Ok(None) => {
            return ResponseBody::Error {
                error: "MCP Client 未启动；请先手动启动 sidecar 或调用 cloudime.mcp.start"
                    .to_owned(),
                client_owned: false,
            };
        }
        Err(error) => {
            return ResponseBody::Error {
                error,
                client_owned: false,
            };
        }
    };
    let owned = link.owns_process();
    let response = match link.exchange(request) {
        Ok(response) if response.id == request.id => response.body,
        Ok(response) => ResponseBody::Error {
            error: format!(
                "MCP sidecar 响应 id 不匹配：要 {}，给 {}",
                request.id, response.id
            ),
            client_owned: owned,
        },
        Err(error) => ResponseBody::Error {
            error,
            client_owned: owned,
        },
    };
    let response = mark_owned(response, owned);
    if matches!(request.action, RequestAction::Shutdown) && owned {
        link.wait_for_exit(Duration::from_secs(2));
    }
    response
}

fn link(shared: &Mutex<Option<Arc<Link>>>) -> Option<Arc<Link>> {
    let mut slot = shared.lock().expect("MCP 代理锁");
    if slot.as_ref().is_some_and(|link| link.alive()) {
        return slot.clone();
    }
    *slot = None;
    None
}

fn link_for(
    shared: &Mutex<Option<Arc<Link>>>,
    config: &Path,
    exe: &Path,
    allow_spawn: bool,
) -> Result<Option<Arc<Link>>, String> {
    let mut slot = shared.lock().expect("MCP 代理锁");
    if let Some(link) = slot.as_ref()
        && link.alive()
    {
        return Ok(Some(link.clone()));
    }
    *slot = None;
    if !allow_spawn {
        return Ok(None);
    }
    let link = Arc::new(Link::open(config, exe)?);
    *slot = Some(link.clone());
    Ok(Some(link))
}

fn mark_owned(mut body: ResponseBody, owned: bool) -> ResponseBody {
    match &mut body {
        ResponseBody::Started { client_owned, .. } | ResponseBody::Error { client_owned, .. } => {
            *client_owned = owned
        }
        _ => {}
    }
    body
}

type Waiter = Sender<Result<McpResponse, String>>;

/// 一条到 sidecar 的连接：写锁 + 后台读线程 + 按请求号等待的槽位。
struct Link {
    writer: Mutex<File>,
    waiters: Arc<Mutex<HashMap<u64, Waiter>>>,
    alive: Arc<AtomicBool>,
    child: Mutex<Option<Child>>,
}

impl Link {
    /// 先连已有管道；没有就启动 sidecar，再等它建好管道。
    fn open(config: &Path, exe: &Path) -> Result<Self, String> {
        let (pipe, mut child) = if let Ok(pipe) = open_pipe() {
            (pipe, None)
        } else {
            let mut child = spawn(exe, config)?;
            let deadline = Instant::now() + PIPE_TIMEOUT;
            loop {
                match open_pipe() {
                    Ok(pipe) => break (pipe, Some(child)),
                    Err(error) => {
                        if let Some(status) = child
                            .try_wait()
                            .map_err(|error| format!("等待 MCP sidecar 启动失败：{error}"))?
                        {
                            return Err(format!("MCP sidecar 启动后退出：{status}"));
                        }
                        if Instant::now() > deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(format!("连接 MCP sidecar 管道超时：{error}"));
                        }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                }
            }
        };
        let writer = match pipe.try_clone() {
            Ok(writer) => writer,
            Err(error) => {
                kill(child.as_mut());
                return Err(format!("复制 MCP 管道句柄失败：{error}"));
            }
        };
        let waiters = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        {
            let waiters = waiters.clone();
            let alive = alive.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("cloudime-mcp-reader".to_owned())
                .spawn(move || read_loop(pipe, waiters, alive))
            {
                kill(child.as_mut());
                return Err(format!("启动 MCP 读线程失败：{error}"));
            }
        }
        Ok(Self {
            writer: Mutex::new(writer),
            waiters,
            alive,
            child: Mutex::new(child),
        })
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    fn owns_process(&self) -> bool {
        self.child.lock().expect("MCP 子进程锁").is_some()
    }

    fn shutdown(&self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Some(mut child) = self.child.lock().expect("MCP 子进程锁").take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// `Shutdown` 之后等 sidecar 自己退（它会先关外部 MCP Server）；超时才强杀。
    fn wait_for_exit(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let mut child = self.child.lock().expect("MCP 子进程锁");
                match child.as_mut() {
                    None => {
                        self.alive.store(false, Ordering::Relaxed);
                        return;
                    }
                    Some(process) => match process.try_wait() {
                        Ok(Some(_)) => {
                            child.take();
                            self.alive.store(false, Ordering::Relaxed);
                            return;
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    },
                }
            }
            if Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.shutdown();
    }

    fn exchange(&self, request: &McpRequest) -> Result<McpResponse, String> {
        if !self.alive() {
            return Err("MCP sidecar 已断开".to_owned());
        }
        let (send, receive) = mpsc::channel();
        self.waiters
            .lock()
            .expect("MCP 等待表")
            .insert(request.id, send);
        {
            let mut writer = self.writer.lock().expect("MCP 写锁");
            if let Err(error) = write_message(&mut *writer, request) {
                self.waiters.lock().expect("MCP 等待表").remove(&request.id);
                self.alive.store(false, Ordering::Relaxed);
                return Err(format!("写 MCP 代理请求失败：{error}"));
            }
        }
        match receive.recv_timeout(REQUEST_BACKSTOP) {
            Ok(result) => result,
            Err(_) => {
                self.waiters.lock().expect("MCP 等待表").remove(&request.id);
                Err("等待 MCP sidecar 响应超时".to_owned())
            }
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.shutdown();
        for (_, waiter) in self.waiters.lock().expect("MCP 等待表").drain() {
            let _ = waiter.send(Err("MCP sidecar 已关闭".to_owned()));
        }
    }
}

fn read_loop(mut pipe: File, waiters: Arc<Mutex<HashMap<u64, Waiter>>>, alive: Arc<AtomicBool>) {
    loop {
        match read_message::<McpResponse>(&mut pipe) {
            Ok(response) => {
                if let Some(waiter) = waiters.lock().expect("MCP 等待表").remove(&response.id) {
                    let _ = waiter.send(Ok(response));
                }
            }
            Err(error) => {
                alive.store(false, Ordering::Relaxed);
                let message = format!("MCP sidecar 连接断开：{error}");
                for (_, waiter) in waiters.lock().expect("MCP 等待表").drain() {
                    let _ = waiter.send(Err(message.clone()));
                }
                break;
            }
        }
    }
}

fn open_pipe() -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).open(IPC_PIPE)
}

fn kill(child: Option<&mut Child>) {
    if let Some(child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn spawn(exe: &Path, config: &Path) -> Result<Child, String> {
    let mut command = Command::new(exe);
    command
        .arg("--pipe")
        .arg(IPC_PIPE)
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 MCP sidecar 失败：{error}"))?;
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                tracing::warn!(mcp_sidecar_stderr = %line, "MCP sidecar 的 stderr");
            }
        });
    }
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tools_and_calls_do_not_start_the_sidecar() {
        let shared = Mutex::new(None);
        let path = Path::new("unused.exe");
        let request = McpRequest {
            id: 1,
            action: RequestAction::Call {
                server: "search".to_owned(),
                tool: "find".to_owned(),
                arguments: json!({ "q": "云朵" }),
                timeout_ms: 1_000,
            },
        };
        match handle_request(&shared, path, path, &request) {
            ResponseBody::Error { error, .. } => assert!(error.contains("未启动"), "{error}"),
            body => panic!("应为 Error：{body:?}"),
        }
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn stopping_without_a_client_is_a_no_op() {
        let shared = Mutex::new(None);
        let path = Path::new("unused.exe");
        let stop = McpRequest {
            id: 2,
            action: RequestAction::StopServer {
                server: "search".to_owned(),
            },
        };
        assert!(matches!(
            handle_request(&shared, path, path, &stop),
            ResponseBody::Stopped { .. }
        ));
        let shutdown = McpRequest {
            id: 3,
            action: RequestAction::Shutdown,
        };
        assert!(matches!(
            handle_request(&shared, path, path, &shutdown),
            ResponseBody::Shutdown
        ));
    }
}
