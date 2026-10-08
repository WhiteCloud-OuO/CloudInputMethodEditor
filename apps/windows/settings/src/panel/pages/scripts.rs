//! 「脚本」页：安装目录 `Scripts\` 下的用户 Lua 脚本（启用 / 禁用 / 编辑 / 删除 / 新建）。
//!
//! 脚本由 Server 加载执行（清单与规则见 `docs/design/script.md`）；这一页只管文件与配置里的
//! `[script] disabled`。**改动一律重启输入法服务后生效** —— Server 只在启动时读一次脚本与名单，
//! 所以「新建脚本」旁边放了一个「重启输入法服务」按钮（走 `crate::server::restart`）。
//!
//! Lua 运行时不进设置程序，所以第二列的介绍是从文件里**扫**出来的（[`describe`]）：扫不到
//! 清单里的 `description` 就退回文件名。这是这一页唯一需要「猜」的地方。

use std::path::{Path, PathBuf};

use windows_reactor::*;

use crate::panel::controls::{feedback, grid_row, note, page, text_cell};
use crate::panel::notepad;
use crate::panel::{Message, Settings};

/// 脚本目录名：安装目录下的这个子目录（`cloudime-script` 的 `DIRECTORY`，与 Server 读的、
/// 安装包装的是同一处）。
const DIRECTORY: &str = "Scripts";

/// 「新建脚本」用的模板：就是仓库 / 安装目录里那份 `Scripts\template.lua`，编进 exe ——
/// 模板文件被删掉也照样建得出来。
const TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../Scripts/template.lua"
));

/// 模板文件名：列表里不显示它（`cloudime-script` 的 `TEMPLATE_FILE`，加载器也不执行它）。
const TEMPLATE_FILE: &str = "template.lua";

/// 注意事项：按定稿措辞照抄，加粗标红显示。
const DISCLAIMER: &str = "声明：云朵输入法提供lua脚本的执行，但不对lua脚本安全性负责。一切由lua运行导致的负面后果由用户个人自行承担。";

/// 表格列宽（与表头一致）：文件名 / 介绍 / 操作。
/// 操作列钉死：表头那行没有控件，用 `Auto` 会塌成 0，表头与数据行就错位了。
const COLUMNS: [GridLength; 3] = [
    GridLength::Pixel(200.0),
    GridLength::STAR,
    GridLength::Pixel(300.0),
];

/// 列表里的一项。
struct Script {
    /// 文件名（含 `.lua`）；启用 / 禁用名单里用的也是它。
    file: String,

    /// 第二列：清单里的 `description`，没有就用文件名。
    description: String,

    /// 现在启用（不在 `[script] disabled` 里）。
    enabled: bool,
}

/// 脚本目录：安装目录下的 `Scripts\`；拿不到随包根退回数据目录（只可能出现在开发或异常环境）。
fn directory(settings: &Settings) -> PathBuf {
    cloudime_platform::resources::bundled_root()
        .unwrap_or_else(|| settings.data_dir().to_path_buf())
        .join(DIRECTORY)
}

/// 列脚本：按文件名排序，跳过模板；第二项是出错原因（空串 = 没问题）。
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
    let mut scripts: Vec<Script> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let file = path.file_name()?.to_str()?.to_owned();
            let is_lua = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lua"));
            (path.is_file() && is_lua && !file.eq_ignore_ascii_case(TEMPLATE_FILE)).then(|| {
                Script {
                    description: describe(&path).unwrap_or_else(|| file.clone()),
                    enabled: !disabled
                        .iter()
                        .any(|name| name.trim().eq_ignore_ascii_case(&file)),
                    file,
                }
            })
        })
        .collect();
    scripts.sort_by(|left, right| left.file.cmp(&right.file));
    (scripts, String::new())
}

/// 扫一个脚本文件的 `description`：**不执行脚本**（Lua 运行时不进设置程序）。
fn describe(path: &Path) -> Option<String> {
    describe_in(&std::fs::read_to_string(path).ok()?)
}

/// [`describe`] 的正文部分：只认清单里那种 `description = "……"` / `description = '……'`
/// （单双引号都行），注释行（`--`）跳过；读不到返回 `None`。
fn describe_in(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("--") {
            continue;
        }
        let Some(rest) = line.strip_prefix("description") else {
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
            let description = value[1..1 + end].trim();
            if !description.is_empty() {
                return Some(description.to_owned());
            }
        }
    }
    None
}

/// 「新建脚本」：在脚本目录里建一个不重名的文件、写上模板，再用记事本打开。
pub(crate) fn create(settings: &mut Settings) {
    let dir = directory(settings);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        settings.script_status = format!("建脚本目录失败：{error}");
        return;
    }
    let file = unique_name(&dir);
    let path = dir.join(&file);
    if let Err(error) = std::fs::write(&path, TEMPLATE) {
        settings.script_status = format!("写 {file} 失败：{error}");
        return;
    }
    settings.script_status = format!("已新建 {file}：改完按 Ctrl + S 保存，再重启输入法服务。");
    notepad::open_with_text(&path, TEMPLATE);
}

/// 取一个不重名的文件名：`script.lua`、`script-2.lua`、`script-3.lua`……
fn unique_name(dir: &Path) -> String {
    let mut index = 1;
    loop {
        let file = if index == 1 {
            "script.lua".to_owned()
        } else {
            format!("script-{index}.lua")
        };
        if !dir.join(&file).exists() {
            return file;
        }
        index += 1;
    }
}

/// 「编辑此脚本」：用记事本打开。
pub(crate) fn edit(settings: &mut Settings, file: &str) {
    let path = directory(settings).join(file);
    if !path.is_file() {
        settings.script_status = format!("{file} 不在了（可能刚被删过）。");
        return;
    }
    settings.script_status.clear();
    notepad::open(&path);
}

/// 「删除此脚本」：确认后删文件，并把它从禁用名单里去掉（别留一条永远不起作用的记录）。
pub(crate) fn remove(settings: &mut Settings, file: &str) {
    let confirmed = matches!(
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("删除脚本")
            .set_description(format!("确定删除 {file} 吗？删掉就找不回来了。"))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show(),
        rfd::MessageDialogResult::Yes
    );
    if !confirmed {
        return;
    }
    let path = directory(settings).join(file);
    if let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        settings.script_status = format!("删除 {file} 失败：{error}");
        return;
    }
    settings.script_status = format!("已删除 {file}，重启输入法服务后生效。");
    let mut disabled = settings.config.script.disabled.clone();
    let before = disabled.len();
    disabled.retain(|name| !name.eq_ignore_ascii_case(file));
    if disabled.len() != before {
        settings.save_array("script", "disabled", &disabled);
    }
}

pub(crate) fn view(settings: &Settings, context: &mut ViewContext<Settings>) -> View {
    let (scripts, error) = list(settings);
    let mut rows: Vec<KeyedView> = vec![KeyedView::new(
        "header",
        grid_row(
            COLUMNS,
            [
                text_cell("header-file", "文件名", 0, true, false),
                text_cell("header-description", "介绍", 1, true, false),
                text_cell("header-actions", "启用 / 操作", 2, true, false),
            ],
        ),
    )];
    if scripts.is_empty() {
        rows.push(KeyedView::new(
            "empty",
            note("还没有脚本。点下面的「新建脚本」建一个。"),
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
        let file = script.file.clone();
        let toggle = {
            let file = file.clone();
            context.callback(move |on: bool| Message::ScriptToggle(file.clone(), on))
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
                    .on_click(context.message(Message::ScriptRemove(file.clone())))
                    .content("删除此脚本"),
                Button::new()
                    .min_width(0.0)
                    .on_click(context.message(Message::ScriptEdit(file.clone())))
                    .content("编辑此脚本"),
            ));
        rows.push(KeyedView::new(
            format!("script-{index}"),
            grid_row(
                COLUMNS,
                [
                    text_cell("file", file, 0, true, false),
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
            "脚本放 {}，一个文件一个脚本，文件最前面要声明清单（照 template.lua 改）。\
             新建 / 删除 / 开关都写在这里或配置的 [script] disabled，重启输入法服务后生效\
             （右边那个按钮就是干这个的）。\
             写法（清单字段、事件、能返回的动作、cloudime 表的全部方法与例子）见安装目录下的 lua.md。",
            directory(settings).display()
        )),
        StackPanel::new().spacing(4.0).keyed_children(rows),
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(12.0)
            .children((
                Button::new()
                    .on_click(context.message(Message::ScriptNew))
                    .content("新建脚本"),
                Button::new()
                    .on_click(context.message(Message::RestartServer))
                    .content("重启输入法服务"),
            )),
        note(&settings.script_status),
        feedback(&settings.notice),
    ]);
    page("脚本", body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 编进 exe 的那份模板里扫得出 `description`，而且值本身没带上引号、注释。
    #[test]
    fn the_template_description_is_read() {
        let description = describe_in(TEMPLATE).expect("模板的清单里要有 description");
        assert!(!description.is_empty());
        assert!(!description.contains(['"', '\'']), "{description}");
        assert!(!description.contains("--"), "{description}");
    }

    /// 单双引号都认；没有这个键、或者值不是字符串时返回 `None`（退回文件名）。
    #[test]
    fn description_takes_both_quotes() {
        let text = "-- description = '注释里的不算'\ncloudime.script{\n    description = '单引号也行',\n}\n";
        assert_eq!(describe_in(text).as_deref(), Some("单引号也行"));
        assert_eq!(describe_in("cloudime.script{\n    name = 'x',\n}\n"), None);
        assert_eq!(describe_in("description = 123\n"), None);
    }

    /// 重名往后顺延：`script.lua` 占了就 `script-2.lua`。
    #[test]
    fn new_script_names_do_not_collide() {
        let dir = std::env::temp_dir().join("cloudime-new-script-names");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_name(&dir), "script.lua");
        std::fs::write(dir.join("script.lua"), "").unwrap();
        assert_eq!(unique_name(&dir), "script-2.lua");
        std::fs::write(dir.join("script-2.lua"), "").unwrap();
        assert_eq!(unique_name(&dir), "script-3.lua");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
