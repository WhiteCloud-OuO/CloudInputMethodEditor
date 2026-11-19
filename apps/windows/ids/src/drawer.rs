//! 难输入部件抽屉：按笔画 / 分类 / 最近浏览，点击填入当前槽。

use std::collections::HashMap;

use cloud_ui::{Rect, TextEditStyle, Theme};
use cloudime_ids::{Component, ComponentRole, IdsDatabase, IdsNode, parse};
use cloudime_render::{
    Element, FontSlot, HitShape, Region, ScrollAxis, ScrollHandle, ScrollTarget, SelfDrawingPanel,
};

use crate::config::Config;
use crate::glyph::GlyphProbe;
use crate::ui;

/// 抽屉宽度（点）。
pub const WIDTH: f32 = 300.0;

const TILE: f32 = 56.0;
const GAP: f32 = 6.0;
const COLUMNS: usize = 4;
const HEADER_HEIGHT: f32 = 24.0;
const TABS_HEIGHT: f32 = 28.0;
const SEARCH_HEIGHT: f32 = 28.0;
const CHIP_HEIGHT: f32 = 24.0;
const CHIP_GAP: f32 = 4.0;
const PREVIEW_HEIGHT: f32 = 18.0;
const FOOTER_HEIGHT: f32 = 22.0;
/// 分类页只列前 40 个，避免几十页。
const ROLE_LIMIT: usize = 40;
/// 分类页收录阈值（千分比）。
const ROLE_MINIMUM: u16 = 100;

/// 抽屉的标签页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Stroke,
    Role,
    Recent,
}

impl Tab {
    const ALL: [Self; 3] = [Self::Stroke, Self::Role, Self::Recent];

    fn index(self) -> usize {
        match self {
            Self::Stroke => 0,
            Self::Role => 1,
            Self::Recent => 2,
        }
    }

    fn from_index(index: usize) -> Option<Self> {
        Some(match index {
            0 => Self::Stroke,
            1 => Self::Role,
            2 => Self::Recent,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Stroke => "笔画",
            Self::Role => "分类",
            Self::Recent => "最近",
        }
    }
}

/// 抽屉状态。
#[derive(Debug, Clone)]
pub struct Drawer {
    pub open: bool,
    pub tab: Tab,
    pub stroke: u8,
    pub role: ComponentRole,
    pub show_typeable: bool,
    pub search: String,
    pub recent: Vec<char>,
    pub scroll: ScrollHandle,

    /// 当前页的部件（`paint` 时刷新），点击 `comp:<tab>:<i>` 用它反查。
    items: Vec<char>,
    /// 指针正压在哪个 tile 上（只影响高亮）。
    hovered: Option<char>,
    /// 预览行针对的部件（离开 tile 后保留）。
    preview: Option<char>,
}

impl Default for Drawer {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Stroke,
            stroke: 3,
            role: ComponentRole::Side,
            show_typeable: false,
            search: String::new(),
            recent: Vec::new(),
            scroll: ScrollHandle::new(0.0),
            items: Vec::new(),
            hovered: None,
            preview: None,
        }
    }
}

impl Drawer {
    /// 按用户配置初始化。
    pub fn with_config(config: &Config) -> Self {
        Self {
            show_typeable: config.show_typeable,
            ..Self::default()
        }
    }

    /// 记一个刚插入的部件（去重、最多 12 个）。
    pub fn push_recent(&mut self, character: char) {
        self.recent.retain(|item| *item != character);
        self.recent.insert(0, character);
        self.recent.truncate(12);
        self.preview = Some(character);
    }

    /// 修改搜索词；调用方负责清空滚动。
    pub fn set_search(&mut self, text: &str) {
        self.search = text.to_owned();
        self.reset_scroll();
    }

    /// 处理抽屉自己的区域名；不是抽屉的返回 `false`。
    pub fn click(&mut self, name: &str) -> bool {
        if name == "drawer:toggle" {
            self.open = !self.open;
            return true;
        }
        if name == "drawer:toggle-typeable" {
            self.show_typeable = !self.show_typeable;
            self.reset_scroll();
            return true;
        }
        if let Some(index) = name
            .strip_prefix("drawer:tab:")
            .and_then(|index| index.parse::<usize>().ok())
            && let Some(tab) = Tab::from_index(index)
        {
            self.tab = tab;
            self.preview = None;
            self.reset_scroll();
            return true;
        }
        if let Some(value) = name
            .strip_prefix("drawer:stroke:")
            .and_then(|value| value.parse::<u8>().ok())
        {
            self.stroke = value;
            self.preview = None;
            self.reset_scroll();
            return true;
        }
        if let Some(index) = name
            .strip_prefix("drawer:role:")
            .and_then(|index| index.parse::<usize>().ok())
            && let Some(role) = ComponentRole::ALL.get(index).copied()
        {
            self.role = role;
            self.preview = None;
            self.reset_scroll();
            return true;
        }
        false
    }

    /// `comp:<tab>:<index>` → 部件字符。
    pub fn component_at(&self, name: &str) -> Option<char> {
        let rest = name.strip_prefix("comp:")?;
        let (tab, index) = rest.split_once(':')?;
        if tab.parse::<usize>().ok()? != self.tab.index() {
            return None;
        }
        let index = index.parse::<usize>().ok()?;
        self.items.get(index).copied()
    }

    fn reset_scroll(&mut self) {
        self.scroll.set(0.0);
    }

    /// 当前页要显示的部件。
    fn items(&self, database: &IdsDatabase, component_ids: &HashMap<char, String>) -> Vec<char> {
        let mut items = Vec::new();
        match self.tab {
            Tab::Stroke => {
                for component in database.components_with_stroke(self.stroke) {
                    if self.visible(component) {
                        push_character(&mut items, component);
                    }
                }
            }
            Tab::Role => {
                for component in database
                    .components_in_role(self.role, ROLE_MINIMUM)
                    .into_iter()
                    .take(ROLE_LIMIT)
                {
                    if self.visible(component) {
                        push_character(&mut items, component);
                    }
                }
            }
            Tab::Recent => {
                for character in &self.recent {
                    if database.component(*character).is_some() {
                        items.push(*character);
                    }
                }
            }
        }
        if !self.search.trim().is_empty() {
            let query = self.search.trim().to_uppercase();
            items.retain(|character| {
                matches_search(*character, component_ids.get(character), &query)
            });
        }
        items
    }

    fn visible(&self, component: &Component) -> bool {
        self.show_typeable || !component.is_typeable()
    }

    /// 画抽屉；没有 `COMP` 时只画一句提示。
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        elements: &mut Vec<Element>,
        regions: &mut Vec<Region>,
        scroll_targets: &mut Vec<ScrollTarget>,
        rect: Rect,
        database: &IdsDatabase,
        theme: &Theme,
        pointer: Option<(f32, f32)>,
        measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
        component_ids: &HashMap<char, String>,
        component_svgs: &HashMap<char, String>,
        glyphs: &mut GlyphProbe,
    ) {
        ui::panel(elements, rect, theme.surface, theme.radius);
        let inner_x = rect.x + 12.0;
        let inner_width = (rect.width - 24.0).max(1.0);

        let mut style = theme.label();
        style.size = 14.0;
        ui::label(
            elements,
            Rect {
                x: inner_x,
                y: rect.y + 8.0,
                width: inner_width,
                height: HEADER_HEIGHT,
            },
            "难输入部件",
            &style,
            measure,
        );

        let mut y = rect.y + 8.0 + HEADER_HEIGHT + 6.0;
        let tab_width = (inner_width - CHIP_GAP * 2.0) / 3.0;
        for (index, tab) in Tab::ALL.into_iter().enumerate() {
            let chip = Rect {
                x: inner_x + (tab_width + CHIP_GAP) * index as f32,
                y,
                width: tab_width,
                height: TABS_HEIGHT,
            };
            let look = if self.tab == tab {
                ui::ButtonLook::primary(theme)
            } else {
                ui::ButtonLook::normal(theme)
            };
            ui::button(
                elements,
                regions,
                chip,
                &format!("drawer:tab:{}", tab.index()),
                tab.name(),
                &look,
                pointer,
                measure,
            );
        }
        y += TABS_HEIGHT + 6.0;

        let search_rect = Rect {
            x: inner_x,
            y,
            width: inner_width,
            height: SEARCH_HEIGHT,
        };
        let edit_style = TextEditStyle {
            height: SEARCH_HEIGHT,
            radius: 6.0,
            size: 12.0,
            fill: Some(theme.background),
            border: Some(theme.border),
            ..TextEditStyle::default()
        };
        ui::text_edit(
            elements,
            search_rect,
            "ids.drawer.search",
            &self.search,
            &edit_style,
        );
        y += SEARCH_HEIGHT + 6.0;

        if !database.has_components() {
            let mut hint = theme.muted();
            hint.size = 12.0;
            ui::label(
                elements,
                Rect {
                    x: inner_x,
                    y,
                    width: inner_width,
                    height: 40.0,
                },
                "部件库缺失：重新生成 ids.qj 时带上 components.tsv",
                &hint,
                measure,
            );
            return;
        }

        match self.tab {
            Tab::Stroke => {
                y = self.stroke_chips(
                    elements,
                    regions,
                    inner_x,
                    inner_width,
                    y,
                    theme,
                    pointer,
                    measure,
                );
            }
            Tab::Role => {
                y = self.role_chips(
                    elements,
                    regions,
                    inner_x,
                    inner_width,
                    y,
                    database,
                    theme,
                    pointer,
                    measure,
                );
            }
            Tab::Recent => {}
        }

        let content_bottom = rect.y + rect.height - PREVIEW_HEIGHT - FOOTER_HEIGHT - 8.0;
        let grid = Rect {
            x: inner_x,
            y,
            width: inner_width,
            height: (content_bottom - y).max(1.0),
        };
        self.items = self.items(database, component_ids);

        let stride = TILE + GAP;
        let rows = self.items.len().div_ceil(COLUMNS);
        let content_height = (rows as f32 * stride - GAP).max(0.0);
        let max = (content_height - grid.height).max(0.0);
        let offset = self.scroll.get().clamp(0.0, max);
        self.scroll.set(offset);

        self.hovered = None;
        let first_row = (offset / stride).floor() as usize;
        let visible_rows = (grid.height / stride).ceil() as usize + 1;
        // 瓦片画进一块裁剪面板：滚到一半的行只显示落在网格里的部分，不会压到
        // 上方的分类 chip 或下方的预览行 / 底部按钮（面板坐标相对网格左上角）。
        let mut tiles: Vec<Element> = Vec::new();
        for row in first_row..(first_row + visible_rows).min(rows) {
            for column in 0..COLUMNS {
                let index = row * COLUMNS + column;
                let Some(&character) = self.items.get(index) else {
                    break;
                };
                let local = Rect {
                    x: (TILE + GAP) * column as f32,
                    y: (TILE + GAP) * row as f32 - offset,
                    width: TILE,
                    height: TILE,
                };
                let tile = Rect {
                    x: grid.x + local.x,
                    y: grid.y + local.y,
                    width: TILE,
                    height: TILE,
                };
                if tile.y + tile.height < grid.y || tile.y > grid.y + grid.height {
                    continue;
                }
                let hovered = pointer.is_some_and(|(x, y)| ui::inside(tile, x, y));
                if hovered {
                    self.hovered = Some(character);
                    self.preview = Some(character);
                }
                ui::panel(
                    &mut tiles,
                    local,
                    if hovered {
                        theme.surface_alt
                    } else {
                        theme.background
                    },
                    8.0,
                );
                if hovered {
                    tiles.push(Element::Rect {
                        x: local.x,
                        y: local.y,
                        width: local.width,
                        height: local.height,
                        radius: 8.0,
                        color: theme.accent,
                    });
                    ui::panel(
                        &mut tiles,
                        Rect {
                            x: local.x + 1.0,
                            y: local.y + 1.0,
                            width: local.width - 2.0,
                            height: local.height - 2.0,
                        },
                        theme.surface_alt,
                        7.0,
                    );
                }
                paint_glyph(
                    &mut tiles,
                    local,
                    character,
                    theme,
                    measure,
                    component_svgs,
                    glyphs,
                );
                // 命中区裁到网格里：滚到一半的瓦片不能把点击带到预览行 / 底部按钮上。
                if let Some(visible) = intersect(tile, grid) {
                    regions.push(Region::new(
                        format!("comp:{}:{index}", self.tab.index()),
                        HitShape::RoundRect {
                            x: visible.x,
                            y: visible.y,
                            width: visible.width,
                            height: visible.height,
                            radius: 8.0,
                        },
                        false,
                    ));
                }
            }
        }
        elements.push(Element::SelfDrawingPanel(SelfDrawingPanel {
            x: grid.x,
            y: grid.y,
            width: grid.width,
            height: grid.height,
            clip: true,
            draw: tiles,
            children: Vec::new(),
            regions: Vec::new(),
        }));

        if max > 0.0 {
            let track_height = grid.height;
            let thumb_height = (track_height * grid.height / content_height).max(20.0);
            let thumb_y = grid.y + (track_height - thumb_height) * (offset / max);
            elements.push(Element::Rect {
                x: rect.x + rect.width - 7.0,
                y: thumb_y,
                width: 3.0,
                height: thumb_height,
                radius: 1.5,
                color: theme.border,
            });
            // 命中区比画出来的宽，方便拖动。
            scroll_targets.push(ScrollTarget {
                name: "ids.drawer".to_owned(),
                axis: ScrollAxis::Vertical,
                area: (grid.x, grid.y, grid.width, grid.height),
                track: (rect.x + rect.width - 12.0, grid.y, 12.0, grid.height),
                thumb: (rect.x + rect.width - 10.0, thumb_y, 9.0, thumb_height),
                max,
                step: stride,
                handle: self.scroll.clone(),
            });
        }

        let preview_rect = Rect {
            x: inner_x,
            y: content_bottom + 2.0,
            width: inner_width,
            height: PREVIEW_HEIGHT,
        };
        let mut preview = theme.muted();
        preview.size = 12.0;
        let text = self
            .preview
            .and_then(|character| database.component(character).map(|item| (character, item)))
            .map_or_else(
                || "悬停查看部件；点击填入当前槽".to_owned(),
                |(character, item)| {
                    format!(
                        "{character} · U+{:04X} · {}画 · {} {}‰",
                        character as u32,
                        if item.stroke == 0 {
                            "?".to_owned()
                        } else if item.stroke == 13 {
                            "13+".to_owned()
                        } else {
                            item.stroke.to_string()
                        },
                        item.primary_role().name(),
                        item.role_score(item.primary_role()),
                    )
                },
            );
        ui::label(elements, preview_rect, &text, &preview, measure);

        let footer = Rect {
            x: inner_x,
            y: rect.y + rect.height - FOOTER_HEIGHT - 4.0,
            width: inner_width,
            height: FOOTER_HEIGHT,
        };
        let label = if self.show_typeable {
            "隐藏可打出部件"
        } else {
            "显示可打出部件"
        };
        let look = if self.show_typeable {
            ui::ButtonLook::primary(theme)
        } else {
            ui::ButtonLook::normal(theme)
        };
        ui::button(
            elements,
            regions,
            footer,
            "drawer:toggle-typeable",
            label,
            &look,
            pointer,
            measure,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn stroke_chips(
        &self,
        elements: &mut Vec<Element>,
        regions: &mut Vec<Region>,
        x: f32,
        width: f32,
        y: f32,
        theme: &Theme,
        pointer: Option<(f32, f32)>,
        measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
    ) -> f32 {
        let strokes = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 0];
        let chip_width = (width - CHIP_GAP * 9.0) / 10.0;
        for (index, stroke) in strokes.into_iter().enumerate() {
            let row = index / 10;
            let column = index % 10;
            let chip = Rect {
                x: x + (chip_width + CHIP_GAP) * column as f32,
                y: y + (CHIP_HEIGHT + CHIP_GAP) * row as f32,
                width: chip_width,
                height: CHIP_HEIGHT,
            };
            let label = match stroke {
                13 => "13+".to_owned(),
                0 => "未".to_owned(),
                value => value.to_string(),
            };
            let look = if self.stroke == stroke {
                ui::ButtonLook::primary(theme)
            } else {
                ui::ButtonLook::normal(theme)
            };
            ui::button(
                elements,
                regions,
                chip,
                &format!("drawer:stroke:{stroke}"),
                &label,
                &look,
                pointer,
                measure,
            );
        }
        y + CHIP_HEIGHT * 2.0 + CHIP_GAP + 8.0
    }

    #[allow(clippy::too_many_arguments)]
    fn role_chips(
        &self,
        elements: &mut Vec<Element>,
        regions: &mut Vec<Region>,
        x: f32,
        width: f32,
        y: f32,
        database: &IdsDatabase,
        theme: &Theme,
        pointer: Option<(f32, f32)>,
        measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
    ) -> f32 {
        let chip_width = (width - CHIP_GAP * 4.0) / 5.0;
        for (index, role) in ComponentRole::ALL.into_iter().enumerate() {
            let count = database.components_in_role(role, ROLE_MINIMUM).len();
            let chip = Rect {
                x: x + (chip_width + CHIP_GAP) * index as f32,
                y,
                width: chip_width,
                height: CHIP_HEIGHT + 2.0,
            };
            let look = if self.role == role {
                ui::ButtonLook::primary(theme)
            } else {
                ui::ButtonLook::normal(theme)
            };
            ui::button(
                elements,
                regions,
                chip,
                &format!("drawer:role:{index}"),
                &format!("{} {count}", role.name()),
                &look,
                pointer,
                measure,
            );
        }
        y + CHIP_HEIGHT + 2.0 + 8.0
    }
}

fn matches_search(character: char, ids: Option<&String>, query: &str) -> bool {
    if character.to_string().to_uppercase() == query {
        return true;
    }
    if format!("{:04X}", character as u32).contains(query) {
        return true;
    }
    ids.is_some_and(|ids| {
        ids.to_uppercase().contains(query) || ids_abbreviation(ids).contains(query)
    })
}

/// 把 IDS 里的运算符换成缩写、叶子保留，供搜索按 `RD` / `UD` 匹配。
fn ids_abbreviation(ids: &str) -> String {
    fn write(node: &IdsNode, out: &mut String) {
        match node {
            IdsNode::Leaf(character) => out.push(*character),
            IdsNode::Unencoded(number) => {
                out.push('{');
                out.push_str(&number.to_string());
                out.push('}');
            }
            IdsNode::Wildcard => out.push('?'),
            IdsNode::Op { op, children } => {
                out.push_str(op.abbreviation());
                for child in children {
                    write(child, out);
                }
            }
        }
    }
    let Ok(node) = parse(ids) else {
        return ids.to_uppercase();
    };
    let mut text = String::new();
    write(&node, &mut text);
    text.to_uppercase()
}

fn push_character(items: &mut Vec<char>, component: &Component) {
    if let Some(character) = component.character() {
        items.push(character);
    }
}

/// 两个矩形的交集；没有重叠返回 `None`。
fn intersect(left: Rect, right: Rect) -> Option<Rect> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    (right_edge > x && bottom_edge > y).then_some(Rect {
        x,
        y,
        width: right_edge - x,
        height: bottom_edge - y,
    })
}

fn paint_glyph(
    elements: &mut Vec<Element>,
    tile: Rect,
    character: char,
    theme: &Theme,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
    component_svgs: &HashMap<char, String>,
    glyphs: &mut GlyphProbe,
) {
    let font = FontSlot::Candidate;
    if let Some(svg) = component_svgs.get(&character) {
        let size = 40.0;
        elements.push(Element::Svg {
            x: tile.x + (tile.width - size) / 2.0,
            y: tile.y + (tile.height - size) / 2.0,
            size,
            source: svg.clone(),
            tint: Some(theme.text),
        });
        return;
    }
    if glyphs.has_glyph(character) {
        let size = 30.0;
        let (width, height) = measure(&character.to_string(), font, size);
        elements.push(Element::Text {
            x: tile.x + (tile.width - width).max(0.0) / 2.0,
            y: tile.y + (tile.height - height).max(0.0) / 2.0,
            text: character.to_string(),
            size,
            slot: font,
            color: theme.text,
            italic: false,
        });
        return;
    }

    // 没有内置 SVG、系统字体也缺字：只标码位，不再画结构缩写。
    let text = format!("U+{:04X}", character as u32);
    let size = 8.0;
    let (width, height) = measure(&text, font, size);
    elements.push(Element::Text {
        x: tile.x + (tile.width - width).max(0.0) / 2.0,
        y: tile.y + (tile.height - height).max(0.0) / 2.0,
        text,
        size,
        slot: font,
        color: theme.muted,
        italic: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_dedupes_and_keeps_twelve() {
        let mut drawer = Drawer::default();
        drawer.push_recent('木');
        drawer.push_recent('水');
        drawer.push_recent('木');
        assert_eq!(drawer.recent, vec!['木', '水']);
        for index in 0..20 {
            drawer.push_recent(char::from_u32('一' as u32 + index).unwrap());
        }
        assert_eq!(drawer.recent.len(), 12);
    }

    #[test]
    fn clicks_recognize_tabs_strokes_and_roles() {
        let mut drawer = Drawer::default();
        assert!(drawer.click("drawer:tab:2"));
        assert_eq!(drawer.tab, Tab::Recent);
        assert!(!drawer.click("drawer:tab:9"));
        assert!(drawer.click("drawer:stroke:13"));
        assert_eq!(drawer.stroke, 13);
        assert!(drawer.click("drawer:role:3"));
        assert_eq!(drawer.role, ComponentRole::Bottom);
        assert!(drawer.click("drawer:toggle-typeable"));
        assert!(drawer.show_typeable);
        assert!(!drawer.click("other"));
    }

    #[test]
    fn component_names_are_parsed_against_the_current_tab() {
        let drawer = Drawer {
            items: vec!['木', '水'],
            ..Drawer::default()
        };
        assert_eq!(drawer.component_at("comp:0:1"), Some('水'));
        assert_eq!(drawer.component_at("comp:1:1"), None);
        assert_eq!(drawer.component_at("comp:0:9"), None);
    }

    #[test]
    fn search_matches_character_codepoint_and_ids() {
        let mut ids = HashMap::new();
        ids.insert('𠂇', "⿸𠂇一".to_owned());
        assert!(matches_search('𠂇', ids.get(&'𠂇'), "𠂇"));
        assert!(matches_search('𠂇', ids.get(&'𠂇'), "20087"));
        assert!(matches_search('𠂇', ids.get(&'𠂇'), "RD"));
        assert!(!matches_search('𠂇', ids.get(&'𠂇'), "木"));
    }

    #[test]
    fn intersect_clips_tiles_to_the_grid() {
        let grid = Rect {
            x: 10.0,
            y: 100.0,
            width: 200.0,
            height: 300.0,
        };
        let partial = Rect {
            x: 20.0,
            y: 90.0,
            width: 56.0,
            height: 56.0,
        };
        let visible = intersect(partial, grid).unwrap();
        assert_eq!(visible.y, 100.0);
        assert_eq!(visible.height, 46.0);
        assert!(
            intersect(
                Rect {
                    x: 20.0,
                    y: 60.0,
                    width: 56.0,
                    height: 30.0,
                },
                grid
            )
            .is_none()
        );
    }
}
