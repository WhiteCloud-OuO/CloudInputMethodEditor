//! 文本列表选择对话框：同步、模态，返回选中项下标（取消 / 关闭为 -1）。
//!
//! 布局与消息 / 文本输入对话框同一套：标题栏 / 说明文本 / 文本列表 / 按钮组（确定 / 取消）。
//! 列表复用 [`build_text_list`]，行命中区域由对话框自己生成；点行只更新选中项，不关闭对话框；
//! 双击一行等同「选中并确定」。每次显示都从**未选中**（`-1`）开始。
//!
//! 对话框是类：`new` 建实例，属性在 `show` 之前改。

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use cloudime_render::{
    Document, Element, HitRect, HitShape, Region, TextListStyle, build_text_list,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;

use super::blocking::{self, BlockingPanel, DialogHandle};
use super::layout;
use super::style::DialogStyle;
use crate::auxiliary_window_template::{PanelAction, PanelEvent};
use crate::current;
use crate::painter::Painter;
use crate::view::Theme;

/// 列表默认视口高度（点）。
const LIST_HEIGHT: f32 = 180.0;

/// 「确定」的内部结果；由 [`TextListDialog::show`] 换成选中项下标。
const OK: i32 = -2;

/// 文本列表选择对话框实例。
///
/// 先在实例上改属性，再调 [`show`](Self::show)；`show` 会吃掉实例，之后不能再改。
#[derive(Debug, Clone)]
pub struct TextListDialog {
    /// 标题栏文字。
    pub title: String,

    /// 列表上方的提示文本，`\n` 换行。
    pub text: String,

    /// 可选项。
    pub items: Vec<String>,

    /// 列表视口高度（点）。
    pub list_height: f32,

    /// 字体 / 颜色等可见属性。
    pub style: DialogStyle,
}

impl TextListDialog {
    /// 建一个文本列表选择对话框实例；属性改完再 [`show`](Self::show)。
    pub fn new<I, S>(title: impl Into<String>, text: impl Into<String>, items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let items: Vec<String> = items.into_iter().map(Into::into).collect();
        Self {
            title: title.into(),
            text: text.into(),
            items,
            list_height: LIST_HEIGHT,
            style: DialogStyle::default(),
        }
    }

    /// 同步显示对话框。
    ///
    /// 每次显示都从**未选中**（-1）开始。点「确定」返回选中项的原下标（0 起）；
    /// 双击一行等同选中并确定。没选中、列表为空或点「取消」/关闭窗口时返回 `-1`。
    pub fn show(self) -> i32 {
        let Self {
            title,
            text,
            items,
            list_height,
            style,
        } = self;
        let selected = None;
        let Some(current) = current::get() else {
            blocking::abort_with("TextListDialog: 没有当前独立窗口（必须在 UI 回调里调用）");
        };
        let theme = style.theme(&blocking::theme());
        let list_style = list_style(&theme, &style, list_height);
        let document = {
            let mut borrowed = current.painter.borrow_mut();
            let Some(painter) = borrowed.as_mut() else {
                blocking::abort_with("TextListDialog: 渲染器未就绪");
            };
            build_document(
                painter,
                &theme,
                &style,
                &list_style,
                &title,
                &text,
                &items,
                selected,
                0.0,
            )
        };

        let selected_cell = Rc::new(Cell::new(selected));
        let scroll = Rc::new(Cell::new(0.0f32));
        let on_event = {
            let selected = Rc::clone(&selected_cell);
            let scroll = Rc::clone(&scroll);
            let painter = current.painter.clone();
            let style = style.clone();
            let title = title.clone();
            let text = text.clone();
            let items = items.clone();
            let mut last_click: Option<(Instant, usize)> = None;
            move |event: &PanelEvent, host: &DialogHandle| {
                match event.action {
                    PanelAction::Click => {
                        let Some(index) = event
                            .region
                            .as_deref()
                            .and_then(|region| region.strip_prefix("text_list_dialog.item."))
                            .and_then(|index| index.parse::<usize>().ok())
                        else {
                            last_click = None;
                            return;
                        };
                        if index >= items.len() {
                            last_click = None;
                            return;
                        }
                        let now = Instant::now();
                        let interval = unsafe { GetDoubleClickTime() };
                        let double = is_double_click(last_click, now, index, interval);
                        last_click = Some((now, index));
                        if double {
                            // 双击一行等同选中并按下「确定」。
                            selected.set(Some(index));
                            host.finish(index as i32);
                            return;
                        }
                        if selected.get() == Some(index) {
                            return;
                        }
                        selected.set(Some(index));
                    }
                    PanelAction::Wheel => {
                        last_click = None;
                        let max = max_scroll(&list_style, items.len(), list_height);
                        let notches = event.delta.unwrap_or(0) as f32 / 120.0;
                        let next = (scroll.get() - notches * list_style.stride()).clamp(0.0, max);
                        if (next - scroll.get()).abs() < 0.01 {
                            return;
                        }
                        scroll.set(next);
                    }
                    _ => {
                        last_click = None;
                        return;
                    }
                }
                let document = {
                    let mut borrowed = painter.borrow_mut();
                    let Some(painter) = borrowed.as_mut() else {
                        return;
                    };
                    build_document(
                        painter,
                        &theme,
                        &style,
                        &list_style,
                        &title,
                        &text,
                        &items,
                        selected.get(),
                        scroll.get(),
                    )
                };
                host.set_document(document);
            }
        };

        let panel = match BlockingPanel::open_with(&current, document, button, on_event) {
            Ok(panel) => panel,
            Err(error) => {
                tracing::warn!(%error, "TextListDialog: 创建窗口失败");
                return -1;
            }
        };
        let value = panel.pump_value();
        if value == OK {
            selected_cell.get().map_or(-1, |index| index as i32)
        } else {
            // 双击行直接返回下标；取消 / 关闭是 -1。
            value
        }
    }
}

/// 对话框按钮区域名 → 结果；「确定」用一个内部值，`show` 里再换成选中项下标。
fn button(region: &str) -> Option<i32> {
    match region {
        "text_list_dialog.ok" => Some(OK),
        "text_list_dialog.cancel" => Some(-1),
        _ => None,
    }
}

/// 两次点在同一行、间隔小于系统双击时间，算一次双击。
fn is_double_click(
    last: Option<(Instant, usize)>,
    now: Instant,
    index: usize,
    interval_ms: u32,
) -> bool {
    last.is_some_and(|(then, last_index)| {
        last_index == index && now.duration_since(then).as_millis() < u128::from(interval_ms)
    })
}

/// 按对话框主题与正文字号生成文本列表样式；固定视口，滚动由滚轮 / 滚动条处理。
fn list_style(theme: &Theme, style: &DialogStyle, list_height: f32) -> TextListStyle {
    let mut list = theme.text_list();
    list.auto_height = false;
    list.height = Some(list_height);
    list.width = None;
    list.font = style.font;
    list.size = style.text_size();
    list
}

fn max_scroll(style: &TextListStyle, items: usize, list_height: f32) -> f32 {
    let content = style.content_height(items);
    let viewport = (list_height - (style.edge() + style.content_inset()) * 2.0).max(0.0);
    (content - viewport).max(0.0)
}

/// 生成对话框的声明式内容。
#[allow(clippy::too_many_arguments)]
fn build_document(
    painter: &mut Painter,
    theme: &Theme,
    style: &DialogStyle,
    list_style: &TextListStyle,
    title: &str,
    text: &str,
    items: &[String],
    selected: Option<usize>,
    scroll: f32,
) -> Document {
    let font = style.font;
    let text_size = style.text_size();
    let title_bar = theme.title_bar();
    let title_size = style.title_size.unwrap_or(title_bar.title_size);
    let mut button_style = theme.button();
    button_style.size = text_size;
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
    let list_rect = (
        layout::PADDING,
        content_top + desc_height + layout::SECTION_GAP,
        layout::WIDTH - layout::PADDING * 2.0,
        list_style.height.unwrap_or(LIST_HEIGHT),
    );
    let button_top = list_rect.1 + list_rect.3 + layout::SECTION_GAP;
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

    let visible: Vec<usize> = (0..items.len()).collect();
    let mut measure = |text: &str, slot, size| painter.measure_text(text, slot, size);
    let list = build_text_list(
        list_style,
        items,
        &visible,
        selected,
        None,
        scroll,
        0.0,
        list_rect,
        "text_list_dialog.list",
        &mut measure,
    );
    let mut regions = Vec::new();
    let edge = list.edge_width;
    let gap = list.content_gap;
    let viewport_width = list.viewport_width();
    let viewport_height = list.viewport_height();
    for row in &list.rows {
        let Some(top) = list.row_view_top(row) else {
            continue;
        };
        let top = top.max(0.0);
        let bottom = (top + row.height).min(viewport_height);
        if bottom <= top {
            continue;
        }
        regions.push(Region::new(
            format!("text_list_dialog.item.{}", row.index),
            HitShape::Rect(HitRect::new(
                list_rect.0 + edge + gap,
                list_rect.1 + edge + gap + top,
                viewport_width,
                bottom - top,
            )),
            false,
        ));
    }
    elements.push(Element::ListBox(list));

    let labels = ["确定", "取消"];
    let widths: Vec<f32> = labels
        .iter()
        .map(|label| layout::button_width(painter, &button_style, label, font))
        .collect();
    let total: f32 =
        widths.iter().sum::<f32>() + layout::BUTTON_GAP * (labels.len().saturating_sub(1)) as f32;
    let mut x = layout::WIDTH - layout::PADDING - total;
    for (index, (label, width)) in labels.iter().zip(widths).enumerate() {
        let region = if index == 0 {
            "text_list_dialog.ok"
        } else {
            "text_list_dialog.cancel"
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

    Document {
        width: layout::WIDTH,
        height,
        regions,
        control_names: Vec::new(),
        instances: Default::default(),
        scroll_targets: Vec::new(),
        elements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_regions_map_to_results() {
        assert_eq!(button("text_list_dialog.ok"), Some(OK));
        assert_eq!(button("text_list_dialog.cancel"), Some(-1));
        assert_eq!(button("text_list_dialog.item.3"), None);
        assert_eq!(button(""), None);
    }

    #[test]
    fn double_click_requires_the_same_row_within_the_interval() {
        let now = Instant::now();
        let last = Some((now, 3));
        assert!(is_double_click(
            last,
            now + std::time::Duration::from_millis(200),
            3,
            500
        ));
        assert!(!is_double_click(
            last,
            now + std::time::Duration::from_millis(200),
            4,
            500
        ));
        assert!(!is_double_click(
            last,
            now + std::time::Duration::from_millis(600),
            3,
            500
        ));
    }

    #[test]
    fn list_metrics_leave_room_for_scrolling() {
        let style = TextListStyle {
            auto_height: false,
            height: Some(100.0),
            ..TextListStyle::default()
        };
        assert!(max_scroll(&style, 10, 100.0) > 0.0);
        assert_eq!(max_scroll(&style, 1, 100.0), 0.0);
    }
}
