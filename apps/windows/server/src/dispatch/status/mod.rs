//! 全局中英模式与悬浮状态条：模式只有 Server 这一份，DLL 切了用 `ModeChanged` 报来，激活 / 获焦 / 轮询时用
//! `SyncMode` 取走（取的同时说明云朵输入法是当前输入法，状态条显示）；切成别的输入法时 DLL 发 `ImeSwitched` 收起。
//! 会话关闭（应用退出）不收——状态条常驻桌面。状态条上的点击经 [`StatusEvent`] 回到这里：
//! 切模式直接改全局模式，各 DLL 下一拍取走；切标点只改会话内状态（不落盘，重启回缺省），拖动写回配置文件（热加载会再读回来）。

mod event;
mod sink;
mod view;

use cloudime_platform::Config;
use cloudime_platform::SimpTrad;
use cloudime_platform::protocol::{IndicatorCommand, InputMode};

pub use self::event::StatusEvent;
pub use self::sink::{NoopStatusSink, StatusSink};
pub use self::view::StatusView;
use super::Router;

impl Router {
    /// DLL 那边用户切了模式（中 / 英或禁用）：成为全局状态。
    pub(super) fn handle_mode_changed(&mut self, mode: InputMode) {
        self.mode = mode;
        self.ime_active = true;
        self.reconcile_status();
    }

    /// 有 DLL 来取模式：云朵输入法是当前输入法。
    pub(super) fn handle_ime_active(&mut self) {
        if !self.ime_active {
            self.ime_active = true;
            self.reconcile_status();
        }
    }

    pub(super) fn handle_ime_switched(&mut self) {
        self.ime_active = false;
        self.reconcile_status();
    }

    /// 状态条上的操作。
    pub fn handle_status_event(&mut self, event: StatusEvent) {
        match event {
            StatusEvent::ToggleLang => {
                // 状态条上的「中 / 英」：中文 ↔ 英文；禁用时点它（一般点不到，状态条收着）回到英文。
                self.mode = if self.mode == InputMode::English {
                    InputMode::Chinese
                } else {
                    InputMode::English
                };
                tracing::info!(mode = ?self.mode, "状态条：切换中英模式（用户点了中 / 英按钮）");
            }
            StatusEvent::TogglePunctuation => {
                // 中英各记一份，切的是当前模式那份；还没报过模式时按中文算。
                // 只改会话内状态，不写回配置文件（这两项不在配置里，重启 Server 回缺省）。
                let english = self.mode.english();
                let full_width = !self.full_width_punctuation_for(english);
                if english {
                    self.config.english_full_width_punctuation = full_width;
                } else {
                    self.config.full_width_punctuation = full_width;
                }
                tracing::debug!(english, full_width, "状态条：切换全角标点");
            }
            StatusEvent::ToggleCharWidthType => {
                // 会话内状态，不分中英（与「全角标点」不同：那个中英各记一份）。
                self.config.full_width_chars = !self.config.full_width_chars;
                tracing::debug!(
                    full_width_chars = self.config.full_width_chars,
                    "状态条：切换全角字符"
                );
            }
            StatusEvent::ToggleSimpTrad => {
                // 繁体是配置文件里的一项（设置页也有），切了立即生效并写回，热加载再读回来是同一个值。
                let traditional = !self.config.traditional;
                self.config.traditional = traditional;
                self.engine.set_traditional_mode(traditional);
                let value = if traditional {
                    SimpTrad::Traditional
                } else {
                    SimpTrad::Simplified
                };
                self.persist("input", "simp_trad_chinese_chars_toggle", value.key());
                tracing::debug!(traditional, "状态条：切换简繁");
            }
            StatusEvent::Moved(x, y) => {
                self.config.status_pos = Some((x, y));
                self.persist("status_bar", "x", i64::from(x));
                self.persist("status_bar", "y", i64::from(y));
            }
        }
        self.reconcile_status();
    }

    /// 任务栏图标右键菜单 / DLL 侧内置热键：标点切换只改会话内状态，全角字符与简繁同状态条那两格。
    pub(super) fn handle_indicator(&mut self, command: IndicatorCommand) {
        match command {
            IndicatorCommand::TogglePunctuation => {
                self.handle_status_event(StatusEvent::TogglePunctuation);
            }
            IndicatorCommand::ToggleCharWidthType => {
                self.handle_status_event(StatusEvent::ToggleCharWidthType);
            }
            IndicatorCommand::ToggleSimpTrad => {
                self.handle_status_event(StatusEvent::ToggleSimpTrad);
            }
            IndicatorCommand::OpenSettings => self.status.open_settings(),
            IndicatorCommand::OpenDownload => self.status.open_download(),
            IndicatorCommand::RestartServer => {
                tracing::info!("任务栏菜单：重启输入法服务");
                spawn_replacement_server();
                self.restart_pending = true;
            }
        }
    }

    /// 写回配置文件一个键；没有配置路径（测试）就只改内存。
    fn persist(&self, section: &str, key: &str, value: impl Into<toml_edit::Value>) {
        let Some(path) = self.config_path() else {
            return;
        };
        if let Err(error) = Config::set_value(path, section, key, value) {
            tracing::warn!(%error, section, key, "写回配置失败");
        }
    }

    /// 当前模式下标点转不转全角：中英各记一份（会话内状态，不进配置文件）。
    pub(super) fn full_width_punctuation_for(&self, english: bool) -> bool {
        if english {
            self.config.english_full_width_punctuation
        } else {
            self.config.full_width_punctuation
        }
    }

    /// 云朵输入法在前台、且没被禁用就显示，否则收起。热加载后也调一次。
    /// `[status_bar] show_status_bar` 关掉时始终收起（这条工具条不出现）。
    pub(super) fn reconcile_status(&mut self) {
        let show = self.config.show_status_bar && self.ime_active && !self.mode.disabled();
        match show.then_some(self.mode.english()) {
            Some(english) => {
                self.status.show_status(StatusView {
                    english,
                    full_width_punctuation: self.full_width_punctuation_for(english),
                    full_width_chars: self.config.full_width_chars,
                    traditional: self.config.traditional,
                    anchor: self.config.status_pos,
                    auto_hide_fullscreen: self.config.auto_hide_float_tool_bar,
                });
            }
            None => self.status.hide_status(),
        }
    }
}

/// 起一个新的 Server 实例接替本进程：带 `--wait-pid <本进程 pid>` 让它等本进程退出后再占命名管道
///（两个 Server 建管道时第二个会被 `FILE_FLAG_FIRST_PIPE_INSTANCE` 挡下）。工作目录设为 exe 所在目录，
/// 随包数据按 exe 位置找；`CREATE_NO_WINDOW` 避免弹控制台。UiAccess 的 exe 由同样带 UiAccess 的 Server 起没问题。
#[cfg(windows)]
fn spawn_replacement_server() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            tracing::error!(%error, "取当前 Server 路径失败，无法重启输入法服务");
            return;
        }
    };
    let mut command = std::process::Command::new(&exe);
    command
        .arg("--wait-pid")
        .arg(std::process::id().to_string())
        .creation_flags(CREATE_NO_WINDOW);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    match command.spawn() {
        Ok(child) => {
            tracing::info!(
                pid = child.id(),
                "已启动新的 cloudime-server，本进程退出后由它接管"
            )
        }
        Err(error) => tracing::error!(%error, "启动新的 cloudime-server 失败"),
    }
}

#[cfg(not(windows))]
fn spawn_replacement_server() {}
