//! 窗口状态、布局与事件。

mod chrome;

use std::collections::{HashMap, HashSet};

use cloud_ui::{
    AppEvent, Application, CLOSE_ACTION, Context, LabelStyle, MAXIMIZE_ACTION, MINIMIZE_ACTION,
    Rect, TextListState, Theme,
};
use cloudime_ids::{IdsDatabase, IdsMatch, IdsOp, source_label};
use cloudime_render::{Document, Element, FontSlot};

use crate::canvas::{self, FallbackGlyph};
use crate::clipboard;
use crate::composer::{Composer, InsertOutcome, Node};
use crate::config::{self, Config};
use crate::data::Database;
use crate::drawer::{self, Drawer};
use crate::glyph::GlyphProbe;
use crate::results;
use crate::ui;

/// 窗口内边距。
const MARGIN: f32 = 20.0;
const GAP: f32 = 10.0;
const TOOLBAR_BUTTON_HEIGHT: f32 = 30.0;
const TOOLBAR_BUTTON_GAP: f32 = 4.0;
const TOOLBAR_PER_ROW: usize = 9;
const INPUT_HEIGHT: f32 = 34.0;
const SMALL_BUTTON_WIDTH: f32 = 54.0;
const QUERY_BUTTON_WIDTH: f32 = 64.0;

pub struct IdsApp {
    database: Result<Database, String>,
    composer: Composer,
    input: String,
    matches: Vec<IdsMatch>,
    total: Option<usize>,
    status: Status,
    results: TextListState,
    pointer: Option<(f32, f32)>,
    focus_input: bool,
    drawer: Drawer,
    glyphs: GlyphProbe,
    component_ids: HashMap<char, String>,
    component_svgs: HashMap<char, String>,
    config: Config,
    theme: Theme,
}

impl IdsApp {
    pub fn new(database: Result<Database, String>, config: Config, theme: Theme) -> Self {
        let component_ids = match &database {
            Ok(database) => build_component_ids(&database.inner),
            Err(_) => HashMap::new(),
        };
        let component_svgs = match &database {
            Ok(database) => build_component_svgs(&database.inner),
            Err(_) => HashMap::new(),
        };
        let drawer = Drawer::with_config(&config);
        let glyphs = GlyphProbe::new(&config.font_family);
        let composer = Composer::with_structure(IdsOp::LeftRight);
        Self {
            database,
            composer,
            input: String::new(),
            matches: Vec::new(),
            total: None,
            status: Status::Idle,
            results: TextListState::default(),
            pointer: None,
            focus_input: false,
            drawer,
            glyphs,
            component_ids,
            component_svgs,
            config,
            theme,
        }
    }

    fn save_config(&mut self) {
        if let Err(error) = config::save(&self.config) {
            self.status = Status::Error(error);
        }
    }

    /// 点槽位时同步到输入框的文字：部件本身，或完整结构对应的字。
    fn slot_input_text(&self, path: &[usize]) -> Option<String> {
        match self.composer.node_at(path)? {
            Node::Char(character) => Some(character.to_string()),
            Node::Op { .. } => {
                let ids = self.composer.ids_at(path).ok()?;
                let database = match &self.database {
                    Ok(database) => &database.inner,
                    Err(_) => return None,
                };
                database
                    .lookup_exact(&ids)
                    .ok()?
                    .first()
                    .map(|item| item.character.to_string())
            }
            Node::Empty | Node::Wildcard => None,
        }
    }

    fn insert_component(&mut self, character: char) {
        match self.composer.insert(Some(character)) {
            InsertOutcome::NoSlot => {
                self.status = Status::Error("当前结构已经填满，先点一个空槽".to_owned());
            }
            _ => {
                self.drawer.push_recent(character);
                self.status = Status::Idle;
            }
        }
    }

    fn click(&mut self, name: &str) {
        if name == "drawer:toggle-typeable" {
            self.drawer.click(name);
            self.config.show_typeable = self.drawer.show_typeable;
            self.save_config();
            return;
        }
        if self.drawer.click(name) {
            return;
        }
        if let Some(character) = self.drawer.component_at(name) {
            self.insert_component(character);
            return;
        }
        if let Some(path) = canvas::parse_slot(name) {
            self.composer.set_active(path.clone());
            self.input = self.slot_input_text(&path).unwrap_or_default();
            self.focus_input = true;
            self.status = Status::Idle;
            return;
        }
        if let Some(rest) = name.strip_prefix("op:")
            && let Ok(index) = rest.parse::<usize>()
            && let Some(op) = IdsOp::ALL.get(index).copied()
        {
            self.composer.split(op);
            self.status = Status::Idle;
            return;
        }
        if let Some(index) = name
            .strip_prefix(results::RESULTS_NAME)
            .and_then(|rest| rest.strip_prefix('#'))
            .and_then(|index| index.parse::<usize>().ok())
        {
            self.results.selected = Some(index);
            return;
        }
        match name {
            "wildcard" => {
                self.composer.insert(None);
                self.status = Status::Idle;
            }
            "backspace" => {
                self.composer.backspace();
                self.status = Status::Idle;
            }
            "clear" => {
                self.composer.clear();
                self.matches.clear();
                self.total = None;
                self.status = Status::Idle;
            }
            "query" => self.query(),
            _ => {}
        }
    }

    fn query(&mut self) {
        let ids = match self.composer.to_ids() {
            Ok(ids) => ids,
            Err(error) => {
                self.status = Status::Incomplete(error.to_string());
                return;
            }
        };
        let database = match &self.database {
            Ok(database) => &database.inner,
            Err(message) => {
                self.status = Status::Error(message.clone());
                return;
            }
        };
        if ids.contains('？') {
            match database.lookup_wildcard(&ids, 200) {
                Ok(found) => {
                    self.total = Some(found.total);
                    self.set_matches(found.matches);
                }
                Err(error) => self.status = Status::Error(error.to_string()),
            }
        } else {
            match database.lookup_exact(&ids) {
                Ok(matches) => {
                    self.total = None;
                    self.set_matches(matches);
                }
                Err(error) => self.status = Status::Error(error.to_string()),
            }
        }
    }

    fn set_matches(&mut self, matches: Vec<IdsMatch>) {
        let total = self.total.unwrap_or(matches.len());
        self.matches = matches;
        self.results.selected = None;
        self.results.scroll.set(0.0);
        self.results.h_scroll.set(0.0);
        self.status = if self.matches.is_empty() {
            Status::NotFound
        } else {
            Status::Found {
                shown: self.matches.len(),
                total,
            }
        };
    }

    fn double_click(&mut self, name: &str) {
        let Some(index) = name
            .strip_prefix(results::RESULTS_NAME)
            .and_then(|rest| rest.strip_prefix('#'))
            .and_then(|index| index.parse::<usize>().ok())
        else {
            return;
        };
        let Some(item) = self.matches.get(index) else {
            return;
        };
        let character = item.character;
        match clipboard::set_text(&character.to_string()) {
            Ok(()) => self.status = Status::Copied(character),
            Err(error) => self.status = Status::Error(error),
        }
    }

    fn result_rows(&self) -> Vec<String> {
        self.matches
            .iter()
            .map(|item| {
                let sources = source_label(item.sources);
                if sources.is_empty() {
                    format!(
                        "{}   U+{:04X}   {}",
                        item.character, item.character as u32, item.ids
                    )
                } else {
                    format!(
                        "{}   U+{:04X}   {}   {}",
                        item.character, item.character as u32, item.ids, sources
                    )
                }
            })
            .collect()
    }
}

impl Application for IdsApp {
    fn update(&mut self, ctx: &mut Context) {
        for event in ctx.events() {
            match event {
                AppEvent::MouseMoved { x, y } => self.pointer = Some((*x, *y)),
                AppEvent::MouseLeft => self.pointer = None,
                AppEvent::RegionClicked(name) if name == MINIMIZE_ACTION => ctx.minimize(),
                AppEvent::RegionClicked(name) if name == MAXIMIZE_ACTION => ctx.toggle_maximize(),
                AppEvent::RegionClicked(name) if name == CLOSE_ACTION => ctx.close(),
                AppEvent::RegionClicked(name) => self.click(name),
                AppEvent::RegionDoubleClicked(name) => self.double_click(name),
                AppEvent::TextChanged { name, text } if name == "ids.input" => {
                    if let Some(character) = single_han(text) {
                        let changed = !self.input.starts_with(character);
                        self.input = character.to_string();
                        if changed {
                            self.insert_component(character);
                        }
                    } else {
                        self.input.clear();
                    }
                }
                AppEvent::TextChanged { name, text } if name == "ids.drawer.search" => {
                    self.drawer.set_search(text);
                }
                AppEvent::TextKey { name, vk, .. } if name == "ids.input" && *vk == 0x0D => {
                    self.query();
                }
                _ => {}
            }
        }
        if self.focus_input {
            ctx.focus_text("ids.input");
            self.focus_input = false;
        }
    }

    fn view(&mut self, ctx: &Context) -> Document {
        let (width, height) = ctx.size();
        let theme = self.theme;
        let layout = Layout::compute(width, height, self.drawer.open);
        let mut elements: Vec<Element> = Vec::new();
        let mut regions = Vec::new();
        let mut scroll_targets = Vec::new();
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
            "组字工具",
            &theme,
            self.pointer,
            &mut measure,
        );

        let database = match &self.database {
            Ok(database) => &database.inner,
            Err(message) => {
                draw_error(&mut elements, &theme, width, height, message, &mut measure);
                return finish(width, height, elements, regions, scroll_targets);
            }
        };

        // 结构按钮行。
        ui::panel(
            &mut elements,
            expand(layout.toolbar, 10.0),
            theme.surface,
            theme.radius,
        );
        for (index, op) in IdsOp::ALL.iter().enumerate() {
            let rect = op_button(layout.toolbar, index);
            let mut look = ui::ButtonLook::normal(&theme);
            look.size = 13.0;
            ui::button(
                &mut elements,
                &mut regions,
                rect,
                &format!("op:{index}"),
                &format!("{} {}", op.symbol(), op.label()),
                &look,
                self.pointer,
                &mut measure,
            );
        }
        let mut toggle_look = ui::ButtonLook::normal(&theme);
        toggle_look.size = 13.0;
        ui::button(
            &mut elements,
            &mut regions,
            op_button(layout.toolbar, IdsOp::ALL.len()),
            "drawer:toggle",
            if self.drawer.open { "收起" } else { "部件" },
            &toggle_look,
            self.pointer,
            &mut measure,
        );

        // 画布。
        ui::panel(
            &mut elements,
            expand(layout.canvas, 8.0),
            theme.surface,
            theme.radius,
        );
        let glyphs = &mut self.glyphs;
        let component_svgs = &self.component_svgs;
        let mut fallback = |character: char| -> Option<FallbackGlyph> {
            if glyphs.has_glyph(character) {
                return None;
            }
            if let Some(svg) = component_svgs.get(&character) {
                return Some(FallbackGlyph::Svg(svg.clone()));
            }
            Some(FallbackGlyph::Text(format!("U+{:04X}", character as u32)))
        };
        canvas::paint(
            &mut elements,
            &mut regions,
            &self.composer,
            layout.canvas,
            &theme,
            self.pointer,
            &mut fallback,
            &mut measure,
        );

        // 表达式行。
        let (expression, incomplete) = ui::expression_text(&self.composer);
        let mut expression_style = theme.muted();
        expression_style.size = 13.0;
        if incomplete {
            expression_style.color = crate::theme::danger(&theme);
        }
        ui::label(
            &mut elements,
            layout.expression,
            &expression,
            &expression_style,
            &mut measure,
        );

        // 部件输入 + 操作按钮。
        let mut edit_style = theme.text_edit();
        edit_style.height = layout.input.height;
        ui::text_edit(
            &mut elements,
            layout.input,
            "ids.input",
            &self.input,
            &edit_style,
        );
        for (rect, name, text) in [
            (layout.wildcard, "wildcard", "？"),
            (layout.backspace, "backspace", "退格"),
            (layout.clear, "clear", "清空"),
        ] {
            ui::button(
                &mut elements,
                &mut regions,
                rect,
                name,
                text,
                &ui::ButtonLook::normal(&theme),
                self.pointer,
                &mut measure,
            );
        }
        ui::button(
            &mut elements,
            &mut regions,
            layout.query,
            "query",
            "查询",
            &ui::ButtonLook::primary(&theme),
            self.pointer,
            &mut measure,
        );

        // 状态行。
        let (message, is_error) = self.status.text();
        let mut status_style = ui::muted_style(&theme);
        if is_error {
            status_style.color = crate::theme::danger(&theme);
        }
        ui::label(
            &mut elements,
            layout.status,
            &message,
            &status_style,
            &mut measure,
        );

        // 结果列表。
        let rows = self.result_rows();
        let title = match (rows.len(), self.total) {
            (0, _) => "结果".to_owned(),
            (shown, Some(total)) if total > shown => {
                format!("结果（共 {total} 条，显示前 {shown} 条）")
            }
            (shown, _) => format!("结果（{shown} 条）"),
        };
        ui::label(
            &mut elements,
            layout.results_label,
            &title,
            &ui::muted_style(&theme),
            &mut measure,
        );
        results::build(
            &mut elements,
            &mut regions,
            &mut scroll_targets,
            layout.results,
            &rows,
            &self.results,
            &theme,
            &mut measure,
        );

        if let Some(rect) = layout.drawer {
            self.drawer.paint(
                &mut elements,
                &mut regions,
                &mut scroll_targets,
                rect,
                database,
                &theme,
                self.pointer,
                &mut measure,
                &self.component_ids,
                &self.component_svgs,
                &mut self.glyphs,
            );
        }

        finish(width, height, elements, regions, scroll_targets)
    }
}

fn finish(
    width: f32,
    height: f32,
    elements: Vec<Element>,
    regions: Vec<cloudime_render::Region>,
    scroll_targets: Vec<cloudime_render::ScrollTarget>,
) -> Document {
    let mut document = Document::empty(width, height);
    document.elements = elements;
    document.regions = regions;
    document.scroll_targets = scroll_targets;
    document
}

fn draw_error(
    elements: &mut Vec<Element>,
    theme: &Theme,
    width: f32,
    height: f32,
    message: &str,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) {
    let rect = Rect {
        x: MARGIN,
        y: chrome::HEIGHT + MARGIN,
        width: width - MARGIN * 2.0,
        height: height - chrome::HEIGHT - MARGIN * 2.0,
    };
    ui::panel(elements, rect, theme.surface, theme.radius);
    let style = LabelStyle {
        size: 16.0,
        color: theme.text,
        ..LabelStyle::default()
    };
    let mut y = rect.y + 24.0;
    for line in message.lines() {
        ui::label(
            elements,
            Rect {
                x: rect.x + 20.0,
                y,
                width: rect.width - 40.0,
                height: 24.0,
            },
            line,
            &style,
            measure,
        );
        y += 26.0;
    }
}

fn single_han(text: &str) -> Option<char> {
    text.chars().rev().find(|character| is_han(*character))
}

fn is_han(character: char) -> bool {
    matches!(
        u32::from(character),
        0x2E80..=0x2FFF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF
    )
}

fn expand(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x - by,
        y: rect.y - by,
        width: rect.width + by * 2.0,
        height: rect.height + by * 2.0,
    }
}

/// 为每个部件取内置 SVG。
fn build_component_svgs(database: &IdsDatabase) -> HashMap<char, String> {
    database
        .components()
        .iter()
        .filter_map(|component| {
            let character = component.character()?;
            let svg = database.component_svg(component)?;
            Some((character, svg.to_owned()))
        })
        .collect()
}

/// 为每个部件找一条 IDS（搜索匹配用）。
fn build_component_ids(database: &IdsDatabase) -> HashMap<char, String> {
    let wanted: HashSet<char> = database
        .components()
        .iter()
        .filter_map(|component| component.character())
        .collect();
    let mut ids_by_character = HashMap::new();
    if wanted.is_empty() {
        return ids_by_character;
    }
    for id in 0..database.len() {
        let Some(ids) = database.key_str(id as u32) else {
            continue;
        };
        let Some(entries) = database.entries_at(id as u32) else {
            continue;
        };
        for entry in entries {
            if let Some(character) = char::from_u32(entry.codepoint)
                && wanted.contains(&character)
                && !ids_by_character.contains_key(&character)
            {
                ids_by_character.insert(character, ids.to_owned());
            }
        }
        if ids_by_character.len() == wanted.len() {
            break;
        }
    }
    ids_by_character
}

fn op_button(toolbar: Rect, index: usize) -> Rect {
    let width = ((toolbar.width - TOOLBAR_BUTTON_GAP * (TOOLBAR_PER_ROW as f32 - 1.0))
        / TOOLBAR_PER_ROW as f32)
        .min(100.0);
    let row = index / TOOLBAR_PER_ROW;
    let column = index % TOOLBAR_PER_ROW;
    Rect {
        x: toolbar.x + column as f32 * (width + TOOLBAR_BUTTON_GAP),
        y: toolbar.y + row as f32 * (TOOLBAR_BUTTON_HEIGHT + TOOLBAR_BUTTON_GAP),
        width,
        height: TOOLBAR_BUTTON_HEIGHT,
    }
}

struct Layout {
    toolbar: Rect,
    canvas: Rect,
    expression: Rect,
    input: Rect,
    wildcard: Rect,
    backspace: Rect,
    clear: Rect,
    query: Rect,
    status: Rect,
    results_label: Rect,
    results: Rect,
    drawer: Option<Rect>,
}

impl Layout {
    fn compute(width: f32, height: f32, drawer_open: bool) -> Self {
        let reserved_width = if drawer_open {
            drawer::WIDTH + GAP
        } else {
            0.0
        };
        let full_width = (width - MARGIN * 2.0).max(200.0);
        let content_width = (full_width - reserved_width).max(200.0);
        let toolbar_height = TOOLBAR_BUTTON_HEIGHT * 2.0 + TOOLBAR_BUTTON_GAP;
        let mut y = chrome::HEIGHT + MARGIN;

        let toolbar = Rect {
            x: MARGIN,
            y,
            width: full_width,
            height: toolbar_height,
        };
        y += toolbar_height + GAP;

        let reserved = 20.0 + 8.0 + INPUT_HEIGHT + 10.0 + 20.0 + 8.0 + 20.0 + 6.0 + 120.0;
        let side = (height - y - MARGIN - reserved)
            .clamp(140.0, 440.0)
            .min(content_width);
        let canvas = Rect {
            x: MARGIN + (content_width - side) / 2.0,
            y,
            width: side,
            height: side,
        };
        y += side + GAP;

        let expression = Rect {
            x: MARGIN,
            y,
            width: content_width,
            height: 20.0,
        };
        y += 20.0 + 8.0;

        let buttons = SMALL_BUTTON_WIDTH * 3.0 + QUERY_BUTTON_WIDTH + 8.0 * 4.0;
        let input = Rect {
            x: MARGIN,
            y,
            width: (content_width - buttons).max(120.0),
            height: INPUT_HEIGHT,
        };
        let mut x = input.x + input.width + 8.0;
        let mut next = |width: f32| {
            let rect = Rect {
                x,
                y,
                width,
                height: INPUT_HEIGHT,
            };
            x += width + 8.0;
            rect
        };
        let wildcard = next(SMALL_BUTTON_WIDTH);
        let backspace = next(SMALL_BUTTON_WIDTH);
        let clear = next(SMALL_BUTTON_WIDTH);
        let query = next(QUERY_BUTTON_WIDTH);
        y += INPUT_HEIGHT + 10.0;

        let status = Rect {
            x: MARGIN,
            y,
            width: content_width,
            height: 20.0,
        };
        y += 20.0 + 8.0;

        let results_label = Rect {
            x: MARGIN,
            y,
            width: content_width,
            height: 20.0,
        };
        y += 20.0 + 6.0;

        let results = Rect {
            x: MARGIN,
            y,
            width: content_width,
            height: (height - y - MARGIN).max(1.0),
        };

        let drawer = drawer_open.then(|| Rect {
            x: width - MARGIN - drawer::WIDTH,
            y: canvas.y - 8.0,
            width: drawer::WIDTH,
            height: (height - MARGIN - (canvas.y - 8.0)).max(1.0),
        });

        Self {
            toolbar,
            canvas,
            expression,
            input,
            wildcard,
            backspace,
            clear,
            query,
            status,
            results_label,
            results,
            drawer,
        }
    }
}

/// 状态行。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Idle,
    Incomplete(String),
    NotFound,
    Found { shown: usize, total: usize },
    Copied(char),
    Error(String),
}

impl Status {
    fn text(&self) -> (String, bool) {
        match self {
            Self::Idle => (
                "输入一个汉字部件，自动填入当前槽；？ 表示任意部件".to_owned(),
                false,
            ),
            Self::Incomplete(message) => (message.clone(), true),
            Self::NotFound => ("没有找到：Unicode IDS 未收录这个结构".to_owned(), true),
            Self::Found { shown, total } if total > shown => {
                (format!("共 {total} 条，显示前 {shown} 条"), false)
            }
            Self::Found { shown, .. } => (format!("找到 {shown} 条；双击结果复制"), false),
            Self::Copied(character) => (format!("已复制：{character}"), false),
            Self::Error(message) => (message.clone(), true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_han_keeps_the_last_chinese_character() {
        assert_eq!(single_han("木"), Some('木'));
        assert_eq!(single_han("a木b水"), Some('水'));
        assert_eq!(single_han("木木"), Some('木'));
        assert_eq!(single_han("abc"), None);
    }

    #[test]
    fn layout_keeps_everything_inside_the_window() {
        let layout = Layout::compute(980.0, 660.0, false);
        for rect in [
            layout.toolbar,
            layout.canvas,
            layout.expression,
            layout.input,
            layout.wildcard,
            layout.backspace,
            layout.clear,
            layout.query,
            layout.status,
            layout.results_label,
            layout.results,
        ] {
            assert!(rect.x >= 0.0);
            assert!(rect.y >= 0.0);
            assert!(rect.width > 0.0);
            assert!(rect.height > 0.0);
        }
    }

    #[test]
    fn small_windows_still_have_a_canvas() {
        let layout = Layout::compute(760.0, 520.0, false);
        assert!(layout.canvas.width >= 140.0);
        assert!(layout.results.height > 0.0);
    }

    #[test]
    fn drawer_shrinks_the_content_and_stays_inside() {
        let layout = Layout::compute(980.0, 660.0, true);
        let drawer = layout.drawer.unwrap();
        assert!(drawer.x > layout.results.x + layout.results.width);
        assert!(drawer.x + drawer.width <= 980.0);
        assert!(layout.results.width < 800.0);
    }

    #[test]
    fn operator_buttons_stay_inside_the_toolbar() {
        let layout = Layout::compute(980.0, 660.0, false);
        let last = op_button(layout.toolbar, IdsOp::ALL.len());
        assert!(last.x + last.width <= layout.toolbar.x + layout.toolbar.width + 0.01);
        assert!(last.y + last.height <= layout.toolbar.y + layout.toolbar.height + 0.01);
    }
}
