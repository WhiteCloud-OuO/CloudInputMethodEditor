mod candidate;
mod debugging;
mod general;
mod input;
mod layout_mode;
mod log_level;
mod phrase;
mod preedit_mode;
mod script;
mod status_bar;
mod switch_key;
mod translate;
mod update;
mod word_bank;

use std::path::Path;

use serde::{Deserialize, Serialize};
use toml_edit::DocumentMut;

use crate::error::ConfigError;

pub use candidate::{
    CandidateConfig, DEFAULT_CANDIDATE_BOX_MINIMUM_WIDTH, DEFAULT_FAMILY, FontChoice,
    ItemNumberStyle, MAX_ASSOCIATION_COUNTS, MAX_CANDIDATE_COUNT, MIN_ASSOCIATION_COUNTS,
    MIN_CANDIDATE_COUNT, MouseWordSelection,
};
pub use debugging::DebuggingConfig;
pub use general::{GeneralConfig, MAX_PAGE_SIZE};
pub use input::{
    DEFAULT_PUNCTUATION_MAPPING, FullHalfPunctuation, InputConfig, MO_HU_YIN_BITS,
    PAIRWISE_COMPLETION_BITS, PUNCTUATION_MAPPING_BITS, SimpTrad, fuzzy_bits, pair_bit, pair_open,
    pairwise_completion,
};
pub use layout_mode::LayoutMode;
pub use log_level::LogLevel;
pub use phrase::PhraseConfig;
pub use preedit_mode::PreeditMode;
pub use script::ScriptConfig;
pub use status_bar::StatusBarConfig;
pub use switch_key::{SwitchKey, SwitchKeys};
pub use translate::{MAX_NEED_TIMES, MIN_NEED_TIMES, TranslateConfig};
pub use update::{UpdateChannel, UpdateConfig};
pub use word_bank::{DEFAULT_USER_WORD_BANK_FILE, WordBankConfig};

/// 用户配置文件（TOML）。所有平台同一份格式，缺省值全部在各分节的 `Default` 里。
///
/// 配置文件是唯一事实源：菜单、设置窗口、手改文件三个入口都只写这个文件，再由壳热加载。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 输入：简拼、模糊音、简繁、中英混输、标点与符号映射、成对补全。
    pub input: InputConfig,

    /// 候选：排布、个数、三个字体、序号样式、最小宽度、按程序隐藏。
    pub candidate: CandidateConfig,

    /// 常规：拼音显示位置、Shift+字母、日志与学习开关（其余节还没搬完的暂放这里）。
    pub general: GeneralConfig,

    /// 自定义短语：软件自带短语是否参与（短语库固定在安装目录 `Phrases\Phrase.db`）。
    pub phrase: PhraseConfig,

    /// 词库：生僻项（稀有组）是否参与查询、用户自造词库的位置；第三方词库目录固定随包根的 `WordBank\`。
    pub word_bank: WordBankConfig,

    /// 悬浮状态条记住的位置（常开，不再有开关）。
    pub status_bar: StatusBarConfig,

    /// 调试：实验性开关。
    pub debugging: DebuggingConfig,

    /// 检查更新。
    pub update: UpdateConfig,

    /// 本地词典的翻译 Tip。
    pub translate: TranslateConfig,

    /// 用户脚本（安装目录 `Scripts\` 下的 `.lua`）：禁用名单。
    pub script: ScriptConfig,
}

/// 首次运行写出的模板：默认值全部列出并注释，用户改一处即可。
pub const TEMPLATE: &str = r#"# 云朵输入法配置。保存后自动生效；也可以在「云朵输入法 设置」里改。

[input]
# 使用简拼：中文模式下打出 YD 就能得到「云朵」，不必打全 YUNDUO。关掉只认完整音节（末尾没打完的照样算）
use_jian_pin = true
# 模糊音列表：勾选项的数值相加 —— zh/z·ch/c·sh/s 1、r/l·n/l 2、ng/n 4、ü/u 8、f/h 16；0 为全关
mo_hu_yin_list = 0
# 简 / 繁输出：simplified 简体 / traditional 繁體
simp_trad_chinese_chars_toggle = "simplified"
# 中英混合输入：中文模式下也出英文词与英文补全（KALAOK 出「卡拉OK」、hello 出 hello）
mixture_input = true
# 标点符号全 / 半角：follow 跟随中文 / 英文模式（中文全角、英文半角）/ full 一律全角 / half 一律半角
full_half_punctuation_marks_toggle = "follow"
# 数字后标点使用半角：23:06、2.36、25+9 里的标点保持半角
use_half_wide_punctuation_marks_after_digital = true
# 状态切换提示：中 / 英、大写锁定、全 / 半角、简 / 繁、中文 / 西文标点变化时，在输入光标附近弹一个停留 1 秒的提示条
show_status_change_tip = true
# 符号成对补全：敲左符号补上右符号、光标停在中间。勾选项数值相加 —— () 1、[] 2、{} 4、"" 8、
# （） 16、【】 32、｛｝ 64、《》 128、“” 256、‘‘ 512；0 为关闭
punctuation_marks_pairwise_completion = 0

# 中文模式下的符号映射：勾选项数值相加，只做单键替换（敲下来的是这个键，就换成右边那个上屏）。
# / → 、 1；小键盘 / → ÷ 2；小键盘 * → × 4；~ → ～ 8；· → ` 16。0 为全关（缺省全关）
punctuation_marks_mapping = 0

[candidate]
# 使用本地整句模型（输入法内置）：消耗一点处理器与内存换更准的整句输入；关掉只用词库与短语匹配
use_local_sentence_organization_model = true
# 候选项排布方向：vertical 竖排 / horizontal 横排（横排只给高亮候选显示译文）
candidate_arrangement_direction = "vertical"
# 候选项个数（5–9）
candidate_count = 9
# 联想候选项目上限（0–4）：候选里「比读法更长的词」（联想）最多留几条；0 表示不显示联想候选。
# 打得短（尤其单个字母）时联想候选会很多，把这个调小能让要选的字 / 词留在候选窗口里
candidate_association_counts = 2
# 候选项序号样式：decimal 1.~9. / circled ①~⑨ / roman Ⅰ~Ⅸ / dingbat ❶~❾ / parenthesized ⑴~⑼
item_number_style = "decimal"
# 候选框最小宽度（物理像素，竖排与横排都生效）
candidate_box_minimum_width = 180
# 展示更多候选项（打字时按 Tab）：组句里 Tab 把候选窗展开成一整屏（竖排 5 列 / 横排 5 行）
show_more_candidate_items = false
# 使用鼠标选词：off 关闭（新用户缺省）/ more_candidates 仅「展示更多候选项」展开时 / always 全部开启
mouse_word_selection = "off"
# 展开后每个候选项的最大宽度（物理像素，0 表示不限）：展开成网格时每格不超过它，太长的截断并显示 …
candidate_item_maximum_width = 420
# 在下列程序中不显示候选框：这些程序里输入法完全不接管、按键原样交给应用（写 exe 文件名，不区分大小写）
program_list_of_hiding_candidate = []
# 组句中的拼音显示在哪：both 行内和候选窗口 / inline 只在行内 / window 只在候选窗口（应用里不放 marked text）
preedit = "both"

# 三个字体：字族名 + 字号（点）。字族空表示用系统界面字体；没装这个字族时自动回到系统字体
[candidate.pinyin_font]
family = "微软雅黑"
size = 11

[candidate.candidate_font]
family = "微软雅黑"
size = 13

[candidate.item_number_font]
family = "微软雅黑"
size = 11

# 翻译 Tip 的字体：候选窗底部那一行的左侧显示高亮候选在本地词典里的释义
[candidate.translate_font]
family = "微软雅黑"
size = 11

[general]
# 中文模式下按住 Shift 敲的字母固定收进组句缓冲区按小写参与匹配（Cpan 与 cpan 一样能出「C盘」），
# 原样上屏（回车 / 没候选）时还原大写；英文模式与英文直输段不受影响。没有开关
# 日志级别：info 缺省 / debug 详细（会记录敲的拼音与上屏的文字，配合作者排查问题时再开）。日志在 %LOCALAPPDATA%\CloudIME\logs\
log_level = "info"
# 输入日志：每次上屏记一行到数据目录的 input-log.jsonl（敲的键、看到的候选、选了什么），只写在这台电脑上，不上传；
# 用来离线评测排序和训练个人模型。false 不记；「调试」页可以清空
input_log = true
# 学习输入习惯：按你的选择调整候选顺序、记新词与敲错纠正。false 不再学，已学的仍参与排序；学习数据在数据目录里，删掉文件即清空
#（自造词库 UserWordBank.db 在安装目录的 WordBank\ 下，位置见 [word_bank] user_file）
learning = true

[phrase]
# 短语库固定在安装目录的 Phrases\Phrase.db（不能改位置）；「设置 → 短语」页可以添加、编辑、删除。
# 输入码敲全时短语出现在你指定的候选位置（1 第一位、2 第二位……），同一位置的多条按保存顺序排。
# 启用软件自带短语：随安装包带的一份常用短语参与出候选；关掉只用自己的短语（自带的始终在库里，不占你的列表）
use_default_phrases = true

[word_bank]
# 从词库中查询生僻项条目：开启后候选与整句才会从词库的生僻字 / 生僻词（方言字、罕见词等）里取词；
# 关闭能加快查询，候选里也不再出现这些冷僻条目
rare_items = false
# 用户自造词库 UserWordBank.db 的位置：相对安装目录，也可写绝对路径。缺省与随包词库同在 WordBank\；
# 安装包给 WordBank\ 开了普通用户可写，想换到别的可写位置就改这里
user_file = "WordBank/UserWordBank.db"

[status_bar]
# 桌面上常驻、可拖动的悬浮状态条（Windows）：一排图标按钮——中 / 英、中文标点 / 英文标点、全角 / 半角、
# 简 / 繁、设置（按钮与顺序见安装目录 data\icons-arrangement.cfg）。只跟「当前输入法是不是云朵输入法」走
# 在屏幕上显示悬浮工具栏；关掉后桌面上不再出现这条工具条
show_status_bar = true
# 记住的屏幕位置（物理像素，拖动后自动写入）；留空则首次出现在屏幕右下角
# x = 0
# y = 0

[debugging]
# 自动隐藏悬浮工具栏（实验性功能）：开启后，前台是全屏应用（游戏 / 看视频）时收起；
# 切到别的输入法、云朵输入法被禁用时始终收起，不受这一项影响。关掉后全屏时也不收起
auto_hide_float_tool_bar = false
# 不处于输入状态时自动禁用输入法（实验性功能）：焦点不在可输入文本区域（只读视图、密码框、
# 没有文本焦点）时自动禁用，回到文本区域再启用。用户手动按 Ctrl + Space 禁用的不受影响
auto_disable_without_text_input = false

[update]
# 检查更新：每天向官网（cloudime.app）读一次版本索引，有新版在菜单与设置的「关于」页提示；请求不带任何标识，不自动下载安装
check = true
# 渠道：stable 只看正式版；beta 还会提示测试版（alpha / beta / rc）
channel = "stable"

[translate]
# 启用翻译 Tip：候选窗底部那一行的左侧显示高亮候选在本地词典里的释义
enabled = true
# 本地词典：安装目录 LocalDictionary\ 下 dictionaries.list 里登记的文件名；空 = 没选，不显示 Tip
dictionary = "glossary-en.db"
# 学会所需上屏次数（3–10）：一个词条的译文上屏这么多次就算学会，Tip 的颜色跟着变
need_times = 3
# 「重置学习内容」每点一次加 1；Server 看到它变了就把选中词典的学习记录清空
reset_counter = 0

[script]
# 禁用的用户脚本（安装目录 Scripts\ 下的文件名，大小写不敏感）：设置页的「启用 / 禁用」开关写这里。
# Server 启动时按它跳过（改完要重启 Server）。每个脚本必须在文件最前面声明清单，见安装目录的 Scripts\template.lua
disabled = []
"#;

impl Config {
    /// 读配置。文件不存在按默认值；存在但解析失败报错，不要静默吞掉用户的笔误。
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let config: Self = toml::from_str(&source).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
        Ok(config)
    }

    /// 原地改一个布尔键，见 [`Self::set_value`]。
    pub fn set_bool(path: &Path, section: &str, key: &str, value: bool) -> Result<(), ConfigError> {
        Self::set_value(path, section, key, value)
    }

    /// 原地改一个键（`[section] key = value`），其余内容、注释与顺序原样保留：
    /// 菜单和设置窗口落盘都走这里。文件不存在时从模板起步；文件有语法错误就报错不写，
    /// 不能替用户「修复」成丢了注释的文件。
    ///
    /// `section` 可以带点表示子表（`"candidate.pinyin_font"` → `[candidate.pinyin_font]`）；
    /// 中间缺的表会补成标准表（不是行内表）。
    pub fn set_value(
        path: &Path,
        section: &str,
        key: &str,
        value: impl Into<toml_edit::Value>,
    ) -> Result<(), ConfigError> {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => TEMPLATE.to_owned(),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let mut document: DocumentMut = source.parse().map_err(|source| ConfigError::Edit {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
        // 分节不存在时先建成标准表，否则 toml_edit 会写成顶层的行内表 `predict = { enabled = true }`；
        // 带点的分节（`candidate.pinyin_font`）逐级往下补表。
        let mut item: &mut toml_edit::Item = document.as_item_mut();
        for part in section.split('.') {
            if !item.is_table() {
                *item = toml_edit::table();
            }
            item = &mut item.as_table_mut().expect("刚补成表")[part];
        }
        if !item.is_table() {
            *item = toml_edit::table();
        }
        item.as_table_mut().expect("刚补成表")[key] = toml_edit::value(value);
        // 写临时文件再改名：输入法进程随时可能被杀，不能留半个配置文件
        write_file(path, &document.to_string())
    }

    /// 原地把一个键改成字符串数组（`[section] key = ["a", "b"]`），其余内容、注释与顺序原样保留。
    /// 设置界面改数组项走这里，[`Self::set_value`] 只能写标量。
    pub fn set_array<S: AsRef<str>>(
        path: &Path,
        section: &str,
        key: &str,
        values: &[S],
    ) -> Result<(), ConfigError> {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => TEMPLATE.to_owned(),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let mut document: DocumentMut = source.parse().map_err(|source| ConfigError::Edit {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
        if !document.get(section).is_some_and(|item| item.is_table()) {
            document[section] = toml_edit::table();
        }
        let mut array = toml_edit::Array::new();
        for value in values {
            array.push(value.as_ref());
        }
        document[section][key] = toml_edit::value(array);
        write_file(path, &document.to_string())
    }

    /// 文件不存在时写出模板（目录一并建），返回是否写了。
    pub fn write_template_if_missing(path: &Path) -> Result<bool, ConfigError> {
        if path.exists() {
            return Ok(false);
        }
        write_file(path, TEMPLATE)?;
        Ok(true)
    }
}

/// 原子写配置文件；数据目录还没有就先建（新账户第一次打开设置时输入法可能还没跑过）。
fn write_file(path: &Path, text: &str) -> Result<(), ConfigError> {
    let write = || {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        cloudime_core::storage::write_atomic_str(path, text)
    };
    write().map_err(|source| ConfigError::Write {
        path: path.to_owned(),
        source,
    })
}

/// 按模板写出整份配置：从模板起步保住注释与顺序，再把 `config` 序列化出来的值覆盖上去。
/// 只给迁移旧配置用（见 [`crate::migrate`]）；平时的改动走 [`Config::set_value`]，不动别的键。
pub(crate) fn write_with_template(path: &Path, config: &Config) -> Result<(), ConfigError> {
    let mut document: DocumentMut = TEMPLATE.parse().expect("配置模板必须是合法 TOML");
    let serialized = toml::to_string(config).map_err(|source| ConfigError::Serialize {
        path: path.to_owned(),
        source: Box::new(source),
    })?;
    let source: DocumentMut = serialized.parse().expect("序列化出来的配置必须是合法 TOML");
    overlay(document.as_table_mut(), source.as_table());
    write_file(path, &document.to_string())
}

/// 把 `source` 的每个键值覆盖到 `target` 上：表递归下去，值只换内容、保留模板原有的注释。
fn overlay(target: &mut toml_edit::Table, source: &toml_edit::Table) {
    for (key, value) in source.iter() {
        match value {
            toml_edit::Item::Table(source) => {
                if !target.get(key).is_some_and(toml_edit::Item::is_table) {
                    target[key] = toml_edit::table();
                }
                overlay(target[key].as_table_mut().expect("刚补成表"), source);
            }
            toml_edit::Item::Value(source) => {
                let mut value = source.clone();
                if let Some(previous) = target.get(key).and_then(toml_edit::Item::as_value) {
                    *value.decor_mut() = previous.decor().clone();
                }
                target[key] = toml_edit::Item::Value(value);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_to_defaults() {
        let config: Config = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(config, Config::default());
    }

    /// 装机默认值：这份是产品定的起点，改它要一起改模板与各分节的 `Default`（上一支测试盯着两者一致）。
    #[test]
    fn shipped_defaults_are_the_agreed_starting_point() {
        let config = Config::default();
        // 联想候选收紧到 2、候选框最小宽度 180、符号映射全关、Tab 展开更多候选默认关
        assert_eq!(config.candidate.candidate_association_counts, 2);
        assert_eq!(config.candidate.candidate_box_minimum_width, 180);
        assert!(!config.candidate.show_more_candidate_items);
        assert_eq!(config.candidate.candidate_item_maximum_width, 420);
        assert_eq!(config.input.punctuation_marks_mapping, 0);
        assert!(config.input.punctuation_mapping().is_empty());
        // 字体：候选 13pt、序号 11pt
        assert_eq!(config.candidate.candidate_font.size, 13.0);
        assert_eq!(config.candidate.item_number_font.size, 11.0);
        // 全屏时不自动收起悬浮工具栏；翻译 Tip 默认选英文词典，重置计数从 0 开始
        assert!(!config.debugging.auto_hide_float_tool_bar);
        assert_eq!(config.translate.dictionary, "glossary-en.db");
        assert_eq!(config.translate.reset_counter, 0);
    }

    #[test]
    fn script_disabled_reads_from_toml_and_defaults_empty() {
        assert!(Config::default().script.disabled.is_empty());
        let config: Config =
            toml::from_str("[script]\ndisabled = [\"a.lua\", \"B.LUA\"]\n").unwrap();
        assert_eq!(config.script.disabled, ["a.lua", "B.LUA"]);
        // 模板里带着这一节（新装的用户不用手加）
        assert!(TEMPLATE.contains("[script]\n"));
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let config: Config =
            toml::from_str("[candidate]\nuse_local_sentence_organization_model = false\n").unwrap();
        assert!(!config.candidate.use_local_sentence_organization_model);
        assert_eq!(config.candidate.candidate_count, 9);
        assert_eq!(config.general.log_level, LogLevel::Info);
    }

    #[test]
    fn word_bank_rare_items_defaults_off_and_reads_from_toml() {
        // 老配置没有这一节时取缺省 false
        let config: Config = toml::from_str("[general]\nlearning = true\n").unwrap();
        assert!(!config.word_bank.rare_items);
        // 显式写 true 能读进来
        let config: Config = toml::from_str("[word_bank]\nrare_items = true\n").unwrap();
        assert!(config.word_bank.rare_items);
        // 模板里就带着这一节
        assert!(TEMPLATE.contains("[word_bank]\n"));
    }

    #[test]
    fn word_bank_user_file_defaults_to_the_bundled_directory() {
        // 缺省不能是空串，否则自造词库会把路径解析成空目录
        assert_eq!(
            WordBankConfig::default().user_file,
            DEFAULT_USER_WORD_BANK_FILE
        );
        let config: Config = toml::from_str("[word_bank]\nrare_items = true\n").unwrap();
        assert_eq!(config.word_bank.user_file, "WordBank/UserWordBank.db");
        // 显式配置能读进来
        let config: Config =
            toml::from_str("[word_bank]\nuser_file = \"D:/CloudIME/user.db\"\n").unwrap();
        assert_eq!(config.word_bank.user_file, "D:/CloudIME/user.db");
        // 模板里带着这一项，解析回缺省
        assert!(TEMPLATE.contains("user_file = \"WordBank/UserWordBank.db\""));
        assert_eq!(
            toml::from_str::<Config>(TEMPLATE).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn status_bar_shows_by_default_when_the_key_is_missing() {
        // 老配置里没有这一项：缺省应当显示（`#[serde(default)]` 取的是 `Default`，不是 `bool::default()`）
        let config: Config = toml::from_str("[status_bar]\nx = 0\n").unwrap();
        assert!(config.status_bar.show_status_bar);
        let off: Config = toml::from_str("[status_bar]\nshow_status_bar = false\n").unwrap();
        assert!(!off.status_bar.show_status_bar);
    }

    #[test]
    fn input_section_parses() {
        let config: Config = toml::from_str(
            "[input]\nuse_jian_pin = false\nmo_hu_yin_list = 5\nfull_half_punctuation_marks_toggle = \"half\"\n",
        )
        .unwrap();
        assert!(!config.input.use_jian_pin);
        // 5 = 1 + 4：zh/z 与 sh/s 两个位（现在一位一条规则）
        let fuzzy = config.input.fuzzy_rules();
        assert!(fuzzy.z_zh && fuzzy.s_sh && !fuzzy.c_ch && !fuzzy.an_ang);
        assert_eq!(
            config.input.full_half_punctuation_marks_toggle,
            FullHalfPunctuation::Half
        );
        assert!(
            !config
                .input
                .full_half_punctuation_marks_toggle
                .full_width(false)
        );
        assert!(
            !config
                .input
                .full_half_punctuation_marks_toggle
                .full_width(true)
        );
        // 「跟随」时中文全角、英文半角
        assert!(FullHalfPunctuation::Follow.full_width(false));
        assert!(!FullHalfPunctuation::Follow.full_width(true));
        // 没写的项按缺省
        assert!(config.input.mixture_input);
        assert_eq!(
            config.input.punctuation_marks_mapping,
            DEFAULT_PUNCTUATION_MAPPING
        );
    }

    #[test]
    fn punctuation_mapping_flags_drive_the_fixed_table() {
        use cloudime_core::punctuation::Punctuation;

        let mapped = |config: &Config, c: char| {
            let mut punctuation = Punctuation::default();
            punctuation.set_mapping(config.input.punctuation_mapping());
            punctuation.symbol(c, false).map(|mapped| mapped.text)
        };
        // 9 = 1 + 8：主键盘 / → 、 与 ~ → ～
        let config: Config = toml::from_str("[input]\npunctuation_marks_mapping = 9\n").unwrap();
        assert_eq!(mapped(&config, '/').as_deref(), Some("、"));
        assert_eq!(mapped(&config, '~').as_deref(), Some("～"));
        assert_eq!(mapped(&config, '·'), None);
        // 老配置里的表（还带两键规则）不报错，退回缺省位图
        let old: Config =
            toml::from_str("[input.punctuation_marks_mapping]\n\"/\" = \"、\"\n\"~=\" = \"≈\"\n")
                .unwrap();
        assert_eq!(
            old.input.punctuation_marks_mapping,
            DEFAULT_PUNCTUATION_MAPPING
        );
        // 0 = 全关
        let off: Config = toml::from_str("[input]\npunctuation_marks_mapping = 0\n").unwrap();
        assert!(off.input.punctuation_mapping().is_empty());
    }

    #[test]
    fn candidate_and_general_sections_parse() {
        let config: Config = toml::from_str(
            "[candidate]\ncandidate_count = 5\ncandidate_arrangement_direction = \"horizontal\"\npreedit = \"window\"\n[candidate.pinyin_font]\nfamily = \"LXGW WenKai\"\nsize = 12\n",
        )
        .unwrap();
        assert_eq!(config.candidate.candidate_count(), 5);
        assert_eq!(
            config.candidate.candidate_arrangement_direction,
            LayoutMode::Horizontal
        );
        assert_eq!(config.candidate.preedit, PreeditMode::Window);
        assert_eq!(config.candidate.pinyin_font.size, 12.0);
        assert_eq!(config.candidate.pinyin_font.family, "LXGW WenKai");
        // 没写的字体项按缺省
        assert_eq!(config.candidate.candidate_font.size, 13.0);
        assert_eq!(config.general.log_level, LogLevel::Info);
    }

    #[test]
    fn set_value_writes_strings_and_integers() {
        let path = std::env::temp_dir().join("cloudime-config-set-value-test.toml");
        let _ = std::fs::remove_file(&path);
        Config::set_value(&path, "candidate", "candidate_count", 5i64).unwrap();
        Config::set_value(
            &path,
            "candidate",
            "candidate_arrangement_direction",
            "horizontal",
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.candidate.candidate_count, 5);
        assert_eq!(
            config.candidate.candidate_arrangement_direction,
            LayoutMode::Horizontal
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn set_bool_keeps_comments_and_flips_only_that_key() {
        let path = std::env::temp_dir().join("cloudime-config-set-bool-test.toml");
        std::fs::write(
            &path,
            "# 头注释\n[input]\n# 说明\nuse_jian_pin = false\nmo_hu_yin_list = 2\n",
        )
        .unwrap();
        Config::set_bool(&path, "input", "use_jian_pin", true).unwrap();
        Config::set_bool(
            &path,
            "candidate",
            "use_local_sentence_organization_model",
            false,
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with(
                "# 头注释\n[input]\n# 说明\nuse_jian_pin = true\nmo_hu_yin_list = 2\n"
            ),
            "{text}"
        );
        let config = Config::load(&path).unwrap();
        assert!(config.input.use_jian_pin && config.input.mo_hu_yin_list == 2);
        assert!(!config.candidate.use_local_sentence_organization_model);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn writes_create_the_data_directory_for_a_fresh_account() {
        let dir = std::env::temp_dir().join("cloudime-config-fresh-account-test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("CloudIME").join("config.toml");
        assert!(Config::write_template_if_missing(&path).unwrap());
        assert!(!Config::write_template_if_missing(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
        // 没有模板直接保存也行
        std::fs::remove_dir_all(&dir).unwrap();
        Config::set_bool(
            &path,
            "candidate",
            "use_local_sentence_organization_model",
            false,
        )
        .unwrap();
        assert!(
            !Config::load(&path)
                .unwrap()
                .candidate
                .use_local_sentence_organization_model
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_bool_starts_from_template_when_missing() {
        let path = std::env::temp_dir().join("cloudime-config-set-bool-missing-test.toml");
        let _ = std::fs::remove_file(&path);
        Config::set_bool(
            &path,
            "candidate",
            "use_local_sentence_organization_model",
            false,
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# 云朵输入法配置"));
        assert!(
            !Config::load(&path)
                .unwrap()
                .candidate
                .use_local_sentence_organization_model
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn set_bool_refuses_broken_file() {
        let path = std::env::temp_dir().join("cloudime-config-set-bool-broken-test.toml");
        std::fs::write(&path, "[input\nuse_jian_pin = false\n").unwrap();
        assert!(matches!(
            Config::set_bool(&path, "input", "use_jian_pin", true),
            Err(ConfigError::Edit { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[input\nuse_jian_pin = false\n"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_default() {
        let path = std::env::temp_dir().join("cloudime-config-missing-test.toml");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Config::load(&path).unwrap(), Config::default());
    }
}
