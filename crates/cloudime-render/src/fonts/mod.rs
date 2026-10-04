//! 字体库：不扫系统字体目录（fontdb 全扫几百毫秒、几十 MB），按清单只加载界面字体、中文、日文、emoji 几个文件。
//!
//! 文件走 fontdb 的 mmap 加载，只解析名字表与 cmap，Apple Color Emoji 那种 190 MB 的文件也只在用到字形时才读页。
//! 中日同形字按 locale 回退（cosmic-text 的平台回退表：zh-CN → PingFang SC，ja → Hiragino Sans）。

#[cfg(target_os = "windows")]
pub mod directwrite;
mod trak;
mod ui_font;
#[cfg(target_os = "windows")]
mod windows;

use std::path::{Path, PathBuf};

use cosmic_text::FontSystem;
use cosmic_text::fontdb::{Database, Family};

use crate::error::RenderError;

pub(crate) use trak::Trak;
pub use ui_font::UiFont;

#[cfg(target_os = "windows")]
use self::windows as platform;

pub struct FontLibrary {
    /// 已加载的字体。
    db: Database,

    /// 界面字体的字族名（`SansSerif` 映射到它）。
    ui_family: String,

    /// 中日同形字回退用的 locale，如 `zh-CN`。
    locale: String,
}

impl FontLibrary {
    /// 按清单加载系统字体。`locale` 决定 Han 字形选哪家（`zh-CN` / `zh-TW` / `ja`）。
    pub fn system(locale: &str) -> Result<Self, RenderError> {
        Self::build(locale, &[])
    }

    /// 界面字体换成用户指定的字族，系统字体仍加载在后面当回退；指定的字体一个文件都没加载到或名字对不上就退回系统字体。
    pub fn with_ui_font(locale: &str, ui_font: &UiFont) -> Result<Self, RenderError> {
        Self::build(locale, std::slice::from_ref(ui_font))
    }

    /// 按配置里的几个字族建库：`fonts[0]` 当界面字体（`SansSerif` 映射到它），其余也加载好、文本样式按名字用；
    /// 每个字族都解析不到时退回系统字体。`fonts` 为空等同于 [`Self::system`]。
    pub fn with_fonts(locale: &str, fonts: &[UiFont]) -> Result<Self, RenderError> {
        Self::build(locale, fonts)
    }

    fn build(locale: &str, fonts: &[UiFont]) -> Result<Self, RenderError> {
        let mut db = Database::new();
        // 逐个字族加载它的文件，记下谁真的可用（文件名对不上 / 一个都没加载到就是不可用）
        let mut custom_family = None;
        for font in fonts {
            let loaded = font.files.iter().filter(|path| load(&mut db, path)).count();
            let found = db.faces().any(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case(&font.family))
            });
            if loaded > 0 && found {
                if custom_family.is_none() {
                    custom_family = Some(font.family.clone());
                }
            } else {
                tracing::warn!(
                    family = font.family,
                    files = font.files.len(),
                    loaded,
                    "指定的候选窗字体没找到，用系统字体"
                );
            }
        }
        let ui =
            load_first(&mut db, &platform::ui_fonts()).ok_or_else(|| RenderError::NoUiFont {
                tried: platform::ui_fonts(),
            })?;
        let ui_family = custom_family.unwrap_or_else(|| {
            db.face(ui)
                .and_then(|face| face.families.first().map(|(name, _)| name.clone()))
                .unwrap_or_else(|| "sans-serif".to_owned())
        });
        db.set_sans_serif_family(ui_family.clone());
        for path in platform::script_fonts(locale)
            .into_iter()
            .chain(platform::emoji_fonts())
        {
            if !load(&mut db, &path) {
                tracing::debug!(path = %path.display(), "字体文件不存在，跳过");
            }
        }
        tracing::debug!(faces = db.len(), ui_family, "字体库就绪");
        Ok(Self {
            db,
            ui_family,
            locale: locale.to_owned(),
        })
    }

    /// 已加载的字族名，按加载顺序去重。
    pub fn families(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for face in self.db.faces() {
            for (name, _) in &face.families {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
        }
        names
    }

    pub fn ui_family(&self) -> &str {
        &self.ui_family
    }

    /// 交给 cosmic-text。
    pub(crate) fn into_font_system(self) -> FontSystem {
        let mut db = self.db;
        db.set_sans_serif_family(self.ui_family);
        FontSystem::new_with_locale_and_db(self.locale, db)
    }
}

/// 界面字体用的字族。
pub(crate) const UI_FAMILY: Family<'static> = Family::SansSerif;

/// 依次尝试，加载成功的第一个文件的第一张面。
fn load_first(db: &mut Database, paths: &[PathBuf]) -> Option<cosmic_text::fontdb::ID> {
    paths.iter().find_map(|path| {
        let before: Vec<_> = db.faces().map(|f| f.id).collect();
        load(db, path)
            .then(|| db.faces().map(|f| f.id).find(|id| !before.contains(id)))
            .flatten()
    })
}

/// 文件存在且能解析就加载；返回是否加载了。
fn load(db: &mut Database, path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    match db.load_font_file(path) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "字体文件解析失败");
            false
        }
    }
}
