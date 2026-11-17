//! 窗口状态、布局与事件。

use std::collections::{HashMap, HashSet};

use cloud_ui::{AppEvent, Application, Context, LabelStyle, Rect, TextListState, Theme};
use cloudime_ids::{IdsDatabase, IdsMatch, IdsOp, source_label};
use cloudime_render::{Color, Document, Element, FontSlot};

use crate::canvas;
use crate::clipboard;
use crate::composer::{Composer, InsertOutcome};
use crate::config::Config;
use crate::data::Database;
use crate::drawer::{self, Drawer};
use crate::glyph::GlyphProbe;
use crate::results;
use crate::ui;

/// 窗口内边距。
const MARGIN: f32 = 20.0;
const GAP: f32 = 10.0;
const TOOLBAR_BUTTON_HEIGHT: f32 = 30.0;
const TOOLBAR_BUTTON_GAP: f32 = 6.0;
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
    drawer: Drawer,
    glyphs: GlyphProbe,
    component_ids: HashMap<char, String>,
}

impl IdsApp {
    pub fn new(database: Result<Database, String>, config: Config) -> Self {
        let component_ids = match &database {
            Ok(database) => build_component_ids(&database.inner),
            Err(_) => HashMap::new(),
        };
        Self {
            database,
            composer: Composer::new(),
            input: String::new(),
            matches: Vec::new(),
            total: None,
            status: Status::Idle,
            results: TextListState::default(),
            pointer: None,
            drawer: Drawer::with_typeable(config.show_typeable),
            glyphs: GlyphProbe::new(&config.font_family),
            component_ids,
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
        if self.drawer.click(name) {
            return;
        }
        if let Some(character) = self.drawer.component_at(name) {
            self.insert_component(character);
            return;
        }
        if let Some(path) = canvas::parse_slot(name) {
            self.composer.set_active(path);
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
            "fill" => self.fill_from_input(),
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

    fn fill_from_input(&mut self) {
        let Some(character) = self.input.trim().chars().next() else {
            self.status = Status::Error("先在输入框里打一个部件".to_owned());
            return;
        };
        match self.composer.insert(Some(character)) {
            InsertOutcome::NoSlot => {
                self.status = Status::Error("当前结构已经填满，先点一个空槽".to_owned());
            }
            _ => {
                self.drawer.push_recent(character);
                self.input.clear();
                self.status = Status::Idle;
            }
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
                AppEvent::RegionClicked(name) => self.click(name),
                AppEvent::RegionDoubleClicked(name) => self.double_click(name),
                AppEvent::TextChanged { name, text } if name == "ids.input" => {
                    self.input = text.clone();
                }
                AppEvent::TextKey { name, vk, .. } if name == "ids.input" && *vk == 0x0D => {
                    self.query();
                }
                _ => {}
            }
        }
    }

    fn view(&mut self, ctx: &Context) -> Document {
        let (width, height) = ctx.size();
        let theme = Theme::dark();
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
            0.0,
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
            ui::button(
                &mut elements,
                &mut regions,
                rect,
                &format!("op:{index}"),
                &format!("{} {}", op.symbol(), op.abbreviation()),
                &ui::ButtonLook::normal(&theme),
                self.pointer,
                &mut measure,
            );
        }
        ui::button(
            &mut elements,
            &mut regions,
            op_button(layout.toolbar, IdsOp::ALL.len()),
            "drawer:toggle",
            if self.drawer.open { "收起" } else { "部件" },
            &ui::ButtonLook::normal(&theme),
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
        let component_ids = &self.component_ids;
        let mut fallback = |character: char| -> Option<String> {
            if glyphs.has_glyph(character) {
                return None;
            }
            let ids = component_ids.get(&character)?;
            Some(crate::glyph::abbreviation(
                ids,
                2,
                component_ids,
                &mut |nested| glyphs.has_glyph(nested),
            ))
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
            expression_style.color = Color::rgb(214, 132, 120);
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
            (layout.fill, "fill", "填入"),
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
            status_style.color = Color::rgb(214, 132, 120);
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
        y: MARGIN,
        width: width - MARGIN * 2.0,
        height: height - MARGIN * 2.0,
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

fn expand(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x - by,
        y: rect.y - by,
        width: rect.width + by * 2.0,
        height: rect.height + by * 2.0,
    }
}

/// 为每个部件找一条 IDS（缺字形时画缩写用）。
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
        .min(64.0);
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
    fill: Rect,
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
        let content_width = (width - MARGIN * 2.0 - reserved_width).max(200.0);
        let toolbar_height = TOOLBAR_BUTTON_HEIGHT * 2.0 + TOOLBAR_BUTTON_GAP;
        let mut y = MARGIN;

        let toolbar = Rect {
            x: MARGIN,
            y,
            width: content_width,
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

        let buttons = SMALL_BUTTON_WIDTH * 4.0 + QUERY_BUTTON_WIDTH + 8.0 * 5.0;
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
        let fill = next(SMALL_BUTTON_WIDTH);
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
            y: MARGIN,
            width: drawer::WIDTH,
            height: (height - MARGIN * 2.0).max(1.0),
        });

        Self {
            toolbar,
            canvas,
            expression,
            input,
            fill,
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
                "点结构按钮开始；部件框可粘贴 IDS（如 LR木木），？ 表示任意部件".to_owned(),
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
    fn layout_keeps_everything_inside_the_window() {
        let layout = Layout::compute(980.0, 660.0, false);
        for rect in [
            layout.toolbar,
            layout.canvas,
            layout.expression,
            layout.input,
            layout.fill,
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
