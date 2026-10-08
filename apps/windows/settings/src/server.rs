//! 与输入法服务（Server）的一次性交互：现在只有「请它重启」这一条。
//!
//! 和任务栏右键菜单的「重启输入法服务」走**同一条路**：往它的命名管道发一条
//! `ClientMessage::Indicator { RestartServer }`。Server 收到后自己起一个新实例（带 `--wait-pid`，
//! 等本进程退出再占管道），回完这一包就退出 —— 新实例几秒后接管，各应用里的 DLL 会重新连上。
//!
//! 这条消息 Server 不回包（`dispatch` 回 `None`），所以发完就完、不用等回包。
//!
//! 设置程序平时不跟 Server 通信（配置写 `config.toml`，Server 自己热加载），这里是唯一的例外。

use cloudime_platform::protocol::{
    ClientMessage, DEFAULT_PIPE_NAME, IndicatorCommand, SessionId, write_message,
};

/// 请正在跑的输入法服务重启（与任务栏右键「重启输入法服务」同一条路）。
/// 服务没在跑 / 连不上时返回一句给用户看的原因。
pub(crate) fn restart() -> Result<(), String> {
    restart_on(DEFAULT_PIPE_NAME)
}

/// 连 `name` 这条管道发重启指令。管道名做成参数，测试能拿一个不存在的名字走错误分支
///（真连上去会把开发机上正在跑的输入法服务重启掉）。
fn restart_on(name: &str) -> Result<(), String> {
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .map_err(|error| format!("连不上输入法服务（{name}）：{error}"))?;
    // 会话号只是给 Server 记日志用，它不解释这个值。
    let message = ClientMessage::Indicator {
        session: SessionId(0),
        command: IndicatorCommand::RestartServer,
    };
    write_message(&mut pipe, &message).map_err(|error| format!("发送重启请求失败：{error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::restart_on;

    /// 服务没在跑（管道不存在）：给一句能看的原因，不 panic。
    #[test]
    fn a_missing_server_reports_a_reason() {
        let error = restart_on(r"\\.\pipe\cloudime-settings-test-no-such-pipe")
            .expect_err("不存在的管道该报错");
        assert!(error.contains("连不上输入法服务"), "{error}");
    }
}
