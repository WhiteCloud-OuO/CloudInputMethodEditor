# 各 crate 的实现要点

CLAUDE.md 只保留目录地图与规则，每个 crate / app / tool 的实现细节收在这里：入口类型、数据文件、常数、生成命令。
改了实现要同步改这里；与代码冲突时以代码为准。

## crates/cloudime-dictionary

词库：随包只有一份 `WordBank\Dict.db`（SQLite，`dictionary/db.rs`），每张表都是 `(text, pinyin, language, weight)` 四列 +
主键 `(text, pinyin, language)`（`WITHOUT ROWID`，即去重键），外加 `meta`（名称 / 许可证 / 署名 / 来源 / 格式版本 / 总条数与各表条数 `entries_words_*`）。
中文行按固定 first-match 判据拆成 6 张表（命中即止、互不重叠）、英文行单独一张：
- `words_rare_char`：`weight == 1`（生僻字 / 方言词）
- `words_hanzi`：恰好 1 个字符（单字 / 一级二级汉字）
- `words_rare_word`：`1 < weight < 100`（生僻词）
- `words_place`：末字 ∈ `省 县 乡 镇 市 区 州 府 旗`（地名，暂按末字判）
- `words_long`：字符数 ≥ 4（多字词）
- `words_common`：其余常见词
- `words_english`：`language == 英文`（只按语言，英文行不参与其它判据）
前两张是**稀有组**（生僻字 + 生僻词），其余四张是**普通组**。写盘先去重（`(text, pinyin, language)` 权重大的胜出）再归类，
所以同一条不会因权重跨判据落进两张表。
装配：`DictDb::from_path` 把普通组装成 `DictDb::chinese`、稀有组装成 `DictDb::rare`、英文装成 `DictDb::english`；
`Dictionary::from_path`（单份中文旧入口）把普通组与稀有组合并读回。**查询仍是原来那套二分收窄**：SQLite 只当存档格式，热路径不查库，
`Match` 里的拼音仍借自内存键 arena。
键按字节序排好，查询逐音节位置二分收窄（简拼位置按音节块跳扫），`lookup_pattern`（≥ 模式长度）与 `lookup_exact`（正好等长）同一套实现。
稀有组可整体跳过：`DictDb::rare` 交给 `Engine::with_rare` 挂上，由 `Engine::set_rare_enabled` 开关（缺省关，装配层按 `[word_bank] rare_items` 设置）；
关掉后不参与词级查询与整句词图。英文仍由 `[input] mixture_input` 门控（`push_english` 里早退），从 `words_english` 装配，不再读 `english.tsv`。
存档版本写在 `meta.format`（当前 `3`）；旧存档仍读得回来：`format` 2（单张 `words` 带 `language`，中文行按同一判据拆两组）、
`format` 1（`keys` + `words(key_id, text, frequency)`，只有中文、没有分流）。
`Dictionary::from_path` 按文件头认格式：SQLite → `db::open`（按语言与稀有度分流），老的 `.qj`（mmap）→ `open_qj`，其余当 TSV 解析（样例词库与测试里的内联词库）；
`write_db` / `DictDb::write` 按已排好的顺序整段落盘 + `VACUUM`；`tools/dict-convert word-bank` 用 `DictDb::write`。
设置页「导入词库」的 `import::import` 只收现成的 `.db`：读一遍校验后原样复制进目录（不重写，保住英文行与元数据）。
词库键以 `v` 表示 ü，TSV 解析、查询与生成工具把 `lue` / `nue` 统一成 `lve` / `nve`。
旧 `.qj` 含这些键时，加载器建立规范化的内存词库。

## crates/cloudime-core

模块：`composition`（未选拼音缓冲区、已选文本段与光标；中文模式下 Shift+字母按小写进 `buffer` 参与匹配、大写记在 `shifted`，`typed_text` 还原后用于原样上屏；选中一个候选不立刻落文档，`select_prefix` 把「显示文本 + 它消耗的原样拼音」并进 `selected`，`unselect_last` 供退格撤回）/ `parser` / `correction`（拼写纠错：一处编辑的换位 / 相邻键替换读法进候选池按权重打折，`typo` 音节级变体的多敲 / 少敲写法还进整句词图，见下）/
`candidate` / `ranking`（候选排序：结构键 + 权重，见下） / `shortcut` / `sentence` / `fuzzy` /
`english`（英文候选：中文模式里的中英混输与前缀补全）/ `engine`（`query::EnglishTail`：句末英文词并入整句，`woxiangxuehaorust` → 我想学好rust，尾段也像拼音时按分数与拼音读法比）。

表达式：`shortcut::candidates` 以固定前缀 `v`（`EXPRESSION_PREFIX`）认表达式模式，算四则运算与中文数字；`rq` / `sj` / `xq` 出日期 / 时间 / 星期，候选是 `CandidateKind::Shortcut`，上屏吃掉整段作用域。`Engine::expression_mode` 决定组句中数字与运算符进缓冲区还是选词。

`Engine` 是对外唯一门面，`Learner` trait 在 `engine` 模块；词库是「用户词 + 主词库 + 稀有词库（`with_rare` / `set_rare_enabled`，缺省关闭）+ 附加词库（`set_extra_dictionaries`，按添加顺序）」的列表，**按优先级从高到低**；同一个词在靠前词库里命中后，后面的词库不再重复产出（跨词库去重、靠前优先：词级在 `Engine::lookup_across_dictionaries`，整句词图在 `sentence::span_candidates`，都按 `text` 挡重），**同一本词库内部的重复不去重**，留给 `ranking::rank` / `dedup_by` 按名次保留最高的一条。繁体输出（`traditional` 开关与 `traditional_map` 映射）依赖 `ferrous-opencc`（`s2tw`）在出候选与上屏边界转换，内部保持简体。
- 候选排序分两级：先按**结构键**，同一结构下再按**权重**降序、`hit.text` 升序。结构键（`ranking::Scored`）依序：
  ① 覆盖的输入字母数降序（`kaif` 的 开发 先于只覆盖 `kai` 的 开）；② 非末尾的简拼音节数升序（`kai f a` 是 1、`kai fa` 是 0）；
  ③ 末音节完整匹配降序（`xian` 的 先 先于被当成没打完的 想）；④ 词库命中 `exact`（音节数正好等于查询位置数）降序；
  ⑤ 读法层级升序（原样 0 / 模糊音 1 / 一处编辑纠错 2）。不单列「音节数少优先」：覆盖满 + 末音节对上已经等价于「整段被完整读出来」。
  **两处排序键必须同序**：词级 `ranking::rank` 与候选池 `rank_pool`（`PoolRank = (coverage, abbreviated, full_last, exact, tier, weight)`）。
  词级命中权重 = `词频 × 用户权重因子 × (1+同输入串选择次数) × 上下文系数 × 纠错折扣 × 联想折扣`；上下文系数 =
  `clamp(exp(transition_log_prob − fallback_log_prob), e^-4, e^4)`（`sentence::transition_log_prob`，模型不认识时 log_prob 等于兜底 → 1）；
  联想折扣 = `0.8^k`，`k = max(0, 词字数 − 产生它的读法音节数)`（`lookup_pattern` 带出的更长词）；模糊音命中 0.5。预选按结构键 + 便宜权重
  （`词频 × 用户权重因子 × 纠错折扣 × 联想折扣`）砍到 `limit×2`，不查上下文与同输入串选择次数。
  用户权重因子 `Learner::rank_weight` 替代了原来的 `1 + 全局选择次数`：自造词的权重就是它在用户词库里的词频（初始 = 各字词库词频最大值，
  每次重选 ×1.2），其余词按全局重复次数每次 ×1.15（封顶见 `cloudime-learning`）。
- 中英混输的英文词、整句与中文词一起进同一个池子按上面的键排。英文词没有音节读法，结构键借用整段读法的简拼数与末音节完整性
  （敲的是同一串字母），否则英文词会凭「没有简拼、末音节必然完整」天然压过同覆盖的中文简拼读法（`mp` 的 MP 压 门票）；
  英文权重 = 英文词频 × (1+选过次数)，没有词频按 1.0。句末英文词并入整句（`EnglishTail`）不受影响。
- `custom_phrase`（`CustomPhrase` = 输入码 + 上屏文本 + 可选候选显示内容 `title` + 固定候选位置）：`validate_phrases` 保存与加载共用
  （输入码 1–32 个小写字母、文本非空、位置 1–9、同码同文本不重复；`normalize_phrases` 把 `title` 去首尾空白、空串归一成 `None`）；
  `insert_custom_phrases` 把敲全的输入码对应的短语插到指定位置（1 第一位、2 第二位……），同码多条按位置升序占位、位置相同的按保存顺序往后排、越界的排到最后；
  候选的 `display` 取 `title`（为空时渲染侧回退到 `text`，上屏始终是 `text`）。`merge_replacements` 仍把外部给的「输入码 → 短语」表并进来（新条目用缺省位置 2），Core 不管数据从哪来。
  短语不再进配置文件，存在安装目录的 SQLite（`cloudime_platform::phrase::PhraseStore`，固定 `Phrases\Phrase.db`，两张表 `user` / `cloudime_default`）；
  内置那份随升级更新的做法是**同步源**：安装包把同一份文件另存为 `{安装根}\data\phrase-default.db`（每次升级都覆盖），
  `Phrases\Phrase.db` 用 `onlyifdoesntexist` 保住用户短语；Server 启动时 `sync_defaults` 把源里的 `cloudime_default`
  与当前比对，不同才整表替换（`user` 不动），相同 / 源不在则什么都不做。
  旧数据目录里单表 `phrases` 的库由迁移读一次（位置列 `position` 旧 0 基或 `weight` 都换算成 1 基）。Server 启动与热加载（看文件 mtime）时经 `set_custom_phrases` 装进引擎；`CandidateKind::Custom` 不带位置载荷。
- 整句候选同样进这个池子：整句按整段读法给结构键（覆盖满、无简拼、末音节完整、`exact` 为真）；整句权重 = 路径词权重（`词频 × (1+选择次数) × 联想折扣`）
  的几何平均 × 模型系数（`rescore_paths` 里 `clamp(exp(λ·(神经分 − 静态分)), e^-4, e^4)`，没拿到神经分 1.0）。Viterbi 为每条部分路径累计 `log_weight`，
  `Conversion` 带 `log_weight` / `neural_factor`；同文本同读音的词候选不重复插、同文本不同读音的去掉词级那条、整句顶上。
- 拼写纠错（`correction`）分两路，都按权重打折、原样读法的候选一直保留：
  ① 整段一处编辑（`engine/query/spelling.rs`）：拼音「不像话」（`unlikely_pinyin`）或末尾落单字母（`trailing_single_letter`）时，把**换位 / 键盘相邻键替换**
  能整段切分的纠正读法作为额外候选（`tier` 2，`correction::loose_segmentation` 允许换位末尾没敲完）；仅末尾落单字母触发时只试换位。折扣换位 0.75、替换 0.70。
  原样输入本身能整段对上一个词 / 短语（`hit.exact` 且读法覆盖整段，或自定义短语 `p.code == scope`）时**不再加任何纠错读法**（否则 `Cpan` 会用补出来的
  字母凑成 磁盘 把 C盘 挤掉），折扣也保留不加回补；否则再乘 1.2 / 1.15。
  ② 音节级敲错边（`correction::typo`）：`Engine::expand_positions`（只给整句词图，词级候选不加）对每个完整音节补 **多敲 / 少敲** 仍是合法音节的写法
  （`gan` → `guan`），按 `TypoCosts::typo_cost`（个人敲错表打折）进词图，命中的词 `tier` 1，这样整段合法却不通的 `meiganxi` 也能转出 没关系；
  换位 / 替换不在这里补（走①），整段一处编辑的变体上也不再叠。不到 `correction::MIN_LETTERS`（4）或非末尾带简拼 / 残缺音节的切分不加；
  `typo` 变体表按全部音节算一次（几毫秒），在 `Engine::new` 预热，别让第一个用到的按键买单。
  `Query::correction` 恒为 `None`、拼音行不再画删除线；上屏按那处编辑或音节级敲错换算消耗与敲错对。`engine/correcting.rs` 已删；
  `TypoCosts::correction_penalty` / `correction_transpose_discount` 已无路径调用（类型保留，CLI `--tune` 的 `correction` / `correction-transpose` 已移除）。

`EngineSession` 保存可挂起的组句、标点、历史与学习链，`Engine::swap_session` 在同一个引擎里交换输入状态，共用词库与落盘服务。切换上下文时清除查询及异步重排缓存，并由平台恢复各自私密状态。

`Engine::raw_preedit()`（`engine/raw/`）只读返回 `RawPreedit { text, cursor_bytes }`：完整未上屏组合（已选文本 + 未选拼音）及 UTF-8 字节光标，与随后 `take_raw()` 共用文本生成，保留大小写、显式分隔符及光标后的剩余内容；不运行候选查询、不学习、不记日志、统计、历史或展示回报。首位固定 0，末位固定完整文本长度；`take_raw()` 的提交和清理顺序不变（回车 = 已选文本 + 剩余拼音原样，去掉手敲的 `'`；已选段在选中时已各自记过输入日志，这里只补未选拼音那一段）。

`Engine::discard_input` / `EngineSession::discard_input` 用于隐私能力变化时无痕清理输入，包括透传缓冲、学习链和暂存词汇曝光；`set_private` 只切换写入开关，保留已输入的组句。

## crates/cloudime-learning

- `FrequencyLearner`：用户选择次数（`user.tsv`）、按输入串记的选择（`user-choices.tsv`，词级排序里同输入串选过的优先）、
  导入 / 旧版用户词（`user-words.tsv`，主词库同格式，词频固定 `USER_WORD_FREQUENCY` = 100）、个人英文词（`user-english.tsv`，
  回车原样上屏的英文词与选过的英文候选，与随包英文词表一起出候选且在前）、
  个人敲错表（`user-typos.tsv`，接受过的 (敲的, 要的) 音节对；上屏纠错读法时用它算敲错对，整句词图的音节级敲错边也按它给 `TypoCosts` 打折）与个人 n-gram（`user-ngram.tsv`，Core `sentence::UserNgram`，
  二元 + 三元在线计数，整句转换与词级排序里与静态模型插值）。`Learner::is_user_word`（在自造词库或导入的用户词里）给候选窗的「造」角标用。
  自动造词另有单独的 SQLite 自造词库（`UserWordBank.db`，`user_word_bank`）：位置由壳决定，Server 按 `[word_bank] user_file`
  解析（缺省 `WordBank/UserWordBank.db`，即安装目录的 `WordBank\`；也可写绝对路径），CLI 缺省仍与词频文件同目录（`FrequencyLearner::from_path`）；
  连着选出的两个词合起来词库没有、且连续两次
  （同一段拼音分次选完 / 分两段打，两条阈值都是 2）就记成用户词；初始权重 = 各字在词库里的词频最大值（上屏时 `Engine::initial_user_weight` 算好传给 `learn_word`），
  之后每重选一次 ×1.2、撤销退一次，权重直接当用户词库的词频用，与 `user-words.tsv` 合成一张小词库（`rank_weight` 对自造词返回 1、其余词按全局重复次数每次 ×1.15，撤销靠 `counts` 回落）。
  表里还有一列 `language`：`中文`（自动造词 / 整句收录，按拼音音节查、进用户词库）与 `英文`（Ctrl + 回车 收录的整串字母，进个人英文词表、大小写不敏感地整串匹配，
  初始权重固定 `ENGLISH_WORD_WEIGHT` = 1000）。旧库没有这一列时按中文读。
- `InputLog`：输入日志（`input-log.jsonl`，每次上屏一行：敲的键、切分、看到的前几个候选、选了第几个、来源、纠错（字段保留、现已恒 false）、撤销，
  Core `InputLogger` trait 的落盘实现，`[general] input_log` 缺省开，只写本机，给离线回归评测与个人模型用）。
- `UsageStats`：输入统计（`usage.tsv`，按天记汉字 / 中文词 / 英文词 / 上屏次数，Core `UsageMeter` trait 的实现，Engine 每次上屏 `Usage::of_text` + 按来源定词数，
  整句按 `segment_text` 切词数；与输入日志无关，设置「调试」页的「输入统计面板」显示）。
- `VocabularyBook`：词汇记录（`user-vocab.tsv`，Core `VocabularyTracker` trait 的实现：候选窗口里出现过 / 上屏过的词各记几次；Core 私密输入统一跳过曝光和提交写入；「看到」按上屏那一刻屏幕上那一页算，壳每次画完 `Engine::note_displayed` 告知当前页）。
- 各表落盘走 Core `storage::write_atomic`（临时文件 + fsync + 改名），加载按行容错（坏行警告跳过，真读不了壳退回内存学习），
  Server 处理消息时顺带按 `LEARNING_FLUSH_INTERVAL`（60 秒）调 `Engine::flush_learning` 落盘；崩溃由 Server 进程兜底重启，见 architecture.md「崩溃不丢」。

## crates/cloudime-format

`.qj` 数据容器（`Container` mmap 读、`Writer` 写、`Table<T>` / `Text` 零拷贝视图、`hash` 可落盘哈希索引、`Metadata` 名称 / 许可证 / 署名）。
魔数是 `CLOUDIME`；改名前的 `QINGJIAN`（随包 `data-v2` 还是它）读的时候一并接受，容器布局一字未动，写出去仍是 `CLOUDIME`。随包数据重生成、`data.lock` 换到新 tag 后可删这段兼容。
语言模型能 `write_qj` / 从 `.qj` 打开，启动 50 ms；`cargo run --release -p cloudime-dict-convert -- pack lm --name … --license …`
生成 `data/generated/lm.qj`，Windows 打包（`apps/windows/installer/build.ps1`）在 TSV 更新时自动重打并只把 `.qj` 与 `WordBank\Dict.db` 打进包。词库改走 SQLite（`word-bank` 子命令写 `WordBank\Dict.db`）。设计见 `docs/design/architecture.md`「数据文件：`.qj` 容器」。

## crates/cloudime-neural

`CharScorer`，Core `sentence::SentenceScorer` trait 的实现：candle 加载字级 Transformer（GPT-2 风格 decoder，训练仓库（本地 `../train`，私有，不在本仓库）导出的
`model.safetensors` + `config.json` + `vocab.json`），给「前文 + 整句」按字累加 log 概率；前文的每层 K / V 缓存（`PrefixCache`），
同一段前文只算一次，每个候选只算自己那几个字。缺省 CPU（candle 自带的 gemm）；crate 还留着 `accelerate` / `metal` 两个 features，Windows 上没人开。
`.qjm`（`qjm` 模块：`.qj` 容器 `Kind::Model` 装 `config` / `vocab` / 权重三节）是随包与用户目录的形态；`find_model` 在目录里挑模型时优先**词表含汉字**的那份（各用途的模型可以放在一起），都不像字级才退回文件名排序第一份。

Engine 侧在 `engine/rescoring/`：接了打分器就取 Viterbi 前 `RESCORE_PATHS` = 6 条路径按 `路径分 + λ·(神经分 − 静态二元分)` 重排（λ `NEURAL_WEIGHT` 0.5，
个人 n-gram / 用户加分 / 代价不动），分走「前文 + 文本 → 神经分」缓存 `NeuralCache`；同步打分器（`with_sentence_scorer`，CLI 评测）当场补分，
异步的（`with_async_sentence_scorer`，后台线程 `RescoreWorker`）查询不等模型：缺分的记下来，壳停键后 `request_rescoring`、`poll_rescoring` 到了再 `query` 一次。
前文优先用壳给的应用光标前文（`set_rescoring_context`），没有用本会话最近 64 个上屏字符。CLI `--neural <导出目录>`（`--neural-weight` / `--neural-context` / `--neural-async`）。

## crates/cloudime-lm

`BigramModel`，Core `sentence::LanguageModel` trait 的实现，从 `data/generated/lm.qj`（或 `lm-unigram.tsv` / `lm-bigram.tsv`）加载
（没有这两个文件就退化为一元词频整句）。数据由 `tools/corpus/parquet_to_text.py`（uv 脚本，HF parquet → 简体纯文本）加
`cargo run --release -p cloudime-dict-convert -- bigram --phrases assets/lexicon/phrases.tsv --phrases assets/lexicon/domain_words.tsv --brand assets/lexicon/brand.tsv --brand assets/lexicon/mixed_words.tsv data/corpus/*.txt` 生成；语料在 `data/corpus/`（gitignore）。
短语层不当 token 统计（分词时摘掉、统计完按成分合成一元 / 二元，短语得分等于原来两个词的路径，见 `bigram.rs` 模块注释），品牌词按给定次数写进一元与句首二元。

## crates/cloudime-platform

`Config`（TOML 配置文件，`[input]` / `[candidate]` / `[general]` / `[phrase]` / `[word_bank]` / `[status_bar]` / `[debugging]` / `[update]` 分节，首次运行写模板，
`set_value` 用 toml_edit 原地改键保留注释（分节可带点，`candidate.pinyin_font` 这种子表也走它））；
`[candidate]` 收拢候选窗口那一套：`use_local_sentence_organization_model`（原 `[model] enabled`）、`candidate_arrangement_direction`、
`candidate_count`（夹 5–9）、`candidate_association_counts`（联想候选项目上限，夹 0–4；候选列表里「比读法更长的词」最多留几条，
见 `ranking::rank` 的 `association_limit`）、`pinyin_font` / `candidate_font` / `item_number_font`（各 `family` + `size` 的子表）、
`item_number_style`（decimal / circled / roman / dingbat / parenthesized）、`candidate_box_minimum_width`（物理像素，竖排时生效）、
`show_more_candidate_items`（只落配置，功能未做）、`program_list_of_hiding_candidate`（名单里的 exe 完全不接管，
`InputSettings.raw_input` 通知 DLL）、`preedit`；`[status_bar]` 只剩位置（开关删了，状态条常开，只跟「当前输入法是不是云朵输入法」走）；
`[status_bar]` 有 `show_status_bar`（缺省开；`StatusBarConfig` 手写 `Default`——`#[serde(default)]` 取的是结构体的 `Default`，不写就变成 `bool::default()` = 关，老配置会莫名其妙不显示工具条）：关掉后 Server 的 `reconcile_status` 始终收起，桌面上不再出现那条工具条（按钮位置 `x` / `y` 照旧）。
`[debugging]` 只有 `auto_hide_float_tool_bar`（缺省开）：开着时前台全屏（`ui/status/fullscreen.rs` 每秒查一次）收起悬浮工具栏，
关掉后全屏也不收；切到别的输入法 / 云朵被禁用时始终收起，与这一项无关（`RouterConfig.auto_hide_float_tool_bar` → `StatusView.auto_hide_fullscreen`）。
配置按「输入 / 候选 / 词库 / 短语」四节重排完成：`[input]`（原来散在 `[general] traditional`、整个 `[fuzzy]`
与写死在 Core 里的标点规则）：`use_jian_pin`、`mo_hu_yin_list`（位图，**一位一条规则**共十二位：zh/z 1、ch/c 2、sh/s 4、r/l 8、n/l 16、
f/h 32、u/ü 64、uo/o 128、an/ang 256、en/eng 512、in/ing 1024、wang/huang 2048，`MO_HU_YIN_BITS`；0.0.3 起从「五组合并位」改成一位一条，
老配置同一数值的含义会变，用户重勾一遍即可。Core 侧的规则见 `FuzzyRules`：`uo_o` 走 `luo`↔`lo` 这类合法音节对，
`wang_huang` 是 `wang`↔`huang` 整音节成对互换，只对完整音节生效）、`simp_trad_chinese_chars_toggle`、`mixture_input`、`full_half_punctuation_marks_toggle`、
`[input.punctuation_marks_mapping]`（位图：`/`→`、` 1、小键盘 `/`→`÷` 2、小键盘 `*`→`×` 4、`~`→`～` 8、`·`→`` ` `` 16，
`PUNCTUATION_MAPPING_BITS`，只做单键替换；老配置里的表与 `~=` 这类两键规则已下线，读到表退回缺省位图）、`punctuation_marks_pairwise_completion`（位图）、
`use_half_wide_punctuation_marks_after_digital`；`[general]` 里 `Shift` + 字母那一项已删（固定进组句，见上），`page_keys` 也已删（翻页键固定主键盘 `-` / `=`），
`[shortcut]` 整节删除（表达式前缀固定 `v`，问字与删候选整体下线）；`[phrase]` 只有 `use_default_phrases`（缺省 `true`：软件自带短语是否参与；短语库固定在安装目录 `Phrases\Phrase.db`，不再有 `file`），
短语读写走 `phrase.rs` 的 `PhraseStore`（SQLite：`user` / `cloudime_default` 两张同构表，字段 `id / code / text / title / position`；`load(use_default)` 先读 `user`、自带那份里 `code` 已被用户占了的丢掉，`save_user` 只覆盖 `user`——先把现有文件拷到同目录临时文件再在临时文件上重写，`cloudime_default` 原样保留——连接都设 busy timeout）；
`[word_bank]` 分节有 `rare_items`（缺省 `false`：开启后候选与整句才从稀有组取词，关闭更快；第三方词库目录固定随包根的 `WordBank\`）
与 `user_file`（用户自造词库位置，相对安装目录、缺省 `WordBank/UserWordBank.db`，也认绝对路径；`DEFAULT_USER_WORD_BANK_FILE`）；
`word_bank.rs` 有 `locate` / `path` / `user_file` / `files` / `imported` / `is_builtin` / `describe` / `load_except` / `main` / `snapshot` / `snapshot_files`
（`user_file` 就按 `[word_bank] user_file` 解析：绝对路径直接用，相对路径相对随包根，即 `WordBank\` 的父目录），
随包主词库是 `WordBank\Dict.db`（7 张表：中文普通组 / 稀有组 + 英文；附加词库按 `Dictionary::from_path` 整份读，不拆稀有组），
设置 → 词库页只列导入的第三方词库（`imported` 过滤掉内置 `Dict.db` / `UserWordBank.db`；两者没有开关、始终加载），
`load_except` 也把这两个内置排除在外——自造词库挪进 `WordBank\` 后不能再当第三方词库装一遍（它由 learner 专门管）；
用户导入的附加词库是同目录下另外的 `.db`（目录里有的全部加载，没有 List.dat 启用清单）；
原来的 `[dictionaries] domains / disabled` 与
`%APPDATA%\CloudIME\dicts\` 都删了；
中英模式没有配置项：`[shortcut] switch_mode`、`[general] english_mode` 与整个 `[apps]` 分节都已删（缺省配置模板里也没有），
`SwitchKeys` 类型只为协议留着（`InputSettings::switch_mode`），Server 恒发 `SwitchKeys::default()`；
`migrate.rs` 是升级时的一次性迁移（`cloudime_platform::migrate::migrate(config_path)`，Server 与设置程序都在读配置前调一次，
幂等）：旧 `[general] page_size / layout / font / preedit / traditional / page_keys`、`[fuzzy]`、`[model] enabled`、`[status_bar] enabled`、
整个 `[shortcut]`（`expression` / `question` / `question_mark` / `delete_candidate`）、
`[[custom_phrases]]`（位置 1–9 直接夹到 1–9）与老数据目录的 `Phrase.db`（单表 `phrases`，位置换算成 1 基；只在新的 `user` 表为空时搬，搬完改名 `Phrase.db.migrated`）搬进短语库的 `user` 表，
`[dictionaries] domains / disabled` 删，写回前留一份 `config.toml.bak`；重写用 `config::write_with_template`（从模板起步保住注释，再把序列化出来的值覆盖上去）；
`protocol` 模块是 Windows Server ↔ TSF DLL 的 IPC 协议类型
（`ClientMessage` / `ServerMessage` / `Frame` / `PreeditSegment`，全 serde，两端共用，见 `docs/design/architecture.md`「Windows：TSF」；
`PROTOCOL_VERSION` = 15（v9 删掉 `CandidateKind::Emoji` 变体——删枚举变体老 DLL 同样整条帧解析失败；
v10 给 `InputSettings` 加了 `full_width_chars`；v11 给 `ServerMessage::KeyResult` 加了 `caret_shift` 与 `delete_before`
（成对补全把光标停在括号中间、两键符号规则撤掉上一个键的输出）；v12 给 `InputSettings` 加了 `raw_input`（「不显示候选框」名单里的
程序完全不接管）；v13 去掉 `CandidateKind::Custom` 的固定位置载荷（短语改成按权重整体排在最前）；
v14 把中英的布尔换成三态 `InputMode`（中文 / 英文 / 禁用，`ModeChanged` 与 `ModeSync` 的字段跟着变），
`IndicatorCommand` 加了 `ToggleCharWidthType` 与 `ToggleSimpTrad`（`Shift + Space`、`Ctrl + Alt + .` 两个内置热键）；
v15 给 `IndicatorCommand` 加了 `RestartServer`（托盘菜单的「重启输入法服务」，见 apps/windows 一节）——加 / 删枚举变体
与删字段一样，老 DLL 整条帧解析失败、必须 +1 并重装 DLL，老 Server 下点了这一项没反应）。
v15 之后给 `Candidate` 加了 `display`（候选里显示的内容，上屏仍用 `text`）：带 `serde(default)` 的新字段两边仍能对话，
老 DLL 只读候选条数、不读内容，行为不变，所以没有 +1。

## crates/cloudime-render

横排矩阵：`Frame::columns` 不为 0 时 `Layout::Horizontal` 走 `renderer/matrix.rs`（列宽用帧里的 `column_ems`，Core `Grid::column_ems` 按整份候选估、
单格封顶 `MAX_CELL_EMS` = 4 字宽，滚动时窗口不跳；网格下固定一行信息，放不下的截断）；视口与高亮移动在 Core `candidate::layout::Grid`（`GRID_ROWS` = 6），
预览示例里有 `matrix-horizontal` 场景。Windows 上这套矩阵按键尚未接线（`Frame::columns` 只有横排固定一行信息）。

自绘渲染器：候选窗一帧 + 主题 → 预乘 RGBA 位图，tiny-skia 栅格 + cosmic-text 文字（fontdb 按清单只加载几个字体文件、不扫系统），
自己解析 `trak` 字距表、按主题 gamma 加深笔画；配色只有一套（`Theme::new()` / `Palette::new()`，浅色单套，不再分深浅；拼音串与候选项序号是纯黑，页码仍是弱化的灰）；
cosmic-text 打了 `opsz` 光学字号补丁（qingjian-team/cosmic-text 分支 `qingjian-opsz`，workspace `[patch.crates-io]` 钉 rev）。
状态条是一排 `StatusCell::Icon`（整份 `<svg>` 源码）：`svg.rs` 用 resvg 0.48（workspace 钉版、`default-features = false`，
不扫系统字体、不要 svgz 与光栅图解码）解析光栅化，按 SVG 宽高比缩进 `BUTTON_SIZE` = 20 pt 的方按钮居中叠上去，解析失败返回 `RenderError::InvalidSvg`；
`render_status` 只画图标、不留文字格与分隔线，按钮之间与四周各留 `BUTTON_GAP` = 6 pt，返回各按钮右边界供点击命中。
`examples/preview.rs` 出 PNG 与真机截图并排比、`--measure` 量宽度。Windows 壳候选窗口与悬浮状态条都走这条渲染路径（`ui/painter/`，`ui/layered/` 贴位图），
字体库加载失败时都不绘制、不显示并记日志；`[general] font` 是候选窗字族名（空为系统字体，`fonts/directwrite.rs` 按字族名找字体文件只加载那几个，没装就回系统字体；
设置页 `candidates.rs` 的字体框是边输入边提示的自动补全）。设计与验收见 `docs/design/rendering.md`。
候选行右侧的**来源角标**：`Row.badge`（Server `dispatch/candidates/mod.rs::badges_of` 按 `kind == Custom` 判「短」、
`kind == Sentence` 判「句」、`Learner::is_user_word` 判「造」，其余没有），用序号字体、`Palette.badge` = `#888888` 右对齐画在候选格内（间距 4 pt）；
横排按项宽、竖排按行宽给它留出位置。
**高亮条移动动画**：`Frame::highlight_animation`（`HighlightAnimation`，起点矩形 + 进度 0..=1）让渲染器从起点矩形 lerp 到目标行矩形
（`frame/highlight/rect.rs` 的 `HighlightRect`，内容区坐标，`HighlightRect::lerp`）；没有它就画在高亮行。三种布局都先把各行 / 各格的高亮矩形量好再把条子画在文字之前，
并把这份矩形随 `Rendered::highlight_rects` 交给壳（壳连按方向键时按当前视觉位置续滑）。纯展示，不影响排序与按键。

## crates/cloudime-update

检查更新（设计见 `docs/design/update.md`）：`index/` 是索引的类型、下载（`fetch.rs`，复用 workspace 的 reqwest + 单线程 tokio，20 秒超时、2 MB 上限）与验签
（`signature.rs`，`PUBLIC_KEYS` 列表，`verify_strict`）；`checker/` 是调度（`Checker::poll` 由壳的每秒定时器调，到点起一次性线程）、落盘状态 `UpdateState`（`update.json`，先写临时文件再改名）
与查到的结果 `Available`。`Version` 自己实现语义化版本比较，不引 semver。`[update]` 配置与 `UpdateChannel` 在 `cloudime-platform`。
`examples/check.rs` 手动走一遍；`tools/release-sign` 是发版侧的 keygen / sign / verify。

## apps/cli

测试工具，`cargo run -p cloudime-cli -- kaifa`。

- `--typing` 逐键计时（性能测试用 release 构建跑，目标每键 10 ms 以内）。
- `--replay <input-log.jsonl>` 回放评测：把日志里每次上屏的键重新喂给引擎，按来源算首选 / 前五命中率、平均名次、不在候选的条数，打印没命中的例子（`--misses N`）；
  只在内存里学习不写文件，加 `--user-dict` 可带上现有学习数据。
- `--tune 名=值`（逗号分隔）覆盖个人 n-gram 插值与敲错代价的常数扫网格（名字见 `apps/cli/src/tuning.rs`，Core 侧是 `Engine::set_interpolation` / `set_typo_costs`，壳只用缺省值）；
  音节级敲错边（多敲 / 少敲）回来之后 `TypoCosts::extra` / `missing` / `typo-cap` 又参与整句打分；`transpose` / `substitute`（只走整段一处编辑，
  用固定折扣 0.75 / 0.70）与 `correction` / `correction-transpose` 仍是惰性旋钮（后两个键已从 CLI 移除）。
- `--eval-text <文本>...` 整句评测：把用户自己写的中文文本按标点切句、按词库读音转成全拼，冷启动喂给引擎看整句能不能还原原句
  （首选命中率 / 字准确率 / 查询耗时；不依赖日志里当时选了什么，给整句排序与语言模型的改动当尺子），`--eval-save` 冻结成 `句子\t拼音\t上文` 三列文件，
  之后直接 `--eval-text` 它保证比的是同一份句子（本机的在 `data/eval/sentences.tsv`）。排序、整句、纠错的改动先跑它们再合。

## apps/windows

一个产品两个 package：`server`（Server 进程：IPC 分派 + Engine + 命名管道 + 自绘候选窗与悬浮状态条）与 `tsf`（TSF 文本服务 DLL，lib 名固定 `cloudime_tsf`），
外加 `settings`（WinUI 3 设置程序）与 `installer`（Inno Setup）。

配置是这些页面背后的唯一通路：设置程序与 Server 都只读写 `config.toml`，Server 激活期间每秒看一次 mtime，变了就 `apply_config` 热加载。
词库在随包根的 `WordBank\` 下，主词库是 `Dict.db`（7 张表：中文普通组 / 稀有组 + 英文，`DictDb` 按语言与稀有度分流；稀有组由 `[word_bank] rare_items` 控制、缺省关闭），用户导入的附加词库也在同目录、都加载。

启动装配（`server/src/assembly/`）里语言模型读不出来就**就地降级**——退化成一元词频整句——而不是让 Server 起不来；
只有主词库坏了才回落 `assets/sample/dict.tsv`，连样例都装不起来才 `exit(1)`。数据坏了只该掉效果：曾因为随包 `.qj` 还是改名前的魔数，
Server 装配直接退出，表现成「装完打不出候选、按键没反应、状态条也不显示」。
本地整句模型：`dispatch/rescore/` 按 `[candidate] use_local_sentence_organization_model` 在后台线程加载预热、停键后重排，见 `docs/design/architecture.md`「本地整句模型」。
热加载的 `WordBank\` 目录与启动同款（按 `WordBank\Dict.db` 与附加词库的 mtime / 长度快照重装）；
短语库（安装目录 `Phrases\Phrase.db`）也按文件 mtime 单独热重读；`[phrase] use_default_phrases` 变了也重读一遍。不合成一个 crate，因为 DLL 不能带 Engine 的依赖树，见 `apps/windows/README.md`；
协议类型在 `cloudime-platform::protocol`，设计见 `docs/design/architecture.md`「Windows：TSF」。
中英模式的两项值（`InputSettings::switch_mode` 切换键、`InputSettings::english_mode` 内置英文模式开关）
由 Server 经协议下发给 DLL（`InputSettings`，见本节末尾），**恒为固定值**（`SwitchKeys::default()` 单击 shift 与 `true`，设置里已没有对应选项）；
`ctrl+alt+space` 协议仍支持，走 TSF 保留键登记（`com/key/preserved.rs` 的 `GUID_SWITCH_MODE`），缺省不启用。
输入法状态是三态（`protocol::InputMode`：中文 / 英文 / 禁用）：中 / 英由单击 Shift（`KeyTap`）与语言栏 / 状态条按钮切；
`KeyTap` 在「别的键还按着」或「按下超过 800 ms」时不认单击——真机上敲 `Shift + 标点` 手滑（标点先按下、Shift 晚一拍）
会把 Shift 抬起误判成单击，输入法莫名切到英文，日志里只看得到隔几十毫秒到几秒的 `切到英文模式`；
2026-10-06 又从 DLL 日志抓到第二种漏网：`Shift + "` **快速**敲（双引号先到、Shift 的按下通知晚到）也会误切，
而慢敲不会——TSF 对修饰键的投递顺序不保证，「别的键按下就作废」早了一步。兜法是 `KeyTap::key_down` 多收一个
`switch_held`（调用方查 `GetKeyState`：勾着的切换键此刻**物理按着**没有），别的键按下时若为真就记下这次切换键
是被当修饰键用的（`held_as_modifier`），抬起不算单击；下一次切换键抬起时清掉这个标记；
系统 Ctrl + Space（「输入法/非输入法切换」）翻的「输入法开 / 关」compartment（`com/mode/sink.rs`）关 = **禁用**、开 = 回到禁用前的中 / 英，
我们切状态时把开关写成一致（`refresh_mode_indicator`：`open = !disabled`）；开关一变也作废被截走 Space 的那次「单击 Ctrl」。
禁用 = 完全不接管（`would_eat` 直接放行）、悬浮状态栏收起（Server `reconcile_status` 只在 `ime_active && !disabled` 时显示）、
任务栏图标用 `mode/off-*.alpha`（`icon.rs` 的 `Glyph::Off`）。Caps Lock 亮着时单击 Shift 会先 `SendInput` 补一次 Caps Lock 清掉锁定、再切英文。
内置热键（固定，不落配置）：`Shift + Space` 翻全角 / 半角（DLL 在 `OnTestKeyDown` / `OnKeyDown` 里拦，发 `IndicatorCommand::ToggleCharWidthType`）、
`Ctrl + Alt + .` 简繁与 `Ctrl + Alt + ,` 标点（两个 TSF 保留键，发 `ToggleSimpTrad` / `TogglePunctuation`，Server 都走 `handle_status_event` 那条路）。
状态全局一份、存在 Server（`Router.mode: InputMode`），DLL 激活 / 得到焦点 / 轮询时 `SyncMode` 取回，用户切了 `ModeChanged` 报上去。会话号用线程 id（`com::session_id`）——TSF 的 client id
各进程都是同样那几个值，拿它当会话号会在 Server 那边撞号。四条切换入口都汇到
`service/mode.rs::switch_mode` 一处拦住（内置英文模式下发为关时才拦英文，协议值现在恒开）；状态条点击在 Server 侧（`dispatch/status/mod.rs`）走同一条路，
「输入」页在 `settings/src/panel/pages/input.rs`，
对应 `[input]` 那八项（模糊音、符号映射、符号成对补全三组勾选都用 `ListView`，每项是一份 `ListViewItem`、内容就是一个自带文本的 `CheckBox`（`CheckBox` 是 `ContentControl`，文本直接 `.content(label)`，不用再套 `TextBlock`）；列表共用 `controls::scroll_list`——`selection_mode(None)` 不要选中高亮，`max_height` 卡在 5 行（`LIST_ROW_HEIGHT × LIST_VISIBLE_ROWS`），超出的组（符号成对补全 10 项）由列表自己出滚动条；简繁单选与候选页的排布单选走 `controls::radio_row`——**独立 `RadioButton`** 横排（同一个 `group_name` 互斥），**不用框架的 `RadioButtons` 容器**：容器一行内容的「期望高度」比实际渲染矮（渲染 32、期望 25），渲染出来的选项比自己盒子低 3.5px，左边的标签按盒子居中后看着总差一点，从外面（`min_height` / 套一层面板）也调不动；独立控件和开关 / 下拉一样是单控件，居中对得上；标点全半角下拉等）；标签与控件默认**垂直居中**（`controls::labeled`，标签不设对齐会被拉伸到整行高、文字却画在自己顶部，40 高的开关行里就偏上约 10px）；一行很高的控件（`ListView`、单选）改用 `controls::labeled_top` / `field_top`，标签顶对齐、与第一行内容对齐；「候选」页在 `pages/candidates.rs`，
对应 `[candidate]`（本地整句模型开关、排布单选、个数滑轨（右侧跟一个当前值数字）、联想候选项目上限滑轨 0–4、三个「字体…」按钮弹系统字体对话框 `font_dialog.rs`、
序号样式下拉、最小宽度、展示更多候选项、按程序隐藏的名单（下面每行一个 2 列 `Grid`——`Star` 列放程序名、「删除」按钮放第二列，外壳用 `controls::scroll_list` 卡 5 行，多了自己滚动））；「短语」页在 `pages/phrase.rs`（顶部「启用软件自带短语」开关；列表每行是一个 5 列 `Grid`——短语内容（`Star` 列，`Wrap` + `max_lines(3)` + 省略号）/ 候选内容 / 触发字母串 / 位置 / 编辑·删除两个按钮，列宽全部钉死（含操作列，否则表头那行没有按钮、`Star` 列会多占一截导致表头与数据行错位）；表单 + 列表读写安装目录 `Phrases\Phrase.db` 的 `user` 表）；
「调试」页在 `pages/debugging.rs`：**原「统计」页整页搬来的输入统计面板**（末尾是「数据与组件」说明）与紧随其后的 `[debugging]` 自动隐藏开关、
原来「高级」页的数据 / 日志入口（打开数据目录 / 打开日志目录 / 打包日志到桌面 / 清空输入日志四个按钮一行）与项目 GitHub 页面、详细日志、学习输入习惯、记录输入日志。
「通用」页已删（`Shift` + 字母固定进组句，见 `dispatch/key/input.rs::apply_chinese`：字母进缓冲区、`Caps Lock` 亮着的仍直通），
「统计」与「高级」两页并进「调试」；「关于」页已整体删除（版本在「数据与组件」里仍有一份，检查更新只剩 Server 侧（查并写 `update.json`，界面上不再提示），许可与数据署名看 `LICENSE` 与 `docs/design/landscape.md`）。
设置窗口的标题栏图标走 `ViewContext::window_visuals(WindowVisuals::new().icon(path))`（`component.rs::window_icon`）——WinUI 3 不会自动取 exe 里的图标资源，必须显式 `AppWindow.SetIcon`，而那个接口只收 `&'static str`，所以算一次「exe 旁 `cloudime.ico`」的绝对路径再 `Box::leak`；装机包由 `cloudime.iss` 装这份 ico，开发时 `settings/build.rs` 往 exe 旁拷一份。
设置窗口打开时的客户区写死 `WINDOW_CLIENT_SIZE`（`component.rs`：本机系统默认 1912×1028 的「宽取 2/3、高不变」= 1275×1028），再用 `clamp_to_work_area` 夹进主显示器工作区，小屏不顶出屏幕。**这个尺寸必须在第一次 publication 里就给具体值**：框架是「建窗 → 应用 `WindowVisuals` → `Activate`（显示）」三步，晚一步（让窗口先按系统默认显示、再靠 `on_window_size` 缩）用户就会看到「先宽后窄」闪一下；而那一刻窗口还没建出来（第一次 `view` 时枚举本进程窗口，一个都没有），量不到系统默认值，所以只能写死。`client_size` 收 DIP，框架自己按窗口 DPI 换算成像素。
窗口**位置**居中走同文件另一个法子：框架的 `WindowVisuals` 没有位置，等它再到组件里跑一趟（下一次 `view`）窗口已经显示了，挪过去会看到「先左后中」闪一下（实测约 2 帧）。所以 `create` 里装一个**本线程的 CBT 钩子**（`SetWindowsHookExW(WH_CBT, …, GetCurrentThreadId())`，本进程自己的窗口、钩子过程不必进 DLL）：`HCBT_ACTIVATE` 在窗口真正显示**之前**同步回调，在那里 `SetWindowPos` 到所在显示器工作区正中就看不到闪动；`view` 里还留一条「按 pid 找窗口再挪」的兜底（`center_window_once`，`CENTERED` 一次性开关）。

托盘「中 / 英」图标的右键菜单（`tsf/src/com/mode/menu.rs`，`TrackPopupMenuEx` 挂输入框所在窗口）固定四项、不再切中英：
灰显的「云朵输入法」标题、分隔线、「设置」（`IndicatorCommand::OpenSettings`）、「重启输入法服务」（`IndicatorCommand::RestartServer`）。
DLL 只把选中的命令发给 Server（`com/service/menu.rs::show_indicator_menu`），原菜单上的中 / 英、全角标点与「有新版本」入口一并去掉；
协议里 `IndicatorState` / `ModeSync.indicator` / `IndicatorCommand::OpenDownload` 都保留，Server 仍算 `update_available`，只是界面上没有入口。
「重启输入法服务」在 Server（`dispatch/status/mod.rs::handle_indicator`）：起一个新实例（`current_exe()` + `--wait-pid <本进程 pid>` +
`CREATE_NO_WINDOW`、工作目录设为 exe 所在目录），置 `Router.restart_pending`；`ipc/pipe.rs::serve_pipe` 把这次回包写出后 `break`、
进程正常返回（日志刷盘，比 `process::exit` 干净）。新实例在 `main.rs` 装好日志后按 `--wait-pid`（纯函数 `restart_wait_pid`）用
`OpenProcess` + `WaitForSingleObject` 最多等旧进程 15 秒再占管道，避开 `FILE_FLAG_FIRST_PIPE_INSTANCE` 的抢管道失败。

「不显示候选框」名单（`[candidate] program_list_of_hiding_candidate`）：Server 按会话的 exe 名（`SessionInfo.app`）算出
`InputSettings.raw_input` 下发给 DLL，DLL 彻底不吃键（`would_eat` / 断连时的兜底都放行），按键原样交给应用 —— 编辑器里
用它自己的补全列表、游戏里不挡画面。

`[input]` 的四项新能力：简拼（`Engine::set_use_jian_pin`，关掉只认完整音节与前缀——`parser::segment_with` 不产生声母缩写，
`segment_longest_prefix` 也不让「截出来的那头以残缺音节结尾」的退让；纠错那边仍按能切到什么看）、中英混输
（`set_mixture_input` 关掉 `insert_english` 与 `split_english_tail`）、中文模式符号映射（`Punctuation::symbol`，两键规则返回
`MappedSymbol { text, delete_before }`，壳把它变成协议里的 `delete_before` 让 DLL 删掉上一个键的输出）、成对补全
（`dispatch/key/input.rs::complete_pair` 按**转换后**的字符查位图，补上右半边并置 `caret_shift = -1`；右半边又敲一次就置 `+1` 跳过，
靠 `Router.pending_close`。没转换的左半边（半角标点、`{` 这类不在全角表里的键）也在这条路上补 ASCII 的一对；
中英一致——只要开了成对补全、键没被全角表转换就补，英文 + 西文符号就是这条路）。成对补全位图按界面顺序连续排（`()` 1、`[]` 2、`{}` 4、`""` 8、`（）` 16、`【】` 32、`｛｝` 64、`《》` 128、`“”` 256、`‘’` 512）。
组句里敲标点（`,` `.` `?` 等可打印 ASCII 标点）先选上当前高亮候选、把剩余拼音原样补完整体上屏，再按「没在组句」处理这一键（走符号映射 / 全角标点 / 成对补全），
见 `dispatch/key/input.rs::{is_commit_punctuation, finish_composition}`：`-` `=` 是翻页键，它们上档的 `_` `+`（标识符里常见）与拼音分隔符 `'` 排除在外、仍进英文直输段。
候选与标点由 `with_prefix` 合成一次 `Changed`，否则应用会先插标点再插词（Windows 放行是同步的、上屏走异步编辑会话）。
组句里按 `Insert`（`codes::INSERT`，DLL 的 `is_edit` 把它算进「组句中要吃的功能键」）：`Engine::take_selected` 只把**已选**的中文交出去、未选的拼音直接丢掉（`云朵shurufa` → 上屏 `云朵`），一段都没选时就只是丢拼音、不往文档里写东西；已选段在选中时已各自记过上屏记录，这里不再补记。
组句中的 `Ctrl + 数字`：用户短语直接上屏、整句候选记录一次再上屏（同一整句记够两次由 `Engine::remember_sentence` 收进
`UserWordBank.db`，与逐词拼共用阈值 2 与初始权重）、其余中文 / 英文候选是**杀词**——
DLL 的 `eats_key` 只为这一种命令键组合放行（其余 Ctrl / Alt / Win 一律归应用），
Server `dispatch/key/input.rs::ctrl_digit` 按键码认数字（按住 Ctrl 时 `character` 是控制字符）、取当前页那一格的候选：
短语 / 整句直接 `commit`（整句先记一次），其余调 `Engine::forget`（自造词从 `UserWordBank.db` 整个删掉、本地不再出；词库已有的词清掉选择次数 /
同输入串选择 / 相关 n-gram，权重回到词库原始词频），写一条候选窗提示并返回 `Changed(None)` 触发重排；
没这一格、或这一格既不是短语 / 整句也不是中文 / 英文候选时回 `Passthrough`，应用照常收到这一键。
组句中的 `Ctrl + 回车`（同样由 DLL `eats_key` 放行）：原样上屏当前字母串（去掉手敲的 `'`，见 `Engine::take_raw`），并先把这一串记一次；
同一串（大小写不敏感）记够两次由 `Engine::take_raw_english` → `learn_english_word` 收进自造词库（`language` = 英文、权重 1000）。
组句中的 `` ` ``：候选里有英文词时由 `dispatch/key/input.rs` 调 `Engine::cycle_english_case` 轮换英文候选大小写
（`EnglishCase` 全小写 / 全大写 / 首字母大写，套在 `push_english` 的展示与上屏文本上，开一段新拼音回到全小写），
所以它从 `is_commit_punctuation` 的「上屏候选 + 上屏标点」里排除；没英文候选时仍进英文直输段。
光标落点与删字都在 DLL 侧（`composition/mod.rs`）：
删字走 `Collapse(TF_ANCHOR_START)` + `ShiftStart(-count)`——这正是 `surrounding.rs` 往左读前文的写法，
判据用 `ShiftStart` 回报的 `pchSkipped`（**不要**用 `IsEmpty`：它在部分宿主上会直接失败，把已经挪动的
范围误判成没挪动，真机踩过），并且要用 `GetSelection` 给的**原始范围**、不能先 `Clone`（真机上 `Clone` 出来的
范围 `ShiftStart` 报告「挪了 0」，怎么都不动），拉不开再按 `ITfRangeACP` 的位置圈；成对补全要停在两符号中间，改**注入方向键**
（`SendInput` 左 / 右，在编辑会话里发、会话返回后应用才处理，那时文本已落好），因为部分宿主会覆盖 TSF 的
`SetSelection`（真机上光标一直停在末尾，而同一应用对方向键的响应是好的）；`SendInput` 被拒（AppContainer / UIPI）
才退回 TSF 位移。两条路失败都只记日志，不把这一键的上屏带崩。

全角字符（状态条「全角 / 半角」）：开关是 Server 的会话内状态（`RouterConfig.full_width_chars`），随 `InputSettings.full_width_chars`
下发给 DLL；DLL 开着时把字母也吃掉送来（`key_sink.rs::eats_key`，英文模式本来送都不送），Server 在直通那条路上问 Core
（`cloudime_core::char_width::full_width`，可打印 ASCII +0xFEE0）再决定自己插（`Effect::Changed`）还是放行；中文标点优先，标点表转了的不再走它。

悬浮状态条（`server/src/ui/status/`）是一排图标按钮，布局在 exe 旁的 `data\icons-arrangement.cfg`（`arrangement.rs` 解析）：
一行一个图标 `button=ch; icon=icons\ch.svg; pos=0`，同一个 `pos` 是同一按钮的几个状态，`pos=-1` 不显示，图标相对 cfg 目录。
`cargo build` 由 `server/build.rs` 把源码里的排布表与 `icons\` 拷到 `target\{profile}\data\`，装机包由 `cloudime.iss` 装到 `{app}\data\`，
Server 按 exe 位置读同一份；因为 `target\debug\data\` 会让 `resources::bundled_root()` 误以为那是装机根，
`has_resources` 现在要求 `data` 与 `assets` **都**在（装机包两者都有，开发时只剩仓库根命中）。点击按按钮换 `StatusEvent` 回 Router，
Caps Lock 不在 Server 手上（DLL 根本没送键过来），状态条自己每 250ms 读一次 `GetKeyState`，变了让 UI 线程重画。
鼠标是普通箭头（类光标 `IDC_ARROW`，不是手形）；悬停提示用系统 tooltip 控件（`tooltip.rs`，`InitCommonControlsEx` 注册类、`TTF_SUBCLASS` 自己盯鼠标），
每次重画按各按钮格子同步一份「功能 + 快捷键」的文字，`sync` 里先删旧工具再挂新的。
「工具」按钮弹的是 exe 旁 `tools\tools.list` 登记的工具菜单（`status/tools.rs`：一行 `短路径=名称`，短路径相对 `tools\`，文件不在的项跳过；用 `TrackPopupMenu` 在中键位置弹），
启动时工作目录设成 `tools\`：**控制台程序**（PE 子系统 3 的 exe、`.bat` / `.cmd`）用 `cmd /k` 起——程序跑完控制台留着，看得见输出、还能接着敲命令（`cwt.exe` 不给参数只打用法，直接起会一闪而过）；窗口程序直接起。
「特殊字符」按钮起 exe 旁的 `SpecialSymbolsInserter.exe`（`ui/mod.rs::open_spec_chars`）：随安装包带的 VFB 成品、装在 `{app}` 根目录，自己画成置顶且不抢焦点的窗口（`WS_EX_NOACTIVATE`），点字符用 `SendInput` + `KEYEVENTF_UNICODE` 注入当前输入框，右键复制到剪贴板。

候选窗（`server/src/ui/candidates/`）只在高亮移动时做动画：方向键页内挪高亮、且行内容与拼音行都没变时，`set_content` 按
`Rendered::highlight_rects` 从「上一段的当前视觉矩形」或「上一高亮行矩形」起滑（连按是连续续滑）；高亮没变的新帧（重排、
异步编辑会话补报的组句矩形）不打断正在跑的动画；翻页 / 新查询 / 隐藏直接画或取消。窗口过程按 16 ms 的 `WM_TIMER` 算进度原地重贴，
约 150 ms 的 cubic ease-out 后收尾。
窗口过程按 HWND 从 UI 线程的表里找回窗口（`attach`）。纯展示，不改窗口位置大小、不涉及协议。

TSF 原有数字 / OEM 标点 / 空格键码按当前布局用 `ToUnicodeEx` 解析（bit 2 避免改变键盘状态），
仅接受单个非代理项 UTF-16 单元。字母、小键盘和 AltGr 处理不变，不保证组合音符输入。
拼音显示位置（`[general] preedit`）在 Windows 上分两处落地：Server 把它读进 `RouterConfig.preedit` 并随 `Frame.preedit_mode`
下发给 DLL，DLL（`com/service/key_sink.rs`）按 `inline()` 决定要不要放行内拼音，Server（`ui/candidates/render_data.rs::window_preedit`）
按 `in_window()` 决定候选窗口顶部画不画拼音行；`window` 模式没有组句范围，光标矩形改从 `com/edit/anchor.rs::caret_rect`（当前选区）量。
连不上 Server 时 DLL 自己拉起它（`tsf/src/com/service/launch.rs`）：`ShellExecuteW` 起与 DLL 同目录的 `cloudime-server.exe`
（`uiAccess=true` 的 exe 用 `CreateProcess` 报 740），进程内 5 秒冷却 + 跨进程命名互斥体防止砸出一串 Server；
起完清掉重连退避，下一键就试。Server 只在登录时由「启动」文件夹拉起，中途挂了以前只能等下次登录。

词库导入（设置「词库」页）走 `cloudime-dictionary::import`，只收现成的 `.db`、校验后原样复制进 `WordBank\`（空词库拒绝），多选批量、页面显示每个文件的结果；
Server 每次轮询比对 `WordBank\` 的路径 / mtime / 长度快照，配置没变也重载新增、同名更新与移除（目录里有的都加载，没有 List.dat 启用开关）。

切换键与内置英文模式开关得在**按键到达之前**就知道
（单击判定在 `OnTestKeyUp`、是否登记语言栏按钮），但 DLL 跑在每个应用进程里、拿不到 Server 那份配置，
`%APPDATA%\CloudIME` 对 AppContainer 里的商店应用也读不到（那是给输入日志和 `.env` 用的目录，不该加 ACE）。
所以由 **Server 经协议下发**（`InputSettings`：`OpenSession` 回包带一次，之后每拍 `SyncMode` 跟着走，值现在恒为固定值）、
DLL 不读文件、不查 mtime。`SessionOpened` 只回过协议版本对得上的 DLL——老的 `open` 是只写不读，
多回一条会被它当成下一次 `Poll` 的应答而报错，那条连接就废了；老 DLL 从 `ModeSync` 那一拍也能拿到同一份（新字段直接忽略）。

## assets

- `assets/sample/`：手写样例词库，不是产品数据。
- 英文词表词频：`uv run tools/corpus/english_frequency.py data/generated/english.tsv -o data/generated/english-frequency.tsv`，再 `... english <词表> --frequency <那个文件>`。

## tools/dict-convert

产品数据的生成工具，输出到 `data/generated/`（gitignore）。

> **2026-10-03**：中文词库的源数据与成品（`assets/lexicon/{01_characters, 02_common, 03_domains, 04_internet_slang, 00_meta}`、
> `dict.tsv`、`dicts/`、`mined_words.tsv`、`phrases.tsv`、`domain_words.tsv`）连同本地生成的
> `data/generated/{dict.qj, dicts/, lm.qj}` 已整体移除，等新的中文词库文件到位后再恢复。
> 也就是说 `lexicon` / `mine` / `phrases` 现在没有输入可跑；`english`、以及 `bigram` 的英文 / 品牌 / 混杂部分仍有效。

- `lexicon`：从 `assets/lexicon/`（自建词库源：规范字 + 常用词 + THUOCL 领域词）加 Unihan 读音（`data/unihan/Unihan_Readings.txt`）、多音字词标注（`--pinyin` 读 JSONL）、语料词频（`lm-unigram.tsv`）建基础词库 `dict.tsv`（8.7 万条），并把 THUOCL 领域词按语料次数 < 50 拆成
  `dicts/<领域>.tsv` + `.db`（11 本、13 万条，`--domain-keep-min`），流程见 `assets/lexicon/CLOUDIME.md`；`--extra-words` 并入人工挑的领域词 `assets/lexicon/domain_words.tsv`。
- `english`：转 `assets/lexicon/05_english/00_all_words.tsv`；同编码优先保留含大写的专名写法（Windows ≠ windows），
  展示写法补充表 `07_display_forms.tsv` 后置读入。中英混杂词源在 `assets/lexicon/mixed_words.tsv`（`lexicon --extra-words`）。
- `bigram`：统计语料；`--phrases` 给短语层、`--brand` 给品牌词（`assets/lexicon/brand.tsv`，云朵输入法 210）与中英混杂词（`mixed_words.tsv`，C盘 / B站：合成计数要成分词在语料里，C 不是 token，只能直接给一元，次数对着真词与整句候选定），领域词也走合成计数（语料里只有几十次的词当 token 统计会吸走成分词的二元证据）。
- `mine`：从语料挖词库没收的高频词并过滤（`oov_filter.rs`：虚词规则 + 相邻字对 PMI≥3，`--candidates` 只重过滤）。
- `phrases`：挖短语层（两遍扫语料：相邻两词、两段二元都够频的相邻三词，总次数与对话语料次数都 ≥ 2000 + 边界规则，读音由成分词拼出；我的 / 不知道 / 有没有 这类常用词表不收的组合，
  `assets/lexicon/phrases.tsv`；词库已并入过短语时重跑加 `--refresh`）。
- `pack dict|lm|model`：`dict` 写单份中文 `.db`，`lm` 写 `.qj`，`model` 写 `.qjm`。
- `word-bank`：合并中文（`data/generated/dict.tsv`）+ 英文（`data/generated/english.tsv`）+ 品牌 / 中英混杂（`assets/lexicon/{brand,mixed_words}.tsv`），
  按上面的判据写出一份 7 张表的 `WordBank\Dict.db`（`--chinese` / `--english` / `--extra` 可换源，`--name` / `--license` / `--source` 给元数据）。
  中文行 `language` = `中文`、`pinyin` 是音节空格分隔；英文行 `language` = `英文`、`text` 是小写编码、`pinyin` 是原样写法、进 `words_english`。
  同一 `(text, pinyin, language)` 先去重（权重大的胜出、相同则先到先得）再归表。命令：
  `cargo run --release -p cloudime-dict-convert -- word-bank --name 云朵基础词库 --license "MIT AND Unicode-3.0" --source assets/lexicon`。
- `phrase-db [路径] [--force]`：写出一份空的自定义短语库（缺省 `Phrases/Phrase.db`，含 `user` / `cloudime_default` 两张空表），
  只用来第一次生成这份文件。仓库根的 `Phrases/Phrase.db` 是**随仓库追踪的产品数据**（`cloudime_default` 表里的内置短语直接在里面手写），
  所以已存在时拒绝覆盖（加 `--force` 才整份换成空库，会丢掉内置短语）。安装包按 `onlyifdoesntexist` 装它。
- `rehead dict|lm|model <输入…>`：把改名前的 `.qj`（魔数 `QINGJIAN`）就地改成当前魔数 `CLOUDIME`，只改头 8 字节。
  改之前按容器完整校验一遍（版本、分节表、`META`）、改完再开一遍，坏文件原样报错不碰，已是新魔数的跳过；
  `tools/release/data-bundle.sh` 发包前拿同一套魔数当门禁，用法见 `docs/notes/release.md`「产品数据从哪来」。

## apps/windows/tools/cloudime-wordbank-transformer

命令行词库转换工具 `cwt.exe`（`cargo run --release -p cloudime-wordbank-transformer -- …`）：把第三方词库
（`.yaml` Rime 词典 / `.tsv` / `.dat` 及其它制表符文本）转成云朵的 `.db`，用法与逐列含义见该目录的 `README.md`。
输出走引擎的 **format 2**（单张 `words(text, pinyin, language, weight)` + `meta`，`meta.format = "2"`），`DictDb::from_path` 直接能读；
语言按词里有没有汉字判（中文 `text` = 词、`pinyin` = 音节空格分隔；英文 `text` = 小写编码、`pinyin` = 原样写法），
同一 `(词, 拼音)` 去重、权重大的胜出——与 `word-bank` / `DictDb::write` 那套一致。

子目录 `cwt-gui/` 是同一工具的窗口外壳（VisualFreeBasic 源码 + 随仓库带的成品 `cwt-gui/release64/cwt-gui.exe`）：
拖入 `.yaml` / `.tsv` / `.dat` 文件后调用**同目录的 `cwt.exe`** 转换、随后自行退出。成品是预编译的，构建时只拷不编
（见根 `build-installer.ps1` 与 `installer/cloudime.iss`），装进 `{app}\tools\`；安装包的 `tools.list` 登记的就是
`cwt-gui.exe`，因此**必须与 `cwt.exe` 同目录**。
