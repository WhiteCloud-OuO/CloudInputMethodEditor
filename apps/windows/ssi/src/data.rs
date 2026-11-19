//! 字符页数据：`assets/pages.tsv` / `glyphs.tsv` / `styles.tsv`，编译期嵌进 exe。
//!
//! 数据来自旧 VFB 工程（`legacy/forms/Form2..Form8.frm`）：前 6 页 546 个字符的
//! 原始坐标（28×28 按钮、步长 31，分组靠坐标空开），第 7 页是 14 种字母样式。

/// 一个符号按钮：旧工程里的左上角坐标 + 要注入的文本（个别是两字，如「浊゛」）。
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub x: f32,
    pub y: f32,
    pub text: String,
}

/// 一页符号。
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub title: String,
    pub glyphs: Vec<Glyph>,
}

/// 所有页面共用的坐标框（旧工程坐标，单位是 VFB 的像素）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridBox {
    pub min_x: f32,
    pub min_y: f32,
    pub width: f32,
    pub height: f32,
}

const PAGES: &str = include_str!("../assets/pages.tsv");
const GLYPHS: &str = include_str!("../assets/glyphs.tsv");
const STYLES: &str = include_str!("../assets/styles.tsv");

/// 读三个 TSV；格式不对的行直接跳过（数据是编译期固定的，测试会盯着数量）。
pub fn load() -> Vec<Page> {
    let mut titles: Vec<(usize, String)> = PAGES
        .lines()
        .filter_map(|line| {
            let (index, title) = line.split_once('\t')?;
            Some((index.parse().ok()?, title.to_owned()))
        })
        .collect();
    titles.sort_by_key(|(index, _)| *index);

    let mut pages: Vec<Page> = titles
        .into_iter()
        .map(|(_, title)| Page {
            title,
            glyphs: Vec::new(),
        })
        .collect();

    for line in GLYPHS.lines() {
        let mut fields = line.splitn(4, '\t');
        let (Some(page), Some(x), Some(y), Some(text)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let (Ok(page), Ok(x), Ok(y)) = (page.parse::<usize>(), x.parse::<f32>(), y.parse::<f32>())
        else {
            continue;
        };
        if let Some(page) = pages.get_mut(page.wrapping_sub(1)) {
            page.glyphs.push(Glyph {
                x,
                y,
                text: text.to_owned(),
            });
        }
    }
    pages
}

/// 14 种字母样式，每种 52 个字符（A–Z + a–z）。
pub fn load_styles() -> Vec<Vec<char>> {
    let mut styles: Vec<Vec<char>> = Vec::new();
    for line in STYLES.lines() {
        let mut fields = line.split('\t');
        let Some(index) = fields.next().and_then(|text| text.parse::<usize>().ok()) else {
            continue;
        };
        let chars: Vec<char> = fields
            .filter_map(|text| u32::from_str_radix(text, 16).ok())
            .filter_map(char::from_u32)
            .collect();
        if chars.len() != 52 {
            continue;
        }
        while styles.len() < index {
            styles.push(Vec::new());
        }
        styles.push(chars);
    }
    styles
}

/// 所有页共用的坐标框：宽度取最右一列，高度取最下一行。
pub fn grid_box(pages: &[Page]) -> GridBox {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for glyph in pages.iter().flat_map(|page| &page.glyphs) {
        min_x = min_x.min(glyph.x);
        min_y = min_y.min(glyph.y);
        max_x = max_x.max(glyph.x + BUTTON);
        max_y = max_y.max(glyph.y + BUTTON);
    }
    if min_x > max_x {
        return GridBox {
            min_x: 0.0,
            min_y: 0.0,
            width: 1.0,
            height: 1.0,
        };
    }
    GridBox {
        min_x,
        min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    }
}

/// 旧工程里每个按钮的边长（像素）。
pub const BUTTON: f32 = 28.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_all_six_pages_and_546_glyphs() {
        let pages = load();
        assert_eq!(pages.len(), 6);
        let counts: Vec<usize> = pages.iter().map(|page| page.glyphs.len()).collect();
        assert_eq!(counts, vec![56, 111, 76, 100, 110, 93]);
        assert_eq!(pages[0].title, "常用符号");
        assert!(pages.iter().all(|page| !page.title.is_empty()));
        assert!(pages[0].glyphs.iter().all(|glyph| !glyph.text.is_empty()));
    }

    #[test]
    fn grid_box_covers_every_glyph() {
        let pages = load();
        let grid = grid_box(&pages);
        assert_eq!(grid.min_x, 4.0);
        assert_eq!(grid.min_y, 4.0);
        assert_eq!(grid.width, 456.0);
        assert_eq!(grid.height, 403.0);
    }

    #[test]
    fn reads_fourteen_alphabet_styles() {
        let styles = load_styles();
        assert_eq!(styles.len(), 14);
        assert!(styles.iter().all(|style| style.len() == 52));
        assert_eq!(styles[10][0], '𝐀');
        assert_eq!(styles[10][51], '𝐳');
    }
}
