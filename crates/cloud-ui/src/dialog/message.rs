//! 消息对话框：类似 Win32 `MessageBoxW` 的同步模态面板。
//!
//! 布局从上到下三段：标题栏 / 内容区（左 SVG + 右文本块）/ 按钮组。没有标题栏关闭按钮，
//! 必须点按钮组里的一个按钮；按钮最多 4 个，`show` 返回被点按钮的下标（0–3）。
//!
//! 对话框是类：`new` 建实例，字体 / 颜色等属性在 `show` 之前改。
//! `show(self)` 是**同步**的：在当前独立窗口的 UI 上下文里创建一块 owned window，跑一个嵌套消息循环，
//! 用户点按钮后返回；期间所有者窗口被 `EnableWindow(FALSE)` 挡住。所有者来自
//! [`crate::current`] 的线程本地当前窗口。

use cloudime_render::{Document, Element};

use super::blocking::{self, BlockingPanel};
use super::layout;
use super::style::DialogStyle;
use crate::current;
use crate::painter::Painter;
use crate::view::Theme;

/// 按钮最多几个。
const MAX_BUTTONS: usize = 4;

/// 图标边长与和正文的间距。
const ICON_SIZE: f32 = 48.0;
const ICON_GAP: f32 = 14.0;

const FAILURE_SVG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator 24.1.1, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->
<svg version="1.1" id="图层_1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" x="0px" y="0px"
	 viewBox="0 0 48 48" style="enable-background:new 0 0 48 48;" xml:space="preserve">
<style type="text/css">
	.st0{fill:#FF8080;}
</style>
<path d="M24,2C11.85,2,2,11.85,2,24c0,12.15,9.85,22,22,22s22-9.85,22-22C46,11.85,36.15,2,24,2z M24,44C12.95,44,4,35.05,4,24
	S12.95,4,24,4s20,8.95,20,20S35.05,44,24,44z"/>
<circle class="st0" cx="24" cy="24" r="20"/>
<path d="M26.11,24.7l8.49,8.49c0.39,0.39,0.39,1.02,0,1.41l0,0c-0.39,0.39-1.02,0.39-1.41,0l-8.49-8.49c-0.39-0.39-1.02-0.39-1.41,0
	l-8.49,8.49c-0.39,0.39-1.02,0.39-1.41,0l0,0c-0.39-0.39-0.39-1.02,0-1.41l8.49-8.49c0.39-0.39,0.39-1.02,0-1.41l-8.49-8.49
	c-0.39-0.39-0.39-1.02,0-1.41l0,0c0.39-0.39,1.02-0.39,1.41,0l8.49,8.49c0.39,0.39,1.02,0.39,1.41,0l8.49-8.49
	c0.39-0.39,1.02-0.39,1.41,0v0c0.39,0.39,0.39,1.02,0,1.41l-8.49,8.49C25.73,23.68,25.73,24.32,26.11,24.7z"/>
</svg>"#;

const SUCCESS_SVG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator 24.1.1, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->
<svg version="1.1" id="图层_1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" x="0px" y="0px"
	 viewBox="0 0 48 48" style="enable-background:new 0 0 48 48;" xml:space="preserve">
<style type="text/css">
	.st0{fill:#80FF8C;}
</style>
<path d="M24,2C11.85,2,2,11.85,2,24c0,12.15,9.85,22,22,22s22-9.85,22-22C46,11.85,36.15,2,24,2z M24,44C12.95,44,4,35.05,4,24
	S12.95,4,24,4s20,8.95,20,20S35.05,44,24,44z"/>
<circle class="st0" cx="24" cy="24" r="20"/>
<g>
	<path d="M36.14,15.62c0.39,0.39,0.39,1.03,0,1.42L21.51,31.68l-0.71,0.71c-0.39,0.39-1.02,0.39-1.41,0l-0.71-0.71l-6.82-6.82
		c-0.39-0.39-0.39-1.02,0-1.41l0.01-0.01c0.39-0.39,1.02-0.39,1.41,0l6.11,6.11c0.39,0.39,1.02,0.39,1.41,0l13.93-13.93
		C35.12,15.23,35.75,15.23,36.14,15.62L36.14,15.62z"/>
</g>
</svg>"#;

const QUERY_SVG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator 24.1.1, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->
<svg version="1.1" id="图层_1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" x="0px" y="0px"
	 viewBox="0 0 48 48" style="enable-background:new 0 0 48 48;" xml:space="preserve">
<style type="text/css">
	.st0{fill:#66CCFF;}
</style>
<path d="M24,2C11.85,2,2,11.85,2,24c0,12.15,9.85,22,22,22s22-9.85,22-22C46,11.85,36.15,2,24,2z M24,44C12.95,44,4,35.05,4,24
	S12.95,4,24,4s20,8.95,20,20S35.05,44,24,44z"/>
<circle class="st0" cx="24" cy="24" r="20"/>
<g>
	<path d="M30.82,16.56c0,3.52-2.6,6.44-6,6.92v7.53c0,0.55-0.45,1-1,1c-0.56,0-1-0.45-1-1v-8.45c0-0.55,0.45-1,1-1
		c0.34,0,0.68-0.03,1-0.1c2.29-0.46,4-2.48,4-4.9c0-2.42-1.71-4.44-4-4.9c-0.32-0.07-0.66-0.1-1-0.1c-0.34,0-0.68,0.03-1,0.1
		c-1.67,0.34-3.04,1.52-3.66,3.07c-0.01,0.05-0.02,0.1-0.04,0.14h-0.01c0,0.03-0.01,0.06-0.03,0.08c-0.15,0.35-0.49,0.58-0.9,0.58
		c-0.55,0-1-0.44-1-1c0-0.12,0.02-0.23,0.07-0.34c0-0.02,0-0.03,0.01-0.05c0.98-2.68,3.55-4.58,6.56-4.58
		C27.69,9.56,30.82,12.69,30.82,16.56z"/>
	<path d="M23.82,38.44L23.82,38.44c-0.55,0-1-0.45-1-1v-1.52c0-0.55,0.45-1,1-1h0c0.55,0,1,0.45,1,1v1.52
		C24.82,37.99,24.37,38.44,23.82,38.44z"/>
</g>
</svg>"#;

const WARNING_SVG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator 24.1.1, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->
<svg version="1.1" id="图层_1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" x="0px" y="0px"
	 viewBox="0 0 48 48" style="enable-background:new 0 0 48 48;" xml:space="preserve">
<style type="text/css">
	.st0{fill:#FFF066;}
</style>
<path d="M24,2C11.85,2,2,11.85,2,24c0,12.15,9.85,22,22,22s22-9.85,22-22C46,11.85,36.15,2,24,2z M24,44C12.95,44,4,35.05,4,24
	S12.95,4,24,4s20,8.95,20,20S35.05,44,24,44z"/>
<circle class="st0" cx="24" cy="24" r="20"/>
<path d="M24,30.55L24,30.55c-0.55,0-1-0.45-1-1V8.33c0-0.55,0.45-1,1-1h0c0.55,0,1,0.45,1,1v21.22C25,30.1,24.55,30.55,24,30.55z"/>
<path d="M24.13,38.98h-0.27c-0.48,0-0.87-0.39-0.87-0.87v-2.83c0-0.48,0.39-0.87,0.87-0.87h0.27c0.48,0,0.87,0.39,0.87,0.87v2.83
	C25,38.59,24.61,38.98,24.13,38.98z"/>
</svg>"#;

/// 消息对话框实例。
///
/// 先在实例上改属性，再调 [`show`](Self::show)；`show` 会吃掉实例，之后不能再改。
#[derive(Debug, Clone)]
pub struct MessageDialog {
    /// 标题栏文字。
    pub title: String,

    /// 内容区文本，`\n` 换行，`\\` 表示反斜杠。
    pub text: String,

    /// 内置消息图案键；未知键不画图。
    pub icon: String,

    /// 1–4 个按钮文字。
    pub buttons: Vec<String>,

    /// 字体 / 颜色等可见属性。
    pub style: DialogStyle,
}

impl MessageDialog {
    /// 建一个消息对话框实例；属性改完再 [`show`](Self::show)。
    pub fn new(
        title: impl Into<String>,
        text: impl Into<String>,
        icon: impl Into<String>,
        buttons: &[&str],
    ) -> Self {
        Self {
            title: title.into(),
            text: text.into(),
            icon: icon.into(),
            buttons: buttons.iter().map(|button| (*button).to_owned()).collect(),
            style: DialogStyle::default(),
        }
    }

    /// 同步显示消息对话框，返回被点按钮的下标（0–3）。
    ///
    /// `buttons` 为空或超过 4 个会异常终止程序。没有当前窗口 / 渲染器未就绪同样走
    /// `std::process::abort()`（和 dialog 的硬规则一致）。
    pub fn show(self) -> u8 {
        let Self {
            title,
            text,
            icon,
            buttons,
            style,
        } = self;
        if !is_valid_button_count(buttons.len()) {
            blocking::abort_with("MessageDialog: buttons 必须是 1..=4 个");
        }
        let Some(current) = current::get() else {
            blocking::abort_with("MessageDialog: 没有当前独立窗口（必须在 UI 回调里调用）");
        };
        let theme = style.theme(&blocking::theme());
        let document = {
            let mut painter = current.painter.borrow_mut();
            let Some(painter) = painter.as_mut() else {
                blocking::abort_with("MessageDialog: 渲染器未就绪");
            };
            build_document(painter, &theme, &style, &title, &text, &icon, &buttons)
        };
        match BlockingPanel::open(&current, document, button_index) {
            Ok(panel) => panel.pump(),
            Err(error) => {
                tracing::warn!(%error, "MessageDialog: 创建窗口失败");
                0
            }
        }
    }
}

/// 生成对话框的声明式内容。
fn build_document(
    painter: &mut Painter,
    theme: &Theme,
    style: &DialogStyle,
    title: &str,
    text: &str,
    icon: &str,
    buttons: &[String],
) -> Document {
    let radius = painter.corner_radius();
    let font = style.font;
    let text_size = style.text_size();
    let mut button_style = theme.button();
    button_style.size = text_size;
    let title_bar = theme.title_bar();
    let title_size = style.title_size.unwrap_or(title_bar.title_size);
    let icon_svg = message_icon(icon);
    let text_x = layout::PADDING + icon_svg.map_or(0.0, |_| ICON_SIZE + ICON_GAP);
    let text_width = (layout::WIDTH - text_x - layout::PADDING).max(1.0);
    // 标题栏和 cloud-ui-demo 主窗口一致：高 36、左留白 16、13pt、次要文字色；下面不画分隔线。
    let content_top = title_bar.height + layout::PADDING;
    let (body, body_height) = layout::body(
        painter,
        text,
        text_x,
        content_top,
        text_width,
        theme.text,
        font,
        text_size,
    );
    let content_height = body_height.max(icon_svg.map_or(0.0, |_| ICON_SIZE));
    let button_top = content_top + content_height + layout::SECTION_GAP;
    let height = button_top + layout::BUTTON_HEIGHT + layout::PADDING;

    let mut elements = Vec::new();
    elements.push(Element::Rect {
        x: 0.0,
        y: 0.0,
        width: layout::WIDTH,
        height,
        radius,
        color: theme.background,
    });
    elements.push(layout::title(painter, &title_bar, title, font, title_size));

    if let Some(source) = icon_svg {
        elements.push(Element::Svg {
            x: layout::PADDING,
            y: content_top + (content_height - ICON_SIZE) / 2.0,
            size: ICON_SIZE,
            source: source.to_owned(),
            tint: None,
        });
    }
    elements.push(body);

    let widths: Vec<f32> = buttons
        .iter()
        .map(|button| layout::button_width(painter, &button_style, button, font))
        .collect();
    let total: f32 =
        widths.iter().sum::<f32>() + layout::BUTTON_GAP * (buttons.len().saturating_sub(1)) as f32;
    let mut x = layout::WIDTH - layout::PADDING - total;
    let mut regions = Vec::with_capacity(buttons.len());
    for (index, (button, width)) in buttons.iter().zip(widths).enumerate() {
        regions.push(layout::push_button(
            painter,
            &mut elements,
            &button_style,
            x,
            button_top,
            width,
            button,
            format!("message.button.{index}"),
            font,
        ));
        x += width + layout::BUTTON_GAP;
    }

    Document {
        width: layout::WIDTH,
        height,
        regions,
        control_names: Vec::new(),
        instances: Default::default(),
        elements,
    }
}

/// 按钮数量必须在 1..=4。
fn is_valid_button_count(count: usize) -> bool {
    (1..=MAX_BUTTONS).contains(&count)
}

/// 按钮区域名 → 返回下标；不是按钮区域时 `None`。
fn button_index(region: &str) -> Option<u8> {
    region
        .strip_prefix("message.button.")?
        .parse::<u8>()
        .ok()
        .filter(|index| usize::from(*index) < MAX_BUTTONS)
}

/// Cloud UI 内置消息图案；空串 = 不显示图案。
fn message_icon(name: &str) -> Option<&'static str> {
    match name {
        "" => None,
        "failure" => Some(FAILURE_SVG),
        "success" => Some(SUCCESS_SVG),
        "query" => Some(QUERY_SVG),
        "warning" => Some(WARNING_SVG),
        other => {
            tracing::debug!(icon = other, "MessageDialog: 未知图标键，不显示图案");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_regions_map_to_indexes() {
        assert_eq!(button_index("message.button.0"), Some(0));
        assert_eq!(button_index("message.button.3"), Some(3));
        assert_eq!(button_index("message.button.4"), None);
        assert_eq!(button_index("other"), None);
        assert_eq!(button_index(""), None);
    }

    #[test]
    fn buttons_must_be_between_one_and_four() {
        assert!(!is_valid_button_count(0));
        assert!(is_valid_button_count(1));
        assert!(is_valid_button_count(4));
        assert!(!is_valid_button_count(5));
    }

    #[test]
    fn icon_keys_map_to_embedded_svgs() {
        assert!(message_icon("").is_none());
        for key in ["failure", "success", "query", "warning"] {
            assert!(
                message_icon(key).is_some_and(|svg| svg.contains("<svg")),
                "{key}"
            );
        }
        assert!(message_icon("info").is_none());
    }
}
