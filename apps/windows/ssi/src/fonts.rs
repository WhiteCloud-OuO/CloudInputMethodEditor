//! 字体探测：某个字族本机装没装（制表符页优先用「宋体-PUA」）。

/// 本机装了的第一个字体族（按 `families` 顺序）；一个都没装返回 `None`。
pub fn installed_family(families: &[&str]) -> Option<String> {
    families
        .iter()
        .copied()
        .find(|family| family_exists(family))
        .map(str::to_owned)
}

/// DirectWrite 的系统字体集里有没有这个字族。
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
