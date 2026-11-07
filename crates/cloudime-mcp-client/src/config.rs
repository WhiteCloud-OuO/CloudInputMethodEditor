//! `mcp.toml` 的读取与校验。

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

/// 配置文件读取失败。
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("read mcp config: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse mcp config: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("mcp server name is empty")]
    EmptyName,

    #[error("mcp server `{0}` has an empty command")]
    EmptyCommand(String),

    #[error("duplicate mcp server `{0}`")]
    Duplicate(String),
}

/// `mcp.toml` 的顶层内容。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpConfig {
    /// 已配置的 MCP Server。
    #[serde(default)]
    pub servers: Vec<ServerConfig>,
}

impl McpConfig {
    /// 按脚本里的名字找一个 Server。
    pub fn server(&self, name: &str) -> Option<&ServerConfig> {
        self.servers.iter().find(|server| server.name == name)
    }
}

/// 一个 stdio MCP Server 的配置。
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// 脚本调用时使用的名字。
    pub name: String,

    /// 启动命令。
    pub command: String,

    /// 命令行参数。
    #[serde(default)]
    pub args: Vec<String>,

    /// 子进程环境变量。
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// 单次 MCP 调用超时（毫秒）。
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,

    /// 允许脚本调用的工具；为空表示不允许任何工具。
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

impl ServerConfig {
    /// 这个工具是否允许被脚本调用。
    pub fn allows(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|allowed| allowed == tool)
    }
}

fn default_timeout() -> u64 {
    10_000
}

/// 读取并校验 `mcp.toml`。
pub fn load_config(path: &Path) -> Result<McpConfig, ConfigError> {
    let text = std::fs::read_to_string(path)?;
    let config: McpConfig = toml::from_str(&text)?;
    let mut names = std::collections::HashSet::new();
    for server in &config.servers {
        if server.name.trim().is_empty() {
            return Err(ConfigError::EmptyName);
        }
        if server.command.trim().is_empty() {
            return Err(ConfigError::EmptyCommand(server.name.clone()));
        }
        if !names.insert(server.name.clone()) {
            return Err(ConfigError::Duplicate(server.name.clone()));
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, text: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cloudime-mcp-config-{name}-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn reads_servers_and_defaults() {
        let path = write(
            "ok",
            r#"
                [[servers]]
                name = "search"
                command = "mcp-search.exe"
                allowed_tools = ["find"]
            "#,
        );
        let config = load_config(&path).unwrap();
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].timeout_ms, 10_000);
        assert!(config.servers[0].allows("find"));
        assert!(!config.servers[0].allows("delete"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_duplicates_and_empty_commands() {
        let duplicate = write(
            "duplicate",
            r#"
                [[servers]]
                name = "a"
                command = "x.exe"

                [[servers]]
                name = "a"
                command = "y.exe"
            "#,
        );
        assert!(matches!(
            load_config(&duplicate),
            Err(ConfigError::Duplicate(_))
        ));

        let empty = write(
            "empty",
            r#"
                [[servers]]
                name = "a"
                command = "  "
            "#,
        );
        assert!(matches!(
            load_config(&empty),
            Err(ConfigError::EmptyCommand(_))
        ));
        let _ = std::fs::remove_file(duplicate);
        let _ = std::fs::remove_file(empty);
    }
}
