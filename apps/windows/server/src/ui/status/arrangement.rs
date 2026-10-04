//! 悬浮状态条的图标按钮：读 exe 旁 `data\icons-arrangement.cfg`，按 `pos` 从左到右排。
//!
//! 一行一个状态：`button=ch; icon=icons\ch.svg; pos=0`。同一个 `pos` 的几行是同一个按钮的不同状态
//!（中 / 英 / 大写锁定、半角 / 全角、中文标点 / 英文标点、简 / 繁），`pos=-1` 的行表示这张图标不显示。
//! 图标路径相对 cfg 所在目录，空行与 `#` 开头的行忽略。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::ACTIONS;
use super::placement::StatusAction;

/// 挑图标要用的当前状态。
#[derive(Debug, Clone, Copy)]
pub(super) struct ButtonState {
    pub(super) english: bool,
    pub(super) caps: bool,
    pub(super) full_width_punctuation: bool,
    pub(super) full_width_chars: bool,
    pub(super) traditional: bool,
}

/// 一个按钮：点击动作 + 它的几个状态图标。
pub(super) struct Button {
    pub(super) action: StatusAction,

    /// 在状态条上从左到右的位置（`pos`，越小越靠左）；`-1` 的行不会出现在这里。
    pos: i32,

    /// 状态名（cfg 里的 `button=`）与 SVG 源码，按 cfg 里的出现顺序。
    variants: Vec<(String, String)>,
}

impl Button {
    /// 当前状态该显示哪张图标：按状态名挑；这一组里没有那个状态就退到第一张（cfg 少写一行也还画得出来）。
    pub(super) fn svg(&self, state: &ButtonState) -> &str {
        let wanted = match self.action {
            StatusAction::ToggleLang => {
                if state.caps {
                    "caps"
                } else if state.english {
                    "en"
                } else {
                    "ch"
                }
            }
            StatusAction::ToggleCharWidthType => {
                if state.full_width_chars {
                    "full"
                } else {
                    "half"
                }
            }
            StatusAction::TogglePunctuation => {
                if state.full_width_punctuation {
                    "ch_marks"
                } else {
                    "en_marks"
                }
            }
            StatusAction::ToggleSimpTrad => {
                if state.traditional {
                    "trad_ch"
                } else {
                    "simp_ch"
                }
            }
            StatusAction::OpenOptions | StatusAction::OpenWidgets | StatusAction::OpenSpecChars => {
                ""
            }
        };
        self.variants
            .iter()
            .find(|(name, _)| name == wanted)
            .or_else(|| self.variants.first())
            .map_or("", |(_, svg)| svg.as_str())
    }
}

/// 一排按钮与 cfg 的最后写入时间（没变就不重读）。
pub(super) struct Arrangement {
    /// 排布文件；取不到 exe 的路径时为 `None`。
    file: Option<PathBuf>,

    /// 上次读它时的时间戳。
    stamp: Option<SystemTime>,

    /// 按 `pos` 排好的按钮。
    pub(super) buttons: Vec<Button>,
}

impl Arrangement {
    /// 读 exe 旁的 `data\icons-arrangement.cfg`；文件不在或读不了就没有按钮（调用方据此不显示状态条）。
    pub(super) fn load() -> Self {
        let mut arrangement = Self {
            file: config_path(),
            stamp: None,
            buttons: Vec::new(),
        };
        arrangement.reload();
        tracing::debug!(
            buttons = arrangement.buttons.len(),
            file = ?arrangement.file,
            "状态条图标排布已装载"
        );
        arrangement
    }

    /// cfg 动过了就重读一遍；返回有没有真的重读过（重读了调用方要重画）。
    pub(super) fn refresh(&mut self) -> bool {
        let stamp = self.file.as_deref().and_then(modified);
        if stamp == self.stamp {
            return false;
        }
        self.reload()
    }

    /// 重读一遍。读不出来（文件不在 / 权限）就沿用上一次那排按钮，时间戳照记，免得每次重画都再报一遍。
    fn reload(&mut self) -> bool {
        let Some(file) = self.file.as_deref() else {
            return false;
        };
        self.stamp = modified(file);
        match std::fs::read_to_string(file) {
            Ok(text) => {
                self.buttons = parse(file, &text);
                true
            }
            Err(error) => {
                tracing::warn!(%error, path = %file.display(), "状态条图标排布读不了，沿用上一次的");
                false
            }
        }
    }
}

/// 图标排布文件：exe 旁的 `data\icons-arrangement.cfg`（`cargo build` 拷贝一份过去，装机包里也是这个位置）。
fn config_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("data").join("icons-arrangement.cfg"))
}

/// 文件最后写入时间；文件不在或取不到为 `None`（与「上次是 `None`」相等，就不会白重读）。
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

/// 解析排布：按 `pos` 并成按钮、`-1` 的行丢掉，看不懂的行各记一条日志跳过。
fn parse(file: &Path, text: &str) -> Vec<Button> {
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    let mut buttons: Vec<Button> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, icon, pos)) = fields(line) else {
            tracing::warn!(line, "状态条图标排布这行看不懂，跳过");
            continue;
        };
        if pos < 0 {
            continue;
        }
        let Some(action) = action_of(&name) else {
            tracing::warn!(button = name, "状态条图标排布里有不认识的按钮名，跳过");
            continue;
        };
        let path = dir.join(&icon);
        let svg = match std::fs::read_to_string(&path) {
            Ok(svg) => svg,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "状态条图标读不了，跳过");
                continue;
            }
        };
        push(&mut buttons, pos, action, name, svg);
    }
    buttons.sort_by_key(|button| button.pos);
    buttons
}

/// `ACTIONS` 里查得到的动作。
fn action_of(name: &str) -> Option<StatusAction> {
    ACTIONS
        .iter()
        .find(|(button, _)| *button == name)
        .map(|(_, action)| *action)
}

/// 把一行塞进按钮表：同一个 `pos` 且同一个动作就并进那一组，否则新起一个按钮。
fn push(buttons: &mut Vec<Button>, pos: i32, action: StatusAction, name: String, svg: String) {
    match buttons.iter_mut().find(|button| button.pos == pos) {
        Some(button) if button.action == action => button.variants.push((name, svg)),
        Some(button) => tracing::warn!(
            pos,
            action = ?button.action,
            "状态条图标排布里同一个 pos 上动作对不上，跳过这一行"
        ),
        None => buttons.push(Button {
            action,
            pos,
            variants: vec![(name, svg)],
        }),
    }
}

/// 拆一行：`button=ch; icon=icons\ch.svg; pos=0`；字段顺序随意、两边空白忽略，缺一个字段就整行不要。
fn fields(line: &str) -> Option<(String, String, i32)> {
    let mut name = None;
    let mut icon = None;
    let mut pos = None;
    for field in line.split(';') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        let (key, value) = field.split_once('=')?;
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "button" => name = Some(value.to_owned()),
            "icon" => icon = Some(value.to_owned()),
            "pos" => pos = value.parse().ok(),
            other => tracing::warn!(field = other, "状态条图标排布里有不认识的字段，忽略"),
        }
    }
    Some((name?, icon?, pos?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ButtonState {
        ButtonState {
            english: false,
            caps: false,
            full_width_punctuation: true,
            full_width_chars: false,
            traditional: false,
        }
    }

    /// 一个临时目录：`icons\` 下三张小图 + 一份排布表。
    fn fixture(name: &str, cfg: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cloudime-arrangement-{name}"));
        let icons = dir.join("icons");
        std::fs::create_dir_all(&icons).unwrap();
        for icon in ["ch", "en", "caps", "half", "full", "options"] {
            std::fs::write(
                icons.join(format!("{icon}.svg")),
                format!("<svg id=\"{icon}\"/>"),
            )
            .unwrap();
        }
        std::fs::write(dir.join("icons-arrangement.cfg"), cfg).unwrap();
        dir.join("icons-arrangement.cfg")
    }

    #[test]
    fn groups_states_of_one_button_and_sorts_by_pos() {
        let file = fixture(
            "groups",
            "button=options; icon=icons\\options.svg; pos=2\n\
             button=en; icon=icons\\en.svg; pos=0\n\
             button=ch; icon=icons\\ch.svg; pos=0\n\
             button=caps; icon=icons\\caps.svg; pos=0\n\
             # 半角不显示\n\
             button=half; icon=icons\\half.svg; pos=-1\n",
        );
        let buttons = parse(&file, &std::fs::read_to_string(&file).unwrap());
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0].action, StatusAction::ToggleLang);
        assert_eq!(buttons[1].action, StatusAction::OpenOptions);
        // 三张图标归一个按钮：中文 / 英文 / Caps
        assert_eq!(buttons[0].svg(&state()), "<svg id=\"ch\"/>");
        let english = ButtonState {
            english: true,
            ..state()
        };
        assert_eq!(buttons[0].svg(&english), "<svg id=\"en\"/>");
        let caps = ButtonState {
            caps: true,
            ..english
        };
        assert_eq!(buttons[0].svg(&caps), "<svg id=\"caps\"/>");
    }

    #[test]
    fn missing_state_falls_back_to_the_first_icon() {
        let file = fixture("fallback", "button=ch; icon=icons\\ch.svg; pos=0\n");
        let buttons = parse(&file, &std::fs::read_to_string(&file).unwrap());
        let english = ButtonState {
            english: true,
            ..state()
        };
        assert_eq!(buttons[0].svg(&english), "<svg id=\"ch\"/>");
    }

    #[test]
    fn unknown_button_name_and_missing_icon_are_skipped() {
        let file = fixture(
            "unknown",
            "button=nope; icon=icons\\ch.svg; pos=0\n\
             button=ch; icon=icons\\gone.svg; pos=1\n\
             button=ch; icon=icons\\ch.svg; pos=2\n",
        );
        let buttons = parse(&file, &std::fs::read_to_string(&file).unwrap());
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].svg(&state()), "<svg id=\"ch\"/>");
    }

    #[test]
    fn refresh_only_reads_after_the_file_changes() {
        let file = fixture("refresh", "button=ch; icon=icons\\ch.svg; pos=0\n");
        let mut arrangement = Arrangement {
            file: Some(file.clone()),
            stamp: None,
            buttons: Vec::new(),
        };
        assert!(arrangement.refresh()); // 首次：stamp 还空着
        assert!(!arrangement.refresh()); // 没动过就不再读
        // 等一会儿再改，保证时间戳真的往后走了（文件系统的时间戳精度不保证到毫秒）
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, "button=options; icon=icons\\options.svg; pos=0\n").unwrap();
        assert!(arrangement.refresh());
        assert!(!arrangement.refresh());
        assert_eq!(arrangement.buttons.len(), 1);
        assert_eq!(arrangement.buttons[0].action, StatusAction::OpenOptions);
    }

    #[test]
    fn parses_fields_in_any_order() {
        assert_eq!(
            fields("pos=3; icon=icons\\ch.svg; button=ch"),
            Some(("ch".to_owned(), "icons\\ch.svg".to_owned(), 3))
        );
        assert_eq!(fields("button=ch; icon=icons\\ch.svg"), None);
        assert_eq!(fields("button=ch; icon=icons\\ch.svg; pos=x"), None);
    }
}
