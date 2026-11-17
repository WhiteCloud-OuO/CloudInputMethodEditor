//! 组字工具自己的配置：`%APPDATA%\CloudIME\ids.toml`。
//!
//! 只认三项：`[font] family`（部件与画布用的字体）、`show_typeable`
//! （部件抽屉是否列出「能用拼音打出」的部件）与 `favorites`（收藏的部件串）。
//! 读不到就用缺省值；收藏 / 开关变化时整份重写。

use std::path::PathBuf;

/// 缺省字体：系统 UI 字体的 GDI 链接不覆盖 IDS 运算符。
pub const DEFAULT_FAMILY: &str = "Microsoft YaHei UI";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub font_family: String,
    pub show_typeable: bool,
    pub favorites: Vec<char>,
    pub aliases: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: DEFAULT_FAMILY.to_owned(),
            show_typeable: false,
            favorites: Vec::new(),
            aliases: false,
        }
    }
}

/// 读用户配置；文件不在 / 读不了就用缺省值。
pub fn load() -> Config {
    let Some(path) = path() else {
        return Config::default();
    };
    std::fs::read_to_string(path).map_or_else(|_| Config::default(), |text| parse(&text))
}

/// 写回配置；目录不存在会建。返回给用户看的原因。
pub fn save(config: &Config) -> Result<(), String> {
    let path = path().ok_or_else(|| "找不到 %APPDATA%，无法保存 ids.toml".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建 {} 失败：{error}", parent.display()))?;
    }
    let favorites: String = config.favorites.iter().collect();
    let text = format!(
        "[font]\nfamily = \"{}\"\nshow_typeable = {}\nfavorites = \"{}\"\naliases = {}\n",
        config.font_family.replace('"', ""),
        config.show_typeable,
        favorites.replace('"', ""),
        config.aliases
    );
    std::fs::write(&path, text).map_err(|error| format!("写 {} 失败：{error}", path.display()))
}

fn path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("CloudIME").join("ids.toml"))
}

/// 极简 TOML：只读 `[font]` 段里的 `family` 和顶层的 `show_typeable` / `favorites`。
pub fn parse(text: &str) -> Config {
    let mut config = Config::default();
    let mut in_font = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_font = line == "[font]";
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        match (in_font, key) {
            (true, "family") if !value.is_empty() => config.font_family = value.to_owned(),
            (_, "show_typeable") => config.show_typeable = value.eq_ignore_ascii_case("true"),
            (_, "aliases") => config.aliases = value.eq_ignore_ascii_case("true"),
            (_, "favorites") => {
                config.favorites = value.chars().collect();
                config.favorites.dedup();
            }
            _ => {}
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_font_typeable_and_favorites() {
        let config = parse(
            "[font]\nfamily = \"SimSun\"\nshow_typeable = true\nfavorites = \"𠂇木\"\naliases = true\n",
        );
        assert_eq!(config.font_family, "SimSun");
        assert!(config.show_typeable);
        assert!(config.aliases);
        assert_eq!(config.favorites, vec!['𠂇', '木']);
    }

    #[test]
    fn ignores_unknown_keys_and_keeps_defaults() {
        let config = parse("# 注释\n[other]\nfamily = \"X\"\n");
        assert_eq!(config, Config::default());
    }
}
