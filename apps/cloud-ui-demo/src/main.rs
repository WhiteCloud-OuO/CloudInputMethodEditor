//! `cloud-ui` 的最小独立窗口示例：普通 Win32 窗口、消息循环、Document 渲染。
//!
//! release 编成 GUI 子系统，避免独立应用弹控制台；debug 保留控制台看 `tracing` 警告。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use cloud_ui::{
    AppEvent, Application, CLOSE_ACTION, Context, Decorations, ListBoxState, ListItem,
    MAXIMIZE_ACTION, MINIMIZE_ACTION, MessageDialog, StackStyle, TextInputDialog, Theme, TrayEvent,
    TrayMenuItem, TrayOptions, View, WindowMask, WindowOptions, run_native,
};
use cloudime_render::Document;

const LIST_ROWS: usize = 5;
const LIST_HEIGHT: f32 = 170.0;
const LOG_ROWS: usize = 12;
const LOG_HEIGHT: f32 = 110.0;

#[derive(Default)]
struct Demo {
    clicks: u32,
    pointer: Option<(f32, f32)>,
    typed: String,
    notes: String,
    log_scroll: f32,
    selected: Option<usize>,
    light: bool,
    masked: bool,
    dialog_result: Option<u8>,
}

impl Demo {
    fn theme(&self) -> Theme {
        if self.light {
            Theme::light()
        } else {
            Theme::dark()
        }
    }
}

impl Application for Demo {
    fn update(&mut self, ctx: &mut Context) {
        for event in ctx.events() {
            match event {
                AppEvent::MouseMoved { x, y } => self.pointer = Some((*x, *y)),
                AppEvent::MouseLeft => self.pointer = None,
                AppEvent::RegionClicked(name) if name == "click" => self.clicks += 1,
                AppEvent::RegionClicked(name) if name == "theme" => self.light = !self.light,
                AppEvent::RegionClicked(name) if name == "mask" => self.masked = !self.masked,
                AppEvent::RegionClicked(name) if name == "dialog" => {
                    let mut dialog = MessageDialog::new(
                        "消息对话框",
                        "这是一个同步消息对话框。\n点下面任意一个按钮返回下标。",
                        "warning",
                        &["确定", "取消"],
                    );
                    // 对话框是类：属性在 `show` 之前改才有效。
                    dialog.style.text_size = Some(15.0);
                    self.dialog_result = Some(dialog.show());
                }
                AppEvent::RegionClicked(name) if name == "text_input" => {
                    let dialog = TextInputDialog::new(
                        "单行文本输入",
                        "请输入一行文本；确定返回内容，取消返回默认值。",
                        false,
                        "默认\n文本",
                    );
                    self.typed = dialog.show();
                }
                AppEvent::RegionClicked(name) if name == "text_box_input" => {
                    let dialog = TextInputDialog::new(
                        "多行文本输入",
                        "可以输入多行；确定返回内容，取消返回默认值。",
                        true,
                        "第一行\n第二行",
                    );
                    self.notes = dialog.show();
                }
                AppEvent::RegionClicked(name) if name == "open" => {
                    let _ = ctx.open_window(
                        WindowOptions {
                            title: "Cloud UI 子窗口".to_owned(),
                            size: (440.0, 300.0),
                            min_size: Some((360.0, 220.0)),
                            decorations: Decorations::None,
                            top_most: false,
                            shadow: true,
                            corner_radius: 12.0,
                            caption_height: Some(36.0),
                            ..WindowOptions::default()
                        },
                        Child::default(),
                    );
                }
                AppEvent::RegionClicked(name) if name == "hide" => ctx.hide(),
                AppEvent::Tray(event) => match event {
                    TrayEvent::LeftClick | TrayEvent::DoubleClick => ctx.show(),
                    TrayEvent::RightClick => {}
                    TrayEvent::Menu(id) => match id.as_str() {
                        "show" => ctx.show(),
                        "hide" => ctx.hide(),
                        "quit" => ctx.close(),
                        _ => {}
                    },
                },
                AppEvent::RegionClicked(name) if name == MINIMIZE_ACTION => ctx.minimize(),
                AppEvent::RegionClicked(name) if name == MAXIMIZE_ACTION => ctx.toggle_maximize(),
                AppEvent::RegionClicked(name) if name == CLOSE_ACTION => ctx.close(),
                AppEvent::RegionClicked(name) => {
                    if let Some(index) = name.strip_prefix("row-")
                        && let Ok(index) = index.parse::<usize>()
                    {
                        self.selected = Some(index);
                    }
                }
                AppEvent::MouseWheel { delta } => {
                    let style = self.theme().list_box(LOG_HEIGHT);
                    let max = style.max_scroll(LOG_ROWS);
                    let stride = style.row_stride();
                    self.log_scroll = (self.log_scroll - delta * stride).clamp(0.0, max);
                }
                AppEvent::TextChanged { name, text } if name == "input" => {
                    self.typed = text.clone();
                }
                AppEvent::TextChanged { name, text } if name == "notes" => {
                    self.notes = text.clone();
                }
                AppEvent::Text(text) => self.typed.push_str(text),
                AppEvent::Key { vk, pressed: true } | AppEvent::TextKey { vk, .. }
                    if *vk == 0x1B =>
                {
                    ctx.close()
                }
                _ => {}
            }
        }
        ctx.set_mask(self.masked.then(|| WindowMask::dim(0.35)));
    }

    fn view(&mut self, ctx: &Context) -> Document {
        let (width, height) = ctx.size();
        let theme = self.theme();
        let items: Vec<ListItem> = (0..LIST_ROWS)
            .map(|index| {
                ListItem::new(format!("第 {} 项", index + 1))
                    .detail(format!("detail {index}"))
                    .action(format!("row-{index}"))
            })
            .collect();
        let list_style = theme.list_box(LIST_HEIGHT);
        let log_items: Vec<ListItem> = (0..LOG_ROWS)
            .map(|index| ListItem::new(format!("日志行 {} · 用于验证滚轮", index + 1)))
            .collect();
        let selected = self.selected;
        let log_scroll = self.log_scroll;
        let pointer = self.pointer;
        let typed = self.typed.clone();
        let notes = self.notes.clone();
        let masked = self.masked;
        let clicks = self.clicks;
        let light = self.light;
        let dpi = ctx.dpi();
        let maximized = ctx.is_maximized();

        let mut view = View::new(width, height);
        if let Some((x, y)) = pointer {
            view.pointer(x, y);
        }
        let radius = if maximized { 0.0 } else { 12.0 };
        view.background_rounded(theme.background, radius);
        view.title_bar("Cloud UI 独立窗口", theme.title_bar(), maximized);
        view.column(StackStyle::padded(10.0, 24.0), |ui| {
            ui.row(StackStyle::gap(16.0), |row| {
                row.label(
                    format!("DPI {dpi} · 客户区 {width:.0} × {height:.0} 点"),
                    theme.muted(),
                );
                row.flex_spacer();
                row.label(
                    self.dialog_result.map_or_else(
                        || "对话框结果：—".to_owned(),
                        |result| format!("对话框结果：{result}"),
                    ),
                    theme.muted(),
                );
                row.label("Esc 关闭窗口", theme.muted());
                row.button("text_input", "单行", theme.button());
                row.button("text_box_input", "多行", theme.button());
                row.button("theme", if light { "深色" } else { "浅色" }, theme.button());
            });
            ui.spacer(0.0, 6.0);
            ui.text_edit("input", typed.clone(), theme.text_edit());
            ui.label(format!("输入：{typed}"), theme.muted());
            ui.spacer(0.0, 6.0);
            ui.text_box("notes", notes.clone(), theme.text_box(80.0));
            ui.label("日志列表（滚轮滚动，行始终完整显示）：", theme.muted());
            ui.list_box(
                "log",
                &log_items,
                theme.list_box(LOG_HEIGHT),
                ListBoxState {
                    selected: None,
                    scroll_offset: log_scroll,
                },
            );
            ui.spacer(0.0, 10.0);
            ui.label(
                format!(
                    "列表盒（选中：{}）",
                    selected
                        .map_or_else(|| "无".to_owned(), |index| format!("第 {} 项", index + 1))
                ),
                theme.label(),
            );
            ui.list_box(
                "list",
                &items,
                list_style,
                ListBoxState {
                    selected,
                    scroll_offset: 0.0,
                },
            );
            ui.spacer(0.0, 8.0);
            ui.row(StackStyle::gap(16.0), |row| {
                row.button("click", format!("点我（{clicks}）"), theme.button());
                row.button("open", "打开子窗口", theme.button());
                row.button(
                    "mask",
                    if masked {
                        "关闭蒙版"
                    } else {
                        "窗口蒙版"
                    },
                    theme.button(),
                );
                row.button("dialog", "消息对话框", theme.button());
                row.button("hide", "隐藏到托盘", theme.button());
            });
        });
        view.build(|text, slot, size| ctx.measure_text(text, slot, size))
    }
}

#[derive(Default)]
struct Child {
    pointer: Option<(f32, f32)>,
    text: String,
}

impl Application for Child {
    fn update(&mut self, ctx: &mut Context) {
        for event in ctx.events() {
            match event {
                AppEvent::MouseMoved { x, y } => self.pointer = Some((*x, *y)),
                AppEvent::MouseLeft => self.pointer = None,
                AppEvent::RegionClicked(name) if name == CLOSE_ACTION => ctx.close(),
                AppEvent::TextChanged { name, text } if name == "child" => {
                    self.text = text.clone();
                }
                AppEvent::Key { vk, pressed: true } | AppEvent::TextKey { vk, .. }
                    if *vk == 0x1B =>
                {
                    ctx.close()
                }
                _ => {}
            }
        }
    }

    fn view(&mut self, ctx: &Context) -> Document {
        let (width, height) = ctx.size();
        let theme = Theme::dark();
        let radius = if ctx.is_maximized() { 0.0 } else { 12.0 };
        let pointer = self.pointer;
        let text = self.text.clone();

        let mut view = View::new(width, height);
        if let Some((x, y)) = pointer {
            view.pointer(x, y);
        }
        view.background_rounded(theme.background, radius);
        view.title_bar("Cloud UI 子窗口", theme.title_bar(), ctx.is_maximized());
        view.column(StackStyle::padded(10.0, 20.0), |ui| {
            ui.label("这是第二个窗口；关掉它不影响主窗口。", theme.label());
            ui.spacer(0.0, 6.0);
            ui.text_edit("child", text.clone(), theme.text_edit());
            ui.label(format!("输入：{text}"), theme.muted());
            ui.spacer(0.0, 6.0);
            ui.label("Esc 关闭子窗口", theme.muted());
        });
        view.build(|text, slot, size| ctx.measure_text(text, slot, size))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn"))
        .try_init();
    run_native(
        WindowOptions {
            title: "Cloud UI Demo".to_owned(),
            size: (760.0, 760.0),
            min_size: Some((420.0, 360.0)),
            decorations: Decorations::None,
            top_most: false,
            shadow: true,
            corner_radius: 12.0,
            caption_height: Some(36.0),
            tray: Some(TrayOptions::new("Cloud UI Demo").menu(vec![
                TrayMenuItem::new("show", "显示窗口"),
                TrayMenuItem::new("hide", "隐藏到托盘"),
                TrayMenuItem::new("quit", "退出"),
            ])),
            ..WindowOptions::default()
        },
        Demo::default(),
    )?;
    Ok(())
}
