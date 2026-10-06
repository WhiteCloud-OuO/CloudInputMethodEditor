//! 各页共用的表单零件（标签行、说明小字、整项、页外壳）与打开文件 / 目录、打包日志的小工具。

use std::path::PathBuf;

use windows_reactor::*;

use super::notice::Notice;
use super::{LABEL_WIDTH, Message, Settings};
use crate::log;

/// 资源管理器打开目录或网址。
pub(super) fn open_with_explorer(target: &str) {
    if let Err(error) = std::process::Command::new("explorer").arg(target).spawn() {
        log::warn(format!("打开 {target} 失败: {error}"));
    }
}

/// 三个进程共用的日志目录，没有就建出来（Server 没跑过时它还不存在）。
pub(super) fn log_dir() -> Option<PathBuf> {
    let dir = cloudime_platform::dirs::log_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// 把整个日志目录加 `config.toml` 打成 `cloudime-logs-<日期>.zip` 放到桌面，再在资源管理器里选中它——
/// 用户反馈问题时一个附件搞定。压缩交给 PowerShell 的 Compress-Archive，不为此拉一个压缩库；
/// 桌面路径也让 PowerShell 取（OneDrive 会把桌面挪到别处）。脚本先写成临时 .ps1 再跑，免得命令行引号转义。
pub(super) fn export_logs() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Some(logs) = log_dir() else {
        return;
    };
    log::warn("用户导出日志");
    let mut sources = vec![format!("'{}\\*'", logs.display())];
    if let Some(config) = cloudime_platform::dirs::config_path().filter(|path| path.is_file()) {
        sources.push(format!("'{}'", config.display()));
    }
    let zip_name = format!(
        "cloudime-logs-{}.zip",
        jiff::Zoned::now().strftime("%Y-%m-%d")
    );
    let script = format!(
        "$zip = Join-Path ([Environment]::GetFolderPath('Desktop')) '{zip_name}'\n\
         Compress-Archive -Path {} -DestinationPath $zip -Force\n\
         explorer.exe \"/select,`\"$zip`\"\"\n",
        sources.join(",")
    );
    let script_path = std::env::temp_dir().join("cloudime-export-logs.ps1");
    if let Err(error) = std::fs::write(&script_path, script) {
        log::warn(format!("写导出脚本失败: {error}"));
        return;
    }
    let spawned = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script_path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if let Err(error) = spawned {
        log::warn(format!("导出日志失败: {error}"));
    }
}

/// 一行设置：标签 + 控件，标签列至少 [`LABEL_WIDTH`] 宽；**标签与控件垂直居中**。
///
/// 标签不设 `VerticalAlignment` 时会被拉伸到整行高（横排 `StackPanel` 的交叉轴默认 Stretch），
/// 而 `TextBlock` 的文字画在自己顶部——40 高的开关行里标签就比开关高约 10px，看着没对齐。
/// 显式居中后 `TextBlock` 只占文字那点高、在行内居中。一行很高的控件改用 [`labeled_top`]。
///
/// 下限用 `min_width` 而不是 `width`：`width` 会把标签钉死在 140 DIP，而 `TextBlock` 缺省
/// `NoWrap` + `TextTrimming::None` 不裁剪，超长标签就按自然宽度画出槽外，被右边后画的控件
/// 压住（「英文模式（Caps Lock）也给候选」就只剩前半截）。改成下限后标签按自然宽度排开、
/// 短标签仍撑到 140，控件紧跟着标签走，文字既不换行也不被遮挡。
pub(super) fn labeled(label: &str, control: impl Into<View>) -> View {
    labeled_at(label, VerticalAlignment::Center, control)
}

/// 同 [`labeled`]，但标签**顶对齐**：给一行很高的控件（列表、一组单选按钮）用，
/// 免得标签垂直居中后飘在这一大块的中间，跟第一行内容对不上。
pub(super) fn labeled_top(label: &str, control: impl Into<View>) -> View {
    labeled_at(label, VerticalAlignment::Top, control)
}

fn labeled_at(label: &str, align: VerticalAlignment, control: impl Into<View>) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(12.0)
        .children([
            TextBlock::new()
                .text(label)
                .min_width(LABEL_WIDTH)
                .vertical_alignment(align)
                .into(),
            control.into(),
        ])
}

/// 灰色小字说明，可换行。
pub(super) fn note(text: &str) -> View {
    TextBlock::new()
        .text(text)
        .text_wrapping(TextWrapping::Wrap)
        .font_size(12.0)
        .opacity(0.6)
        .into()
}

/// 一整项：「标签 + 控件」一行，下接说明（`hint` 为空则不加）；标签与控件垂直居中。
pub(super) fn field(label: &str, hint: &str, control: impl Into<View>) -> View {
    with_hint(labeled(label, control), hint)
}

/// 同 [`field`]，但标签顶对齐（控件是一行很高的列表 / 单选组时用）。
pub(super) fn field_top(label: &str, hint: &str, control: impl Into<View>) -> View {
    with_hint(labeled_top(label, control), hint)
}

fn with_hint(row: View, hint: &str) -> View {
    if hint.is_empty() {
        row
    } else {
        StackPanel::new().spacing(4.0).children([row, note(hint)])
    }
}

/// 一行「名称 · N 条 · 许可证」，坏文件标出来：词库页用。
pub(super) fn entry_title(name: &str, entries: usize, license: &str, broken: bool) -> String {
    if broken {
        return format!("{name}（文件损坏）");
    }
    let mut text = format!("{name} · {entries} 条");
    if !license.is_empty() {
        text.push_str(&format!(" · {license}"));
    }
    text
}

/// 一行「词库名 + 移除按钮」：词库页用，内置词库不列出、都带移除。
pub(super) fn bank_row(
    stem: &str,
    label: String,
    remove: Message,
    context: &mut ViewContext<Settings>,
) -> KeyedView {
    let title = TextBlock::new()
        .text(label)
        .text_wrapping(TextWrapping::Wrap);
    let row = StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(12.0)
        .children((
            title,
            Button::new()
                .on_click(context.message(remove))
                .content("移除"),
        ));
    KeyedView::new(stem.to_owned(), row)
}

/// 页面底部的提示：成功统计一行灰字、失败一行红字，都没有就不画。
pub(super) fn feedback(notice: &Notice) -> View {
    let mut lines: Vec<KeyedView> = Vec::new();
    if let Some(text) = &notice.note {
        lines.push(KeyedView::new("notice-note", note(text)));
    }
    if let Some(text) = &notice.error {
        lines.push(KeyedView::new(
            "notice-error",
            TextBlock::new()
                .text(text)
                .text_wrapping(TextWrapping::Wrap)
                .font_size(12.0)
                .foreground(ThemeBrush::SystemCritical),
        ));
    }
    StackPanel::new().spacing(4.0).keyed_children(lines)
}

/// 一页外壳：可滚动 + 大标题 + 内容。
pub(super) fn page(title: &str, body: impl Into<View>) -> View {
    ScrollViewer::new().content(
        StackPanel::new().spacing(16.0).margin(24.0).children([
            TextBlock::new()
                .text(title)
                .font_size(24.0)
                .font_weight(FontWeight::SEMI_BOLD)
                .into(),
            body.into(),
        ]),
    )
}

/// 列表行高（WinUI `ListViewItem` 的缺省 `MinHeight`）与最多同时显示的行数。
///
/// 两个数相乘就是列表的高度上限：超过的项由列表自己出滚动条，页面本身的滚动不受影响。
const LIST_ROW_HEIGHT: f64 = 40.0;
const LIST_VISIBLE_ROWS: f64 = 5.0;

/// 一组横排的单选按钮：和左边的标签同一行、垂直居中。
///
/// 不用框架的 `RadioButtons` 容器：它的期望高度比实际渲染矮（一行渲染 32、期望只有 25），渲染出来的
/// 选项比自己盒子低 3.5px，左边的标签按盒子居中后看着总差一点，从外面也调不动。独立的 `RadioButton`
/// 和开关 / 下拉一样是单个控件，垂直居中对得上。
///
/// `options` 给「标签 + 是否选中」；`message` 收到动作的下标（未选中那侧回传 `None`，页里当空操作）。
pub(super) fn radio_row(
    group: &str,
    options: impl IntoIterator<Item = (&'static str, bool)>,
    message: impl Fn(Option<usize>) -> Message + Clone + 'static,
    context: &mut ViewContext<Settings>,
) -> View {
    let items = options
        .into_iter()
        .enumerate()
        .map(|(index, (label, checked))| {
            let message = message.clone();
            KeyedView::new(
                format!("radio-{index}"),
                RadioButton::new()
                    .group_name(group)
                    .is_checked(checked)
                    .on_checked(context.callback(move |on: bool| message(on.then_some(index))))
                    .content(label),
            )
        });
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(16.0)
        .keyed_children(items)
}

/// 勾选 / 名单列表的通用外壳：`ListView` 装给定的项，不要选中行为，最多同时显示 5 行。
///
/// windows-reactor 没有 XAML 那种 `DataTemplate` / `ItemsSource`，所谓「项目模板」就是在 Rust 里
/// 给每一项建好一份视图（`ListViewItem` 之类）再用 `collection_slot` 交给列表。
pub(super) fn scroll_list(items: impl IntoIterator<Item = KeyedView>) -> View {
    ListView::new()
        // 列表只用来摆项，不要选中高亮与单/多选行为。
        .selection_mode(ListViewSelectionMode::None)
        .max_height(LIST_ROW_HEIGHT * LIST_VISIBLE_ROWS)
        .collection_slot(ListViewSlot::Items, items)
}
