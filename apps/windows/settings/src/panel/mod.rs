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

use cloudime_platform::{Config, FontChoice};
use windows_reactor::*;

pub(crate) use self::message::Message;
use self::notice::Notice;
use self::pages::candidates::FontRole;
use self::pages::phrase::PhraseForm;
use self::pages::{candidates, debugging, dictionaries, input, phrase, scripts, translate};

/// CLOUDIME_VERSION 由 build.rs 给：-dev 版接 git 短哈希。
pub(crate) const VERSION: &str = env!("CLOUDIME_VERSION");

/// 项目 GitHub 仓库。
pub(crate) const REPOSITORY_URL: &str = "https://github.com/WhiteCloud-OuO/CloudInputMethodEditor";

/// 左侧标签列的下限宽度，让短标签行的控件对齐；超长标签会把本行控件往右顶
/// （见 `controls::labeled`）。
const LABEL_WIDTH: f64 = 140.0;

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

    fn page_content(&self, context: &mut ViewContext<Self>) -> View {
        match self.page.as_str() {
            "candidates" => candidates::view(self, context),
            "dictionaries" => dictionaries::view(self, context),
            "phrase" => phrase::view(self, context),
            "scripts" => scripts::view(self, context),
            "translate" => translate::view(self, context),
            "debugging" => debugging::view(self, context),
            _ => input::view(self, context),
        }
    }
}
