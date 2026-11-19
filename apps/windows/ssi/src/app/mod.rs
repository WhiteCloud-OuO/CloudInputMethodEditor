//! 特殊字符输入器主窗口：7 个标签页、符号网格、英文花体转换。
//!
//! 窗口默认置顶且不抢焦点（`WindowOptions.no_activate`）：点符号时焦点还在目标程序里，
//! 左键用 `SendInput` 把字符敲进去、右键复制；第 7 页要输入英文，切过去时临时允许激活。

pub(crate) mod chrome;

use cloud_ui::{AppEvent, Application, CLOSE_ACTION, Context, MouseButton, Rect, Theme};
use cloudime_render::{Document, Element, FontSlot, Region};

use crate::data::{self, GridBox, Page};
use crate::fancy;
use crate::inject;
use crate::kana::{self, Variant};
use crate::ui;

/// 窗口内边距。
const MARGIN: f32 = 16.0;
/// 标签栏高度。
const TAB_HEIGHT: f32 = 30.0;
/// 标签之间的间隙。
const TAB_GAP: f32 = 4.0;
/// 底部状态行高度。
const STATUS_HEIGHT: f32 = 22.0;
/// 符号网格与面板边缘之间至少留这么多，免得按钮压到面板圆角。
const GRID_PADDING: f32 = 12.0;
/// 第 7 页（英文花体）的页号。
const FANCY: usize = 6;
/// 第 5 页（日文假名）的页号。
const KANA_PAGE: usize = 4;
/// 第 6 页（制表符 / 箭头）的页号。
const BOX_PAGE: usize = 5;

pub struct SsiApp {
    pages: Vec<Page>,
    styles: Vec<Vec<char>>,
    grid: GridBox,
    theme: Theme,
    page: usize,
    style: usize,
    input: String,
    result: String,
    /// 日文假名页的变体模式（无 / 浊゛ / 半゜）。
    kana: Variant,
    /// 状态行里的提示 / 错误；`None` 时按悬停显示码位。
    message: Option<String>,
    pointer: Option<(f32, f32)>,
    hovered: Option<usize>,
    /// 切到第 7 页后要等窗口去掉 `WS_EX_NOACTIVATE`，下一帧再激活并聚焦。
    pending_activate: bool,
    focus_input: bool,
    /// 本窗口句柄（`isize`），用于判断前台是不是自己。
    own: Option<isize>,
    /// 上一个目标窗口（`SendInput` 往哪儿送）。
    target: Option<isize>,
    /// 这一帧的符号格子，右键命中用。
    glyph_rects: Vec<(Rect, String)>,
}

impl SsiApp {
    pub fn new(theme: Theme, target: Option<isize>) -> Self {
        let pages = data::load();
        let grid = data::grid_box(&pages);
        let styles = data::load_styles();
        Self {
            pages,
            styles,
            grid,
            theme,
            page: 0,
            style: 0,
            input: String::new(),
            result: String::new(),
            kana: Variant::Plain,
            message: None,
            pointer: None,
            hovered: None,
            pending_activate: false,
            focus_input: false,
            own: None,
            target,
            glyph_rects: Vec::new(),
        }
    }

    fn page_count(&self) -> usize {
        self.pages.len() + 1
    }

    fn tab_title(&self, index: usize) -> &str {
        if index == FANCY {
            "英文花体"
        } else {
            self.pages.get(index).map_or("", |page| page.title.as_str())
        }
    }

    /// 切页：进第 7 页允许激活并聚焦输入框，离开时恢复「不抢焦点」。
    fn set_page(&mut self, page: usize, ctx: &Context) {
        if page == self.page || page >= self.page_count() {
            return;
        }
        let leaving_fancy = self.page == FANCY;
        self.page = page;
        self.message = None;
        if page == FANCY {
            ctx.set_no_activate(false);
            // 去掉 NOACTIVATE 的窗口命令在这一帧画完才执行，激活推迟到下一帧。
            self.pending_activate = true;
        } else if leaving_fancy {
            ctx.set_no_activate(true);
            self.pending_activate = false;
            self.focus_input = false;
        }
    }

    /// 把字符送进目标程序：先把前台还回去（需要时），再 `SendInput`。
    fn insert(&mut self, text: &str) {
        let own = self.own;
        if let Some(foreground) = inject::foreground_window()
            && Some(foreground) != own
        {
            self.target = Some(foreground);
        }
        if let Some(target) = self.target
            && Some(target) != own
            && inject::foreground_window() == own
        {
            inject::activate(target);
        }
        self.message = Some(match inject::inject(text) {
            Ok(()) => format!("已输入：{text}"),
            Err(error) => error,
        });
    }

    fn copy(&mut self, text: &str) {
        self.message = Some(match inject::copy(text) {
            Ok(()) => format!("已复制：{text}"),
            Err(error) => error,
        });
    }

    fn refresh_result(&mut self) {
        self.result = self
            .styles
            .get(self.style)
            .map_or_else(String::new, |style| fancy::convert(&self.input, style));
    }

    fn click(&mut self, name: &str, ctx: &Context) {
        if name == CLOSE_ACTION {
            ctx.close();
            return;
        }
        if let Some(index) = name.strip_prefix("tab:") {
            if let Ok(index) = index.parse::<usize>() {
                self.set_page(index, ctx);
            }
            return;
        }
        if let Some(index) = name.strip_prefix("glyph:") {
            if let Ok(index) = index.parse::<usize>()
                && let Some(glyph) = self
                    .pages
                    .get(self.page)
                    .and_then(|page| page.glyphs.get(index))
            {
                let text = self.output_text(&glyph.text);
                self.message = None;
                self.insert(&text);
            }
            return;
        }
        if let Some(mode) = name.strip_prefix("kana:") {
            self.kana = match mode {
                "voiced" => Variant::Voiced,
                "semi" => Variant::SemiVoiced,
                _ => Variant::Plain,
            };
            self.message = None;
            return;
        }
        if let Some(index) = name.strip_prefix("style:") {
            if let Ok(index) = index.parse::<usize>() {
                self.style = index.min(self.styles.len().saturating_sub(1));
                self.refresh_result();
            }
            return;
        }
        if name == "copy" {
            let result = self.result.clone();
            if result.is_empty() {
                self.message = Some("还没有可复制的结果".to_owned());
            } else {
                self.copy(&result);
            }
        }
    }

    /// 右键：复制鼠标下的符号。
    fn right_click(&mut self, x: f32, y: f32) {
        if let Some(text) = self
            .glyph_rects
            .iter()
            .find(|(rect, _)| ui::inside(*rect, x, y))
            .map(|(_, text)| self.output_text(text))
        {
            self.copy(&text);
        }
    }

    /// 真正要输入 / 复制的文本：日文页按当前变体模式转换，其余页原样。
    fn output_text(&self, text: &str) -> String {
        if self.page == KANA_PAGE {
            kana::apply(text, self.kana)
        } else {
            text.to_owned()
        }
    }

    /// 按钮上要显示的文本：假名页的浊音 / 半浊音模式换成对应写法；
    /// 没有变体的假名返回 `None`（隐藏，和旧版一致）。
    fn display_text(&self, text: &str) -> Option<String> {
        if self.page != KANA_PAGE {
            return Some(text.to_owned());
        }
        match self.kana {
            Variant::Plain => Some(text.to_owned()),
            mode => text
                .chars()
                .next()
                .and_then(|character| mode.map(character))
                .map(String::from),
        }
    }
}

impl Application for SsiApp {
    fn update(&mut self, ctx: &mut Context) {
        self.own = Some(ctx.hwnd().0 as isize);
        if self.pending_activate && self.page == FANCY {
            self.pending_activate = false;
            if let Some(own) = self.own {
                inject::activate(own);
            }
            self.focus_input = true;
        }
        let events = ctx.events().to_vec();
        for event in events {
            match event {
                AppEvent::MouseMoved { x, y } => self.pointer = Some((x, y)),
                AppEvent::MouseLeft => self.pointer = None,
                AppEvent::RegionClicked(name) => self.click(&name, ctx),
                AppEvent::MouseButton {
                    button: MouseButton::Right,
                    pressed: false,
                    x,
                    y,
                } => self.right_click(x, y),
                AppEvent::TextChanged { name, text } if name == "ssi.input" => {
                    self.input = text;
                    self.refresh_result();
                }
                _ => {}
            }
        }
        if self.focus_input {
            ctx.focus_text("ssi.input");
            self.focus_input = false;
        }
    }

    fn view(&mut self, ctx: &Context) -> Document {
        let (width, height) = ctx.size();
        let theme = self.theme;
        let mut elements: Vec<Element> = Vec::new();
        let mut regions: Vec<Region> = Vec::new();
        let mut measure =
            |text: &str, slot: FontSlot, size: f32| ctx.measure_text(text, slot, size);

        ui::panel(
            &mut elements,
            Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            theme.background,
            if ctx.is_maximized() { 0.0 } else { 12.0 },
        );
        chrome::paint(
            &mut elements,
            &mut regions,
            width,
            "特殊字符输入器",
            &theme,
            self.pointer,
            &mut measure,
        );

        let (tab_rects, tab_size) = self.tab_rects(width, &mut measure);
        for (index, rect) in tab_rects.iter().enumerate() {
            let mut look = if index == self.page {
                ui::ButtonLook::primary(&theme)
            } else {
                ui::ButtonLook::normal(&theme)
            };
            look.size = tab_size;
            ui::button(
                &mut elements,
                &mut regions,
                *rect,
                &format!("tab:{index}"),
                self.tab_title(index),
                &look,
                self.pointer,
                &mut measure,
            );
        }

        let content_top = chrome::HEIGHT + MARGIN + TAB_HEIGHT + 10.0;
        let content = Rect {
            x: MARGIN,
            y: content_top,
            width: (width - MARGIN * 2.0).max(1.0),
            height: (height - MARGIN - STATUS_HEIGHT - content_top).max(1.0),
        };
        self.glyph_rects.clear();
        self.hovered = None;
        if self.page == FANCY {
            self.paint_fancy(&mut elements, &mut regions, content, &mut measure);
        } else {
            self.paint_symbols(&mut elements, &mut regions, content, &mut measure);
        }

        // 状态行。
        let mut style = ui::muted(&theme, 12.0);
        if self
            .message
            .as_deref()
            .is_some_and(|text| text.starts_with("已"))
        {
            style.color = theme.accent;
        }
        let text = self.status_text();
        ui::label(
            &mut elements,
            Rect {
                x: MARGIN,
                y: height - MARGIN - STATUS_HEIGHT + 2.0,
                width: (width - MARGIN * 2.0).max(1.0),
                height: STATUS_HEIGHT,
            },
            &text,
            &style,
            &mut measure,
        );

        let mut document = Document::empty(width, height);
        document.elements = elements;
        document.regions = regions;
        document
    }
}

impl SsiApp {
    /// 标签宽度按文字量出来；窗口窄了就先缩字号，实在放不下才等分。
    fn tab_rects(
        &self,
        width: f32,
        measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
    ) -> (Vec<Rect>, f32) {
        let count = self.page_count();
        let available = (width - MARGIN * 2.0).max(1.0);
        let titles: Vec<&str> = (0..count).map(|index| self.tab_title(index)).collect();
        let mut size = 10.0;
        let mut widths: Vec<f32> = Vec::new();
        for candidate in [13.0, 12.0, 11.0, 10.0] {
            let measured: Vec<f32> = titles
                .iter()
                .map(|title| measure(title, FontSlot::Candidate, candidate).0 + 16.0)
                .collect();
            let total: f32 =
                measured.iter().sum::<f32>() + TAB_GAP * (count.saturating_sub(1)) as f32;
            if total <= available {
                size = candidate;
                widths = measured;
                break;
            }
        }
        if widths.is_empty() {
            let each = ((available - TAB_GAP * (count - 1) as f32) / count as f32).max(1.0);
            widths = vec![each; count];
        }
        let total = widths.iter().sum::<f32>() + TAB_GAP * (count - 1) as f32;
        let mut rects = Vec::with_capacity(count);
        let mut x = MARGIN + (available - total).max(0.0) / 2.0;
        let top = chrome::HEIGHT + MARGIN;
        for width in widths {
            rects.push(Rect {
                x,
                y: top,
                width,
                height: TAB_HEIGHT,
            });
            x += width + TAB_GAP;
        }
        (rects, size)
    }

    fn paint_symbols(
        &mut self,
        elements: &mut Vec<Element>,
        regions: &mut Vec<Region>,
        content: Rect,
        measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
    ) {
        let theme = self.theme;
        ui::panel(elements, content, theme.surface, theme.radius);
        // 网格缩到面板内圈：按钮不贴面板边缘，圆角处不会看起来「溢出」。
        let area = Rect {
            x: content.x + GRID_PADDING,
            y: content.y + GRID_PADDING,
            width: (content.width - GRID_PADDING * 2.0).max(1.0),
            height: (content.height - GRID_PADDING * 2.0).max(1.0),
        };
        let grid = self.grid;
        let scale = (area.width / grid.width)
            .min(area.height / grid.height)
            .max(0.1);
        let width = grid.width * scale;
        let height = grid.height * scale;
        let origin = (
            area.x + (area.width - width) / 2.0,
            area.y + (area.height - height) / 2.0,
        );
        let page = &self.pages[self.page];
        for (index, glyph) in page.glyphs.iter().enumerate() {
            let rect = Rect {
                x: origin.0 + (glyph.x - grid.min_x) * scale,
                y: origin.1 + (glyph.y - grid.min_y) * scale,
                width: data::BUTTON * scale,
                height: data::BUTTON * scale,
            };
            // 日文页右侧三个「无 / 浊゛ / 半゜」是变体开关，不注入。
            if self.page == KANA_PAGE
                && let Some((name, variant)) = kana_control(&glyph.text)
            {
                let mut look = if self.kana == variant {
                    ui::ButtonLook::primary(&theme)
                } else {
                    ui::ButtonLook::normal(&theme)
                };
                look.size = (rect.width * 0.34).clamp(9.0, 14.0);
                ui::button(
                    elements,
                    regions,
                    rect,
                    name,
                    &glyph.text,
                    &look,
                    self.pointer,
                    measure,
                );
                continue;
            }
            // 假名页按当前变体模式换字；没有对应变体的假名在浊音 / 半浊音模式下隐藏（旧版做法）。
            let Some(shown) = self.display_text(&glyph.text) else {
                continue;
            };
            if self.pointer.is_some_and(|(x, y)| ui::inside(rect, x, y)) {
                self.hovered = Some(index);
            }
            // 制表符页的框线字符优先用宋体-PUA（旧版逐个按钮指定的字体）。
            let slot = if self.page == BOX_PAGE && is_box_drawing(&glyph.text) {
                FontSlot::ItemNumber
            } else {
                FontSlot::Candidate
            };
            ui::tile(
                elements,
                regions,
                rect,
                &format!("glyph:{index}"),
                &shown,
                slot,
                &theme,
                self.pointer,
                measure,
            );
            self.glyph_rects.push((rect, shown));
        }
    }

    fn paint_fancy(
        &mut self,
        elements: &mut Vec<Element>,
        regions: &mut Vec<Region>,
        content: Rect,
        measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
    ) {
        let theme = self.theme;
        ui::panel(elements, content, theme.surface, theme.radius);
        let inner = Rect {
            x: content.x + 12.0,
            y: content.y + 10.0,
            width: (content.width - 24.0).max(1.0),
            height: (content.height - 20.0).max(1.0),
        };

        let label_style = ui::muted(&theme, 12.0);
        ui::label(
            elements,
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 20.0,
            },
            "输入英文文本：",
            &label_style,
            measure,
        );
        let mut edit = theme.text_edit();
        edit.height = 34.0;
        edit.size = 14.0;
        edit.fill = Some(theme.background);
        edit.border = Some(theme.border);
        let input = Rect {
            x: inner.x,
            y: inner.y + 22.0,
            width: inner.width,
            height: 34.0,
        };
        ui::text_edit(elements, input, "ssi.input", &self.input, &edit);

        ui::label(
            elements,
            Rect {
                x: inner.x,
                y: input.y + 44.0,
                width: inner.width,
                height: 20.0,
            },
            "字母样式：",
            &label_style,
            measure,
        );
        let columns = 4;
        let gap = 6.0;
        let button_width = ((inner.width - gap * (columns - 1) as f32) / columns as f32).max(1.0);
        let style_top = input.y + 66.0;
        for (index, style) in self.styles.iter().enumerate() {
            let row = index / columns;
            let column = index % columns;
            let rect = Rect {
                x: inner.x + (button_width + gap) * column as f32,
                y: style_top + (34.0 + gap) * row as f32,
                width: button_width,
                height: 34.0,
            };
            let mut look = if index == self.style {
                ui::ButtonLook::primary(&theme)
            } else {
                ui::ButtonLook::normal(&theme)
            };
            look.size = 15.0;
            let label = fancy::label(style);
            ui::button(
                elements,
                regions,
                rect,
                &format!("style:{index}"),
                &label,
                &look,
                self.pointer,
                measure,
            );
        }
        let rows = self.styles.len().div_ceil(columns);
        let result_top = style_top + (34.0 + gap) * rows as f32 + 10.0;
        ui::label(
            elements,
            Rect {
                x: inner.x,
                y: result_top,
                width: inner.width,
                height: 20.0,
            },
            "结果：",
            &label_style,
            measure,
        );
        let copy_width = 96.0;
        let result = Rect {
            x: inner.x,
            y: result_top + 22.0,
            width: (inner.width - copy_width - gap).max(1.0),
            height: 34.0,
        };
        // 结果用自绘 label：RichEdit 只认配置的那一个字族，小号大写字母（ᴀʙᴄ）会变方块；
        // 自绘走 cosmic-text 的逐字回退，和上面的样式按钮同一套字体。结果只读，复制走右边按钮。
        ui::panel(elements, result, theme.border, 6.0);
        ui::panel(
            elements,
            Rect {
                x: result.x + 1.0,
                y: result.y + 1.0,
                width: (result.width - 2.0).max(1.0),
                height: (result.height - 2.0).max(1.0),
            },
            theme.background,
            5.0,
        );
        let mut result_style = theme.label();
        result_style.size = 14.0;
        let result_text = fit_text(
            &self.result,
            (result.width - 16.0).max(1.0),
            result_style.slot,
            result_style.size,
            measure,
        );
        ui::label(
            elements,
            Rect {
                x: result.x + 8.0,
                y: result.y,
                width: (result.width - 16.0).max(1.0),
                height: result.height,
            },
            &result_text,
            &result_style,
            measure,
        );
        ui::button(
            elements,
            regions,
            Rect {
                x: result.x + result.width + gap,
                y: result.y,
                width: copy_width,
                height: 34.0,
            },
            "copy",
            "复制结果",
            &ui::ButtonLook::primary(&theme),
            self.pointer,
            measure,
        );
    }

    fn status_text(&self) -> String {
        if let Some(message) = &self.message {
            return message.clone();
        }
        if self.page == FANCY {
            return "输入英文文本，选一种样式，再点「复制结果」".to_owned();
        }
        if let Some(index) = self.hovered
            && let Some(glyph) = self
                .pages
                .get(self.page)
                .and_then(|page| page.glyphs.get(index))
        {
            let display = self.output_text(&glyph.text);
            let character = display.chars().next().unwrap_or(' ');
            return format!(
                "{display} · U+{:04X}    左键输入 · 右键复制",
                character as u32
            );
        }
        if self.page == KANA_PAGE {
            let mode = match self.kana {
                Variant::Plain => "无",
                Variant::Voiced => "浊゛",
                Variant::SemiVoiced => "半゜",
            };
            return format!("假名变体：{mode}    点右侧 无 / 浊゛ / 半゜ 切换");
        }
        "左键输入 · 右键复制".to_owned()
    }
}

/// 日文页右侧的三个变体开关：区域名 + 模式。
fn kana_control(text: &str) -> Option<(&'static str, Variant)> {
    match text {
        "无" => Some(("kana:plain", Variant::Plain)),
        "浊゛" => Some(("kana:voiced", Variant::Voiced)),
        "半゜" => Some(("kana:semi", Variant::SemiVoiced)),
        _ => None,
    }
}

/// 制表符页的框线字符（U+2500..U+257F）。
fn is_box_drawing(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some(character), None) if ('\u{2500}'..='\u{257F}').contains(&character)
    )
}

/// 结果行放不下时截断加「…」；复制按钮仍然复制完整结果。
fn fit_text(
    text: &str,
    max_width: f32,
    slot: FontSlot,
    size: f32,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) -> String {
    if text.is_empty() || measure(text, slot, size).0 <= max_width {
        return text.to_owned();
    }
    let characters: Vec<char> = text.chars().collect();
    for count in (0..characters.len()).rev() {
        let candidate: String = characters[..count]
            .iter()
            .copied()
            .chain(std::iter::once('…'))
            .collect();
        if measure(&candidate, slot, size).0 <= max_width {
            return candidate;
        }
    }
    "…".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kana_controls_are_recognized() {
        assert_eq!(kana_control("无"), Some(("kana:plain", Variant::Plain)));
        assert_eq!(kana_control("浊゛"), Some(("kana:voiced", Variant::Voiced)));
        assert_eq!(
            kana_control("半゜"),
            Some(("kana:semi", Variant::SemiVoiced))
        );
        assert_eq!(kana_control("あ"), None);
    }

    #[test]
    fn box_drawing_characters_are_recognized() {
        assert!(is_box_drawing("┌"));
        assert!(is_box_drawing("╔"));
        assert!(is_box_drawing("┉"));
        assert!(!is_box_drawing("→"));
        assert!(!is_box_drawing("あ"));
        assert!(!is_box_drawing(""));
    }

    #[test]
    fn output_text_applies_the_kana_variant() {
        let mut app = SsiApp::new(cloud_ui::Theme::dark(), None);
        app.page = KANA_PAGE;
        assert_eq!(app.output_text("か"), "か");
        app.kana = Variant::Voiced;
        assert_eq!(app.output_text("か"), "が");
        app.kana = Variant::SemiVoiced;
        assert_eq!(app.output_text("は"), "ぱ");
    }

    #[test]
    fn display_text_shows_variants_and_hides_the_rest() {
        let mut app = SsiApp::new(cloud_ui::Theme::dark(), None);
        app.page = KANA_PAGE;
        assert_eq!(app.display_text("か").as_deref(), Some("か"));
        app.kana = Variant::Voiced;
        assert_eq!(app.display_text("か").as_deref(), Some("が"));
        assert_eq!(app.display_text("ん"), None);
        assert_eq!(app.display_text("ア"), None);
        app.kana = Variant::SemiVoiced;
        assert_eq!(app.display_text("は").as_deref(), Some("ぱ"));
        assert_eq!(app.display_text("か"), None);
    }

    #[test]
    fn fit_text_truncates_with_an_ellipsis() {
        // 每个字符按 10 点宽算。
        let mut measure =
            |text: &str, _slot: FontSlot, _size: f32| (text.chars().count() as f32 * 10.0, 16.0);
        assert_eq!(
            fit_text("abc", 100.0, FontSlot::Candidate, 14.0, &mut measure),
            "abc"
        );
        assert_eq!(
            fit_text("abcdef", 55.0, FontSlot::Candidate, 14.0, &mut measure),
            "abcd…"
        );
        assert_eq!(
            fit_text("ab", 5.0, FontSlot::Candidate, 14.0, &mut measure),
            "…"
        );
    }
}
