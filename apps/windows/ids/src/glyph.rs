//! 字形探测与缺字兜底。
//!
//! 部件抽屉用系统字体画部件；字体里没有字形时，画它 IDS 的 ASCII 缩写
//! （`UD丿罒`），仍然不可用就退回 `U+XXXX`。
//!
//! 探测走 DirectWrite 的 `IDWriteFont::HasCharacter`：GDI 的 `GetGlyphIndicesW`
//! 不支持非 BMP 字符，扩展区部件会全部误判成缺字。

use std::collections::{HashMap, HashSet};

use cloudime_ids::{IdsNode, parse};

/// 探测某个字体族有没有某个字符的字形。
pub struct GlyphProbe {
    cached: HashMap<char, bool>,
    font: Option<windows::Win32::Graphics::DirectWrite::IDWriteFont>,
}

impl std::fmt::Debug for GlyphProbe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GlyphProbe")
            .field("cached", &self.cached.len())
            .field("has_font", &self.font.is_some())
            .finish()
    }
}

impl GlyphProbe {
    pub fn new(family: &str) -> Self {
        Self {
            cached: HashMap::new(),
            font: find_font(family),
        }
    }

    /// 有没有字形；拿不到字体信息时按「有」处理，避免把整屏部件都改成缩写。
    pub fn has_glyph(&mut self, character: char) -> bool {
        if let Some(value) = self.cached.get(&character) {
            return *value;
        }
        let value = match &self.font {
            Some(font) => unsafe { font.HasCharacter(character as u32) }
                .map(|exists| exists.as_bool())
                .unwrap_or(true),
            None => true,
        };
        self.cached.insert(character, value);
        value
    }
}

#[cfg(windows)]
fn find_font(family: &str) -> Option<windows::Win32::Graphics::DirectWrite::IDWriteFont> {
    use windows::Win32::Graphics::DirectWrite::{
        DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_WEIGHT_NORMAL, DWriteCreateFactory, IDWriteFactory,
    };
    use windows::core::{BOOL, PCWSTR};

    unsafe {
        let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
        let mut collection = None;
        factory
            .GetSystemFontCollection(&mut collection, false)
            .ok()?;
        let collection = collection?;
        let name: Vec<u16> = family.encode_utf16().chain(std::iter::once(0)).collect();
        let mut index = 0u32;
        let mut exists = BOOL(0);
        collection
            .FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists)
            .ok()?;
        if !exists.as_bool() {
            return None;
        }
        let family = collection.GetFontFamily(index).ok()?;
        family
            .GetFirstMatchingFont(
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
            )
            .ok()
    }
}

#[cfg(not(windows))]
fn find_font(_family: &str) -> Option<()> {
    None
}

/// 把 IDS 写成 ASCII 缩写，叶子缺字形时递归展开（最多 `depth` 层）。
pub fn abbreviation(
    ids: &str,
    depth: usize,
    component_ids: &HashMap<char, String>,
    has_glyph: &mut dyn FnMut(char) -> bool,
) -> String {
    let Ok(node) = parse(ids) else {
        return String::new();
    };
    let mut visited = HashSet::new();
    let mut text = String::new();
    write_abbreviation(
        &node,
        depth,
        component_ids,
        has_glyph,
        &mut visited,
        &mut text,
    );
    let count = text.chars().count();
    if count > 10 {
        let mut short: String = text.chars().take(9).collect();
        short.push('…');
        return short;
    }
    text
}

fn write_abbreviation(
    node: &IdsNode,
    depth: usize,
    component_ids: &HashMap<char, String>,
    has_glyph: &mut dyn FnMut(char) -> bool,
    visited: &mut HashSet<char>,
    out: &mut String,
) {
    match node {
        IdsNode::Leaf(character) => {
            if has_glyph(*character) {
                out.push(*character);
                return;
            }
            if depth > 0
                && visited.insert(*character)
                && let Some(ids) = component_ids.get(character)
                && let Ok(tree) = parse(ids)
            {
                let mut nested = String::new();
                write_abbreviation(
                    &tree,
                    depth - 1,
                    component_ids,
                    has_glyph,
                    visited,
                    &mut nested,
                );
                if !nested.is_empty() {
                    out.push_str(&nested);
                    return;
                }
            }
            out.push('?');
        }
        IdsNode::Unencoded(number) => {
            out.push('{');
            out.push_str(&number.to_string());
            out.push('}');
        }
        IdsNode::Wildcard => out.push('?'),
        IdsNode::Op { op, children } => {
            out.push_str(op.abbreviation());
            for child in children {
                write_abbreviation(child, depth, component_ids, has_glyph, visited, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available(set: &'static str) -> impl FnMut(char) -> bool {
        move |character| set.contains(character)
    }

    #[test]
    fn abbreviates_ids_with_ascii_operators() {
        let mut has_glyph = available("");
        assert_eq!(
            abbreviation("⿱丿罒", 2, &HashMap::new(), &mut has_glyph),
            "UD??"
        );
        let mut has_glyph = available("丿罒");
        assert_eq!(
            abbreviation("⿱丿罒", 2, &HashMap::new(), &mut has_glyph),
            "UD丿罒"
        );
    }

    #[test]
    fn missing_leaves_expand_through_known_components() {
        let mut map = HashMap::new();
        map.insert('木', "⿻十人".to_owned());
        let mut has_glyph = available("十人");
        assert_eq!(abbreviation("木", 2, &map, &mut has_glyph), "XX十人");
    }

    #[test]
    fn long_abbreviations_are_truncated() {
        let mut has_glyph = available("");
        let text = abbreviation("⿰⿰⿰⿰一二三四五", 0, &HashMap::new(), &mut has_glyph);
        assert!(text.chars().count() <= 10);
        assert!(text.ends_with('…'));
    }
}
