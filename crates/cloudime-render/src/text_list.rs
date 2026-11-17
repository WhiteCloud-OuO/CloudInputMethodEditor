//! 文本列表：把一组字符串按行铺进 [`ListBox`]，带模糊过滤、排序与行选区。
//!
//! 数据模型只有文本，没有子页面 / 行内控件；过滤与排序在 [`visible_items`] 里做，
//! `cloud-ui::view` 与 `cloudime-script` 的元素表共用同一套顺序。
//! 行本身的绘制、裁剪、滚动条与选中 / 悬停仍由 [`ListBox`] 负责；放不下的长文本按
//! [`TextListStyle::allow_text_wrapping`] 决定是截断加「…」还是出水平滚动条。

use crate::color::Color;
use crate::control::{ListBox, ListBoxAlign, ListBoxRow, RowBackground};
use crate::document::{Element, FontSlot};

/// 文本列表的样式（点）。
#[derive(Debug, Clone, Copy)]
pub struct TextListStyle {
    /// 固定宽度（点）；`None` = 横向铺满父容器（`cloud-ui::view` 用，JSON 布局那边用 `w`）。
    pub width: Option<f32>,

    /// 固定视口高度；`None` = 按行数自动算（`cloud-ui::view` 用，JSON 布局那边用 `h`）。
    ///
    /// 只有 `auto_height = false` 时才使用；自动高度时忽略。
    pub height: Option<f32>,

    /// 自动调节高度（缺省 true）：按可见行数算高度，并让项目到边框的间隙等于项目间隙。
    pub auto_height: bool,

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

    /// 画不画边框（缺省 true）；关闭时边框宽度按 0 算。
    pub show_border: bool,

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

    /// `true`：一行放不下时把文字截断并在末尾补「…」；`false`：放不下就出水平滚动条。
    pub allow_text_wrapping: bool,
}

impl Default for TextListStyle {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            auto_height: true,
            row_height: 26.0,
            gap: 2.0,
            content_gap: 4.0,
            radius: 8.0,
            fill: Some(Color::rgb(30, 30, 38)),
            edge_color: Color::rgb(58, 58, 70),
            edge_width: 1.0,
            show_border: true,
            padding_x: 10.0,
            row_radius: 5.0,
            size: 14.0,
            font: FontSlot::Candidate,
            text: Color::rgb(236, 236, 242),
            selection: Color::rgb(48, 108, 220),
            selection_text: Color::rgb(255, 255, 255),
            hover: Color::rgb(45, 45, 56),
            allow_text_wrapping: true,
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

    /// 实际边框宽度：`show_border = false` 时不画边框，也不占内缩。
    pub fn edge(&self) -> f32 {
        if self.show_border {
            self.edge_width.max(0.0)
        } else {
            0.0
        }
    }

    /// 边框到内容的间隙：自动高度时取项目间隙，保证「项目到边框」与「项目之间」相等；
    /// 固定高度时用显式的 `content_gap`。
    pub fn content_inset(&self) -> f32 {
        if self.auto_height {
            self.gap.max(0.0)
        } else {
            self.content_gap.max(0.0)
        }
    }

    /// `rows` 行时控件应有的总高度（含边框与内间隙），自动高度用它。
    pub fn auto_height(&self, rows: usize) -> f32 {
        self.content_height(rows) + (self.edge() + self.content_inset()) * 2.0
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

/// 量一组可见项目里最宽的一条（含左右留白）。
pub fn measured_width(
    style: &TextListStyle,
    items: &[String],
    visible: &[usize],
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) -> f32 {
    visible
        .iter()
        .map(|&index| {
            let text = items.get(index).map(String::as_str).unwrap_or("");
            measure(text, style.font, style.size).0
        })
        .fold(0.0f32, f32::max)
        + style.padding_x.max(0.0) * 2.0
}

/// 自动高度：`allow_text_wrapping = false` 且内容超宽时，把底部水平滚动条的高度也算进去。
pub fn auto_height(style: &TextListStyle, rows: usize, width: f32, content_width: f32) -> f32 {
    let mut height = style.auto_height(rows);
    let inner = (width - (style.edge() + style.content_inset()) * 2.0).max(0.0);
    if !style.allow_text_wrapping && content_width > inner + 0.01 {
        height += ListBox::SCROLLBAR_THICKNESS + ListBox::SCROLLBAR_GAP;
    }
    height
}

/// 构造一帧文本列表。
///
/// `selected` / `hovered` 是**原 `items` 的下标**（过滤 / 排序后仍然指向同一项），
/// 函数内部换算成显示行；`scroll_offset` / `h_scroll_offset` 会被内容范围夹住。
/// `measure` 量一段文字的尺寸，用来判断长文本是截断还是出水平滚动条。
#[allow(clippy::too_many_arguments)]
pub fn build(
    style: &TextListStyle,
    items: &[String],
    visible: &[usize],
    selected: Option<usize>,
    hovered: Option<usize>,
    scroll_offset: f32,
    h_scroll_offset: f32,
    rect: (f32, f32, f32, f32),
    region_name: &str,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) -> ListBox {
    let (x, y, width, height) = rect;
    let padding = style.padding_x.max(0.0);
    let content_height = style.content_height(visible.len());
    let measured_width = measured_width(style, items, visible, measure);
    // 截断模式下永远不出水平滚动条：内容宽度按 0 算，行按可视宽度铺满。
    let content_width = if style.allow_text_wrapping {
        0.0
    } else {
        measured_width
    };

    let mut list = ListBox {
        x,
        y,
        width,
        height,
        radius: style.radius,
        fill: style.fill,
        edge_color: style.edge_color,
        edge_width: style.edge(),
        content_gap: style.content_inset(),
        gap: style.gap,
        // 与列表盒一致：视口装不下全部行时，把剩余高度平均成行间隙，保证每行完整、
        // 相邻行之间的垂直空隙相等。
        auto_item_gap_averaging: true,
        align: ListBoxAlign::Left,
        rows: Vec::with_capacity(visible.len()),
        row_count: visible.len(),
        scroll_offset,
        content_width,
        h_scroll_offset,
        content_height,
        allow_select: true,
        selected_index: -1,
        auto_center_selected: false,
        hovered_index: None,
        region_name: region_name.to_owned(),
    };

    let (vertical, horizontal) = list.scrollbars();
    let available = list.content_width(vertical);
    // 出水平滚动条时行按内容宽度铺开；截断模式下按可视宽度铺满。
    let row_width = if horizontal {
        measured_width.max(available)
    } else {
        available
    };
    let text_limit = (row_width - padding * 2.0).max(0.0);
    for (position, &index) in visible.iter().enumerate() {
        let text = items.get(index).cloned().unwrap_or_default();
        let shown = if style.allow_text_wrapping {
            truncate_to_width(&text, text_limit, style, measure)
        } else {
            text.clone()
        };
        list.rows.push(ListBoxRow {
            index: position,
            top: position as f32 * style.stride(),
            height: style.row_height,
            width: row_width,
            key: Some(text),
            elements: row_elements(style, &shown, style.text),
            selected_elements: row_elements(style, &shown, style.selection_text),
            hovered_elements: row_elements(style, &shown, style.text),
            hits: Vec::new(),
            background: None,
            selected_background: Some(RowBackground {
                color: style.selection,
                radius: style.row_radius,
            }),
            hovered_background: Some(RowBackground {
                color: style.hover,
                radius: style.row_radius,
            }),
        });
    }

    let position_of = |index: Option<usize>| {
        index
            .and_then(|index| visible.iter().position(|&shown| shown == index))
            .map_or(-1, |position| position as i32)
    };
    list.selected_index = position_of(selected);
    let hovered = position_of(hovered);
    list.hovered_index = (hovered >= 0).then_some(hovered);
    list.h_scroll_offset = list.clamp_h_scroll();
    list
}

/// 把 `text` 截到 `max` 宽以内，末尾补「…」；已经放得下就原样返回。
fn truncate_to_width(
    text: &str,
    max: f32,
    style: &TextListStyle,
    measure: &mut dyn FnMut(&str, FontSlot, f32) -> (f32, f32),
) -> String {
    if max <= 0.0 {
        return String::new();
    }
    if measure(text, style.font, style.size).0 <= max {
        return text.to_owned();
    }
    let characters: Vec<char> = text.chars().collect();
    let (mut low, mut high) = (0usize, characters.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        let mut candidate: String = characters[..middle].iter().collect();
        candidate.push('…');
        if measure(&candidate, style.font, style.size).0 <= max {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let mut truncated: String = characters[..low].iter().collect();
    truncated.push('…');
    truncated
}

fn row_elements(style: &TextListStyle, text: &str, color: Color) -> Vec<Element> {
    let text_height = style.size * 1.25;
    vec![Element::Text {
        x: style.padding_x,
        y: ((style.row_height - text_height) / 2.0).max(0.0),
        text: text.to_owned(),
        size: style.size,
        slot: style.font,
        color,
        italic: false,
    }]
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

    /// 等宽假量尺：一个字符 10 点宽。
    fn measure(text: &str, _slot: FontSlot, _size: f32) -> (f32, f32) {
        (text.chars().count() as f32 * 10.0, 14.0)
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
        let mut measure = measure;
        let list = build(
            &style,
            &all,
            &shown,
            Some(1),
            Some(2),
            0.0,
            0.0,
            (0.0, 0.0, 100.0, style.auto_height(shown.len())),
            "list",
            &mut measure,
        );
        assert_eq!(list.row_count, 2);
        assert_eq!(list.selected_index, 1, "乙丙 是可见行里的第 2 行");
        assert_eq!(list.hovered_index, Some(0));
        assert_eq!(list.rows[1].key.as_deref(), Some("乙丙"));
        assert!(!list.scrollbar_visible(), "自动高度下没有滚动条");
        assert!(!list.h_scrollbar_visible(), "文字放得下，不出水平条");
    }

    #[test]
    fn fixed_viewport_distributes_remaining_height_between_visible_rows() {
        let style = TextListStyle {
            row_height: 20.0,
            gap: 2.0,
            ..TextListStyle::default()
        };
        let all = items(&["a", "b", "c", "d", "e", "f"]);
        let shown = visible_items(&all, "", false);
        let mut measure = measure;
        let list = build(
            &style,
            &all,
            &shown,
            None,
            None,
            0.0,
            0.0,
            (0.0, 0.0, 100.0, 80.0),
            "list",
            &mut measure,
        );
        assert!(list.auto_item_gap_averaging, "文本列表与列表盒同一套规则");
        assert!(list.scrollbar_visible());
        let viewport = list.viewport_height();
        let visible: Vec<(f32, f32)> = list
            .rows
            .iter()
            .filter_map(|row| list.row_view_top(row).map(|top| (top, row.height)))
            .collect();
        assert!(visible.len() >= 2, "{visible:?}");
        assert!(visible[0].0.abs() < 0.01, "第一行贴顶：{visible:?}");
        let gap = visible[1].0 - (visible[0].0 + visible[0].1);
        for pair in visible.windows(2) {
            let next_gap = pair[1].0 - (pair[0].0 + pair[0].1);
            assert!((next_gap - gap).abs() < 0.01, "间隙应相等：{visible:?}");
        }
        for (top, height) in &visible {
            assert!(*top >= -0.01, "行顶被裁：{visible:?}");
            assert!(top + height <= viewport + 0.01, "行底被裁：{visible:?}");
        }
        assert!(gap > style.gap, "剩余高度应平均进间隙：{gap}");
        let row = list.row_at(visible[1].0 + list.edge_width + list.content_gap + 1.0);
        assert_eq!(row, Some(1), "命中要按平均后的行位置");
    }

    #[test]
    fn auto_height_uses_the_item_gap_between_rows_and_border() {
        let style = TextListStyle {
            auto_height: true,
            gap: 6.0,
            content_gap: 4.0,
            ..TextListStyle::default()
        };
        let all = items(&["a", "b", "c"]);
        let shown = visible_items(&all, "", false);
        let mut measure = measure;
        let height = style.auto_height(shown.len());
        let list = build(
            &style,
            &all,
            &shown,
            None,
            None,
            0.0,
            0.0,
            (0.0, 0.0, 100.0, height),
            "list",
            &mut measure,
        );
        assert!(
            (list.content_gap - 6.0).abs() < 0.01,
            "边框到内容的间隙应等于项目间隙"
        );
        assert!((list.gap - 6.0).abs() < 0.01);
        let expected = style.content_height(shown.len()) + (style.edge_width + style.gap) * 2.0;
        assert!((height - expected).abs() < 0.01, "{height} != {expected}");
        assert!(!list.scrollbar_visible(), "自动高度不应出现滚动条");

        // 固定高度仍用显式的 content_gap。
        let fixed = TextListStyle {
            auto_height: false,
            gap: 6.0,
            content_gap: 4.0,
            ..TextListStyle::default()
        };
        let mut ruler = measure;
        let list = build(
            &fixed,
            &all,
            &shown,
            None,
            None,
            0.0,
            0.0,
            (0.0, 0.0, 100.0, 80.0),
            "list",
            &mut ruler,
        );
        assert!((list.gap - 6.0).abs() < 0.01);
        assert!((list.content_gap - 4.0).abs() < 0.01);
    }

    #[test]
    fn hiding_the_border_removes_the_stroke_and_its_inset() {
        let style = TextListStyle {
            show_border: false,
            ..TextListStyle::default()
        };
        let all = items(&["a"]);
        let shown = visible_items(&all, "", false);
        let mut measure = measure;
        let height = style.auto_height(1);
        let list = build(
            &style,
            &all,
            &shown,
            None,
            None,
            0.0,
            0.0,
            (0.0, 0.0, 100.0, height),
            "list",
            &mut measure,
        );
        assert_eq!(list.edge_width, 0.0, "关闭边框后不画描边");
        let expected = style.content_height(1) + style.content_inset() * 2.0;
        assert!((height - expected).abs() < 0.01);
    }

    #[test]
    fn long_text_truncates_with_ellipsis_when_wrapping_is_allowed() {
        let style = TextListStyle {
            allow_text_wrapping: true,
            padding_x: 10.0,
            ..TextListStyle::default()
        };
        let all = items(&["12345678901234567890"]);
        let shown = visible_items(&all, "", false);
        let mut measure = measure;
        // 宽 100：内容区约 100 - 2*(1+4) = 90，文字可用 90 - 20 = 70 → 7 个字符 + …
        let list = build(
            &style,
            &all,
            &shown,
            None,
            None,
            0.0,
            0.0,
            (0.0, 0.0, 100.0, 40.0),
            "list",
            &mut measure,
        );
        let Some(Element::Text { text, .. }) = list.rows[0].elements.last() else {
            panic!("行里应有文字");
        };
        assert!(text.ends_with('…'), "{text}");
        assert!(text.chars().count() <= 8, "{text}");
        assert!(!list.h_scrollbar_visible());
    }

    #[test]
    fn long_text_shows_a_horizontal_scrollbar_when_wrapping_is_off() {
        let style = TextListStyle {
            allow_text_wrapping: false,
            ..TextListStyle::default()
        };
        let all = items(&["12345678901234567890"]);
        let shown = visible_items(&all, "", false);
        let mut measure = measure;
        let list = build(
            &style,
            &all,
            &shown,
            None,
            None,
            0.0,
            0.0,
            (0.0, 0.0, 100.0, 40.0),
            "list",
            &mut measure,
        );
        assert!(list.h_scrollbar_visible(), "长文本要出水平滚动条");
        let Some(Element::Text { text, .. }) = list.rows[0].elements.last() else {
            panic!("行里应有文字");
        };
        assert_eq!(text, "12345678901234567890", "不截断");
        assert!(list.max_h_scroll_px() > 0.0);
        assert!(list.h_thumb_rect().2 > 0.0);
        let (_, track_y, _, track_height) = list.h_track_rect();
        assert!(track_y + track_height <= list.height);
    }
}
