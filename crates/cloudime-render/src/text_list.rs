//! 文本列表：把一组字符串按行铺进 [`ListBox`]，带模糊过滤、排序与行选区。
//!
//! 数据模型只有文本，没有子页面 / 行内控件；过滤与排序在 [`visible_items`] 里做，
//! `cloud-ui::view` 与 `cloudime-script` 的元素表共用同一套顺序。
//! 行本身的绘制、裁剪、滚动条与选中 / 悬停仍由 [`ListBox`] 负责。

use crate::color::Color;
use crate::control::{ListBox, ListBoxAlign, ListBoxRow};
use crate::document::{Element, FontSlot};

/// 文本列表的样式（点）。
#[derive(Debug, Clone, Copy)]
pub struct TextListStyle {
    /// 固定视口高度；`None` = 按行数自动算（`cloud-ui::view` 用，JSON 布局那边用 `h`）。
    pub height: Option<f32>,

    /// 每行高度。
    pub row_height: f32,

    /// 行与行之间的空隙。
    pub gap: f32,

    /// 边框内侧到内容区的间隙。
    pub content_gap: f32,

    pub radius: f32,
    pub fill: Option<Color>,
    pub edge_color: Color,
    pub edge_width: f32,

    /// 行内文字左右留白。
    pub padding_x: f32,

    /// 行选中 / 悬停底色的圆角。
    pub row_radius: f32,

    pub size: f32,
    pub font: FontSlot,
    pub text: Color,
    pub selection: Color,
    pub selection_text: Color,
    pub hover: Color,
}

impl Default for TextListStyle {
    fn default() -> Self {
        Self {
            height: None,
            row_height: 26.0,
            gap: 2.0,
            content_gap: 4.0,
            radius: 8.0,
            fill: Some(Color::rgb(30, 30, 38)),
            edge_color: Color::rgb(58, 58, 70),
            edge_width: 1.0,
            padding_x: 10.0,
            row_radius: 5.0,
            size: 14.0,
            font: FontSlot::Candidate,
            text: Color::rgb(236, 236, 242),
            selection: Color::rgb(48, 108, 220),
            selection_text: Color::rgb(255, 255, 255),
            hover: Color::rgb(45, 45, 56),
        }
    }
}

impl TextListStyle {
    /// 每行（含空隙）占的高度。
    pub fn stride(&self) -> f32 {
        self.row_height.max(1.0) + self.gap.max(0.0)
    }

    /// `rows` 行内容的净高度（最后一行不带尾部空隙）。
    pub fn content_height(&self, rows: usize) -> f32 {
        if rows == 0 {
            0.0
        } else {
            rows as f32 * self.stride() - self.gap.max(0.0)
        }
    }

    /// `rows` 行时控件应有的总高度（含边框与内间隙），自动高度用它。
    pub fn auto_height(&self, rows: usize) -> f32 {
        self.content_height(rows) + (self.edge_width.max(0.0) + self.content_gap.max(0.0)) * 2.0
    }
}

/// 模糊匹配：空查询命中一切；否则要求 `query` 的字符按顺序出现在 `text` 里。
/// 分数越大越靠前：整串相等 > 前缀 > 子串 > 子序列，越短、越靠前、间隙越少越高。
pub fn fuzzy_score(text: &str, query: &str) -> Option<i32> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let haystack = text.to_lowercase();
    if haystack == query {
        return Some(10_000);
    }
    if let Some(rest) = haystack.strip_prefix(&query) {
        return Some(8_000 - chars(rest) as i32);
    }
    if let Some(position) = haystack.find(&query) {
        let start = chars(&haystack[..position]) as i32;
        let tail = chars(&haystack[position + query.len()..]) as i32;
        return Some(6_000 - start * 4 - tail);
    }
    // 子序列：逐字往后找，记录起点与跳过的字符数。
    let haystack_chars: Vec<char> = haystack.chars().collect();
    let query_chars: Vec<char> = query.chars().collect();
    let mut start = None;
    let mut gaps = 0;
    let mut cursor = 0;
    for (position, character) in haystack_chars.iter().enumerate() {
        if cursor < query_chars.len() && *character == query_chars[cursor] {
            if start.is_none() {
                start = Some(position);
            }
            cursor += 1;
            if cursor == query_chars.len() {
                break;
            }
        } else if start.is_some() {
            gaps += 1;
        }
    }
    if cursor < query_chars.len() {
        return None;
    }
    let start = start.unwrap_or(0) as i32;
    let extra = haystack_chars.len().saturating_sub(query_chars.len()) as i32;
    Some(2_000 - start * 8 - gaps * 6 - extra)
}

/// 过滤 + 排序，返回原 `items` 的下标。
///
/// - 有查询：按匹配分数从高到低；分数相同时 `sort` 为真按文本、否则按原顺序。
/// - 没查询：`sort` 为真按文本，否则原顺序。
pub fn visible_items(items: &[String], query: &str, sort: bool) -> Vec<usize> {
    let mut shown: Vec<(usize, i32)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, text)| fuzzy_score(text, query).map(|score| (index, score)))
        .collect();
    if query.trim().is_empty() {
        if sort {
            shown.sort_by(|left, right| {
                items[left.0]
                    .cmp(&items[right.0])
                    .then_with(|| left.0.cmp(&right.0))
            });
        }
    } else {
        shown.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| {
                    if sort {
                        items[left.0].cmp(&items[right.0])
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| left.0.cmp(&right.0))
        });
    }
    shown.into_iter().map(|(index, _)| index).collect()
}

/// 构造一帧文本列表。
///
/// `selected` / `hovered` 是**原 `items` 的下标**（过滤 / 排序后仍然指向同一项），
/// 函数内部换算成显示行；`scroll_offset` 会被内容高度夹住。
#[allow(clippy::too_many_arguments)]
pub fn build(
    style: &TextListStyle,
    items: &[String],
    visible: &[usize],
    selected: Option<usize>,
    hovered: Option<usize>,
    scroll_offset: f32,
    rect: (f32, f32, f32, f32),
    region_name: &str,
) -> ListBox {
    let (x, y, width, height) = rect;
    let edge = style.edge_width.max(0.0);
    let gap = style.content_gap.max(0.0);
    let viewport = (height - (edge + gap) * 2.0).max(0.0);
    let content_height = style.content_height(visible.len());
    let scrollbar = content_height > viewport + 0.01;
    let row_width = (width - (edge + gap) * 2.0 - if scrollbar { 8.0 + 4.0 } else { 0.0 }).max(1.0);
    let scroll_offset = scroll_offset.clamp(0.0, (content_height - viewport).max(0.0));

    let mut rows = Vec::with_capacity(visible.len());
    for (position, &index) in visible.iter().enumerate() {
        let text = items.get(index).cloned().unwrap_or_default();
        rows.push(ListBoxRow {
            index: position,
            top: position as f32 * style.stride(),
            height: style.row_height,
            width: row_width,
            key: Some(text.clone()),
            elements: row_elements(style, &text, row_width, None, style.text),
            selected_elements: row_elements(
                style,
                &text,
                row_width,
                Some(style.selection),
                style.selection_text,
            ),
            hovered_elements: row_elements(style, &text, row_width, Some(style.hover), style.text),
            hits: Vec::new(),
        });
    }

    let position_of = |index: Option<usize>| {
        index
            .and_then(|index| visible.iter().position(|&shown| shown == index))
            .map_or(-1, |position| position as i32)
    };

    ListBox {
        x,
        y,
        width,
        height,
        radius: style.radius,
        fill: style.fill,
        edge_color: style.edge_color,
        edge_width: style.edge_width,
        content_gap: style.content_gap,
        gap: style.gap,
        auto_item_gap_averaging: false,
        align: ListBoxAlign::Left,
        rows,
        row_count: visible.len(),
        scroll_offset,
        content_height,
        allow_select: true,
        selected_index: position_of(selected),
        auto_center_selected: false,
        hovered_index: {
            let hovered = position_of(hovered);
            (hovered >= 0).then_some(hovered)
        },
        region_name: region_name.to_owned(),
    }
}

fn row_elements(
    style: &TextListStyle,
    text: &str,
    width: f32,
    background: Option<Color>,
    color: Color,
) -> Vec<Element> {
    let mut elements = Vec::new();
    if let Some(fill) = background {
        elements.push(Element::Rect {
            x: 0.0,
            y: 0.0,
            width,
            height: style.row_height,
            radius: style.row_radius,
            color: fill,
        });
    }
    let text_height = style.size * 1.25;
    elements.push(Element::Text {
        x: style.padding_x,
        y: ((style.row_height - text_height) / 2.0).max(0.0),
        text: text.to_owned(),
        size: style.size,
        slot: style.font,
        color,
        italic: false,
    });
    elements
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(text: &[&str]) -> Vec<String> {
        text.iter().map(|text| (*text).to_owned()).collect()
    }

    #[test]
    fn fuzzy_scores_rank_exact_then_prefix_then_substring_then_subsequence() {
        let exact = fuzzy_score("张伟", "张伟").unwrap();
        let prefix = fuzzy_score("张伟明", "张伟").unwrap();
        let substring = fuzzy_score("小张伟", "张伟").unwrap();
        let subsequence = fuzzy_score("张小明伟", "张伟").unwrap();
        assert!(exact > prefix && prefix > substring && substring > subsequence);
        assert_eq!(fuzzy_score("李娜", "张伟"), None);
        assert_eq!(fuzzy_score("随便", ""), Some(0));
    }

    #[test]
    fn fuzzy_is_case_insensitive_and_matches_latin_subsequences() {
        assert!(fuzzy_score("Alpha", "al").unwrap() > 0);
        assert!(fuzzy_score("alphabet", "apb").is_some());
        assert_eq!(fuzzy_score("alpha", "apz"), None);
    }

    #[test]
    fn visible_items_rank_by_score_and_keep_original_order_on_ties() {
        let all = items(&["小张伟", "张伟", "张伟明", "李娜"]);
        let shown = visible_items(&all, "张伟", false);
        assert_eq!(shown, [1, 2, 0]);

        let shown = visible_items(&all, "张伟", true);
        assert_eq!(shown, [1, 2, 0], "同分时按文本；这里分数不同");
    }

    #[test]
    fn visible_items_sort_without_query() {
        let all = items(&["丙", "甲", "乙"]);
        assert_eq!(visible_items(&all, "", false), [0, 1, 2]);
        assert_eq!(visible_items(&all, "", true), [0, 2, 1]);
    }

    #[test]
    fn build_maps_selection_by_original_index() {
        let style = TextListStyle::default();
        let all = items(&["甲", "乙丙", "丙"]);
        let shown = visible_items(&all, "丙", false);
        assert_eq!(shown, [2, 1], "整串相等排在子串前面");
        let list = build(
            &style,
            &all,
            &shown,
            Some(1),
            Some(2),
            0.0,
            (0.0, 0.0, 100.0, style.auto_height(shown.len())),
            "list",
        );
        assert_eq!(list.row_count, 2);
        assert_eq!(list.selected_index, 1, "乙丙 是可见行里的第 2 行");
        assert_eq!(list.hovered_index, Some(0));
        assert_eq!(list.rows[1].key.as_deref(), Some("乙丙"));
        assert!(!list.scrollbar_visible(), "自动高度下没有滚动条");
    }
}
