//! 字形探测：某个字体族有没有某个字符的字形。
//!
//! 探测走 DirectWrite 的 `IDWriteFont::HasCharacter`：GDI 的 `GetGlyphIndicesW`
//! 不支持非 BMP 字符，扩展区部件会全部误判成缺字。

use std::collections::HashMap;

/// 结构符号（`⿰` `⿱` 这类）优先用的字体族，按顺序找第一个装了的。
///
/// 霞鹜文楷（LXGW WenKai）把 IDS 运算符画得比系统界面字体清楚；没装就退回配置里的字体。
pub const STRUCTURE_FAMILIES: &[&str] = &[
    "LXGW WenKai",
    "霞鹜文楷",
    "LXGW WenKai Screen",
    "霞鹜文楷 Screen",
    "LXGW WenKai GB",
    "霞鹜文楷 GB",
    "LXGW WenKai TC",
    "霞鹜文楷 TC",
];

/// 本机装了的第一个字体族（按 `families` 顺序）；一个都没装返回 `None`。
pub fn installed_family(families: &[&str]) -> Option<String> {
    families
        .iter()
        .copied()
        .find(|family| family_exists(family))
        .map(str::to_owned)
}

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

    /// 有没有字形；拿不到字体信息时按「有」处理，避免把整屏部件都改成码位。
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

/// DirectWrite 的系统字体集里有没有这个字族；`installed_family` 用。
#[cfg(windows)]
fn family_exists(family: &str) -> bool {
    use windows::Win32::Graphics::DirectWrite::{
        DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory,
    };
    use windows::core::{BOOL, PCWSTR};

    unsafe {
        let Ok(factory) = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) else {
            return false;
        };
        let mut collection = None;
        if factory
            .GetSystemFontCollection(&mut collection, false)
            .is_err()
        {
            return false;
        }
        let Some(collection) = collection else {
            return false;
        };
        let name: Vec<u16> = family.encode_utf16().chain(std::iter::once(0)).collect();
        let mut index = 0u32;
        let mut exists = BOOL(0);
        collection
            .FindFamilyName(PCWSTR(name.as_ptr()), &mut index, &mut exists)
            .is_ok()
            && exists.as_bool()
    }
}

#[cfg(not(windows))]
fn family_exists(_family: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn unknown_family_is_not_installed() {
        assert_eq!(
            super::installed_family(&["__CloudIME No Such Font__"]),
            None
        );
    }
}
