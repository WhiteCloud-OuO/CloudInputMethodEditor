//! 难输入部件抽屉：按笔画 / 分类 / 最近浏览，点击填入当前槽。

use std::collections::HashMap;

use cloud_ui::{Rect, Theme};
use cloudime_ids::{Component, ComponentRole, IdsDatabase};
use cloudime_render::{
    Element, FontSlot, HitShape, Region, ScrollAxis, ScrollHandle, ScrollTarget,
};

use crate::glyph::GlyphProbe;
use crate::ui;

/// 抽屉宽度（点）。
pub const WIDTH: f32 = 300.0;

const TILE: f32 = 56.0;
const GAP: f32 = 6.0;
const COLUMNS: usize = 4;
const HEADER_HEIGHT: f32 = 24.0;
const TABS_HEIGHT: f32 = 28.0;
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
    pub recent: Vec<char>,
    pub scroll: ScrollHandle,

    /// 当前页的部件（`paint` 时刷新），点击 `comp:<tab>:<i>` 用它反查。
    items: Vec<char>,
    hovered: Option<char>,
}

impl Default for Drawer {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Stroke,
            stroke: 3,
            role: ComponentRole::Side,
            show_typeable: false,
            recent: Vec::new(),
            scroll: ScrollHandle::new(0.0),
            items: Vec::new(),
            hovered: None,
        }
    }
}

impl Drawer {
    /// 按用户配置决定是否显示「可打出」部件。
    pub fn with_typeable(show_typeable: bool) -> Self {
        Self {
            show_typeable,
            ..Self::default()
        }
    }

    /// 记一个刚插入的部件（去重、最多 12 个）。
    pub fn push_recent(&mut self, character: char) {
        self.recent.retain(|item| *item != character);
        self.recent.insert(0, character);
        self.recent.truncate(12);
    }

    /// 处理抽屉自己的区域名；不是抽屉的返回 `false`。
    pub fn click(&mut self, name: &str) -> bool {
        if name == "drawer:toggle" || name == "drawer:close" {
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
            self.reset_scroll();
            return true;
        }
        if let Some(value) = name
            .strip_prefix("drawer:stroke:")
            .and_then(|value| value.parse::<u8>().ok())
        {
            self.stroke = value;
            self.reset_scroll();
            return true;
        }
        if let Some(index) = name
            .strip_prefix("drawer:role:")
            .and_then(|index| index.parse::<usize>().ok())
            && let Some(role) = ComponentRole::ALL.get(index).copied()
        {
            self.role = role;
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
    fn items(&self, database: &IdsDatabase) -> Vec<char> {
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
                width: inner_width - 24.0,
                height: HEADER_HEIGHT,
            },
            "难输入部件",
            &style,
            measure,
        );
        let close = Rect {
            x: rect.x + rect.width - 30.0,
            y: rect.y + 8.0,
            width: 22.0,
            height: 22.0,
        };
        ui::button(
            elements,
            regions,
            close,
            "drawer:close",
            "×",
            &ui::ButtonLook::normal(theme),
            pointer,
            measure,
        );

        let mut y = rect.y + 8.0 + HEADER_HEIGHT + 6.0;
        let tab_width = (inner_width - CHIP_GAP * 2.0) / 3.0;
        for (index, tab) in [Tab::Stroke, Tab::Role, Tab::Recent]
            .into_iter()
            .enumerate()
        {
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
        y += TABS_HEIGHT + 8.0;

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
        self.items = self.items(database);

        let stride = TILE + GAP;
        let rows = self.items.len().div_ceil(COLUMNS);
        let content_height = (rows as f32 * stride - GAP).max(0.0);
        let max = (content_height - grid.height).max(0.0);
        let offset = self.scroll.get().clamp(0.0, max);
        self.scroll.set(offset);

        self.hovered = None;
        let first_row = (offset / stride).floor() as usize;
        let visible_rows = (grid.height / stride).ceil() as usize + 1;
        for row in first_row..(first_row + visible_rows).min(rows) {
            for column in 0..COLUMNS {
                let index = row * COLUMNS + column;
                let Some(&character) = self.items.get(index) else {
                    break;
                };
                let tile = Rect {
                    x: grid.x + (TILE + GAP) * column as f32,
                    y: grid.y + (TILE + GAP) * row as f32 - offset,
                    width: TILE,
                    height: TILE,
                };
                if tile.y + tile.height < grid.y || tile.y > grid.y + grid.height {
                    continue;
                }
                let hovered = pointer.is_some_and(|(x, y)| ui::inside(tile, x, y));
                if hovered {
                    self.hovered = Some(character);
                }
                ui::panel(
                    elements,
                    tile,
                    if hovered {
                        theme.surface_alt
                    } else {
                        theme.background
                    },
                    8.0,
                );
                if hovered {
                    elements.push(Element::Rect {
                        x: tile.x,
                        y: tile.y,
                        width: tile.width,
                        height: tile.height,
                        radius: 8.0,
                        color: theme.accent,
                    });
                    ui::panel(
                        elements,
                        Rect {
                            x: tile.x + 1.0,
                            y: tile.y + 1.0,
                            width: tile.width - 2.0,
                            height: tile.height - 2.0,
                        },
                        theme.surface_alt,
                        7.0,
                    );
                }
                paint_glyph(
                    elements,
                    tile,
                    character,
                    theme,
                    measure,
                    component_ids,
                    glyphs,
                );
                regions.push(Region::new(
                    format!("comp:{}:{index}", self.tab.index()),
                    HitShape::RoundRect {
                        x: tile.x,
                        y: tile.y,
                        width: tile.width,
                        height: tile.height,
                        radius: 8.0,
                    },
                    false,
                ));
            }
        }

        if max > 0.0 {
            let track_height = grid.height;
            let thumb_height = (track_height * grid.height / content_height).max(20.0);
            let thumb_y = grid.y + (track_height - thumb_height) * (offset / max);
            elements.push(Element::Rect {
                x: rect.x + rect.width - 6.0,
                y: thumb_y,
                width: 3.0,
                height: thumb_height,
                radius: 1.5,
                color: theme.border,
            });
            scroll_targets.push(ScrollTarget {
                name: "ids.drawer".to_owned(),
                axis: ScrollAxis::Vertical,
                area: (grid.x, grid.y, grid.width, grid.height),
                track: (rect.x + rect.width - 6.0, grid.y, 3.0, grid.height),
                thumb: (rect.x + rect.width - 6.0, thumb_y, 3.0, thumb_height),
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
            .hovered
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

fn push_character(items: &mut Vec<char>, component: &Component) {
    if let Some(character) = component.character() {
        items.push(character);
    }
}

fn paint_glyph(
    elements: &mut Vec<Element>,
    tile: Rect,
    character: char,
    theme: &Theme,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
    component_ids: &HashMap<char, String>,
    glyphs: &mut GlyphProbe,
) {
    let font = FontSlot::Candidate;
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

    // 系统字体缺字：画 ASCII 缩写（`UD丿罒`），最多展开两层。
    let text = component_ids
        .get(&character)
        .map(|ids| {
            let mut has_glyph = |candidate| glyphs.has_glyph(candidate);
            crate::glyph::abbreviation(ids, 2, component_ids, &mut has_glyph)
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| format!("U+{:04X}", character as u32));
    let size = 10.0;
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
    // 右下角小点：提醒这不是正常字形。
    elements.push(Element::Rect {
        x: tile.x + tile.width - 8.0,
        y: tile.y + tile.height - 8.0,
        width: 3.0,
        height: 3.0,
        radius: 1.5,
        color: theme.accent,
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
        assert!(drawer.click("drawer:tab:1"));
        assert_eq!(drawer.tab, Tab::Role);
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
}
