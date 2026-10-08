use std::path::PathBuf;

use cloudime_platform::{
    Config, ItemNumberStyle, LayoutMode, MouseWordSelection, PreeditMode, SimpTrad,
};

use super::RenderSettings;

/// Router 要用的配置项。
#[derive(Debug, Clone, PartialEq)]
pub struct RouterConfig {
    /// 候选项个数（`[candidate] candidate_count`，5–9）。
    pub page_size: usize,

    /// 候选排布（`[candidate] candidate_arrangement_direction`）。
    pub layout: LayoutMode,

    /// 拼音串字体：字族名（空为系统界面字体）+ 字号（点）。
    pub pinyin_font: (String, f32),

    /// 候选词字体。
    pub candidate_font: (String, f32),

    /// 序号字体。
    pub item_number_font: (String, f32),

    /// 翻译 Tip 的字体（`[candidate] translate_font`）。
    pub translate_font: (String, f32),
    /// 序号的写法（`[candidate] item_number_style`）。
    pub item_number_style: ItemNumberStyle,

    /// 候选窗口的最小宽度（`[candidate] candidate_box_minimum_width`，物理像素）：竖排、横排都生效。
    pub min_width_pixels: f32,

    /// 展开（「展示更多候选项」）成网格时每格的最大宽度
    /// （`[candidate] candidate_item_maximum_width`，点；`0` 表示不限）。
    pub max_cell_width: f32,

    /// 拼音显示位置（`[candidate] preedit`）。
    pub preedit: PreeditMode,

    /// 「展示更多候选项」（`[candidate] show_more_candidate_items`）：组句里 Tab 把候选窗展开成
    /// 一屏（竖排 5 列 / 横排 5 行）；关掉时 Tab 吃掉但什么也不做。
    pub show_more_candidate_items: bool,

    /// 使用鼠标选词（`[candidate] mouse_word_selection`）：鼠标能不能在候选窗里悬停 / 点选候选。
    /// `off` 一律不能（缺省）/ `more_candidates` 只有「展示更多候选项」展开成网格时能 / `always` 都能。
    pub mouse_word_selection: MouseWordSelection,

    /// 当前主题的文件名（`[theme] curr_theme`，`Themes\` 下）。
    pub curr_theme: String,

    /// 启用翻译 Tip（`[translate] enabled`）。
    pub translate_enabled: bool,

    /// 选中的本地词典文件名（`[translate] dictionary`）。
    pub translate_dictionary: String,

    /// 学会所需上屏次数（`[translate] need_times`，3–10）。
    pub translate_need_times: u32,
    /// 「重置学习内容」的次数（`[translate] reset_counter`）：变了就清空该词典的学习记录。
    pub translate_reset_counter: u64,

    /// 在「不显示候选框」名单里的程序（`[candidate] program_list_of_hiding_candidate`，exe 文件名）。
    pub hiding_candidates: Vec<String>,

    /// 中文模式下不在组句时的标点转全角；不进配置文件，状态条 / 右键菜单可切，会话内有效、重启回缺省（开）。
    /// 初值来自 `[input] full_half_punctuation_marks_toggle`（follow 中文全角、英文半角）。
    pub full_width_punctuation: bool,

    /// 英文模式的那一份，中英各记一份；会话内有效、重启回缺省（半角）。
    pub english_full_width_punctuation: bool,

    /// 直通给应用的可打印 ASCII 转全角（状态条上的「全角 / 半角」开关）；同标点，会话内有效、重启回缺省（半角）。
    pub full_width_chars: bool,

    /// 繁体输出（`[input] simp_trad_chinese_chars_toggle`）；状态条「简 / 繁」按钮与设置页共用这一份。
    pub traditional: bool,

    /// 中文模式的符号映射（`[input] punctuation_marks_mapping`）。
    pub punctuation_mapping: cloudime_core::Mapping,

    /// 符号成对补全的位图（`[input] punctuation_marks_pairwise_completion`）。
    pub pairwise_completion: u32,

    /// 状态条记住的位置（`[status_bar] x` / `y`，内容左上角物理像素）。
    pub status_pos: Option<(i32, i32)>,

    /// 在屏幕上显示悬浮工具栏（`[status_bar] show_status_bar`）：关掉后桌面上不再出现那条工具条。
    pub show_status_bar: bool,

    /// 自动隐藏悬浮工具栏（`[debugging] auto_hide_float_tool_bar`）：前台全屏时收起。
    /// 切到别的输入法、云朵被禁用时始终收起，与这一项无关。
    pub auto_hide_float_tool_bar: bool,

    /// 不处于输入状态时自动禁用输入法（`[debugging] auto_disable_without_text_input`）。
    /// Server 只把它下发给 DLL，判断与动作都在 DLL（那边才拿得到 TSF 的焦点与上下文）。
    pub auto_disable_without_text_input: bool,

    /// 状态切换提示（`[input] show_status_change_tip`）：状态一变在光标附近弹一个 1 秒的提示条。
    pub show_status_change_tip: bool,

    /// 用户脚本目录（**安装目录**的 `Scripts\`，与 `Phrases\`、`WordBank\` 同级），`None` = 不加载脚本。
    ///
    /// **不放**在 [`RouterConfig::from`] 里解析：测试与 `RouterConfig::default()` 会去读开发机上
    /// 真实的脚本目录。只有 Server 正式跑时经 [`RouterConfig::with_bundled_scripts`] 指过来。
    pub scripts_dir: Option<PathBuf>,

    /// 禁用的脚本文件名（`[script] disabled`，大小写不敏感）：加载时跳过它们。
    pub script_disabled: Vec<String>,

    /// 脚本在本组句里要的候选窗**最小宽度**（`cloudime.candidate.set_min_width`，物理像素）：
    /// `None` = 用配置里的 `min_width_pixels`。**只在本次组句内有效**，组句结束清掉。
    pub script_min_width: Option<f32>,

    /// 脚本在本组句里要的**一页候选数**（`cloudime.candidate.set_page_size`，5–9）：
    /// `None` = 用配置里的 `page_size`。同样组句结束清掉。
    pub script_page_size: Option<usize>,

    /// 脚本在本组句里要的**缩放倍数**（`cloudime.candidate.set_scale`）：`None` = 按用户 `Ctrl + 滚轮`。
    pub script_scale: Option<f32>,
}

impl RouterConfig {
    /// 把脚本目录指到**安装目录**的 `Scripts\`（与 `Phrases\`、`WordBank\` 同级）。
    pub fn with_bundled_scripts(mut self) -> Self {
        self.scripts_dir = cloudime_platform::resources::bundled_root()
            .map(|root| root.join(cloudime_script::DIRECTORY));
        self
    }

    /// 交给 UI 线程的渲染设置。
    pub fn render_settings(&self) -> RenderSettings {
        RenderSettings {
            pinyin_font: self.pinyin_font.clone(),
            candidate_font: self.candidate_font.clone(),
            item_number_font: self.item_number_font.clone(),
            translate_font: self.translate_font.clone(),
            min_width_pixels: self.script_min_width.unwrap_or(self.min_width_pixels),
            item_number_style: self.item_number_style,
            scale: self.script_scale,
            max_cell_width: self.max_cell_width,
            theme: load_theme(&self.curr_theme),
        }
    }

    /// 这个程序在不在「不显示候选框」名单里。
    pub fn hides_candidate_for(&self, program: &str) -> bool {
        let program = program.trim();
        !program.is_empty()
            && self
                .hiding_candidates
                .iter()
                .any(|name| name.trim().eq_ignore_ascii_case(program))
    }
}

/// 主题文件的内容缓存：同一个文件、mtime 没变就直接用上次读的，省掉一次读盘 + JSON 解析。
/// （目录扫描还是每次都做 —— 它决定「用户目录盖过随包」的优先级。）
static THEME_CACHE: std::sync::Mutex<
    Option<(
        std::path::PathBuf,
        std::time::SystemTime,
        cloudime_platform::ThemeFile,
    )>,
> = std::sync::Mutex::new(None);

/// 读当前主题文件（用户目录优先）；找不到 / 读不动就用缺省主题。
fn load_theme(name: &str) -> cloudime_platform::ThemeFile {
    let root = cloudime_platform::resources::bundled_root();
    let Some(entry) = cloudime_platform::find_theme(root.as_deref(), name) else {
        return cloudime_platform::ThemeFile::default();
    };
    let modified = std::fs::metadata(&entry.path)
        .and_then(|meta| meta.modified())
        .ok();
    // 命中缓存：路径与 mtime 都没变
    if let Ok(cache) = THEME_CACHE.lock()
        && let Some((path, cached, theme)) = cache.as_ref()
        && *path == entry.path
        && Some(*cached) == modified
    {
        return *theme;
    }
    match cloudime_platform::ThemeFile::load(&entry.path) {
        Ok(theme) => {
            if let (Some(modified), Ok(mut cache)) = (modified, THEME_CACHE.lock()) {
                *cache = Some((entry.path, modified, theme));
            }
            theme
        }
        Err(error) => {
            tracing::warn!(%error, path = %entry.path.display(), "主题文件读不动，用缺省主题");
            cloudime_platform::ThemeFile::default()
        }
    }
}

impl From<&Config> for RouterConfig {
    fn from(config: &Config) -> Self {
        let input = &config.input;
        let candidate = &config.candidate;
        let punctuation = input.full_half_punctuation_marks_toggle;
        Self {
            page_size: candidate.candidate_count(),
            layout: candidate.candidate_arrangement_direction,
            pinyin_font: (
                candidate.pinyin_font.family.clone(),
                candidate.pinyin_font.size,
            ),
            candidate_font: (
                candidate.candidate_font.family.clone(),
                candidate.candidate_font.size,
            ),
            item_number_font: (
                candidate.item_number_font.family.clone(),
                candidate.item_number_font.size,
            ),
            translate_font: (
                candidate.translate_font.family.clone(),
                candidate.translate_font.size,
            ),
            item_number_style: candidate.item_number_style,
            min_width_pixels: candidate.candidate_box_minimum_width as f32,
            max_cell_width: candidate.candidate_item_maximum_width as f32,
            preedit: candidate.preedit,
            show_more_candidate_items: candidate.show_more_candidate_items,
            mouse_word_selection: candidate.mouse_word_selection,
            curr_theme: config.theme.curr_theme.clone(),
            translate_enabled: config.translate.enabled,
            translate_dictionary: config.translate.dictionary.clone(),
            translate_need_times: config.translate.need_times(),
            translate_reset_counter: config.translate.reset_counter,
            hiding_candidates: candidate.program_list_of_hiding_candidate.clone(),
            // 「标点全 / 半角」的初值：follow 中文全角、英文半角，full / half 一律；之后还能用状态条那一格会话内切
            full_width_punctuation: punctuation.full_width(false),
            english_full_width_punctuation: punctuation.full_width(true),
            full_width_chars: false,
            traditional: input.simp_trad_chinese_chars_toggle == SimpTrad::Traditional,
            punctuation_mapping: input.punctuation_mapping(),
            pairwise_completion: input.punctuation_marks_pairwise_completion,
            status_pos: config.status_bar.x.zip(config.status_bar.y),
            show_status_bar: config.status_bar.show_status_bar,
            auto_hide_float_tool_bar: config.debugging.auto_hide_float_tool_bar,
            auto_disable_without_text_input: config.debugging.auto_disable_without_text_input,
            show_status_change_tip: input.show_status_change_tip,
            scripts_dir: None,
            script_disabled: config.script.disabled.clone(),
            // 脚本设的候选窗尺寸：会话内状态，配置（热）加载时保留，只有 `Router::reset_composition` 清
            script_min_width: None,
            script_page_size: None,
            script_scale: None,
        }
    }
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self::from(&Config::default())
    }
}
