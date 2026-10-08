//! 注入与开关：词库、模糊音、学习 / 整句重打分等 trait 实现的挂接，以及相应的只读访问。

use super::*;

impl Engine {
    /// 设置中文模式的标点转换。
    pub fn set_full_width_punctuation(&mut self, enabled: bool) {
        self.full_width_punctuation = enabled;
    }

    /// 使用简拼（配置 `[input] use_jian_pin`，缺省开）。关掉只认完整音节与末尾没打完的前缀。
    pub fn set_use_jian_pin(&mut self, on: bool) {
        if self.use_jian_pin != on {
            // 切分变了，整句格子缓存一起作废
            self.forget_span_cache();
        }
        self.use_jian_pin = on;
    }

    pub fn use_jian_pin(&self) -> bool {
        self.use_jian_pin
    }

    /// 中英混合输入（配置 `[input] mixture_input`，缺省开）：关掉中文模式不再出英文词与英文补全。
    pub fn set_mixture_input(&mut self, on: bool) {
        if self.mixture_input != on {
            self.forget_span_cache();
        }
        self.mixture_input = on;
    }

    pub fn mixture_input(&self) -> bool {
        self.mixture_input
    }

    /// 英文候选当前的大小写档位。
    pub fn english_case(&self) -> EnglishCase {
        self.english_case
    }

    /// 反引号：英文候选轮换到下一档大小写（全小写 → 全大写 → 首字母大写）。
    pub fn cycle_english_case(&mut self) -> EnglishCase {
        self.english_case = self.english_case.next();
        self.english_case
    }

    /// 联想候选项目上限（配置 `[candidate] candidate_association_counts`，0–4）：候选列表里
    /// 「比读法更长的词」（联想）最多留几条；`0` 表示不显示联想候选。
    pub fn set_association_counts(&mut self, count: usize) {
        self.association_counts = count.min(MAX_ASSOCIATION_COUNTS);
    }

    pub fn association_counts(&self) -> usize {
        self.association_counts
    }

    /// 中文模式下的符号映射（配置 `[input] punctuation_marks_mapping`）。
    pub fn set_punctuation_mapping(&mut self, mapping: crate::punctuation::Mapping) {
        self.punctuation.set_mapping(mapping);
    }

    /// 数字后标点是否保持半角（配置 `[input] use_half_wide_punctuation_marks_after_digital`，缺省开）。
    pub fn set_half_wide_after_digit(&mut self, on: bool) {
        self.punctuation.set_half_after_digit(on);
    }

    /// 原子更新自定义短语，非法规则保持旧值。`title` 在这里统一去掉首尾空白、空串归一成 `None`。
    pub fn set_custom_phrases(
        &mut self,
        mut phrases: Vec<crate::CustomPhrase>,
    ) -> Result<(), String> {
        crate::custom_phrase::normalize_phrases(&mut phrases);
        crate::custom_phrase::validate_phrases(&phrases)?;
        self.custom_phrases = phrases;
        Ok(())
    }

    /// 学习开关（`[general] learning`）：关掉后不再记词频、用户词、个人 n-gram 与敲错表，已学的照常参与排序；
    /// 私密输入是另一个独立的开关（[`Self::set_private`]）。
    pub fn set_learning(&mut self, enabled: bool) {
        self.learner.set_disabled(!enabled);
    }

    /// 設置是否啟用繁體輸出模式。
    pub fn set_traditional_mode(&mut self, on: bool) {
        self.traditional = on;
        if on && self.opencc.is_none() {
            match ferrous_opencc::OpenCC::from_config(ferrous_opencc::config::BuiltinConfig::S2tw) {
                Ok(opencc) => self.opencc = Some(opencc),
                Err(error) => tracing::warn!(%error, "繁体转换器初始化失败，候选仍是简体"),
            }
        }
    }

    /// 光标后剩余拼音的显示形式：能切就按音节用 `'` 连上，切不动就原样。
    pub(super) fn marked_rest(&self, rest: &str) -> String {
        marked_rest(rest, self.use_jian_pin)
    }

    pub fn with_fuzzy(mut self, rules: FuzzyRules) -> Self {
        self.fuzzy = rules;
        self
    }

    /// 换模糊音规则：格子缓存里的代价随写法变，一起作废。
    pub fn set_fuzzy(&mut self, rules: FuzzyRules) {
        if self.fuzzy != rules {
            self.forget_span_cache();
        }
        self.fuzzy = rules;
    }

    pub fn fuzzy(&self) -> FuzzyRules {
        self.fuzzy
    }

    /// 挂上同步的整句重打分器（字级 Transformer，查询里当场打分，评测用）。`weight` 是神经分的权重 λ，
    /// `margin` 是参与重排的路径分门槛（nat），`context` 是给模型看的前文字符数；
    /// `None` 用缺省 [`NEURAL_WEIGHT`] / [`NEURAL_MARGIN`] / [`RESCORE_CONTEXT_CHARS`]。
    pub fn with_sentence_scorer(
        mut self,
        scorer: Box<dyn SentenceScorer>,
        weight: Option<f64>,
        margin: Option<f64>,
        context: Option<usize>,
    ) -> Self {
        self.sentence_scorer = Some(scorer);
        self.rescorer = None;
        self.set_neural_parameters(weight, margin, context);
        self
    }

    /// 挂上异步的整句重打分器：打分在后台线程，查询不等它，壳在停顿后 [`Self::request_rescoring`]、
    /// 结果到了 [`Self::poll_rescoring`] 后再查一次。参数同 [`Self::with_sentence_scorer`]。
    pub fn with_async_sentence_scorer(
        mut self,
        scorer: Box<dyn SentenceScorer>,
        weight: Option<f64>,
        margin: Option<f64>,
        context: Option<usize>,
    ) -> Self {
        self.set_async_sentence_scorer(Some(scorer));
        self.set_neural_parameters(weight, margin, context);
        self
    }

    /// 运行时换 / 卸异步重打分器（壳里模型在后台加载完才接上，配置关掉就卸）。
    pub fn set_async_sentence_scorer(&mut self, scorer: Option<Box<dyn SentenceScorer>>) {
        self.sentence_scorer = None;
        self.rescorer = scorer.map(super::rescoring::RescoreWorker::spawn);
        *self.neural_cache.borrow_mut() = super::rescoring::NeuralCache::default();
        self.forget_span_cache();
    }

    /// 换一组个人 n-gram 插值参数（回放调参用）；整句格子缓存作废。
    pub fn set_interpolation(&mut self, interpolation: Interpolation) {
        self.interpolation = interpolation;
        self.forget_span_cache();
    }

    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    /// 换一组敲错纠正代价（回放调参用）；整句格子缓存作废。
    pub fn set_typo_costs(&mut self, costs: TypoCosts) {
        self.typo_costs = costs;
        self.forget_span_cache();
    }

    pub fn typo_costs(&self) -> TypoCosts {
        self.typo_costs
    }

    /// 整句转换与词级排序用的个人部分：学习器的个人 n-gram 配上当前插值参数。
    pub(super) fn personal(&self) -> Personal<'_> {
        Personal {
            ngram: self.learner.user_ngram(),
            interpolation: self.interpolation,
        }
    }

    /// 神经分的权重 λ（0 到 1）。
    pub fn set_neural_weight(&mut self, weight: f64) {
        self.neural_weight = weight.clamp(0.0, 1.0);
        self.forget_span_cache();
    }

    fn set_neural_parameters(
        &mut self,
        weight: Option<f64>,
        margin: Option<f64>,
        context: Option<usize>,
    ) {
        self.neural_weight = weight.unwrap_or(NEURAL_WEIGHT).clamp(0.0, 1.0);
        self.neural_margin = margin.unwrap_or(NEURAL_MARGIN).max(0.0);
        self.neural_context = context.unwrap_or(RESCORE_CONTEXT_CHARS);
        self.forget_span_cache();
    }

    pub fn with_language_model(mut self, model: Box<dyn LanguageModel>) -> Self {
        self.language_model = model;
        self
    }

    /// 静态语言模型（没接就是 [`NoLanguageModel`]）：评测工具拿它按 [`crate::sentence::segment_text`] 切汉字文本。
    pub fn language_model(&self) -> &dyn LanguageModel {
        &*self.language_model
    }

    pub fn history(&self) -> &InputHistory {
        &self.history
    }

    pub fn history_mut(&mut self) -> &mut InputHistory {
        &mut self.history
    }

    /// 进入 / 离开英文模式。英文模式下 [`Self::query`] 只给英文词表的候选，回车与空格仍由壳原样上屏敲的字母，
    /// 也不把原样上屏记成「不纠这个串」。
    pub fn set_english_mode(&mut self, on: bool) {
        self.english_mode = on;
    }

    pub fn english_mode(&self) -> bool {
        self.english_mode
    }

    pub fn with_english(mut self, words: WordList) -> Self {
        self.english = Some(words);
        self
    }

    pub fn with_learner(mut self, learner: Box<dyn Learner>) -> Self {
        self.learner.replace(learner);
        self.forget_span_cache();
        self
    }

    pub fn with_input_logger(mut self, logger: Box<dyn InputLogger>) -> Self {
        self.logger.replace(logger);
        self
    }

    /// 运行时换输入日志的落盘方（开关、清空之后）。旧的先 flush。
    pub fn set_input_logger(&mut self, logger: Box<dyn InputLogger>) {
        self.logger.flush();
        self.logger.replace(logger);
    }

    pub fn input_logger_mut(&mut self) -> &mut dyn InputLogger {
        self.logger.inner_mut()
    }

    pub fn with_usage_meter(mut self, meter: Box<dyn UsageMeter>) -> Self {
        self.meter = meter;
        self
    }

    /// 输入统计的汇总（「设置 → 统计」页）。
    pub fn usage_summary(&self) -> UsageSummary {
        self.meter.summary()
    }

    pub fn with_vocabulary_tracker(mut self, tracker: Box<dyn VocabularyTracker>) -> Self {
        self.vocabulary = tracker;
        self
    }

    pub fn dictionary(&self) -> &Dictionary {
        &self.dictionary
    }

    /// 挂上稀有词库（生僻字 / 生僻词，来自 `Dict.db` 的稀有组）。格子缓存随之作废。
    pub fn with_rare(mut self, rare: Dictionary) -> Self {
        self.rare = Some(rare);
        self.forget_span_cache();
        self
    }

    pub fn rare_dictionary(&self) -> Option<&Dictionary> {
        self.rare.as_ref()
    }

    /// 稀有组开关（缺省关）：打开后整组参与词级查询与整句词图。
    /// `[word_bank] rare_items` 只翻转它，不新增查询路径。
    pub fn set_rare_enabled(&mut self, on: bool) {
        if self.rare_enabled != on {
            self.forget_span_cache();
        }
        self.rare_enabled = on;
    }

    pub fn rare_enabled(&self) -> bool {
        self.rare_enabled
    }

    /// 表达式计算面板开着没有（V 模式里按 Tab 进来）：开着时候选只有结果那一条。
    pub fn calculator(&self) -> bool {
        self.calculator
    }

    /// 进 / 出表达式计算面板（壳在按 Tab / Esc 时调）。组句结束壳要清掉。
    pub fn set_calculator(&mut self, on: bool) {
        self.calculator = on;
    }

    /// 脚本给的加权 / 降权（`词文本 → 系数`：大于 1 加权、小于 1 降权、给 0 就沉底）。**整份替换**，
    /// 空的就是全清；壳（Server 的脚本接口）每次脚本给就调一次，内容没变时什么都不做。
    ///
    /// 系数乘在**词频那一层**：排序规则本身仍在 Core（结构键、读法层级、联想折扣都照旧），脚本只调参 ——
    /// 覆盖字母少的词再重也不会因此跑到覆盖满的词前面（见 `ranking` 的模块文档）。
    /// 空词、非有限 / 负数的系数直接丢掉。
    pub fn set_word_adjustments(&mut self, adjustments: impl IntoIterator<Item = (String, f64)>) {
        let mut next = HashMap::new();
        for (word, factor) in adjustments {
            if word.is_empty() || !factor.is_finite() || factor < 0.0 {
                continue;
            }
            next.insert(word, factor);
        }
        if next == self.word_adjustments {
            return;
        }
        self.word_adjustments = next;
        // 排序变了：整句那边的格子缓存留着只会给出旧顺序
        self.forget_span_cache();
    }

    /// 脚本给过几条加权 / 降权（日志用）。
    pub fn word_adjustment_count(&self) -> usize {
        self.word_adjustments.len()
    }

    /// 换掉全部附加词库（导入、移除、开关之后），**按第三方词库的添加顺序**排列：查词时靠前的优先，
    /// 同一个词靠前命中后后面的不再重复产出。格子缓存随之作废。
    pub fn set_extra_dictionaries(&mut self, dictionaries: Vec<Dictionary>) {
        self.extra_dictionaries = dictionaries;
        self.forget_span_cache();
    }

    pub fn extra_dictionaries(&self) -> &[Dictionary] {
        &self.extra_dictionaries
    }

    /// 查词用的全部词库，按优先级从高到低：用户词、主词库（云朵基础词库）、稀有词库（开着时才含）、
    /// 附加词库（第三方词库，按添加顺序）。同一个词在靠前的词库里命中后，后面的词库不再重复产出（跨词库去重见
    /// [`Self::lookup_across_dictionaries`]）。
    pub(super) fn all_dictionaries(&self) -> Vec<&Dictionary> {
        let mut all = Vec::with_capacity(self.extra_dictionaries.len() + 3);
        if let Some(user) = self.learner.user_words() {
            all.push(user);
        }
        all.push(&self.dictionary);
        if self.rare_enabled
            && let Some(rare) = &self.rare
        {
            all.push(rare);
        }
        all.extend(self.extra_dictionaries.iter());
        all
    }

    /// 全部词库的词频之和，词频归一化成概率时用。
    pub(super) fn total_frequency(&self) -> u64 {
        self.all_dictionaries()
            .iter()
            .map(|d| d.total_frequency())
            .sum()
    }

    pub fn learner(&self) -> &dyn Learner {
        self.learner.inner()
    }

    /// 拿到可变的 Learner 就当它要改：格子缓存一起作废。
    pub fn learner_mut(&mut self) -> &mut dyn Learner {
        self.forget_span_cache();
        self.learner.inner_mut()
    }
}
