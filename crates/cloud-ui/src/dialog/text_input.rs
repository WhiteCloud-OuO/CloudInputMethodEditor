//! 文本输入对话框：同步、模态、返回字符串。
//!
//! 布局与消息对话框同一套三段式：标题栏 / 内容区（说明文本 + 编辑框）/ 按钮组（确定 / 取消）。
//! 编辑框由 [`TextHostWindow`] 承载：对话框模板只画背景 / 边框，真正的 RichEdit 宿主盖在
//! `TextEdit::edit_rect` / `TextBox::edit_rect` 上。
//!
//! 对话框是类：`new` 建实例，字体 / 颜色等属性在 `show` 之前改。

use std::rc::Rc;

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

use cloudime_render::{Color, Document, Element};

use super::blocking::{self, BlockingPanel};
use super::layout;
use super::style::DialogStyle;
use crate::current;
use crate::painter::Painter;
use crate::text_edit::TextHostWindow;
use crate::view::{TextEditStyle, Theme};

/// 多行编辑框高度（点）。
const MULTILINE_HEIGHT: f32 = 110.0;

/// 文本输入对话框实例。
///
/// 先在实例上改属性，再调 [`show`](Self::show)；`show` 会吃掉实例，之后不能再改。
#[derive(Debug, Clone)]
pub struct TextInputDialog {
    /// 标题栏文字。
    pub title: String,

    /// 内容区说明文本，`\n` 换行。
    pub text: String,

    /// `true` 用多行 `text_box`，否则用单行 `text_edit`。
    pub allow_multiline: bool,

    /// 打开时编辑框里的文本；取消时也返回它。
    pub default_text: String,

    /// 字体 / 颜色等可见属性。
    pub style: DialogStyle,
}

impl TextInputDialog {
    /// 建一个文本输入对话框实例；属性改完再 [`show`](Self::show)。
    pub fn new(
        title: impl Into<String>,
        text: impl Into<String>,
        allow_multiline: bool,
        default_text: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            text: text.into(),
            allow_multiline,
            default_text: default_text.into(),
            style: DialogStyle::default(),
        }
    }

    /// 同步显示文本输入对话框。
    ///
    /// 确定返回编辑框内容；取消返回 `default_text`（为空则空串）。
    /// 单行模式下 `default_text` 里的换行会显示成字面量 `\n`。
    pub fn show(self) -> String {
        let Self {
            title,
            text,
            allow_multiline,
            default_text,
            style,
        } = self;
        let Some(current) = current::get() else {
            blocking::abort_with("TextInputDialog: 没有当前独立窗口（必须在 UI 回调里调用）");
        };
        let theme = style.theme(&blocking::theme());
        let content = {
            let mut painter = current.painter.borrow_mut();
            let Some(painter) = painter.as_mut() else {
                blocking::abort_with("TextInputDialog: 渲染器未就绪");
            };
            build_layout(
                painter,
                &theme,
                &style,
                &title,
                &text,
                allow_multiline,
                &default_text,
            )
        };
        let panel = match BlockingPanel::open(&current, content.document, button_index) {
            Ok(panel) => panel,
            Err(error) => {
                tracing::warn!(%error, "TextInputDialog: 创建窗口失败");
                return default_text;
            }
        };
        let host = if panel.is_visible() {
            input_host(
                panel.template(),
                current.top_most,
                &content.style,
                content.family.as_deref(),
                content.input,
                allow_multiline,
                shown_text(&default_text, allow_multiline),
            )
        } else {
            None
        };
        if let Some(host) = &host {
            // 对话框自己的编辑框也是独立窗口：拖动 dialog 时一起走。
            panel.template().follow_edit_host(host.clone());
        }
        let value = panel.pump();

        let output = if value == 0 {
            host.as_ref()
                .map(|host| normalize_text(&host.text_string()))
                .unwrap_or_else(|| default_text.clone())
        } else {
            default_text
        };

        // `host` 后声明、先析构：先收起编辑框，再让 `panel` 恢复所有者。
        drop(host);
        output
    }
}

struct Content {
    document: Document,
    /// 编辑框宿主在内容区里的矩形（点）。
    input: (f32, f32, f32, f32),
    style: TextEditStyle,
    family: Option<String>,
}

fn build_layout(
    painter: &mut Painter,
    theme: &Theme,
    dialog_style: &DialogStyle,
    title: &str,
    text: &str,
    allow_multiline: bool,
    default_text: &str,
) -> Content {
    let font = dialog_style.font;
    let text_size = dialog_style.text_size();
    let title_bar = theme.title_bar();
    let title_size = dialog_style.title_size.unwrap_or(title_bar.title_size);
    let mut button_style = theme.button();
    button_style.size = text_size;
    let mut style = if allow_multiline {
        theme.text_box(MULTILINE_HEIGHT)
    } else {
        theme.text_edit()
    };
    style.font = font;
    style.size = text_size;
    let family = painter.text_font(font).0;
    let content_top = title_bar.height + layout::PADDING;
    let (body, desc_height) = layout::body(
        painter,
        text,
        layout::PADDING,
        content_top,
        layout::WIDTH - layout::PADDING * 2.0,
        theme.text,
        font,
        text_size,
    );
    let input_top = content_top + desc_height + layout::SECTION_GAP;
    let button_top = input_top + style.height + layout::SECTION_GAP;
    let height = button_top + layout::BUTTON_HEIGHT + layout::PADDING;

    let mut elements = Vec::new();
    elements.push(Element::Rect {
        x: 0.0,
        y: 0.0,
        width: layout::WIDTH,
        height,
        radius: painter.corner_radius(),
        color: theme.background,
    });
    elements.push(layout::title(painter, &title_bar, title, font, title_size));
    elements.push(body);

    let input_rect = cloud_ui_core::Rect::new(
        layout::PADDING,
        input_top,
        layout::WIDTH - layout::PADDING * 2.0,
        style.height,
    );
    let text = shown_text(default_text, allow_multiline);
    if allow_multiline {
        crate::view::text_edit::build_box(
            &mut elements,
            input_rect,
            "text_input.value",
            &text,
            &style,
        );
    } else {
        crate::view::text_edit::build(&mut elements, input_rect, "text_input.value", &text, &style);
    }
    let edit = match elements.last() {
        Some(Element::TextEdit(edit)) => edit.edit_rect(),
        Some(Element::TextBox(edit)) => edit.edit_rect(),
        _ => (0.0, 0.0, input_rect.width, input_rect.height),
    };
    let input = (input_rect.x + edit.0, input_rect.y + edit.1, edit.2, edit.3);

    let labels = ["确定", "取消"];
    let widths: Vec<f32> = labels
        .iter()
        .map(|label| layout::button_width(painter, &button_style, label, font))
        .collect();
    let total: f32 =
        widths.iter().sum::<f32>() + layout::BUTTON_GAP * (labels.len().saturating_sub(1)) as f32;
    let mut x = layout::WIDTH - layout::PADDING - total;
    let mut regions = Vec::with_capacity(labels.len());
    for (index, (label, width)) in labels.iter().zip(widths).enumerate() {
        let region = if index == 0 {
            "text_input.ok"
        } else {
            "text_input.cancel"
        };
        regions.push(layout::push_button(
            painter,
            &mut elements,
            &button_style,
            x,
            button_top,
            width,
            label,
            region.to_owned(),
            font,
        ));
        x += width + layout::BUTTON_GAP;
    }

    Content {
        document: Document {
            width: layout::WIDTH,
            height,
            regions,
            control_names: Vec::new(),
            instances: Default::default(),
            elements,
        },
        input,
        style,
        family,
    }
}

fn input_host(
    template: &crate::auxiliary_window_template::AuxiliaryWindowTemplate,
    top_most: bool,
    style: &TextEditStyle,
    family: Option<&str>,
    input: (f32, f32, f32, f32),
    multiline: bool,
    text: String,
) -> Option<Rc<TextHostWindow>> {
    let mut window = RECT::default();
    unsafe {
        let _ = GetWindowRect(template.hwnd(), &mut window);
    }
    let margin = template.margin();
    let dpi = unsafe { GetDpiForWindow(template.hwnd()) }.max(96);
    let scale = dpi as f32 / 96.0;
    let x = window.left + margin + (input.0 * scale).round() as i32;
    let y = window.top + margin + (input.1 * scale).round() as i32;
    let width = (input.2 * scale).round().max(1.0) as i32;
    let height = (input.3 * scale).round().max(1.0) as i32;
    let text_color = style.text_color.unwrap_or(Color::rgb(236, 236, 242));
    let host = TextHostWindow::create(
        template.hwnd(),
        &text,
        family,
        style.size,
        colorref(style.fill.unwrap_or(Color::rgb(24, 24, 30))),
        colorref(text_color),
        colorref(
            style
                .selection_background
                .unwrap_or(Color::rgb(0, 120, 212)),
        ),
        colorref(style.selection_color.unwrap_or(Color::rgb(255, 255, 255))),
        colorref(style.caret_color.unwrap_or(text_color)),
        0,
        false,
        true,
        false,
        multiline,
        false,
        multiline,
        0,
        top_most,
        dpi,
    )
    .inspect_err(|error: &windows::core::Error| {
        tracing::warn!(%error, "TextInputDialog: 创建编辑框宿主失败");
    })
    .ok()?;
    if !host.set_bounds(x, y, width, height, true) {
        tracing::debug!("TextInputDialog: 编辑框宿主位置没变");
    }
    host.show(true);
    if multiline {
        host.ensure_layout();
    }
    // 打开即全选缺省文本：直接输入就是替换，点确定则原样返回。
    host.select_all(true);
    // 把插入符放进编辑框，省得用户先点一下。
    host.focus();
    Some(host)
}

fn button_index(region: &str) -> Option<u8> {
    match region {
        "text_input.ok" => Some(0),
        "text_input.cancel" => Some(1),
        _ => None,
    }
}

fn colorref(color: Color) -> COLORREF {
    COLORREF(u32::from(color.r) | (u32::from(color.g) << 8) | (u32::from(color.b) << 16))
}

/// 编辑框里显示的初始文本：单行模式把真实换行转成字面量 `\n`。
fn shown_text(default_text: &str, allow_multiline: bool) -> String {
    let text = normalize_text(default_text);
    if allow_multiline {
        text
    } else {
        text.replace('\n', "\\n")
    }
}

fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_escapes_newlines() {
        assert_eq!(shown_text("a\nb", false), "a\\nb");
        assert_eq!(shown_text("a\r\nb", false), "a\\nb");
        assert_eq!(shown_text("a\nb", true), "a\nb");
    }

    #[test]
    fn button_regions_map_to_buttons() {
        assert_eq!(button_index("text_input.ok"), Some(0));
        assert_eq!(button_index("text_input.cancel"), Some(1));
        assert_eq!(button_index("text_input.value"), None);
    }
}
