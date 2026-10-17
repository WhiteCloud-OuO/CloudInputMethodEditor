//! 「键位映射参考」的画法：三排**键盘格子**（逐排错开半格，模仿真实键盘）+ 右侧一张零声母表。
//!
//! 版面（哪一格写什么字）由壳（Server）按当前方案准备好交进来（`input_schemes::KeyMapLayout`），
//! 这里只管尺寸、间距、颜色与文字摆位。
//!
//! 尺寸都用**点**（进画布前乘缩放倍数）：
//!
//! - 格子是**正方形**、相互紧挨；四周各留 [`PADDING`] 点 —— 键盘第一排的上边、第三排的下边、
//!   第一排第一列左边都从这里起算；
//! - 第二排比第一排**右移半格**、第三排再右移半格（[`ROW_STAGGER`]），像真实键盘那样错开；
//! - 零声母表在键盘**右侧**隔 [`TABLE_GAP`] 点：**固定 3 行**（跟键盘一样高）、列按项数往上取，
//!   `[Shengmu]` 最多 12 项就是 4 列。所以窗口**高度恒定**（三排键盘那点高），随方案变的只有宽度。

use crate::color::Color;
use crate::elements::{RectElement, TextElement};
use crate::error::RenderError;
use crate::renderer::{Metrics, Renderer, truncate_with};
use crate::shadow::Shadow;
use crate::text::TextStyle;
use crate::theme::{FontSpec, Theme};

/// 四周留白（点）：键盘第一排的上边、第三排的下边、第一排第一列左边都从这里起算。
pub const PADDING: f32 = 8.0;

/// 格线的宽度（点）：**相邻两格共用一条线**，所以两格之间的线也是这么宽（不是两倍）。
pub const LINE: f32 = 1.0;

/// 格子边长的下限（点）：目标高度太小时不至于挤成一条。
const MIN_CELL: f32 = 24.0;

/// 键盘与**下方**零声母表之间的间距（点）。
const TABLE_GAP: f32 = 8.0;

/// 零声母表的列数：`[Shengmu]` 最多 12 项 → 6 列 × 2 行。
const TABLE_COLUMNS: usize = 6;

/// 零声母表的行数。
const TABLE_ROWS: usize = 2;

/// 零声母表格子的宽度占键盘格子的比例（宽一点，好放「按键=韵母」一行）。
const ZERO_WIDTH_RATIO: f32 = 4.0 / 3.0;

/// 零声母表格子的高度占键盘格子的比例（矮一点，一行字就够）。
const ZERO_HEIGHT_RATIO: f32 = 1.0 / 3.0;

/// 键盘格子左上角那颗键的字号占格子边长的比例（基准：46 点的格子里 7 点）。
const KEY_FONT_RATIO: f32 = 7.0 / 46.0;

/// 格子中间韵母的字号占格子边长的比例（基准：46 点的格子里 11 点）。
const MAPPING_FONT_RATIO: f32 = 11.0 / 46.0;

/// 格子右上角声母（`zh` / `ch` / `sh`）的字号占格子边长的比例：**与中间那行韵母一样大**。
const INITIAL_FONT_RATIO: f32 = MAPPING_FONT_RATIO;

/// 格子里文字与边框之间的内边距占格子边长的比例（基准：46 点的格子里 3 点）。
const CELL_INSET_RATIO: f32 = 3.0 / 46.0;

/// 声母（`zh` / `ch` / `sh`）在格子里是棕色。
const INITIAL_COLOR: Color = Color::rgb(0x88, 0x00, 0x15);

/// 一格里要画的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCap {
    /// **左上角**那颗键（`Q` / `;`；零声母格子里是两个键 `AY`）。
    pub key: String,

    /// **右上角**的声母（`zh` / `ch` / `sh`，棕色）；没有就空着。
    pub initial: String,

    /// 中间那几行**韵母**，一义一行（用换行符分开）；每行第一颗字母按 [`head_color`] 上色。
    pub mapping: String,
}

/// 韵母**第一个字母**的颜色：a 红、o 橙、e 绿、i 蓝、u 紫、ü 棕；其余字母用普通文字色（黑）。
fn head_color(head: char) -> Option<Color> {
    match head {
        'a' => Some(Color::rgb(0xF4, 0x20, 0x20)),
        'o' => Some(Color::rgb(0xFF, 0x80, 0x40)),
        'e' => Some(Color::rgb(0x00, 0xA8, 0x69)),
        'i' => Some(Color::rgb(0x48, 0xAB, 0xF8)),
        'u' => Some(Color::rgb(0x8D, 0x3C, 0xE8)),
        'ü' => Some(INITIAL_COLOR),
        _ => None,
    }
}

/// 键位映射参考的版面。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyMapLayout {
    /// 三排键盘格子（顺序就是键盘顺序，每排从左到右）。
    pub rows: Vec<Vec<KeyCap>>,

    /// 零声母表：从左上往下填，每列 [`TABLE_ROWS`] 个。
    pub zero_initials: Vec<KeyCap>,
}

impl KeyMapLayout {
    /// 一格占的步距：格子边长减去与下一格共用的那条线。
    fn pitch(cell: f32) -> f32 {
        (cell - LINE).max(LINE)
    }

    /// 每排相对上一排右移多少（点）：半格。
    fn stagger(cell: f32) -> f32 {
        cell / 2.0
    }

    /// `count` 个格子排一行的跨度（点）：格子边长 × 个数 − 中间共用的线。
    fn span(count: usize, cell: f32) -> f32 {
        count as f32 * cell - count.saturating_sub(1) as f32 * LINE
    }

    /// 键盘那一块有多宽（点）：每排「跨度 + 它错开的那半格」，取最宽的一排。
    fn keyboard_width(&self, cell: f32) -> f32 {
        self.rows
            .iter()
            .enumerate()
            .map(|(row, caps)| Self::span(caps.len(), cell) + row as f32 * Self::stagger(cell))
            .fold(0.0, f32::max)
            .max(cell)
    }

    /// 零声母表占几行（6 列一行；一项都没有就不画表）。
    ///
    /// 有表就**固定两行**（不足 12 项也留着空位）—— 高度要恒定，不能随项数变。
    fn table_rows(&self) -> usize {
        if self.zero_initials.is_empty() {
            0
        } else {
            TABLE_ROWS
        }
    }
    /// 零声母表那块的宽度（点）：6 个格子，每个 = 键盘格子 × [`ZERO_WIDTH_RATIO`]。
    fn table_width(&self, cell: f32) -> f32 {
        Self::span(TABLE_COLUMNS, cell * ZERO_WIDTH_RATIO)
    }

    /// 零声母表那块的高度（点）：两行，每行 = 键盘格子 × [`ZERO_HEIGHT_RATIO`]。
    fn table_height(&self, cell: f32) -> f32 {
        Self::span(TABLE_ROWS, cell * ZERO_HEIGHT_RATIO)
    }
}

/// 目标高度（点）→ 格子边长（点）：**三排键盘 + 零声母表两行**（每行一格的三分之一；相邻两排共用一条线）、
/// 中间隔 [`TABLE_GAP`]、加两边留白，正好等于目标高度。
///
/// 竖排时这个目标就是「候选窗口装 9 个候选项」的高度（之前定义好的那个缺省高度），
/// 横排也用同一个尺寸 —— 两种排布下辅助窗口一样大。
pub fn key_map_cell(target_height: f32) -> f32 {
    let rows_height = 3.0 + TABLE_ROWS as f32 * ZERO_HEIGHT_RATIO;
    // 相邻两排共用一条线：键盘里 2 条、零声母表里 1 条（两处之间隔着 TABLE_GAP，不是共用线）
    let shared_lines = 3.0;
    ((target_height - PADDING * 2.0 - TABLE_GAP + shared_lines * LINE) / rows_height).max(MIN_CELL)
}

/// 「键位映射参考」的内容区尺寸（**点**）：**高度恒定**（三排键盘 + 零声母表两行 + 两边留白 + 表前间距），
/// 宽度 = 两边留白 + max(键盘（含错开）, 零声母表)。
pub fn key_map_size(layout: &KeyMapLayout, cell: f32) -> (f32, f32) {
    let rows = layout.rows.len().max(1);
    let mut height = PADDING * 2.0 + KeyMapLayout::span(rows, cell);
    let mut width = PADDING * 2.0 + layout.keyboard_width(cell);
    if layout.table_rows() > 0 {
        height += TABLE_GAP + layout.table_height(cell);
        width = width.max(PADDING * 2.0 + layout.table_width(cell));
    }
    (width, height)
}

impl Renderer {
    /// 画一块「键位映射参考」面板：主题背景 + 圆角（+ 阴影边），上面三排键盘格子与右侧的零声母表。
    ///
    /// `cell` 是键盘格子的边长（点）—— 由调用方按目标高度算好（[`key_map_cell`]），竖排取
    /// 「候选窗口装 9 个候选项」的高度、横排用同一个值。内容区尺寸见 [`key_map_size`]。
    pub fn render_key_map(
        &self,
        layout: &KeyMapLayout,
        cell: f32,
        theme: &Theme,
        scale: f32,
        shadow: Option<&Shadow>,
    ) -> Result<crate::renderer::Rendered, RenderError> {
        let (width, height) = key_map_pixel_size(layout, cell, scale);
        let (mut render, margin) = self.panel_canvas(width, height, theme, scale, shadow)?;
        draw_key_map(&mut render, layout, cell, theme, scale, (margin, margin));
        Ok(Self::panel_rendered(render, margin, (width, height), scale))
    }
}

/// 键位映射参考的内容尺寸（**设备像素**）：格子先按缩放倍数取整到设备像素（相邻两格共用一条线，
/// 落在整像素上才是干净的 1 点），再按版面算。
pub fn key_map_pixel_size(layout: &KeyMapLayout, cell: f32, scale: f32) -> (f32, f32) {
    let cell = (cell * scale).round().max(1.0);
    key_map_size(layout, cell)
}

/// 把「键位映射参考」画到任何能画元素的目标上（眼下只有渲染的 [`crate::Render`]）：
/// `origin` 是**内容区**左上角（设备像素），`cell` 是格子边长（**点**）。
///
/// 两条路共用这一份版面：格子尺寸、逐排错开、零声母表位置、文字摆位都只在这里算一次；量字走目标自己的
/// [`TextElement::measure`]（渲染是 cosmic-text，绘制是 GDI），所以截断位置可能差一两个像素，版面不变。
pub fn draw_key_map<T: RectElement + TextElement>(
    target: &mut T,
    layout: &KeyMapLayout,
    cell: f32,
    theme: &Theme,
    scale: f32,
    origin: (f32, f32),
) {
    let metrics = Metrics { theme, scale };
    let styles = KeyMapStyles::new(&metrics, cell);
    let cell = metrics.px(cell).round().max(1.0);
    // 相邻两格共用一条线：步距 = 边长 − 线宽
    let pitch = KeyMapLayout::pitch(cell);
    let stagger = KeyMapLayout::stagger(cell).round();
    let left = origin.0 + metrics.px(PADDING);
    let top = origin.1 + metrics.px(PADDING);
    for (row, caps) in layout.rows.iter().enumerate() {
        // 每排再右移半格：像真实键盘那样错开
        let row_left = left + row as f32 * stagger;
        for (column, cap) in caps.iter().enumerate() {
            let x = row_left + column as f32 * pitch;
            let y = top + row as f32 * pitch;
            draw_key_cap(target, cap, &styles, &metrics, (x, y, cell, cell));
        }
    }
    // 零声母表：键盘**下方**、隔 [`TABLE_GAP`]，6 列 × 最多 2 行；格子矮一点、宽一点（放「按键=韵母」一行）
    if layout.table_rows() > 0 {
        let table_top = top + KeyMapLayout::span(layout.rows.len(), cell) + metrics.px(TABLE_GAP);
        let zero_width = (cell * ZERO_WIDTH_RATIO).round().max(1.0);
        let zero_height = (cell * ZERO_HEIGHT_RATIO).round().max(1.0);
        for (index, cap) in layout.zero_initials.iter().enumerate() {
            let x = left + (index % TABLE_COLUMNS) as f32 * KeyMapLayout::pitch(zero_width);
            let y = table_top + (index / TABLE_COLUMNS) as f32 * KeyMapLayout::pitch(zero_height);
            draw_zero_cap(
                target,
                cap,
                &styles,
                &metrics,
                (x, y, zero_width, zero_height),
            );
        }
    }
}

/// 画零声母表的一格：**一行**「按键=韵母」（`AY=ang`）居中，韵母开头的字母照 [`head_color`] 上色。
fn draw_zero_cap<T: RectElement + TextElement>(
    target: &mut T,
    cap: &KeyCap,
    styles: &KeyMapStyles,
    metrics: &Metrics,
    (x, y, width, height): (f32, f32, f32, f32),
) {
    let line = metrics.px(LINE).max(1.0);
    target.rect(x, y, width, height, styles.line);
    target.rect(
        x + line,
        y + line,
        (width - line * 2.0).max(0.0),
        (height - line * 2.0).max(0.0),
        styles.background,
    );
    if cap.mapping.is_empty() {
        return;
    }
    let prefix = format!("{}=", cap.key);
    let mut chars = cap.mapping.chars();
    let head = chars.next().unwrap_or_default();
    let rest = chars.as_str();
    let head_style = if head_color(head).is_some() {
        styles.head_style(metrics, head)
    } else {
        styles.mapping.clone()
    };
    let prefix_width = target.measure(&prefix, &styles.mapping).width;
    let head_width = target.measure(&head.to_string(), &head_style).width;
    let rest_width = target.measure(rest, &styles.mapping).width;
    let mut text_x = x + (width - prefix_width - head_width - rest_width) / 2.0;
    let text_y = y + (height - styles.mapping.line_height) / 2.0;
    target.text(&prefix, &styles.mapping, text_x, text_y);
    text_x += prefix_width;
    target.text(&head.to_string(), &head_style, text_x, text_y);
    text_x += head_width;
    target.text(rest, &styles.mapping, text_x, text_y);
}

/// 画一格：一格宽的边框，**左上角**那颗键、**右上角**的声母（棕色）、中间几行韵母
/// （一义一行，每行第一颗字母按 [`head_color`] 上色，其余用普通文字色）。
///
/// 相邻格之间**共用**这条线：调用方把两格摆成相差「边长 − 线宽」，各自画出来正好重叠在一条上。
fn draw_key_cap<T: RectElement + TextElement>(
    target: &mut T,
    cap: &KeyCap,
    styles: &KeyMapStyles,
    metrics: &Metrics,
    (x, y, width, height): (f32, f32, f32, f32),
) {
    let line = metrics.px(LINE).max(1.0);
    // 边框：先铺一层格线色，再把里面挖成背景色
    target.rect(x, y, width, height, styles.line);
    target.rect(
        x + line,
        y + line,
        (width - line * 2.0).max(0.0),
        (height - line * 2.0).max(0.0),
        styles.background,
    );
    let inset = metrics.px(width * CELL_INSET_RATIO);
    // 左上角那颗键
    if !cap.key.is_empty() {
        target.text(&cap.key, &styles.key, x + inset, y + inset);
    }
    // 右上角的声母（棕色，右对齐）
    if !cap.initial.is_empty() {
        let initial_width = target.measure(&cap.initial, &styles.initial).width;
        target.text(
            &cap.initial,
            &styles.initial,
            x + width - inset - initial_width,
            y + inset,
        );
    }
    let budget = width - inset * 2.0;
    if cap.mapping.is_empty() || budget <= 0.0 {
        return;
    }
    let lines = mapping_lines(target, &cap.mapping, &styles.mapping, budget);
    let line_height = styles.mapping.line_height;
    let block = lines.len() as f32 * line_height;
    let mut text_y = y + (height - block) / 2.0;
    for text in &lines {
        // 第一颗字母换个颜色（a 红 / o 橙 / e 绿 / i 蓝 / u 紫 / ü 棕），其余字母黑色
        let mut chars = text.chars();
        let head = chars.next().unwrap_or_default();
        let rest = chars.as_str();
        let head_style = styles
            .head
            .as_ref()
            .filter(|_| head_color(head).is_some())
            .map(|_| styles.head_style(metrics, head))
            .unwrap_or_else(|| styles.mapping.clone());
        let head_width = target.measure(&head.to_string(), &head_style).width;
        let rest_width = target.measure(rest, &styles.mapping).width;
        let text_x = x + (width - head_width - rest_width) / 2.0;
        target.text(&head.to_string(), &head_style, text_x, text_y);
        target.text(rest, &styles.mapping, text_x + head_width, text_y);
        text_y += line_height;
    }
}

/// 一格里的韵母：**按换行符拆行**（一个键多个义时一义一行），每行放不下就截尾。
fn mapping_lines<T: TextElement>(
    target: &T,
    text: &str,
    style: &TextStyle,
    budget: f32,
) -> Vec<String> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            truncate_with(
                |text, style| target.measure(text, style),
                line,
                style,
                budget,
            )
            .0
        })
        .collect()
}

/// 键位映射参考用到的颜色与文字样式（按主题 + 缩放算一次，整幅图共用）。
struct KeyMapStyles {
    background: crate::color::Color,
    line: crate::color::Color,
    key: TextStyle,
    initial: TextStyle,
    mapping: TextStyle,
    /// 有 `Some` 表示韵母首字母要单独上色（颜色按字母现取）。
    head: Option<()>,
}

impl KeyMapStyles {
    /// `cell` 是格子边长（**点**）：格子里的字号与内边距都按它等比算 —— 格子大了字也大。
    fn new(metrics: &Metrics, cell: f32) -> Self {
        let colors = metrics.theme.colors;
        let family = metrics.theme.text_family.as_ref();
        Self {
            background: colors.background,
            // 格线借序号 / 页脚那档浅灰，跟候选窗的次要文字一致
            line: colors.badge,
            key: metrics
                .style(FontSpec::with_size(cell * KEY_FONT_RATIO), colors.badge)
                .with_family(family),
            initial: metrics
                .style(
                    FontSpec::with_size(cell * INITIAL_FONT_RATIO),
                    INITIAL_COLOR,
                )
                .with_family(family),
            mapping: metrics
                .style(FontSpec::with_size(cell * MAPPING_FONT_RATIO), colors.text)
                .with_family(metrics.theme.pinyin_family.as_ref()),
            head: Some(()),
        }
    }

    /// 韵母首字母的样式：颜色按 [`head_color`] 取（取不到就用普通文字色）。
    fn head_style(&self, metrics: &Metrics, head: char) -> TextStyle {
        let color = head_color(head).unwrap_or(metrics.theme.colors.text);
        metrics
            .style(FontSpec::with_size(self.head_size(metrics)), color)
            .with_family(metrics.theme.pinyin_family.as_ref())
    }

    /// 韵母那行的字号（点）：跟 `mapping` 一致。
    fn head_size(&self, metrics: &Metrics) -> f32 {
        self.mapping.size / metrics.scale.max(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试里用到的格子边长（点）：随便一个值，尺寸公式与具体边长无关。
    const CELL: f32 = 46.0;

    fn cap(key: &str) -> KeyCap {
        KeyCap {
            key: key.to_owned(),
            initial: String::new(),
            mapping: String::new(),
        }
    }

    fn keyboard(zero: usize) -> KeyMapLayout {
        KeyMapLayout {
            rows: vec![
                (0..10).map(|_| cap("Q")).collect(),
                (0..10).map(|_| cap("A")).collect(),
                (0..7).map(|_| cap("Z")).collect(),
            ],
            zero_initials: (0..zero).map(|_| cap("AA")).collect(),
        }
    }

    /// 邻居之间只隔一条线（跨度 = 格子边长 × 个数 − 共用的那条线）。
    #[test]
    fn neighbours_share_a_single_line() {
        assert_eq!(KeyMapLayout::span(1, CELL), CELL, "一格就一条边");
        assert_eq!(KeyMapLayout::span(10, CELL), 10.0 * CELL - 9.0 * LINE);
        // 步距 = 边长 − 线宽：两格摆出来正好共用一条线
        assert_eq!(KeyMapLayout::pitch(CELL), CELL - LINE);
    }

    /// 格子里的字号按格子等比放大：46 点的格子还是原来的 7 / 11 点，格子翻倍字也翻倍。
    #[test]
    fn the_fonts_scale_with_the_cell() {
        let theme = Theme::new();
        let metrics = Metrics {
            theme: &theme,
            scale: 1.0,
        };
        let small = KeyMapStyles::new(&metrics, CELL);
        let large = KeyMapStyles::new(&metrics, CELL * 2.0);
        let close = |value: f32, wanted: f32| (value - wanted).abs() < 0.01;
        assert!(close(small.key.size, 7.0), "46 点的格子里是 7 点");
        assert!(close(small.mapping.size, 11.0), "46 点的格子里是 11 点");
        assert!(
            close(small.initial.size, 11.0),
            "右上角声母与中间的韵母一样大"
        );
        assert!(close(large.key.size, 14.0), "格子翻倍字也翻倍");
        assert!(close(large.mapping.size, 22.0));
        assert!(close(large.initial.size, 22.0));
    }

    /// 韵母首字母的颜色：a 红 / o 橙 / e 绿 / i 蓝 / u 紫 / ü 棕，其余（以及数字、符号）不上色。
    #[test]
    fn the_leading_vowel_has_its_own_colour() {
        assert_eq!(head_color('a'), Some(Color::rgb(0xF4, 0x20, 0x20)));
        assert_eq!(head_color('o'), Some(Color::rgb(0xFF, 0x80, 0x40)));
        assert_eq!(head_color('e'), Some(Color::rgb(0x00, 0xA8, 0x69)));
        assert_eq!(head_color('i'), Some(Color::rgb(0x48, 0xAB, 0xF8)));
        assert_eq!(head_color('u'), Some(Color::rgb(0x8D, 0x3C, 0xE8)));
        assert_eq!(head_color('ü'), Some(INITIAL_COLOR), "ü 与声母同一个棕");
        assert_eq!(head_color('b'), None, "别的字母照常是黑的");
    }

    /// 目标高度 → 格子边长：三排键盘 + 零声母表两行（每行一格的三分之一；共用线、中间隔 TABLE_GAP）
    /// 加两边留白 = 目标高度。
    #[test]
    fn the_cell_fills_the_target_height() {
        let target = 300.0;
        let cell = key_map_cell(target);
        let height = key_map_size(&keyboard(12), cell).1;
        assert!(
            (height - target).abs() < 0.01,
            "三排 + 表两行 + 留白正好等于候选窗 9 个候选项那个高度（{height} vs {target}）"
        );
        assert_eq!(key_map_cell(10.0), MIN_CELL, "太矮就按下限");
    }

    /// 高度恒定（零声母表也是 6 列 × 两行）；宽度 = 留白 + max(键盘含错开, 零声母表)。
    #[test]
    fn the_height_is_constant_and_the_width_follows_the_widest_row() {
        let cell = 46.0;
        let constant_height = PADDING * 2.0
            + KeyMapLayout::span(3, cell)
            + TABLE_GAP
            + keyboard(1).table_height(cell);
        // 键盘那块的宽：最宽的是第二排（10 格 + 它错开的那半格）
        let keyboard_width = KeyMapLayout::span(10, cell) + KeyMapLayout::stagger(cell);
        assert_eq!(
            key_map_size(&keyboard(0), cell),
            (
                PADDING * 2.0 + keyboard_width,
                PADDING * 2.0 + KeyMapLayout::span(3, cell)
            ),
            "没有零声母表就矮一块（宽度是键盘的）"
        );
        for zero in [6, 12] {
            let size = key_map_size(&keyboard(zero), cell);
            assert_eq!(size.1, constant_height, "有零声母表时高度恒定");
            assert_eq!(
                size.0,
                PADDING * 2.0 + keyboard_width,
                "6 格的零声母表（每格 4/3 宽）仍比键盘窄：宽度还是键盘那句"
            );
        }
    }

    /// 零声母表项数不到 6 也占一整行的宽度；零声母表比键盘宽时按表格算（这里键盘只有一格）。
    #[test]
    fn a_short_zero_table_still_keeps_its_width() {
        let layout = KeyMapLayout {
            rows: vec![vec![cap("Q")]],
            zero_initials: (0..5).map(|_| cap("AA")).collect(),
        };
        assert_eq!(
            layout.table_rows(),
            TABLE_ROWS,
            "有表就固定两行，高度才恒定"
        );
        let (width, height) = key_map_size(&layout, CELL);
        assert_eq!(
            height,
            PADDING * 2.0 + CELL + TABLE_GAP + layout.table_height(CELL),
            "一排键盘 + 零声母表两行（每行一格的三分之一高）"
        );
        assert_eq!(
            width,
            PADDING * 2.0 + layout.table_width(CELL),
            "零声母表 6 格（每格 4/3 宽）比一格键盘宽，按它算"
        );
        let empty = KeyMapLayout::default();
        assert_eq!(
            key_map_size(&empty, CELL),
            (PADDING * 2.0 + CELL, PADDING * 2.0 + CELL)
        );
    }
}
