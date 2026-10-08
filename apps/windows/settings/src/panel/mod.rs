//! 设置窗口根组件：左侧导航 + 右侧当前分节页。每改一项就原地写回 `config.toml`（保留注释）再重读，
//! 界面始终反映文件内容；Server 每秒看 mtime 热加载。
//! 状态在这里，消息在 [`message`]，生命周期在 [`component`]，表单零件在 [`controls`]，各页在 [`pages`]。

mod component;
mod controls;
mod font_dialog;
mod message;
mod notepad;
mod notice;
mod pages;

use std::path::{Path, PathBuf};

use cloudime_platform::{Config, FontChoice, ThemeFile};
use windows_reactor::*;

use crate::color_dialog::ColorDialog;

pub(crate) use self::message::Message;
use self::notice::Notice;
use self::pages::candidates::FontRole;
use self::pages::phrase::PhraseForm;
use self::pages::theme::Slot;
use self::pages::{candidates, debugging, dictionaries, input, phrase, scripts, theme, translate};

/// CLOUDIME_VERSION 由 build.rs 给：-dev 版接 git 短哈希。
pub(crate) const VERSION: &str = env!("CLOUDIME_VERSION");

/// 项目 GitHub 仓库。
pub(crate) const REPOSITORY_URL: &str = "https://github.com/WhiteCloud-OuO/CloudInputMethodEditor";

/// 左侧标签列的下限宽度，让短标签行的控件对齐；超长标签会把本行控件往右顶
/// （见 `controls::labeled`）。
const LABEL_WIDTH: f64 = 140.0;

/// 由「新建名字框 + 当前选中的主题」定出要写的主题名（去 `.json`）。
///
/// 名字框空着就沿用选中的那份（改色保存）；都没有、或者与随包两份**内置主题**同名（`Default` /
/// `Panic`，大小写不敏感、带 `.json` 也认）则返回给用户看的原因 —— 内置那两份不该被用户目录里同名的盖掉。
fn theme_stem(typed: &str, selected: Option<&str>) -> Result<String, String> {
    let raw = if typed.trim().is_empty() {
        selected.unwrap_or_default()
    } else {
        typed
    };
    let stem = raw.trim().trim_end_matches(".json").trim();
    if stem.is_empty() {
        return Err("先给主题起个名字。".to_owned());
    }
    if stem.eq_ignore_ascii_case("default") || stem.eq_ignore_ascii_case("panic") {
        return Err("「Default」「Panic」是随包内置主题的名字，换一个。".to_owned());
    }
    Ok(stem.to_owned())
}

/// 设置窗口状态。
pub(crate) struct Settings {
    /// 当前配置，每次改动后从盘上重读。
    pub(super) config: Config,

    /// `config.toml` 路径。
    path: PathBuf,

    /// 当前导航分节 tag。
    page: String,

    /// 页面底部的临时提示（导入统计 / 失败原因）。
    notice: Notice,

    /// 最近一次词库操作的结果，显示在词库页。
    dictionary_status: String,

    /// 「不显示候选框」程序名单的输入框里正在敲的名字；`None` 显示空。
    program_query: Option<String>,

    /// 短语库里的短语（打开设置时读一次，每次增删改后更新）。
    phrases: Vec<cloudime_core::CustomPhrase>,

    /// 短语页表单里正在编辑的内容。
    phrase_form: PhraseForm,

    /// 正在编辑第几条短语（`None` 是新增）。
    phrase_edit: Option<usize>,

    /// 最近一次短语操作的结果，显示在短语页。
    phrase_status: String,

    /// 最近一次脚本操作的结果，显示在脚本页（新建 / 删除 / 改名）。
    script_status: String,

    /// 「主题」页：可选主题名（去 `.json`）。
    theme_names: Vec<String>,

    /// 「主题」页选中的主题名（去 `.json`）。
    theme_selected: Option<String>,

    /// 「主题」页正在编辑的草稿；点「确认保存」才落盘。
    theme_draft: ThemeFile,

    /// 「主题」页「新建主题」名字框里的字。
    theme_new_name: String,

    /// 「主题」页那一行状态提示（保存结果 / 校验失败）。
    theme_status: String,

    /// 改颜色用的对话框。
    color_dialog: ColorDialog,

    /// 颜色对话框现在改的是哪个槽。
    color_slot: Option<Slot>,
}

impl Settings {
    /// `%APPDATA%\CloudIME\config.toml`；取不到 `APPDATA` 退回工作目录。
    fn config_path() -> PathBuf {
        cloudime_platform::dirs::config_path().unwrap_or_else(|| PathBuf::from("config.toml"))
    }

    /// 配置文件不在就写出模板：这个账户下 Server 还没跑过时，打开数据目录前也要有文件。
    fn ensure_config_file(path: &Path) {
        if let Err(error) = Config::write_template_if_missing(path) {
            crate::log::warn(format!("写配置模板失败: {error}"));
        }
    }

    /// 数据目录 `%APPDATA%\CloudIME`。
    fn data_dir(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// 落盘一个配置值再重读。失败只打印。
    fn save(&mut self, section: &str, key: &str, value: impl Into<toml_edit::Value>) {
        if let Err(error) = Config::set_value(&self.path, section, key, value) {
            crate::log::warn(format!("保存 [{section}] {key} 失败: {error}"));
            return;
        }
        self.reload();
    }

    /// 落盘一个字符串数组再重读。
    fn save_array(&mut self, section: &str, key: &str, values: &[String]) {
        if let Err(error) = Config::set_array(&self.path, section, key, values) {
            crate::log::warn(format!("保存 [{section}] {key} 失败: {error}"));
            return;
        }
        self.reload();
    }

    /// 落盘一个字体项：`[candidate]` 下的 `pinyin_font` / `candidate_font` / `item_number_font` 两个键。
    fn save_font(&mut self, role: FontRole, choice: &FontChoice) {
        let section = match role {
            FontRole::Pinyin => "candidate.pinyin_font",
            FontRole::Candidate => "candidate.candidate_font",
            FontRole::ItemNumber => "candidate.item_number_font",
            FontRole::Translate => "candidate.translate_font",
        };
        let family = choice.family.trim();
        if let Err(error) = Config::set_value(&self.path, section, "family", family) {
            crate::log::warn(format!("保存 {section} 的字族失败: {error}"));
            return;
        }
        let size = f64::from(choice.size.max(1.0));
        if let Err(error) = Config::set_value(&self.path, section, "size", size) {
            crate::log::warn(format!("保存 {section} 的字号失败: {error}"));
            return;
        }
        self.reload();
    }

    fn reload(&mut self) {
        if let Ok(config) = Config::load(&self.path) {
            self.config = config;
        }
    }

    /// 重扫 `Themes\` 两个目录，刷新「主题」页的可选列表（选中的没了就清掉）。
    fn refresh_theme_names(&mut self) {
        let root = cloudime_platform::resources::bundled_root();
        self.theme_names = cloudime_platform::list_themes(root.as_deref())
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        if self
            .theme_selected
            .as_ref()
            .is_some_and(|name| !self.theme_names.contains(name))
        {
            self.theme_selected = None;
        }
    }

    /// 选中某个主题：读进草稿，清掉「新建」名字框。
    fn select_theme(&mut self, name: &str) {
        self.theme_new_name.clear();
        self.theme_selected = Some(name.to_owned());
        let root = cloudime_platform::resources::bundled_root();
        match cloudime_platform::find_theme(root.as_deref(), name) {
            Some(entry) => match ThemeFile::load(&entry.path) {
                Ok(theme) => {
                    self.theme_draft = theme;
                    self.theme_status = format!("已载入 {}。", entry.path.display());
                }
                Err(error) => {
                    self.theme_status = format!("读 {} 失败：{error}", entry.path.display());
                }
            },
            None => self.theme_status = format!("找不到主题「{name}」。"),
        }
    }

    /// 软件默认主题（`Themes\default.json`，找不到就用内置缺省）：「新建主题」拿它当底。
    fn default_theme_draft() -> ThemeFile {
        let root = cloudime_platform::resources::bundled_root();
        cloudime_platform::find_theme(root.as_deref(), cloudime_platform::DEFAULT_THEME_FILE)
            .and_then(|entry| ThemeFile::load(&entry.path).ok())
            .unwrap_or_default()
    }

    /// 「确认保存」：草稿写到用户目录 `%APPDATA%\CloudIME\Themes\<名字>.json`。
    /// **只存不换** —— 要让三个窗口换上它，还得点「应用主题」。
    fn save_theme(&mut self) {
        let stem = match theme_stem(&self.theme_new_name, self.theme_selected.as_deref()) {
            Ok(stem) => stem,
            Err(reason) => {
                self.theme_status = reason;
                return;
            }
        };
        let Some(dir) =
            cloudime_platform::dirs::user_dir().map(|dir| dir.join(cloudime_platform::THEMES_DIR))
        else {
            self.theme_status = "找不到用户目录（%APPDATA%），主题存不了。".to_owned();
            return;
        };
        let path = dir.join(format!("{stem}.json"));
        if let Err(error) = self.theme_draft.save(&path) {
            self.theme_status = format!("写主题文件失败：{error}");
            return;
        }
        self.theme_new_name.clear();
        self.theme_selected = Some(stem.clone());
        self.refresh_theme_names();
        self.theme_status = format!(
            "已保存到 {}；点「应用主题」让三个窗口换上。",
            path.display()
        );
    }

    /// 「应用主题」：把选中的主题写进 `[theme] curr_theme`（同值也写）—— mtime 变一下，
    /// Server 的热加载就会重读主题、三个窗口一起换。
    fn apply_theme(&mut self) {
        let Some(name) = self.theme_selected.clone() else {
            self.theme_status =
                "先在「选择主题」里选一份，或保存一份新的，再点「应用主题」。".to_owned();
            return;
        };
        let value = format!("{name}.json");
        self.save("theme", "curr_theme", value.as_str());
        self.theme_status = format!("已应用「{name}」，三个窗口一秒内换上。");
    }

    fn page_content(&self, context: &mut ViewContext<Self>) -> View {
        match self.page.as_str() {
            "candidates" => candidates::view(self, context),
            "dictionaries" => dictionaries::view(self, context),
            "phrase" => phrase::view(self, context),
            "scripts" => scripts::view(self, context),
            "theme" => theme::view(self, context),
            "translate" => translate::view(self, context),
            "debugging" => debugging::view(self, context),
            _ => input::view(self, context),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_typed_name_wins_and_drops_the_json_suffix() {
        assert_eq!(
            theme_stem("my-theme.json", Some("default")),
            Ok("my-theme".to_owned())
        );
    }

    #[test]
    fn an_empty_name_falls_back_to_the_selected_theme() {
        assert_eq!(theme_stem("  ", Some("custom")), Ok("custom".to_owned()));
        assert_eq!(theme_stem("", None), Err("先给主题起个名字。".to_owned()));
    }

    #[test]
    fn the_reserved_names_are_rejected() {
        for name in ["default", "default.json", "Default", "panic", "PANIC.json"] {
            assert!(theme_stem(name, None).is_err(), "{name} 应该被拒");
        }
    }
}
