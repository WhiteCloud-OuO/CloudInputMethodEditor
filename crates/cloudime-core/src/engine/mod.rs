//! Engine：Core 对外的唯一门面。
//!
//! 平台层只跟这里打交道：喂按键、拿候选、上屏。学习通过 trait 注入，
//! 默认实现都是空操作，所以单元测试和 CLI 不需要真实词典也能跑。

mod alignment;
mod commit;
mod composing;
mod english_case;
mod extras;
mod input_log;
mod learning;
mod marked;
mod privacy;
mod query;
mod raw;
mod rescoring;
mod session;
mod setup;
mod statistics;
mod timings;
mod vocabulary;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use cloudime_dictionary::{Dictionary, Match, WordList};

pub use alignment::Alignment;
pub use commit::{LastCommit, Transition};
pub use english_case::EnglishCase;
pub use input_log::{
    CommitEntry, INPUT_LOG_VERSION, InputLogEntry, InputLogger, InputSource, LOGGED_CANDIDATES,
    NoInputLogger,
};
pub use learning::{Forgotten, Learner, NoLearner};
pub use marked::{MarkedKind, MarkedSegment};

pub use query::Query;
pub use raw::RawPreedit;
pub use session::EngineSession;
pub use statistics::{NoUsageMeter, Usage, UsageMeter, UsageSummary};
pub use timings::Timings;
pub use vocabulary::{NoVocabularyTracker, VocabularySummary, VocabularyTracker};

use crate::candidate::{Candidate, CandidateKind, CandidateList};
use crate::composition::Composition;
use crate::correction::{self, TypoCosts};
use crate::english;
use crate::fuzzy::{Expanded, FuzzyRules};
use crate::history::InputHistory;
use crate::parser::{self, ParseError, Segmentation};
use crate::punctuation::{MappedSymbol, Punctuation};
use crate::ranking::{self, Scored};
use crate::sentence::{
    self, Conversion, Interpolation, LanguageModel, NoLanguageModel, Personal, SentenceScorer,
};
use crate::shortcut::{self, EXPRESSION_PREFIX};

use commit::CommitChain;

pub struct Engine {
    /// 静态词库。
    dictionary: Dictionary,

    /// 稀有词库（生僻字 / 生僻词，来自 `Dict.db` 的稀有组）；可用 [`Self::set_rare_enabled`] 整组跳过。
    rare: Option<Dictionary>,

    /// 稀有组是否参与查词（缺省关，由 `[word_bank] rare_items` 决定）。
    rare_enabled: bool,

    /// 附加词库（领域词库、用户导入的第三方词库），**按添加顺序**与主词库一起查词、一起进整句词图；
    /// 靠前的优先，同一个词靠前命中后后面的不再重复产出。不参与语言模型（它们没有 bigram，
    /// 走词频兜底）。壳按用户目录 `dicts/` 与配置 `[dictionaries]` 装配。
    extra_dictionaries: Vec<Dictionary>,

    /// 用户词频，缺省为 [`NoLearner`]；私密输入期间只读不写（[`learning::MutedLearner`]）。
    learner: learning::MutedLearner,

    /// 当前拼音缓冲区。
    composition: Composition,

    /// 英文词表，中英混输用；没有就不出英文候选。
    english: Option<WordList>,

    /// 英文模式（壳里 Caps Lock 亮着）：缓冲区里的字母不当拼音，候选来自英文词表的补全与纠正。
    english_mode: bool,

    /// 全角标点与引号配对状态。
    punctuation: Punctuation,

    /// 中文标点转换开关。
    full_width_punctuation: bool,

    /// 使用简拼（配置 `[input] use_jian_pin`，缺省开）：关掉只认完整音节与末尾没打完的前缀。
    use_jian_pin: bool,

    /// 中英混合输入（配置 `[input] mixture_input`，缺省开）：中文模式下也出英文词与英文补全。
    mixture_input: bool,

    /// 英文候选的大小写档位（反引号轮换）：每次开一段新拼音回到原样。
    english_case: EnglishCase,

    /// 联想候选项目上限（配置 `[candidate] candidate_association_counts`，0–4）：候选列表里
    /// 「比读法更长的词」（联想）最多留几条，避免单字输入时被大量联想候选淹没。
    association_counts: usize,

    /// 表达式计算面板（V 模式里按 Tab 进来）：只给结果那一条候选、拼音行显示算式本身。
    /// 由壳在进 / 出面板时设（见 `apps/windows` 的按键处理），组句结束要清掉。
    calculator: bool,

    /// 脚本给的加权 / 降权（词文本 → 系数）：乘进词频那一层，排序规则本身仍由 Core 定。
    /// 由壳用 [`Self::set_word_adjustments`] 设，缺省空。
    word_adjustments: HashMap<String, f64>,

    /// 用户定义的固定位置文本。
    custom_phrases: Vec<crate::CustomPhrase>,

    /// 整句转换的语言模型，缺省为 [`NoLanguageModel`]（退化成一元词频）。
    language_model: Box<dyn LanguageModel>,

    /// 整句路径的同步神经重打分器（字级 Transformer，查询里当场打分；CLI 评测用）。
    sentence_scorer: Option<Box<dyn SentenceScorer>>,

    /// 异步重打分：后台线程里的打分器，壳在停顿后送任务、轮询结果（见 [`rescoring`]）。
    rescorer: Option<rescoring::RescoreWorker>,

    /// 「前文 + 整句文本 → 神经分」缓存，同步与异步打分共用。
    neural_cache: std::cell::RefCell<rescoring::NeuralCache>,

    /// 壳给的应用里光标前的文本；`None` 时前文用本会话历史。
    rescoring_before: Option<String>,

    /// 重打分时神经得分的权重 λ：最终分 = 路径分 + λ·(神经分 − 静态分)。
    neural_weight: f64,

    /// 只有路径分与最优路径差距在这么多 nat 以内的路径才参与重排：差距大的多半是个人 n-gram 拉开的，通用模型不该翻盘。
    neural_margin: f64,

    /// 重打分给模型看的前文长度（本会话最近上屏的字符数），0 为不给前文。
    neural_context: usize,

    /// 个人 n-gram 与静态模型插值的参数；只有回放调参会改（`set_interpolation`），壳用缺省值。
    interpolation: Interpolation,

    /// 敲错纠正的代价；同上，只有回放调参会改（`set_typo_costs`）。
    typo_costs: TypoCosts,

    /// 最近几次上屏各记了哪些学习、之后退格了几个字；用户把它们删掉重选时把学习退回去（见 [`Self::note_backspace`]）。
    /// 最新的在末尾，最多留 [`RECENT_COMMITS`] 条。
    recent_commits: Vec<LastCommit>,

    /// 本次 commit 里记下的词转移，commit 结束时搬进 `last_commit`。
    recording: Vec<Transition>,

    /// 输入日志的落盘方；缺省不记，私密输入期间一律不记（[`input_log::MutedLogger`]）。
    logger: input_log::MutedLogger,

    /// 私密输入中（见 [`Self::set_private`]）：不学、不记。
    private: bool,

    /// 输入日志条目的序号。
    log_sequence: u64,

    /// 最近一次查询的候选顺序是否经过神经重排（`rescore_paths` 置位，`query` 开头清零），写进输入日志。
    last_rescored: std::cell::Cell<bool>,

    /// 这段组句里第一次退格前的缓冲区：上屏时与最终键串不同就记一条 `retype`。
    retype_snapshot: Option<String>,

    /// 组句外直通给应用的字符，攒到下一次上屏或上文断开时写成一条 `passthrough`。
    passthrough_pending: String,

    /// 这段组句翻了几页候选。
    page_turns: u32,

    /// 这段组句第一键的时刻（算首键到上屏的毫秒）。
    composition_started: Option<Instant>,

    /// 正在输入的应用标识，壳在焦点变化时给；写进输入日志。
    application: Option<String>,

    /// 上次记 `break` 之后有没有上屏过：没有就不再记，免得失焦一次记一条。
    committed_since_break: bool,

    /// 输入统计的累计方（打了多少字）；缺省不记。
    meter: Box<dyn UsageMeter>,

    /// 词汇记录（候选窗口里见过 / 上屏过哪些词）；缺省不记。
    vocabulary: Box<dyn VocabularyTracker>,

    /// 候选窗口当前页上的词（壳每次画完告知），上屏时记成「看到过」。
    displayed: Vec<String>,

    /// 上一次查询的摘要，上屏时写进输入日志。
    last_query: std::cell::RefCell<Option<query::QuerySnapshot>>,

    /// 整句转换的格子候选缓存：跨按键复用，学习数据一变就清（见 [`Self::forget_span_cache`]）。
    span_cache: std::cell::RefCell<sentence::SpanCache>,

    /// 本次会话经我们上屏的文本，应用不给上下文时用它联想。
    history: InputHistory,

    /// 连续上屏的链，个人 n-gram 与自动造词靠它。
    chain: CommitChain,

    /// 模糊音开关，缺省全关。
    fuzzy: FuzzyRules,

    /// 繁体输出模式。
    traditional: bool,

    /// 繁体转换器。
    opencc: Option<ferrous_opencc::OpenCC>,

    /// 繁体输出时「繁体 → 原简体」的映射，组句结束清空；学习与撤销都按简体原文走。
    traditional_map: std::cell::RefCell<HashMap<String, String>>,
}

/// 英文补全最多几条（`compa` → company / compare / …）。
const ENGLISH_COMPLETIONS: usize = 3;

/// 原样上屏的字母串至少几个字母才当英文词学：单字母（`a`、`I`）不值得记。
const MIN_ENGLISH_WORD_LETTERS: usize = 2;

/// 英文模式一次最多给几条候选：两页足够，再往后没人翻。
const ENGLISH_MODE_CANDIDATES: usize = 18;

/// 英文补全至少要几个字母：太短的前缀谁都像。
const MIN_COMPLETION_LETTERS: usize = 3;

/// 整段末尾当英文词的尾段至少几个字母，前面的拼音头至少几个字母（见 `query::EnglishTail`）。
const MIN_ENGLISH_TAIL_LETTERS: usize = 2;
const MIN_ENGLISH_TAIL_HEAD_LETTERS: usize = 2;

/// 尾段自己也是合法拼音时（`fan`、`database`）至少几个字母才考虑英文读法：三个字母的拼音音节太多。
const MIN_PINYIN_LIKE_TAIL_LETTERS: usize = 4;

/// 句中切到英文的代价（log 概率）：尾段像拼音时英文读法要比拼音读法高出这么多才胜出。拍的，攒够日志后用 `--replay` 调。
const ENGLISH_SWITCH_PENALTY: f64 = 3.0;

/// 英文词频的下限（Zipf）：没有词频的词按百万分之一算。
const ENGLISH_ZIPF_FLOOR: f64 = 3.0;

/// 自动造词：用户连着选出的两个词，合起来不在词库里、且这条接续已记过这么多次，就记成用户词。
/// 同一段拼音里连着选出来的（`cloudime` 选 青 再选 简）是「用户把它当一个词打」的强信号，两次就够；
/// 第一次可能是误选或偶然。
const AUTO_WORD_THRESHOLD_SAME_BUFFER: u32 = 2;

/// 分两段打的（`qing` 选 青、再打 `jian` 选 简）信号弱一些，但同样连续两次就够，与同段一致。
const AUTO_WORD_THRESHOLD: u32 = 2;

/// 退格撤销最多回看几次上屏：删掉「沃德 书」两个词再重打时，要能找到两个词之前的那一次。
const RECENT_COMMITS: usize = 4;

/// 自动造出的词最多几个字：再长就不是词而是短语了。
const AUTO_WORD_MAX_CHARS: usize = 4;

/// Ctrl + 回车 收进自造词库的英文词的初始权重。
pub const ENGLISH_WORD_WEIGHT: f64 = 1000.0;

/// 用户自己点选的词，转移记几份；整句路径里顺带的记一份。
/// 整句是模型自己算出来的，按空格接受它会把这条路径喂回模型，形成自我强化；用户明确改选的词要能压过这种回声。
pub const EXPLICIT_TRANSITION_WEIGHT: u32 = 2;

/// 一次查询最多给壳多少条候选。同音字最多的音节也不到这个数，再往后都是长词，没人会翻到。
const MAX_CANDIDATES: usize = 500;

/// 联想候选项目上限的缺省值与取值范围的上限（配置 `[candidate] candidate_association_counts`，0–4）。
/// 缺省取上限（最宽松）：`0` 表示不显示联想候选。
const MAX_ASSOCIATION_COUNTS: usize = 4;

/// 神经重打分看 Viterbi 的前几条路径：束宽是 8，再多也没有。
const RESCORE_PATHS: usize = 6;

/// 神经重打分的缺省权重 λ（见 `Engine::neural_weight`）：整句评测集上 0.5 到 1.0 一样好、0.75 最高（见 docs/notes/neural-rescoring.md），
/// 取 0.5 给个人 n-gram 留余量；回放里看到的「λ 大整句掉」是那把尺子的偏差。
pub const NEURAL_WEIGHT: f64 = 0.5;

/// 神经重打分的缺省门槛（nat）：路径分落后最优路径超过这么多的不参与重排。缺省不设（4 nat 试过没帮助），留作调参的旋钮。
pub const NEURAL_MARGIN: f64 = f64::INFINITY;

/// 重打分给模型看的前文：本次会话最近上屏的这么多个字符。
pub const RESCORE_CONTEXT_CHARS: usize = 64;

impl Engine {
    pub fn new(dictionary: Dictionary) -> Self {
        // 敲错变体表按全部音节算一次（几毫秒）：构造时预热，别让第一个用到敲错边的按键买单。
        correction::typo::warm();
        Self {
            dictionary,
            rare: None,
            rare_enabled: false,
            extra_dictionaries: Vec::new(),
            learner: learning::MutedLearner::new(Box::new(NoLearner)),
            composition: Composition::default(),
            english: None,
            english_mode: false,
            punctuation: Punctuation::default(),
            full_width_punctuation: true,
            use_jian_pin: true,
            mixture_input: true,
            english_case: EnglishCase::Lower,
            association_counts: MAX_ASSOCIATION_COUNTS,
            calculator: false,
            word_adjustments: HashMap::new(),
            custom_phrases: Vec::new(),
            language_model: Box::new(NoLanguageModel),
            sentence_scorer: None,
            rescorer: None,
            neural_cache: std::cell::RefCell::new(rescoring::NeuralCache::default()),
            rescoring_before: None,
            neural_weight: NEURAL_WEIGHT,
            neural_margin: NEURAL_MARGIN,
            interpolation: Interpolation::DEFAULT,
            typo_costs: TypoCosts::DEFAULT,
            neural_context: RESCORE_CONTEXT_CHARS,
            span_cache: std::cell::RefCell::new(sentence::SpanCache::default()),
            recent_commits: Vec::new(),
            logger: input_log::MutedLogger::new(Box::new(NoInputLogger)),
            private: false,
            log_sequence: 0,
            last_rescored: std::cell::Cell::new(false),
            retype_snapshot: None,
            passthrough_pending: String::new(),
            page_turns: 0,
            composition_started: None,
            application: None,
            committed_since_break: false,
            meter: Box::new(NoUsageMeter),
            vocabulary: Box::new(NoVocabularyTracker),
            displayed: Vec::new(),
            last_query: std::cell::RefCell::new(None),
            recording: Vec::new(),
            history: InputHistory::default(),
            chain: CommitChain::default(),
            fuzzy: FuzzyRules::default(),
            traditional: false,
            opencc: None,
            traditional_map: std::cell::RefCell::new(HashMap::new()),
        }
    }
}

/// 缓冲区是否是英文直输段：含小写字母与 `'` 以外的字符（`no-way`、`a.b`），且不是表达式模式。
fn is_raw(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with(EXPRESSION_PREFIX)
        && text.chars().any(|c| !(c.is_ascii_lowercase() || c == '\''))
}

/// 命中是否靠模糊音：某个音节不被敲的那个模式接受。
/// 原样上屏的字母串像不像一个英文词：纯 ASCII 字母、至少两个。中文模式下还要求它**不能**切成完整的拼音
/// （`hao` 回车多半是要拼音字母本身，`gist` / `python` / `hello` 切不干净才是英文）；英文模式下敲的全是英文，不用判。
fn looks_like_english_word(raw: &str, english_mode: bool) -> bool {
    if raw.len() < MIN_ENGLISH_WORD_LETTERS || !raw.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    english_mode || !parser::is_fully_segmentable(&raw.to_ascii_lowercase())
}

/// 模式的记忆化键：完整音节原样，前缀音节后加 `*`。
fn pattern_key(pattern: &[cloudime_dictionary::SyllablePattern<'_>]) -> String {
    let mut key = String::with_capacity(pattern.len() * 7);
    for p in pattern {
        key.push_str(p.text);
        if !p.complete {
            key.push('*');
        }
        key.push(' ');
    }
    key
}

/// 末尾 `count` 个字符。
fn take_last_chars(text: &str, count: usize) -> String {
    let total = text.chars().count();
    text.chars().skip(total.saturating_sub(count)).collect()
}

/// 光标后剩余拼音的显示形式：能切就按音节用 `'` 连上，切不动就原样。
fn marked_rest(rest: &str, abbreviations: bool) -> String {
    if rest.is_empty() {
        return String::new();
    }
    match segment_longest_prefix(rest, abbreviations) {
        Ok((segmentations, tail)) => query::join_marked(&segmentations, tail),
        Err(_) => rest.to_owned(),
    }
}

/// 整段切不动时退而求其次：找能切分的最长前缀，剩余字母作为尾部返回。
/// `kaifv` → (`kai f…` 的切分, `v`)。连第一个字母都切不动才报错。
fn segment_longest_prefix(
    text: &str,
    abbreviations: bool,
) -> Result<(Vec<Segmentation>, &str), ParseError> {
    match parser::segment_with(text, abbreviations) {
        Ok(segmentations) => Ok((segmentations, "")),
        Err(ParseError::NoSegmentation) => (1..text.len())
            .rev()
            .filter_map(|end| {
                parser::segment_with(&text[..end], abbreviations)
                    .ok()
                    .map(|s| (s, &text[end..]))
            })
            // 关掉简拼时，被切出来的那截末尾不能是残缺音节：那样 `yd` 会拿 `y` 当声母前缀命中一堆词，
            // 与「不认简拼」矛盾。`kaifx` 这种还会退到更短的完整音节（`kai`）上。
            .find(|(segmentations, _)| {
                abbreviations
                    || !segmentations
                        .first()
                        .is_some_and(Segmentation::last_is_partial)
            })
            .ok_or(ParseError::NoSegmentation),
        Err(error) => Err(error),
    }
}

/// 候选的音节序列在 `input` 开头覆盖了多少个字节。音节之间允许有 `'`。
///
/// 每个音节吃掉输入里与它相同的最长前缀：全拼 `kaifa` 的 开发 吃 5 个，简拼 `kf` 的 开发 吃 2 个，
/// 未打完的 `kaif` 也吃完。吃不到任何字母说明候选与输入的切分方式不一致，就此停止。
/// 按输入串记选择用的键：作用域开头 `len` 个字节里的字母（去掉分隔符 `'`），
/// 查询时按候选覆盖的字母数截取同一个串，两边才对得上。
fn choice_key(scope: &str, len: usize) -> String {
    scope[..len.min(scope.len())]
        .chars()
        .filter(|c| *c != '\'')
        .collect()
}

#[cfg(test)]
mod tests;
