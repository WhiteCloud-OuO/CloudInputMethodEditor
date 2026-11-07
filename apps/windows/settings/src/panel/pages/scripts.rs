//! 「脚本」页：安装目录 `Scripts\` 下的用户脚本（每个脚本一个文件夹；启用 / 禁用 / 编辑 /
//! 删除 / 新建）。
//!
//! 脚本由 Server 加载执行（清单与规则见 `docs/design/script.md`）；这一页只管文件与配置里的
//! `[script] disabled`。**改动一律重启输入法服务后生效** —— Server 只在启动时读一次脚本与名单，
//! 所以「打开脚本目录」「重启输入法服务」与「在线获取脚本」放在同一行（重启走 `crate::server::restart`）。
//!
//! Lua 运行时不进设置程序，所以第二列的介绍是从入口文件里**扫**出来的（[`describe`]）：扫不到
//! 清单里的 `description` 就退回文件夹名。这是这一页唯一需要「猜」的地方。

use std::path::{Path, PathBuf};

use windows_reactor::*;

use crate::panel::controls::{
    feedback, grid_row, header_row, labeled, note, open_with_explorer, page, table, text_cell,
};
use crate::panel::notepad;
use crate::panel::{Message, Settings};

/// 脚本目录名：安装目录下的这个子目录（`cloudime-script` 的 `DIRECTORY`，与 Server 读的、
/// 安装包装的是同一处）。
const DIRECTORY: &str = "Scripts";

/// 模板文件名：根部的这个文件不显示（`cloudime-script` 的 `TEMPLATE_FILE`，加载器也不执行它）。
const TEMPLATE_FILE: &str = "template.lua";

/// 注意事项：按定稿措辞照抄，加粗标红显示。
const DISCLAIMER: &str = "声明：云朵输入法提供lua脚本的执行，但不对lua脚本安全性负责。一切由lua运行导致的负面后果由用户个人自行承担。";

/// 表格列宽（与表头一致）：脚本文件夹 / 介绍 / 操作。
/// 操作列钉死：表头那行没有控件，用 `Auto` 会塌成 0，表头与数据行就错位了。
const COLUMNS: [GridLength; 3] = [
    GridLength::Pixel(200.0),
    GridLength::STAR,
    GridLength::Pixel(300.0),
];

/// 在线脚本表格列宽（名字 / 介绍 / 操作）。
const REMOTE_COLUMNS: [GridLength; 3] = [
    GridLength::Pixel(200.0),
    GridLength::STAR,
    GridLength::Pixel(100.0),
];

/// 列表里的一项。
struct Script {
    /// 文件夹名（`Scripts\<id>\`）；启用 / 禁用名单里用的也是它。
    id: String,

    /// 第一列：清单里的 `name`，没有就用文件夹名。
    name: String,

    /// 第二列：清单里的 `description`，没有就用名称。
    description: String,

    /// 现在启用（不在 `[script] disabled` 里）。
    enabled: bool,
}

/// 脚本页的在线脚本列表状态。
pub(crate) enum RemoteState {
    /// 还没有点「在线获取脚本」。
    Idle,
    /// 正在下载索引。
    Loading,
    /// 索引已加载。
    Loaded(cloudime_script_store::Catalog),
    /// 下载或解析失败。
    Failed(String),
}

/// 脚本目录：安装目录下的 `Scripts\`；拿不到随包根退回数据目录（只可能出现在开发或异常环境）。
pub(crate) fn directory(settings: &Settings) -> PathBuf {
    cloudime_platform::resources::bundled_root()
        .unwrap_or_else(|| settings.data_dir().to_path_buf())
        .join(DIRECTORY)
}

/// 两个名字是不是同一个脚本：新写法是文件夹名，旧配置里可能还带着 `.lua`
///（升级前 `[script] disabled` 存的是文件名）。
pub(crate) fn same_script(entry: &str, id: &str) -> bool {
    let entry = entry.trim();
    if entry.eq_ignore_ascii_case(id) {
        return true;
    }
    entry.rsplit_once('.').is_some_and(|(stem, extension)| {
        extension.eq_ignore_ascii_case("lua") && stem.eq_ignore_ascii_case(id)
    })
}

/// 列脚本：只认 `Scripts\` 下的一级文件夹，按文件夹名排序；第二项是提示（旧版平铺文件 / 读目录失败）。
fn list(settings: &Settings) -> (Vec<Script>, String) {
    let dir = directory(settings);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        // 一个脚本都没建过时目录还不存在，不算错
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Vec::new(), String::new());
        }
        Err(error) => return (Vec::new(), format!("脚本目录读不出来：{error}")),
    };
    let disabled = &settings.config.script.disabled;
    let mut scripts: Vec<Script> = Vec::new();
    let mut legacy: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lua"))
                && !name.eq_ignore_ascii_case(TEMPLATE_FILE)
            {
                legacy.push(name.to_owned());
            }
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let (entry, problem) = match entry_file(&path) {
            Ok(entry) => (Some(entry), String::new()),
            Err(EntryProblem::Missing) => continue,
            Err(problem) => (None, format!("（{}）", problem.message())),
        };
        let source = entry
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let name = source
            .as_deref()
            .and_then(|text| field_in(text, "name"))
            .unwrap_or_else(|| id.to_owned());
        let description = source
            .as_deref()
            .and_then(|text| field_in(text, "description"))
            .unwrap_or_else(|| name.clone());
        scripts.push(Script {
            id: id.to_owned(),
            name: if problem.is_empty() {
                name
            } else {
                format!("{problem} {name}")
            },
            description,
            enabled: !disabled.iter().any(|name| same_script(name, id)),
        });
    }
    scripts.sort_by(|left, right| left.id.cmp(&right.id));
    let warning = if legacy.is_empty() {
        String::new()
    } else {
        format!(
            "Scripts 根部还有旧版平铺脚本（{}）不会加载；请为每个脚本建一个文件夹，\
             把入口 .lua 放进去（共享 Lua / JSON / DLL / 数据分别放 shared、layouts、dlls、saved）。",
            legacy.join("、")
        )
    };
    (scripts, warning)
}

/// 一个脚本文件夹里的入口问题。
#[derive(Debug)]
enum EntryProblem {
    /// 根部没有 `.lua`：这不是一个脚本文件夹，列表里直接不显示。
    Missing,
    /// 根部有多个 `.lua`：说不清哪个是入口。
    Many(usize),
    /// 文件夹读不出来。
    Unreadable,
}

impl EntryProblem {
    fn message(&self) -> String {
        match self {
            Self::Missing => "文件夹里没有 .lua".to_owned(),
            Self::Many(count) => format!("文件夹根部有 {count} 个 .lua，只能有一个入口"),
            Self::Unreadable => "文件夹读不出来".to_owned(),
        }
    }
}

/// 一个脚本文件夹里的入口：根部必须**正好有一个** `.lua`。
fn entry_file(folder: &Path) -> Result<PathBuf, EntryProblem> {
    let entries = std::fs::read_dir(folder).map_err(|_| EntryProblem::Unreadable)?;
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("lua"))
        })
        .collect();
    files.sort();
    match files.len() {
        1 => files.pop().ok_or(EntryProblem::Missing),
        0 => Err(EntryProblem::Missing),
        count => Err(EntryProblem::Many(count)),
    }
}

/// 从脚本正文里取清单字段（`name = "……"` / `description = '……'`）：
/// **不执行脚本**（Lua 运行时不进设置程序），注释行（`--`）跳过。
fn field_in(text: &str, field: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("--") {
            continue;
        }
        let Some(rest) = line.strip_prefix(field) else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim();
        let Some(quote @ ('"' | '\'')) = value.chars().next() else {
            continue;
        };
        if let Some(end) = value[1..].find(quote) {
            let value = value[1..1 + end].trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// 「打开脚本目录」：在资源管理器里打开 `Scripts\`（不存在就先建出来）。
pub(crate) fn open_directory(settings: &mut Settings) {
    let dir = directory(settings);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        settings.script_status = format!("建脚本目录失败：{error}");
        return;
    }
    settings.script_status.clear();
    open_with_explorer(&dir.to_string_lossy());
}

/// 「编辑此脚本」：用记事本打开入口 `.lua`。
pub(crate) fn edit(settings: &mut Settings, id: &str) {
    let folder = directory(settings).join(id);
    let path = match entry_file(&folder) {
        Ok(path) => path,
        Err(problem) => {
            settings.script_status = format!("{id}：{}", problem.message());
            return;
        }
    };
    settings.script_status.clear();
    notepad::open(&path);
}

/// 「删除此脚本」：确认后删整个脚本文件夹，并把它从禁用名单里去掉（别留一条永远不起作用的记录）。
pub(crate) fn remove(settings: &mut Settings, id: &str) {
    let confirmed = matches!(
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("删除脚本")
            .set_description(format!(
                "确定删除脚本文件夹 {id} 吗？里面的脚本、布局与 saved 数据都会删掉，找不回来。"
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show(),
        rfd::MessageDialogResult::Yes
    );
    if !confirmed {
        return;
    }
    let path = directory(settings).join(id);
    if let Err(error) = std::fs::remove_dir_all(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        settings.script_status = format!("删除 {id} 失败：{error}");
        return;
    }
    settings.script_status = format!("已删除 {id}，重启输入法服务后生效。");
    let mut disabled = settings.config.script.disabled.clone();
    let before = disabled.len();
    disabled.retain(|name| !same_script(name, id));
    if disabled.len() != before {
        settings.save_array("script", "disabled", &disabled);
    }
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let (scripts, error) = list(settings);
    let mut rows: Vec<KeyedView> = vec![KeyedView::new(
        "header",
        header_row(
            COLUMNS,
            [
                text_cell("header-id", "脚本", 0, true, false),
                text_cell("header-description", "介绍", 1, true, false),
                text_cell("header-actions", "启用 / 操作", 2, true, false),
            ],
        ),
    )];
    if scripts.is_empty() {
        rows.push(KeyedView::new(
            "empty",
            note("还没有脚本。点「打开脚本目录」建一个脚本文件夹。"),
        ));
    }
    if !error.is_empty() {
        rows.push(KeyedView::new(
            "list-error",
            TextBlock::new()
                .text(error)
                .text_wrapping(TextWrapping::Wrap)
                .font_size(12.0)
                .foreground(ThemeBrush::SystemCritical),
        ));
    }
    for (index, script) in scripts.iter().enumerate() {
        let id = script.id.clone();
        let toggle = {
            let id = id.clone();
            context.callback(move |on: bool| Message::ScriptToggle(id.clone(), on))
        };
        let actions = StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(4.0)
            .vertical_alignment(VerticalAlignment::Center)
            .grid_column(2)
            .children((
                // 开关只留本体：自带的「开 / 关」文字白占一截（Windows 11 设置里也是光板开关），
                // 再去掉 WinUI 默认的最小宽度 —— 否则这一行放不下三个控件，最后那个按钮会被右边缘切掉。
                ToggleSwitch::new()
                    .is_on(script.enabled)
                    .min_width(0.0)
                    .on_toggled(toggle)
                    .slots([
                        SlotView::new(ToggleSwitchSlot::OnContent, ""),
                        SlotView::new(ToggleSwitchSlot::OffContent, ""),
                    ]),
                Button::new()
                    .min_width(0.0)
                    .on_click(context.message(Message::ScriptRemove(id.clone())))
                    .content("删除此脚本"),
                Button::new()
                    .min_width(0.0)
                    .on_click(context.message(Message::ScriptEdit(id.clone())))
                    .content("编辑此脚本"),
            ));
        rows.push(KeyedView::new(
            format!("script-{index}"),
            grid_row(
                COLUMNS,
                [
                    text_cell("name", script.name.clone(), 0, true, false),
                    text_cell("description", script.description.clone(), 1, false, true),
                    KeyedView::new("actions", actions),
                ],
            ),
        ));
    }

    let body = StackPanel::new().spacing(12.0).children([
        TextBlock::new()
            .text(DISCLAIMER)
            .text_wrapping(TextWrapping::Wrap)
            .font_weight(FontWeight::BOLD)
            .foreground(ThemeBrush::SystemCritical)
            .into(),
        note(&format!(
            "脚本放 {}，一个脚本一个文件夹（文件夹名就是脚本名）：入口 .lua 直接放文件夹根部、\
             只能有一个，文件最前面要声明清单（照 template.lua 改）；共享 Lua 放这个文件夹的 shared\\，\
             JSON 布局放 layouts\\，动态链接库放 dlls\\，脚本数据用 cloudime.saved_path 放到 saved\\。\
             用「打开脚本目录」建文件夹；删除 / 开关都写在这里或配置的 [script] disabled，\
             重启输入法服务后生效。\
             写法（清单字段、事件、能返回的动作、cloudime 表的全部方法与例子）见安装目录下的 lua.md。",
            directory(settings).display()
        )),
        table(rows),
        remote_section(settings, context),
        StackPanel::new().spacing(4.0).children([
            labeled(
                "为脚本提供全局热键",
                ToggleSwitch::new()
                    .is_on(settings.config.script.global_hotkeys)
                    .min_width(0.0)
                    .on_toggled(context.callback(Message::ScriptGlobalHotkeys))
                    .slots([
                        SlotView::new(ToggleSwitchSlot::OnContent, ""),
                        SlotView::new(ToggleSwitchSlot::OffContent, ""),
                    ]),
            ),
            note(
                "通过 TSF 把脚本声明的组合快捷键注册为全局热键，只在中文输入状态抢占；\
                 英文或禁用输入时放行按键，不吞键。修改后重启输入法服务生效。",
            ),
        ]),
        note(&settings.script_status),
        feedback(&settings.notice),
    ]);
    page("脚本", body)
}

/// 在线脚本区域：列表、错误与「获取」按钮。
fn remote_section(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let mut children: Vec<KeyedView> = Vec::new();
    children.push(KeyedView::new(
        "remote-note",
        note(
            "在线脚本：列表来自官方脚本仓库，每个脚本装进 Scripts\\<脚本文件夹>\\（入口 .lua 与 shared、layouts、dlls 子目录）。\
             已存在且内容不同的文件不会被覆盖；获取后点「重启输入法服务」生效。",
        ),
    ));
    let refresh = Button::new()
        .on_click(context.message(Message::ScriptRefreshOnline))
        .content(if matches!(settings.remote_scripts, RemoteState::Loading) {
            "正在获取…"
        } else {
            "在线获取脚本"
        });
    children.push(KeyedView::new(
        "remote-refresh",
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .children((
                refresh,
                Button::new()
                    .on_click(context.message(Message::ScriptOpenDirectory))
                    .content("打开脚本目录"),
                Button::new()
                    .on_click(context.message(Message::RestartServer))
                    .content("重启输入法服务"),
            )),
    ));
    match &settings.remote_scripts {
        RemoteState::Idle => {
            children.push(KeyedView::new(
                "remote-idle",
                note("点「在线获取脚本」从官方仓库拉取列表。"),
            ));
        }
        RemoteState::Loading => {
            children.push(KeyedView::new(
                "remote-loading",
                note("正在获取在线脚本列表…"),
            ));
        }
        RemoteState::Failed(error) => {
            children.push(KeyedView::new(
                "remote-error",
                TextBlock::new()
                    .text(error.clone())
                    .text_wrapping(TextWrapping::Wrap)
                    .font_size(12.0)
                    .foreground(ThemeBrush::SystemCritical),
            ));
        }
        RemoteState::Loaded(catalog) => {
            if catalog.scripts.is_empty() {
                children.push(KeyedView::new(
                    "remote-empty",
                    note("线上仓库暂时没有脚本。"),
                ));
            } else {
                let mut rows: Vec<KeyedView> = vec![KeyedView::new(
                    "remote-header",
                    header_row(
                        REMOTE_COLUMNS,
                        [
                            text_cell("remote-header-name", "脚本", 0, true, false),
                            text_cell("remote-header-description", "介绍", 1, true, false),
                            text_cell("remote-header-action", "操作", 2, true, false),
                        ],
                    ),
                )];
                for (index, script) in catalog.scripts.iter().enumerate() {
                    rows.push(KeyedView::new(
                        format!("remote-{index}"),
                        grid_row(
                            REMOTE_COLUMNS,
                            [
                                text_cell("name", script.name.clone(), 0, true, false),
                                text_cell(
                                    "description",
                                    script.description.clone(),
                                    1,
                                    false,
                                    true,
                                ),
                                KeyedView::new(
                                    "install",
                                    StackPanel::new()
                                        .orientation(Orientation::Horizontal)
                                        .grid_column(2)
                                        .children((Button::new()
                                            .on_click(
                                                context
                                                    .message(Message::ScriptInstallOnline(index)),
                                            )
                                            .content("获取"),)),
                                ),
                            ],
                        ),
                    ));
                }
                children.push(KeyedView::new("remote-table", table(rows)));
            }
        }
    }
    StackPanel::new().spacing(8.0).keyed_children(children)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 清单里的 `name` 也按同样规则扫出来；值本身不带引号、注释。
    #[test]
    fn the_manifest_name_is_read() {
        let text = "cloudime.script{\n    name = '小牛整句翻译（独立窗口）',\n    description = '说明',\n}\n";
        assert_eq!(
            field_in(text, "name").as_deref(),
            Some("小牛整句翻译（独立窗口）")
        );
        assert_eq!(field_in(text, "description").as_deref(), Some("说明"));
    }

    /// 单双引号都认；没有这个键、或者值不是字符串时返回 `None`。
    #[test]
    fn manifest_fields_take_both_quotes() {
        let text = "-- description = '注释里的不算'\ncloudime.script{\n    description = '单引号也行',\n}\n";
        assert_eq!(field_in(text, "description").as_deref(), Some("单引号也行"));
        assert_eq!(
            field_in("cloudime.script{\n    name = 'x',\n}\n", "name").as_deref(),
            Some("x")
        );
        assert_eq!(
            field_in("cloudime.script{\n    name = 'x',\n}\n", "description"),
            None
        );
        assert_eq!(field_in("description = 123\n", "description"), None);
    }

    /// 一个脚本文件夹根部必须正好有一个 `.lua`；0 个 / 多个都不选入口。
    #[test]
    fn entry_file_requires_exactly_one_lua() {
        let dir = std::env::temp_dir().join("cloudime-script-entry");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("layouts")).unwrap();
        assert!(matches!(entry_file(&dir), Err(EntryProblem::Missing)));

        std::fs::write(dir.join("main.lua"), "").unwrap();
        assert_eq!(entry_file(&dir).unwrap().file_name().unwrap(), "main.lua");

        std::fs::write(dir.join("other.lua"), "").unwrap();
        assert!(matches!(entry_file(&dir), Err(EntryProblem::Many(2))));
        assert!(dir.join("layouts").is_dir(), "子目录里的 .lua 不算入口");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
