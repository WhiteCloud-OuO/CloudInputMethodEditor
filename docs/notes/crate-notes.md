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

`shortcut::evaluator` 是**复数内核**（实部 + 虚部，`Complex`）：`+ - * / % ^ ! @`、括号、一元正负号，
**不做隐式乘法**（`6i`、`2(3+4)`、`2x3` 都是语法错，`x` 也不当乘号）；函数见 `call`（三角函数收角度、
带 `r` 的收弧度、反三角返回角度、`lg/ln/log/sqrt/sinh/cosh/tanh/arrange/combine/avg/vari/sum`）。
`pow` 两边都是实数时走 `powf`（`2^10` 精确），否则走极坐标（`sqrt(-4)` = `2i`）—— 极坐标带出来的
`cos(π/2)` 那种 1e-17 噪声由 `denoise` 按**相对**量级收掉；`Complex::new` 顺手把 `-0.0` 收成 `0.0`
（不然 `atan2(-0.0, -4)` 给出 `-π`，`sqrt(-4)` 会算成 `-2i`）。算不了（语法错、除零、定义域外、
溢出 / 非有限）一律 `None` = 空候选。

**表达式计算器面板**：`Engine.calculator`（壳在 `v` 模式里按 `Tab` 设、`Tab` / `Esc` 清，组句结束也清）——
开着时 `query_expression` 走 `shortcut::result_candidates`：**只给结果那一条候选**
（算式空着给 `0`，算错给空），拼音行用 `typed_display` 显示算式本身（去掉 `v`）。壳那一侧：
`apply_function_key` 的 `TAB` / `ESCAPE` 先看 `calculator`（进 / 出面板，不展开候选窗）、
`apply_printable` 在面板里把**所有可打印 ASCII**都 `push` 进算式（标点按西文，忽略全角 / 符号映射），
`composed::expression_hint` 按状态给底部那一行的提示（`v` 模式：「输入数字或表达式」；
面板：「按Tab/Esc退出表达式计算」）。面板里回车走 `calculator_expression`：算得出来就上屏「算式=结果」
（`5*6=30`），否则原样上屏（`take_raw` 顺手清组句，`calculator` 由 `handle_key` 的空组句那一拍清掉）。

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
  脚本给的加权 / 降权（`Engine::set_word_adjustments`，词文本 → 系数）也乘在这一层：结构键、读法层级、纠错与联想折扣
  都不动，所以系数只改「同一档里谁更靠前」，覆盖少的词再重也不会跑到覆盖满的词前面。Core 还把这层的分数按文本
  记进 `Query::weights`（`rank_pool` 交回「候选 + 权重」，平台层用它给脚本看排序依据）。
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

拼音串**显示**走另一条路：`Query::marked_segments()` 把最优切分按音节用 `'` 拼起来（`join_marked`），光标后的剩余拼音另起一段、前面带一颗 `'`，所以显示串可能与敲的原文不同（`ni'hao` 是拼出来的）。光标前**末尾**那颗手敲的 `'` 没有「下一个音节」可接，`join_marked` 会丢，所以在 `marked_segments` 里补回来（光标后还有拼音时不补 —— 那一段自己就带一颗）；`segments_cursor` 早就按「显示串里有它」算光标，补上正好对齐。

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

## crates/cloudime-translate

本地词典与翻译 Tip：查词条释义（`.qj` 青简容器按 `Kind::Glossary = 3` 映射读取，或 `.db` SQLite），
以及「这个词条学没学会」的学习状态。词典在安装目录 `LocalDictionary\`（清单 `dictionaries.list` 一行
`显示名=文件名`，[`Manifest`](../../crates/cloudime-translate/src/manifest.rs) 解析；坏行警告跳过，文件不在就是空清单），
词典文件只读、学习状态在用户数据目录的 `translate.db`：

```sql
CREATE TABLE entries (dictionary TEXT, word TEXT, input_times INTEGER, learned INTEGER,
                      PRIMARY KEY (dictionary, word));
```

`.db` 词典的表结构（见 `glossary/db.rs` 的文件头注释）：总表 `words(id, word, translation, pos, reading)`（一个中文词可有多条、
每条一个译词，`pos` 可复合、`reading` 是译词的读音（日语假名），没有就空）+ 副表 `contents(id, initials, pinyin, word, reading)`
（拼音首字母 / 拼音 / 中文词的索引，`reading` 与 `pinyin` 同值；查词先走它、再回总表；副表要列全，漏登记的词查不到）。
读取端对**没有 `reading` 列的老结构**降级兼容（`Db::has_reading`，查不到读音、其余照常）。
`Learning::record_commit` 把译文上屏的次数 +1、到 `need_times` 就把 `learned` 置上；「重置学习内容」= `reset`（删掉该词典的行）。

两种词典的查词开销（`cargo test -p cloudime-translate --release -- --ignored --nocapture lookup_latency`，
拿 `glossary-en.qj`（232213 词）与它转出来的 `.db` 各查 2 万次命中 + 2 万次未命中，三轮取稳）：

| | 打开 | 命中 | 未命中 |
|---|---|---|---|
| `.qj`（mmap + 开放寻址哈希） | ~11 ms（逐条校验偏移，安全换来的） | **0.30 µs** | **0.07 µs** |
| `.db`（SQLite，`prepare_cached` 复用语句） | ~0.6 ms | 42 µs | 20 µs |

差两个数量级，但**开销在 SQLite 的每次查询机制上**（VM 执行、B 树定位、行解码、`TEXT` 拷成 `String`），
不是读盘：开 `PRAGMA mmap_size` 只快约 12%（42→37 / 20→18 µs），所以读取端不开它、留住小页缓存。
对翻译 Tip 没有影响——每帧只查**高亮那一个**候选（按键 / 轮询时才出帧），40 µs 在噪音里；
真要为「一屏全查」这类批量用法铺路时，`.qj` 才是那份快的格式。

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
见 `ranking::rank` 的 `association_limit`；缺省 2）、`pinyin_font` / `candidate_font` / `item_number_font`（各 `family` + `size` 的子表；
缺省微软雅黑 11 / 13 / 11 pt）、
`item_number_style`（decimal / circled / roman / dingbat / parenthesized）、`candidate_box_minimum_width`（物理像素，竖排与横排都生效；缺省 180）、
`show_more_candidate_items`（缺省关；开着时组句里 Tab 把候选窗展开成一整屏，见候选窗那一节）、`mouse_word_selection`
（off 关闭（缺省）/ more_candidates 仅展开成网格时 / always 全部开启；控制候选窗的鼠标悬停 / 点选，见候选窗那一节）、
`candidate_item_maximum_width`
（展开后每格的最大宽度，物理像素 / 点；缺省 420，0 不限 —— 在 `matrix_cells` 里给格宽封顶，超出的候选截尾加「…」；
`Theme.max_cell_width` ← `RenderSettings.max_cell_width`）、`program_list_of_hiding_candidate`（名单里的 exe 完全不接管，
`InputSettings.raw_input` 通知 DLL）、`preedit`；`[status_bar]` 只剩位置（开关删了，状态条常开，只跟「当前输入法是不是云朵输入法」走）；
`[status_bar]` 有 `show_status_bar`（缺省开；`StatusBarConfig` 手写 `Default`——`#[serde(default)]` 取的是结构体的 `Default`，不写就变成 `bool::default()` = 关，老配置会莫名其妙不显示工具条）：关掉后 Server 的 `reconcile_status` 始终收起，桌面上不再出现那条工具条（按钮位置 `x` / `y` 照旧）。
`[debugging]` 只有 `auto_hide_float_tool_bar`（缺省关）：开着时前台全屏（`ui/status/fullscreen.rs` 每秒查一次）收起悬浮工具栏，
关掉后全屏也不收；切到别的输入法 / 云朵被禁用时始终收起，与这一项无关（`RouterConfig.auto_hide_float_tool_bar` → `StatusView.auto_hide_fullscreen`）。
`auto_disable_without_text_input`（缺省关）的判断与动作都在 DLL，见 TSF 那一节。

**装机默认值**（`Config::default` 与 [`TEMPLATE`]，两者一致由 `template_parses_to_defaults` 盯着；这是产品定的起点，改要一起改）：
联想上限 2、候选框最小宽度 180、符号映射**全关**（`DEFAULT_PUNCTUATION_MAPPING` = 0）、Tab 展开更多候选关、
候选字体 13pt / 序号 11pt、全屏不自动收起悬浮工具栏、翻译词典缺省 `glossary-en.db`、`translate.reset_counter` 缺省 0
（它是「重置学习内容」的计数器，不是开关：改小 / 改大都会让 Server 把那本词典的学习记录清掉）。
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
再往后 +1 的三次：**v16** 给 `ServerMessage::Update` 加了 `commit`（鼠标点候选窗上屏，`Frame` 顺带加 `columns`）；
**v17** 给 `ClientMessage::SyncMode` 加了 `in_text_input` / `caps`（状态切换提示）；
**v18** 给 `InputSettings` 加了 `script_key_modifiers`（组合键脚本在清单里声明的 `combination_modifiers` 的并集：修饰键位图的第 N 位置位 = 那一套组合键要送上来）；同一版还加了 `script_keys`（`trigger_condition = "key"` 脚本声明的键：虚拟键码的 256 位位图 `[u64; 4]`，这样 `InputSettings` 仍是 `Copy`，DLL 据此在没组句时也把这些键送来）与 `script_wants_text`（有 `key` 脚本时为真，DLL 每个按键前现读一份光标前文送上来）。三者都带 `serde(default)`，老 DLL 不读也不受影响，所以没有再次 +1。

**主题文件**（`theme.rs`）：`Themes\*.json` 装候选窗口 / 悬浮工具栏 / 状态切换提示三个窗口的 21 个颜色，分 `candidate`（15 项）/ `bar`（背景 / 图标 / 阴影）/ `tip`（同三项）三组；`ThemeColor` 是一个 `#AARRGGBB`（也认 `#RRGGBB`，serde 走字符串、缺键回缺省，`Default` 对齐现有观感）。`ThemeFile::{load, save}`（`save` 建父目录 + 美化 JSON）；`theme_dirs(bundled_root)` 给出两个候选目录 —— 用户目录 `%APPDATA%\CloudIME\Themes`（可写）与随包目录 `<安装目录>\Themes`（只读），`list_themes` 两处都扫、**用户目录优先**（同名盖掉随包的）、按名字排序，`find_theme` 在它上面按名字找（可带 / 不带 `.json`、大小写不敏感）；`DEFAULT_THEME_FILE` = `Default.json`（随包另有一份 `Panic.json`，两份都是内置主题）、`THEMES_DIR` = `Themes`。`[theme]` 只有一个 `curr_theme`（文件名，不是路径；缺省 `Default.json`），设置页「主题」页写它、Server 靠它选主题。注解色（`gloss` / `pos` / `fresh`）暂不进主题文件。

## crates/cloudime-render

矩阵：`Frame::columns` 不为 0 时走 `renderer/matrix.rs`（展开「更多候选项」就是这条；竖排 5 列 / 横排 5 行都是它），
每格一样宽——取收起时那条高亮条的宽度（`Frame::min_cell_width`），比它长的候选截尾加「…」；没给这个宽度时
（这次组句还没画过收起态）退回「一屏里最长的那条」、单格封顶 `MAX_CELL_EMS` = 4 字宽。候选不够一屏时不补空列。
**展开态另有两条按候选字宽算的下限**（`Renderer::char_width` 量一个汉字，`horizontal_badge_gap` /
`horizontal_min_cell_width` / `badge_char_width`）：

- **横排**：每格最小宽度 = 6 个字宽 + 该屏最宽的角标；**收起**时候选词与来源角标之间的最小间隔是 2 个字宽
  （竖排收起仍用 `BADGE_GAP` = 4pt——角标在自己的列里）。
- **竖排**：每格最小宽度 = 这一屏最长的候选 + 2 个字宽 + **一个角标字宽**（`badge_char_width` 量「造」，
  不管这一屏有没有角标都算），所以格子一定装得下最长那条、不会被截断。

格子里的角标一律**贴格右边缘**（`matrix.rs` 用 `BADGE_GAP` 往里的位置），并把它占的这点位置算进 `chrome`
（候选词的截断上限跟着让出来，文字与角标不会叠）。矩阵横竖共用，所以两条下限各按 `Layout` 走
（`matrix_size` / `draw_matrix` / `matrix_cells` 因此都要传 `Layout`）。预览示例里有 `matrix-horizontal` /
`matrix-vertical` 两个场景（矩阵那个带一个「造」角标，竖排那屏里还有一条 12 字的候选）用来看这两条规则。
网格下固定留一行信息（被截断的高亮候选全文、译文、页码）。
（早期那套「列宽由帧给 `column_ems`、滚动时不跳宽」的设计已随 `Frame::column_ems` 一起删掉——没有壳在用它。）

底部那一行（`Frame::tip` + `Frame::footer`）：**左侧翻译 Tip、右侧页码**，由 `Renderer::bottom_line_height` /
`draw_bottom_line` 统一画，竖排 / 横排 / 矩阵三条路都走它（矩阵原来那行「被截断候选的全文 + 译文」被它取代）。
页码一直显示（只有一页也是 `1/1`，由壳决定内容），颜色用 `colors.footer`（主题里的「候选窗口页码颜色」，缺省 `#888888`）。
Tip 是 `Vec<TipSegment>`（文本 + 深浅 + 是否斜体）：词性、释义之间的分隔符、读音括号统一走
[`Tone::TranslateMeta`]（词性再叠斜体；统一 `colors.translate_meta` —— 主题里的「翻译 Tip 词性和分号颜色」，缺省 `#333333`），
释义按词条 `learned` 取 `colors.translate_fresh`（主题「未学习释义颜色」，缺省 `#ff7f27`）或 `colors.translate_learned`（主题「已学习释义颜色」，缺省 `#333333`）。
斜体要真的斜得有斜体字面：`fonts/windows.rs` 因此补了 `segoeuii.ttf` / `ariali.ttf`
（微软雅黑没有斜体面，只请求 `Style::Italic` 会回落到正体）。Tip 放不下时截断加「…」（窗口宽度不为它撑大）。

底部信息区还在本地 Tip 那一行**下面**多画一行**在线翻译**（内容由脚本给，见 `docs/design/online-translate.md`）：
`Frame::online` 与 `Frame::tip` 用同一套 `TipSegment`，`Tone::Online` 走 `colors.extra`（主题里的「候选项额外内容颜色」，缺省 `#0f6cbd`）；
两行合起来的高度由 `Renderer::info_height` / `draw_info` 算并画（竖排 / 横排 / 矩阵三条路共用），
等待「翻译中…」、成功译文、失败原因三态的颜色由壳在 `ui/candidates/render_data.rs::online_segments` 里定。
`examples/preview.rs` 的两个 `online-*` 场景就是拿来看这一行的。

自绘渲染器：候选窗一帧 + 主题 → 预乘 RGBA 位图，tiny-skia 栅格 + cosmic-text 文字（fontdb 按清单只加载几个字体文件、不扫系统），
自己解析 `trak` 字距表、按主题 gamma 加深笔画；配色只有一套（`Theme::new()` / `Palette::new()`，浅色单套，不再分深浅；拼音串与候选项序号是纯黑，页码仍是弱化的灰）；
`Palette` 里三个窗口那 21 项可被主题文件覆盖（`candidate` / `bar` / `tip`，壳侧 `apply_theme_colors` 从 `ThemeFile` 套进去），`gloss` / `pos` / `fresh` 这些注解色还没进主题文件；
cosmic-text 打了 `opsz` 光学字号补丁（qingjian-team/cosmic-text 分支 `qingjian-opsz`，workspace `[patch.crates-io]` 钉 rev）。
状态条是一排 `StatusCell::Icon`（整份 `<svg>` 源码）：`svg.rs` 用 resvg 0.48（workspace 钉版、`default-features = false`，
不扫系统字体、不要 svgz 与光栅图解码）解析光栅化，按 SVG 宽高比缩进 `BUTTON_SIZE` = 20 pt 的方按钮居中叠上去，解析失败返回 `RenderError::InvalidSvg`；
`draw_svg` 收一个 `Option<Color>`，给了就按每个像素的 alpha 把整张图标染成这个颜色（工具栏 / 提示窗的图标色靠它，SVG 里写的是什么色都只取轮廓）；
`render_status` 只画图标、不留文字格与分隔线，按钮之间与四周各留 `BUTTON_GAP` = 6 pt，返回各按钮右边界供点击命中；背景与阴影由壳按窗口传入（工具栏 / 状态切换提示各一套主题色）。
`examples/preview.rs` 出 PNG 与真机截图并排比、`--measure` 量宽度。Windows 壳候选窗口与悬浮状态条都走这条渲染路径（`ui/painter/`，`ui/layered/` 贴位图），
字体库加载失败时都不绘制、不显示并记日志；`[general] font` 是候选窗字族名（空为系统字体，`fonts/directwrite.rs` 按字族名找字体文件只加载那几个，没装就回系统字体；
设置页 `candidates.rs` 的字体框是边输入边提示的自动补全）。设计与验收见 `docs/design/rendering.md`。
候选行右侧的**来源角标**：`Row.badge`（Server `dispatch/candidates/mod.rs::badges_of` 按 `kind == Custom` 判「短」、
`kind == Sentence` 判「句」、`Learner::is_user_word` 判「造」，其余没有），用序号字体、`Palette.badge` = `#888888` 右对齐画在候选格内（间距 4 pt）；
横排按项宽、竖排按行宽给它留出位置。
**高亮条移动动画**：`Frame::highlight_animation`（`HighlightAnimation`，起点矩形 + 进度 0..=1）让渲染器从起点矩形 lerp 到目标行矩形
（`frame/highlight/rect.rs` 的 `HighlightRect`，内容区坐标，`HighlightRect::lerp`）；没有它就画在高亮行。三种布局都先把各行 / 各格的高亮矩形量好再把条子画在文字之前，
并把这份矩形随 `Rendered::highlight_rects` 交给壳（壳连按方向键时按当前视觉位置续滑）。纯展示，不影响排序与按键。
壳侧起不起滑动由 `ui/candidates/mod.rs::plan_slide` 判：高亮变了、且「内容没变」才从上一格续滑。**「内容没变」里不比
翻译 Tip**——Tip 显示的就是高亮候选的译文（壳每帧跟着高亮重算），拿它判断会让「挪到有译文的候选」统统被当成内容变化、
滑动被清掉，真机表现是「一部分候选之间有动画、一部分没有」。

**展开（Tab）的过渡**由壳做（`ui/candidates/mod.rs`）：渲染器提供一个纯位图工具 `clip_pixmap`（按左上角裁 / 补透明）。
壳在 300 ms 里把「内容尺寸」从切换前插值到切换后（`transition_frame`），每帧把渲染结果裁到这一块再贴出去，整张的额外不透明度走
`UpdateLayeredWindow` 的 `SourceConstantAlpha`（`ui/layered::present` 因此多一个 `alpha` 参数，从 170 淡入到 255）。
**位图不缩放**——内容是逐步露出来，字始终按最终字号栅格化，不会糊。触发条件是 `RenderData.columns` 由 0 变成非 0
（Tab 展开；释义列表临时收起也走这条），起点取上一帧真正贴出去的尺寸（`last_content`）；定时器与高亮滑动共用
（`SLIDE_TIMER_ID` / 16 ms），两个动画都走完才 `stop_animation`；起止各留一行 `info` 日志（含帧数），排查这类问题很有用。

**收起方向没有过渡，是试过之后放弃的**（`transition_plan` 里只认 `columns` 0 → 非 0，注释里写了原因）。踩过的三条：

1. **`SlidePlan::Clear` 只停滑动**，不能顺手停过渡：Tab 换内容时 `plan_slide` 判的正是 `Clear`，早期在这里清掉过渡 → 完全没有动画。
2. **「旧的那一屏」在整段过渡里必须是切换前那一张**，不能每帧更新成这一帧的渲染结果——否则从第二帧起旧新是同一张，
   合成的永远是最终样子（展开靠画布长大所以不暴露，收起一暴露就是「一帧跳过去」）。
3. **分层窗口「缩小时不一定立刻重画」没法稳定绕开**：试过交叉淡化 + 把旧屏裁到这一帧的尺寸 + 固定画布（不缩窗口，
   把缩小画在画布内部），真机上收起方向始终看不出变化。这条按决定放弃，只保留展开。
   要再试：把 `transition_plan` 的 `expanding` 换回 `switched`，渲染器里那两个工具（`cross_fade` / `transition_pixmap`）
   与壳里的「上一帧位图」缓存（`last_pixmap`）都在 git 历史里。

底部那一行（Tip + 页码）的**基准三处布局统一**：竖排 / 横排 / 矩阵都画在「最后一行的底边 + 一个行内留白」处
（矩阵原来多算了 `row_padding / 2`、横排多算了一个 `row_padding`），展开 / 收起切换时 Tip 不会上下跳那几像素。

**横排里信息区排在「高亮候选的译文」那一行下面**（`draw_horizontal`）：横排的译文是候选下面单独一行
（竖排它在候选右侧那一列），`horizontal_size` 预留的高度本来就是「候选行 + 译文 + 信息区」，
但画的时候信息区落在 `top + row_height`、译文落在 `y + row_height + row_padding() / 2`，
两段同一处 —— 译文长一点就和 Tip / 页码叠在一起（预览 `nihao-horizontal` / `online-horizontal` 能看出来）。
现在有译文时信息区改画在 `y + row_height + annotation_height`（与预留一致），没有译文时保持原基准不动。

**缩放只有一个倍数**：渲染器只认「点 → 像素」一个 `scale`，主题里所有长度（字号、留白、间距、圆角、阴影）都乘它。
所以壳的滚轮缩放不用改渲染器：候选窗把缩放的倍数折进传给 painter 的 DPI（`ui/candidates/mod.rs::effective_dpi` =
显示器 DPI × `1.2^级数`），框、字、留白、阴影就一起变。级数存的是整数，上去再下来精确回 100%；`hide()` 里归零
（窗口一关就回 100%，与「收起态宽度」一样属于这次弹窗的临时状态）。两处随之放宽：`ui/painter::scale` 不再拿 96 当下限
（缩小时倍数小于 1），`preferred_size` 里的最小宽度改成按点算（`m.px(theme.min_width_pixels)`，原来是
`min_width_pixels / scale`，缩放时会朝反方向跑）。

## crates/cloudime-update

检查更新（设计见 `docs/design/update.md`）：`index/` 是索引的类型、下载（`fetch.rs`，复用 workspace 的 reqwest + 单线程 tokio，20 秒超时、2 MB 上限）与验签
（`signature.rs`，`PUBLIC_KEYS` 列表，`verify_strict`）；`checker/` 是调度（`Checker::poll` 由壳的每秒定时器调，到点起一次性线程）、落盘状态 `UpdateState`（`update.json`，先写临时文件再改名）
与查到的结果 `Available`。`Version` 自己实现语义化版本比较，不引 semver。`[update]` 配置与 `UpdateChannel` 在 `cloudime-platform`。
`examples/check.rs` 手动走一遍；`tools/release-sign` 是发版侧的 keygen / sign / verify。

## crates/cloudime-script

用户脚本的运行时：`mlua` + LuaJIT（版本在根 `[workspace.dependencies]` 里定，特性 `luajit` + `vendored`）——
LuaJIT 源码随 crate 编译进二进制，装机包不依赖机器上装的 Lua，与 `rusqlite` 的 `bundled` 一个路子；
`mlua` 只在这个 crate 声明一次，别处用 `cloudime_script::mlua` 这个再导出。LuaJIT 在 x64 与 i686 都编得出来
（TSF DLL 有 32 位那份），2026-10-07 起。

`runtime.rs` 是全部：`Runtime::load(dir)` 读 `dir` 下的 `*.lua`（按文件名排序）依次 `exec`，`Runtime::none()`
是不读盘的空运行时（`RouterConfig::scripts_dir` 为 `None` 时用它）；脚本用全局表 `cloudime` 的
`on(事件名, 处理函数)` 登记、`log(文本)` 写日志，另读 `cloudime.context`（光标前文，派发前由 `set_context` 刷新）。
规则（没脚本走老路 / 有脚本在固定四处跑、风险自担）见 `docs/design/script.md`，这里只记实现。
**清单**：每个脚本必须最先声明 `cloudime.script{…}`（字段见上面那份文档），`runtime.rs::parse_manifest`
校验；缺 / 不合法 → 那个脚本不加载，注册过一半的处理函数也撤掉（`retract_handlers`）。`apps` 进 `dispatch`
过滤（`None` = 不过滤，`startup` / HTTP 回调就是）、`priority` 决定派发顺序（大的在后，合并时盖住前面的）、
`budget` / `timeout` 由 `Runtime::arm` 在**每次调用前**按该脚本的清单上紧、出错时先调清单里的 `on_error`；
`cloudime` 表锁成**只读代理**（`__index` 指向本体、`__newindex` 报错），本体留在 `Runtime::host` 里
给运行时自己写 `context`；`rawset` 能绕过这道锁，但只改到脚本自己看到的那份（运行时的函数本体不受影响）。
`cloudime.on` 只在加载期允许（`loading` 标记）；`http_get` 的响应时间限制不能超过清单里的 `timeout`。
`Runtime::load(dir, disabled)` 读**安装目录**的 `Scripts\`（Server 里 `RouterConfig::from(&config).with_bundled_scripts()`
指过去）：按文件名排序执行，**跳过 `template.lua`**（新建脚本的模板）与 `[script] disabled` 里列到的文件（大小写不敏感）。
清单的 `description` 只是记下来进日志（设置页自己扫文件取它，见 `docs/design/script.md`）。
`dispatch` 把处理函数**返回的表**按登记顺序递回调用方
（返回 `nil` / 别的类型的跳过）—— 返回值是什么意思本 crate 不管，由派发方（Server）定；`has_handlers(事件名)`
让派发方在没人关心时连载荷都不拼。`wanted_keys()` 汇总清单里 `keys` 声明的那几个虚拟键码、
`wants_surrounding_text()` 从「有没有 `key` 脚本」推出真 —— Server 把它们折进 `InputSettings.script_keys` /
`script_wants_text`（见 apps/windows server 一节的触发条件）。

**HTTP**（`http.rs`）：`cloudime.http_get(url, timeout_ms, callback)` 与
`cloudime.http_post(url, body, { timeout_ms, headers }, callback)` 共用同一条管线
（`Pending { url, deadline, callback, manifest }` + 一个 `mpsc` 回信通道）。请求在**一次性线程**里跑
（与 `cloudime-update` 同一套 reqwest + 单线程 tokio），派发线程只做「收结果 → 调回调」——不阻塞输入；
`http_post` 的表头在 Lua 那层先验一遍（名字 / 值不合法当场报 Lua 错），免得后台线程 panic。
**`timeout_ms` 都是必给参数**（给 0 / 不给直接报错、请求不发，且不能超过清单里的 `timeout`），
`Runtime::poll_requests(候选表)` 交付结果时还会再比一次时限：**超过时限才回来的收信直接丢掉**（记一条日志）。
结果表成功是 `{url, status, body}`、失败是 `{url, error}`；`poll_requests` 返回回调**返回的表**
（同事件那套语义），另给 `has_pending_requests()` 让调用方在没挂着请求时跳过整段。

**脚本拿到的是 mlua 的「安全子集」标准库**（`io` / `os` / `package` 都在，`debug` 没有）加上几处刻意改动
（`os.exit` / `os.execute` / `io.popen` / `io.stdin` / `coroutine`，见 `docs/design/script.md`）—— 定调是
「脚本就是用户自己写的本机程序，风险自担」，但**先把能卡住工人线程的入口堵掉**。
加载期与派发期的 Lua 错误都只记日志、跳过那一个
脚本 / 处理函数，绝不让脚本错误影响输入法；派发时先把处理函数 clone 出来再放开 `handlers` 借用，处理函数里
再 `cloudime.on` 不会重入 panic。

**引擎挂在 Server，不挂 DLL**（2026-10-07 定）：DLL 被加载进每个应用、脚本的 JIT 在开了 ACG 的宿主里会失败、
脚本一崩还会连累宿主应用；而 Server 正对着协议两头（DLL 送来的按键 / 前景 / 光标矩形，DLL 要照做的上屏 /
帧 / 状态条），「脚本操作输入法」在 Server 侧就够。DLL 侧真要 TSF 直通再单独评估。见 `apps/windows` 那节的
「用户脚本」。

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

**用户脚本**（0.1.0 起，运行时见 `crates/cloudime-script`）：脚本放数据目录 `%APPDATA%\CloudIME\scripts\*.lua`，
`RouterConfig::scripts_dir` 由 `main.rs` 的 `RouterConfig::from(&config).with_user_scripts()` 指过去
（`RouterConfig::default()` 不带它 —— 测试不该去读开发机上真实的脚本目录）。`Router::new` 里 `Runtime::load`
加载，之后每次按键在 `handle_key` **最前面**派 `key`（脚本看到的是这一键**处理之前**的状态），
`recompose()` 把引擎排好的候选交出去派 `candidates`，全部加载完再派一次 `startup`。
事件名与载荷在 `server/src/dispatch/script.rs`：

- `startup`：空表。
- `key`：`app`（宿主 exe 名）/ `vk` / `char` / `ctrl` / `alt` / `shift` / `caps` / `english_mode` / `composing` / `mode`
  （`chinese` / `english` / `disabled`）。
- `candidates`：1 起的数组（另有 `app` 字段说明这是哪个应用），每项 `{ text, display, pinyin, weight, kind }` —— `pinyin` 是词库读音、
  `weight` 是 **Core 的排名权重**（`Query::weights`，词频 × 用户权重 × 选择次数 × 上下文 × 纠错 × 联想；
  快捷候选不在表里，没有这一项），`kind` 是 `chinese` / `english` / `shortcut` / `custom` / `sentence`。
  脚本据此自己定序（见 `order` 与 `adjust`）。

脚本还能读 `cloudime.context`：光标前文 —— 应用里已经输入、不在候选窗口里的那段文本。它本来就是 DLL 送来的
`ClientMessage::Surrounding`（DLL 在组句起始时读 TSF 的光标前文，见日志里的「送光标前文 N 字」），Router 在
`set_surrounding` 里留一份、`stop_rescoring` 里清掉，**不需要 Core 侧接口，也不需要 `WM_GETTEXT`**。

处理函数**返回一张表**就是「这一拍要改什么」：多个脚本按登记顺序合并，后面的盖前面的（表里缺的项不动），
读的时候按类型严格匹配（**不让 Lua 把数字悄悄转成字符串**，`commit = 42` 当写错了记一条日志），认得的键：

| 键 | 类型 | 意思 |
|---|---|---|
| `passthrough` | `true` | 这一键不吃、原样交给应用（游戏里抢键就靠它） |
| `commit` | 字符串 | 吃掉这一键、直接上屏这段文本（`reset_composition` 清掉当前组句） |
| `notice` | 字符串 | 候选窗里显示一行提示（`Frame::notice`，随这一帧下发、下一次按键清） |
| `order` | `{2, 1}` | 候选排序：1 起下标按这个顺序排到前面，没列到的按原顺序接在后面 |
| `display` | `{[1] = "译①"}` | 候选显示：原下标（1 起）→ 显示的文本（上屏的仍是 `text`） |
| `adjust` | `{["云朵"] = 2.0}` | 加权 / 降权：词文本 → 系数（>1 加权、<1 降权、0 沉底）；整份替换，空表全清 |
| `theme` | 字符串 / `false` | 换主题（写进配置并重启输入法服务；`false` 回本进程启动时那份），见下 |

`order` / `display` **不绕开引擎**（2026-10-07 定）：`apply_candidate_actions` 把重排 / 改显示落到
`Composed::Candidates` 的 `CandidateLayout` 上（`CandidateLayout::new` 重建），之后的方向键、数字键、鼠标
点选都按新顺序走，选中第几个仍由 `Engine::commit` 去上屏 / 并进组句；高亮按「原下标 → 新下标」跟着原来那个
候选走。越界 / 重复的项记日志跳过，脚本写错不会把候选搞没。

`adjust` **排序环节仍在 Core**（2026-10-07 定）：`handle_key` 里把它递给 `Engine::set_word_adjustments`
（内容没变时 Core 自己跳过），Core 在 `engine/query` 里把它乘进每条命中的**用户权重因子**那一层 ——
结构键（覆盖字母数、末音节、`exact`）、读法层级、纠错与联想折扣都不动，所以系数只改「同一档里谁更靠前」，
覆盖少的词再重也不会跑到覆盖满的词前面。脚本调参前想知道 Core 怎么排的，看 `candidates` 载荷里的 `weight`。

`notice` 与 `adjust` 在**没有按键的那两拍**（`candidates` 事件、HTTP 回调）也认（`apply_common_actions`）；
`passthrough` / `commit` 要有按键才谈得上，那两拍给了就记一条日志忽略。

**`cloudime.candidate.redraw()`**：脚本请 Server 把当前这一屏重算一遍再重画。Runtime 里就是一个
`Cell<bool>` 脏标记（`take_redraw_request`），Server 在两处兑现：按键那一拍（`handle_key` 末尾）与
收异步回调那一拍（`poll_scripts_requests`，**即使回调没返回动作表**也照样重画 —— 这正是它存在的理由）；
`candidates` 那一拍里调会被忽略（那一拍本来就在重算，免得自己套自己）。

**量文字**（`cloudime.ui.measure` / `measure_tip`）：`ui/measure.rs` 里的 `SharedMeasurer` 自己建一个
`Renderer`（**懒加载**，读系统字体一次几百毫秒）+ 一份与候选窗同源的 `Theme`（字族 / 字号从同一份
`RenderSettings` 算），用 `Renderer::measure_font`（`cloudime-render` 新加的公开接口，走同一套整形 /
回退链）量出**点**宽高。**不借候选窗那个渲染器**：`SharedPainter` 是 `Rc<RefCell<…>>` 只有 UI 线程能用，
从工人线程去借要么卡输入要么抢锁。Server 在 `Router::new` 里把这个量尺装进脚本运行时
（`Runtime::set_measure`），字体设置变了由 `apply_config` 刷新（量尺自己比对后重建）。

**截断 / 折行**（`cloudime.ui.truncate`）：就用上面那个量尺，`"ellipsis"` 从尾部去字补「…」；
`"wrap"` 贪心按行宽折（优先在空白处断、空白甩掉不带进下一行，没有空白的中文从任意处断），
最多 `max_lines` 行、剩下的在末行补「…」（与渲染器同一个省略号），返回
`{ text, lines, truncated, width }`。

**问窗口多大**（`cloudime.candidate.width()`）：UI 线程每画一帧把内容区尺寸（点）写进 `ui/viewport.rs` 的
`Viewport`（两个 `AtomicU32` 存 f32 位模式：`0` = 还没画过 / 收起时 `clear`），`UiHandle`（也就是
`CandidateSink` 的实现）把它报给 Router（`CandidateSink::viewport`）；Router 在每次派发脚本事件 /
收回调之前 `scripts.set_viewport(...)`（与 `set_context` 同一时机），Lua 那边读一个 `RefCell`。
工人线程读、UI 线程写，所以用原子量而不是 `Rc<Cell>`。

**在线那一行支持多行**：`Frame.online` 还是 `Vec<TipSegment>`，渲染器在 `info_height` / `draw_info`
里用 `online_lines` 按 `\n` 拆成几行（每行各自左对齐、各自按宽度截断），窗口高度 = 底部那行 +
行数 × 翻译行高 + 行间距 —— 所以脚本把折好的多行文本写进 `online`，窗口自己变高，不用 `set_page_size`。
一行都不写换行时与以前完全一样。

**读输入框文本**（`cloudime.text.all` / `before` / `after`）：只有 TSF DLL 读得到 ——
`com/edit/surrounding.rs::document_text` 在起组句的那次编辑会话里读：光标前后各读一半（各 10 万 UTF-16 单元
上限）拼成「以光标为中心」的 `DocumentText { text, caret }`，私密框不读。两半各用 COM 的
`ITfRange::Clone`（`surrounding.rs::clone_range`）拿一份**独立** range 再挪 —— COM 接口的 Rust `.clone()`
只是 `AddRef`、仍指向同一个对象，挪前一半会把后一半也带偏（真机表现为整篇翻倍，如「测试文本」变两遍）；
同一坑在 `composition/mod.rs` 各处（`move_selection` / `shift_caret` / `delete_before_caret`）早就是用
`unsafe { range.Clone()? }` 避开的。Server 侧 `dispatch/document.rs`
把它当快照、按显示宽度切片（`weight`：中文 / 全角 2、西文 1）：`before` 从头部丢、`after` 从尾部丢、
`all` 走 `keep_around`（前一半 + 后一半）。**快照组句结束不清** —— 清了的话每段组句的第一键都是 `nil`
（DLL 那份要到组句起始的编辑会话才读得到，晚于第一键）；只在**换焦点 / 会话**（`ensure_focus` /
`OpenSession` 重连 / `CloseSession`）或**转私密**时清。读取时机由 `ServerMessage::ModeSync.want_document`
请：**有脚本就一直请**（每段组句起始读一份新的，快照跟着文档走；没脚本 `false`、零读取），DLL 下一次
`ClientMessage::Surrounding.document` 带来 —— 两个方向都是**加字段**（serde default），老 DLL / 老 Server
各自忽略，**不升协议版本**。`Document::store` **只认 `Some`**：没请 DLL 读的那段组句送的是 `None`，照单
覆盖会让快照在「读到」与「抹掉」之间来回抖、脚本时灵时不灵，`None` 一律当「这次没有」忽略。
`document_text` 光标前后两半各自 `unwrap_or_default`：光标在文档最开头时前一半为空，不算失败（两侧都空
才是没有文本）。

**已知坑：有的宿主只给「局部上下文」**（2026-10-08 真机排查）。Windows 11 记事本实测：起组句那一拍
`GetSelection` / `GetStart` / `GetEnd` 都是 `0..0`（空），上屏之后又只剩刚上屏那一段（真机文档 6 个字、
`GetText` 只回最后 2 个）——宿主把 TSF 上下文按选区 / 组句圈成了局部。这类宿主里 `cloudime.text.*`
**读不到整篇**（给 `nil`）；Firefox / Word / 多数 Electron 应用给的是整篇，正常。曾试过「绕开 TSF
从窗口层读」（`WM_GETTEXT` + UI Automation）：`WM_GETTEXT` 对记事本的 RichEdit 只回 1 个字；UIA 从
**焦点元素**取到的是那段行内拼音（marked text）、从**前台窗口元素**再 `FindFirst(IsTextPatternAvailable)`
在记事本里也没找到 Provider。**这条路暂时打住**，要通吃最省事的是注入 `Ctrl+A` + `Ctrl+C` 读剪贴板
（注入前必须先收掉组句，否则 `Ctrl+A` 会被自家 DLL 吃掉、又派发回脚本成环）。另记一条硬教训：读窗口是
**跨进程**调用，**绝不能**放在服务所有应用的那条工人线程上——工人线程一卡，所有应用等它回包就一起没响应
（真机踩过）；要单独线程 + 超时。

**剪贴板**（`cloudime.clipboard.settext` / `gettext`）：Lua 标准库没有剪贴板，实现在 Server 侧
`dispatch/clipboard.rs`：写走 `EmptyClipboard` + `GlobalAlloc(GMEM_MOVEABLE)` + `SetClipboardData(CF_UNICODETEXT)`
（UTF-16 + 结尾 0），读走 `GetClipboardData` + `GlobalLock` + 逐 UTF-16 单元转字符串；`OpenClipboard`
短等重试 5 × 10ms，`Drop` 里 `CloseClipboard`。Server 就在用户会话里，所以不必经 DLL；接口由
`Runtime::set_clipboard` 装进去。剪贴板是全局资源：打不开报错、里面没有文本给 `nil`。
（`apps/windows/server/Cargo.toml` 因此加了 `Win32_System_DataExchange` / `Win32_System_Memory` /
`Win32_System_Ole` 三个 windows feature。）

**当前时间**（`cloudime.get_time`）：`jiff::Zoned::now()`（`cloudime-script` 因此加了 `jiff`）拼一张**本地时间**的表
—— `year` / `month` / `day` / `hour` / `minute` / `second` / `weekday`（1 = 周一 … 7 = 周日）/ `unix`（Unix 秒）。
脚本本来也能用标准库的 `os.date` / `os.time`（`os` 表只换了 `exit` / `execute`），这一份是不依赖标准库的稳定形状。

**换主题**（`cloudime.apply_theme(名字)` 与动作表的 `theme`）：两条路都落到 `apply_script_theme` →
**候选窗开着时先存进 `Router.pending_theme`**（换主题要重建 painter、整屏重画），组句结束、候选窗关掉那两处
（`handle_key` 的「不在组句了」与 `reset_composition`）调 `flush_pending_theme` 补上；真换在
`switch_script_theme`：先改 `RouterConfig.script_theme`（`render_settings` 里它盖过 `curr_theme`）即时换上、
`candidates.configure` 重建 painter、状态条重画一帧；要落到配置的主题名和 `curr_theme` 不一样时再
`persist("theme", "curr_theme", …)` 写配置 + `spawn_replacement_server()` + `restart_pending = true` **重启输入法服务**
（与设置页「应用主题」同一套 —— 重启才一定换上、选择也留了下来）。方法走请求通道（`Runtime::take_theme_request` →
`Option<Option<String>>`：没提 / 回本进程启动时那份 `/ 换到某份`），Server 在按键、`candidates`、回调三处取；
动作表那条由 `apply_common_actions` 落地，一拍里两个都给时**返回值优先**。`false` 回 `Router.theme_at_startup`
（建 Router 时记下的 `curr_theme`）；名字找不到时记一条日志、按缺省主题画，**不写配置也不重启**。
`apply_config` 仍保留 `script_theme`（热加载不打断脚本设的那份）。

**候选窗尺寸**（`cloudime.candidate.set_min_width` / `set_page_size` / `set_scale`）：Runtime 里收成一个
`SizeRequest`（三项都是 `Option<Option<T>>`：没提 / 恢复默认 / 设成某值，`take_size_request` 取一次就清），
Server 用 `apply_size_request` 落到 `RouterConfig` 的 `script_min_width` / `script_page_size` / `script_scale`
上 —— 最小宽度与缩放折进 `RenderSettings`（`min_width_pixels` 与新增的 `scale`，变了才重新下发给候选窗；
窗口 `effective_dpi` 用它覆盖 `Ctrl + 滚轮` 的级数，用户一滚就清掉自己那份让滚轮接管），一页候选数进
`Router::page_size()` 所以要 `recompose` 才看得见。三项**只在本次组句内有效**：`handle_key` 末尾的
「不在组句了」那一处与 `reset_composition` 都调 `clear_script_size()` 回配置值；配置热加载时也特意保留
（与三个全角 / 半角字段同样处理）。

**主题**（三个窗口换配色）：`RouterConfig.curr_theme`（`[theme] curr_theme` 的文件名）折进 `RenderSettings.theme`（一个 `cloudime_platform::ThemeFile`），
`RouterConfig::render_settings()` 里由 `load_theme(name)` 读出来 —— 先 `find_theme` 定文件（用户目录优先），再按「路径 + mtime」缓存内容，命中就跳过读盘 + JSON 解析
（目录扫描每次照做，它决定优先级）；读不到 / 读不动就用缺省主题。`Painter::configure` 比 `RenderSettings` 不等就整体重建：`apply_theme_colors` 把 21 项套到 `Theme.colors`，
候选窗 / 工具栏 / 提示三份 `Shadow` 也按主题里的阴影色重新构建。画的时候候选窗走 `candidate_shadow`，状态条与状态切换提示经
`Painter::render_status(.., StatusKind::Bar / StatusKind::Tip)` 各取自己那套背景 / 图标 / 阴影色。
`apply_config` **不再**用 `settings != previous` 当门：那种 `previous` 是拿**磁盘上刚改过的**主题文件读出来的，两边会相等，
「重新应用同一个主题」就被吞掉（只有换主题名才生效）——现在每次热加载都推给 `Painter::configure`，由它跟**当前真正生效的**设置比对。

**输入法自己的组合键优先**：`Ctrl+数字`（主键盘 / 小键盘）、`Ctrl+Enter`、`Ctrl+反引号`、`Shift+反引号`
这四类**不派发给脚本**（`handle_key` 开头用 `input::reserved_combo` 判一下，命中的那几拍脚本完全看不到），
Server 只按自己的语义处理 —— 谁先定义谁优先，脚本抢不走输入法的核心操作。别的 `Ctrl` 组合与不带修饰键的键
照旧派发（脚本能 `passthrough` / `commit`）。

**异步请求**：`cloudime.http_get(url, timeout_ms, callback)` 与
`cloudime.http_post(url, body, options, callback)`（见 `crates/cloudime-script`）的结果在**以后某一拍**由
`tick()` 里的 `poll_scripts_requests` 收：调
`Runtime::poll_requests`（第二个参数是**那一刻**的候选，回调不必依赖请求发出时的旧下标），回调返回的表按同一套
动作语义合并后落到当前这一屏，再 `reconcile_candidates` 重画。这一拍**没有按键**，所以 `passthrough` / `commit`
从语义上无从谈起：给了就记一条日志忽略；`order` / `display` / `notice` / `adjust` / `online` 照常生效。

**触发条件**（清单必填的 `trigger_condition`）：`combination_key` / `key` / `sys_time` / `candidate_context`，
`sys_time` 还要 `trigger_time`（本地 `"HH:MM"`）；`combination_key` 还要 `combination_modifiers`（要哪一套修饰键，
10 选一：`ctrl` / `ctrl+alt` / `ctrl+shift` / `alt+shift` / `alt` / `win+ctrl` / `win+alt` / `win+shift` /
`win+alt+ctrl` / `win+ctrl+shift`，位图 bit0 ctrl / bit1 alt / bit2 shift / bit3 win）；`key` 还要 `keys`
（具体的键，26 个键名 → 虚拟键码，`KEY_NAMES` 表，必填、去重、有一个不认识整个脚本判无效）。`runtime.rs::parse_manifest`
里解析，`Manifest::accepts` 按它过滤事件 —— `startup` 谁都收，`key` / `candidates` / `time` 只给对应的那一种
（`key` 两种触发条件都收 `key`；`key` 触发还要**对得上清单声明的 `keys`**：组句里 Server 会把所有按键派出去
给引擎，但运行时只把声明的那些转给 `key` 脚本；`time` 还要对得上 `trigger_time` 的**当前本地整分**）。`Runtime::wanted_keys()` 汇总出
声明的虚拟键码（`wanted_keys` 供 Server 折成 `InputSettings.script_keys` 位图），`wants_surrounding_text()`
从「有没有 `key` 脚本」推出 `InputSettings.script_wants_text`。Server 这边两处用它：`RouterConfig::from` 不收，
`input_settings()` 里 `combination_key_masks()` 折成 `InputSettings.script_key_modifiers`、`config::script_key_bits()`
把 `wanted_keys()` 折成 `script_keys`（虚拟键码的 256 位位图；DLL 据此在没组句时也送这些键，**精确到键、不看修饰键**，
见 `wants_declared_key`）；
`Router::tick` 里 `poll_script_time()` 先问 `Runtime::sys_time_due()`（本地整分去重，没有 `sys_time` 脚本时
一次都不花），到点就 `dispatch("time", time_payload(focused_app()), …)` 并按「没有按键的那一拍」落地
（`apply_common_actions` + 重画；载荷同 `get_time()` 另加 `app`）。`cloudime.foreground_app()` 与那个 `app`
读的都是 `Router::sync_focused_app` 维护的快照（焦点一变就刷新：`ensure_focus` 与两处断开焦点）。
DLL 那边：`would_eat` 命中声明的修饰键或声明的键（`wants_script_key` / `wants_declared_key`）就答「吃」，
Server 回 Passthrough 时用 `SendInput` `replay_to_app` 把键还回应用（`SendInput` 只注键本身，修饰键按用户手上还按着的算），
并给 `replaying` 打标记让下一次 `OnTestKeyDown` 别重复吃（`is_replay`，别的键先到就作废）。
**重放延后**：**不带修饰键的键**（`Enter` / `Tab` 这类）延后 ~150ms（`event.rs::send_key_deferred` 起个小线程）——
物理键还按着那一刻注入同名键会被系统标成「自动重复」（`lParam` previous-state 位），Chromium 系（Electron）
直接丢掉这个键（真机：OpenCode 里回车失效）；等物理键松开再注入就不是重复键了。**带修饰键的组合**
（`Ctrl+C` 这类）**立刻注入**：延后的话用户多半已经松开修饰键，应用只看到光秃秃的那个键、复制粘贴失效
（组合键在 Chromium 里也不受自动重复影响）。注入带**扫描码**（`MapVirtualKeyW`），有的宿主按它映射按键。
⚠️ 脚本自己上屏的文本里带 `\r\n` 时，**紧接着读回的前文可能少最后一个字符**（Firefox 实测：宿主把光标停在
最后一个字符之前，不是读取的锅，`ShiftStart` 与 ACP 两种读法都如此）—— `key` 脚本自己上屏后下一键读前文
得自己做点容错（自动序号示例记着刚写的序号补全）。
`script_wants_text` 为真时 DLL 还在**没组句时每个按键之前**请一个**异步只读编辑会话**（`com/edit/surrounding.rs::request_surrounding_now`
→ `SurroundingSession`），现读一份光标前文发 `ClientMessage::Surrounding` —— 读发生在按键之后，所以那一份反映的是
**上一键之后**的文档（自动序号按 `Enter` 时能读到刚敲的 `1.`）。这份前文经 `set_surrounding` 落进脚本的
**`cloudime.context`**（`document` 仍是 `None`，整篇快照只随组句起始更新；`set_surrounding` 现在没组句也收，
只喂脚本、不喂引擎）。**只能异步**：同步编辑会话会把沉浸式应用的宿主搞崩
（见 `com/edit/update.rs` 文件头）。
**修饰键本身**（Ctrl / Alt / Shift / Win，含左右键）在 `would_eat` / `eats_key` 里一律放行
（`is_modifier_key`）—— 吃了 `Ctrl` 的 key-down，后面那个键在 TSF 与应用眼里就没有修饰键了
（`Ctrl+A` 变成打出一个 `a`、全选失效）。

**还没接的**（目标清单）：改「组句里已经选中的那一段」（Engine 没有「改组句内容」的入口，要 Core 侧新接口）、
把 Core 的排序数据（词频 / 出处）交给脚本（`candidates` 载荷现在只有文本与来源，脚本改权重时不看 Core 的数字）。
**启动可执行文件**不用接：完整标准库的 `os.execute('start notepad.exe')` 已经能做。

改脚本要重启 Server 才生效（托盘右键「重启输入法服务」），没有热重载。
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
**「不处于输入状态时自动禁用输入法」**（`[debugging] auto_disable_without_text_input`，缺省关）：Server 把开关随
`InputSettings` 下发给 DLL，判断与动作都在 DLL（只有它拿得到 TSF 的焦点）。`com/context.rs::in_text_input` 看本线程
有没有文本焦点（`ITfThreadMgr::GetFocus`）以及那个上下文有没有被标成 `EMPTYCONTEXT` / `KEYBOARD_DISABLED`
（只读视图、密码框），不在就按「禁用」走、回到文本区域再回到原来的中 / 英（`ModeState::resume_mode` 记着）——
走的是与系统 Ctrl + Space **同一条路**（`follow_system_disabled`：落定拼音、改模式、报给 Server），
所以按键放行、语言栏图标「禁」、Server 收起状态条都自动对上。

**但它刻意不写系统那条「输入法开 / 关」compartment**（`refresh_mode_indicator` 里用 `auto_disabled` 拦掉）：
2026-10-07 真机上先按「写 compartment」实现，结果**禁用之后回不来**——写关之后系统不再把文档焦点给文本服务，
`ITfThreadMgr::GetFocus` 也拿不到东西，焦点回到文本区域时我们收不到任何恢复的依据。只在本进程里按禁用走就没这个问题，
系统状态（别的输入法、Ctrl + Space）也不受影响。判定挂在轮询定时器那一拍（约 320 ms，见 `com/poll/mod.rs`），
改设置同样在下一拍生效。
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
「主题」页在 `pages/theme/`：`mod.rs` 是页面（`Slot` 是那 21 个颜色槽，`ALL` / `label()` / `get()` / `set()`，顺序与页面一致；高亮 / 普通候选序号各一项；「新建主题」按钮 + 名字 `TextBox` + 「确认保存」按钮、「选择主题」`ComboBox` + 「刷新主题」/「应用主题」两个按钮、21 行「名称 + 可点的 `#aarrggbb`」，末尾挂 `color_dialog::ColorDialog`）；
`preview.rs` 是页顶那块预览，**走真渲染器**：按草稿 + 配置里的四项字体拼一份 `cloudime_render::Theme`（`min_width_pixels` 固定 200），拼一帧样例（拼音行、两条候选、角标「句」/「造」、页码、译文 Tip、在线那一行），用 `Renderer::render` 画候选窗、`render_status` 画悬浮工具栏（7 个图标）与状态切换提示（前 4 个），三份各自带主题阴影色的 `Shadow`；
图标用 `include_str!` 把 `server/src/ui/status/icons/*.svg` 编进来（那几个文件是 cargo 的编译依赖，改了会重编）；位图 `encode_png()` 后交给 `Image::source_data(EncodedImage)`、`Stretch::None` 1:1 贴出来——所以预览与真实窗口像素一致（真阴影 / 真尺寸 / 真图标并按主题色染色）。
`theme/preview.rs` 里的 `apply_colors` 与 Server 的 `apply_theme_colors` 是同一份映射（两边依赖的东西不同，谁也不能反过来依赖谁，所以各留一份，**改键名 / 加颜色时两处一起改**）。
「导入主题」挑 `.json` → 先 `ThemeFile::load` 验一遍 → `save` 进用户目录 → `refresh_theme_names()` + `select_theme()`（自动刷新列表，但不自动应用）；「导出当前主题」把草稿 `save` 到用户挑的位置。
两行控件的首列固定 120（`PICKER_LABEL_WIDTH`，WinUI 的 `Button` 最小宽度本来就是 120），让「选择主题」的下拉与上面一行的名字框左边对齐。
状态在 `panel/mod.rs`（`theme_names` / `theme_selected` / `theme_draft` / `theme_new_name` / `theme_status` / `color_dialog` / `color_slot`），消息 `Theme*` 在 `message.rs`，处理在 `component.rs`：**改色只动草稿** —— 点颜色（`ThemeColorOpen(slot)`）开对话框，`ColorDialog`「确定」后 `slot.set(&mut theme_draft, ..)`，预览与那一行跟着变。「确认保存」只 `ThemeFile::save` 到 `%APPDATA%\CloudIME\Themes\<名字>.json`（**存下来但不换**）；「应用主题」才把选中的名字写进 `[theme] curr_theme`（同值也写）并**立刻重启输入法服务**（`crate::server::restart`，与托盘右键 / 脚本页那条同一条路）—— 重启才盖得过脚本临时换过的主题（没选中就提示先选一份，也不走保留名校验）。「新建主题」从内置的 `Themes\Default.json` 复制（`find_theme` 找不到就 `ThemeFile::default()`）；名字由纯函数 `theme_stem` 定（名字框空则沿用当前选中的那份；与随包两份内置主题 `Default` / `Panic` 同名的一律拒 —— 大小写不敏感，带 `.json` 后缀也认）。「刷新主题」重扫 `list_themes`。
「脚本」页在 `pages/scripts.rs`：顶部一行加粗红字的声明、三列 `Grid` 列表（文件名 / 介绍 / 启用开关 + 「删除此脚本」「编辑此脚本」）、「新建脚本」按钮；列宽与单元格抽成通用的 `controls::grid_row` / `controls::text_cell`（上面短语页那个 5 列 `Grid` 也改用它俩）；操作列宽 300 DIP，开关与两个按钮都放开最小宽度（`min_width(0)`，WinUI 的 `Button` 默认最小宽度是 120）、开关用 `ToggleSwitchSlot::OnContent` / `OffContent` 置空自带的「开 / 关」文字 —— 否则三个控件按默认宽度加起来顶出列外，最后那个按钮会被右边缘切掉一截。脚本目录 = 安装目录 `Scripts\`（与 `cloudime-script` 的 `DIRECTORY`、Server 读的同一处），列表跳过 `template.lua`；第二列的介绍是从文件里**文本扫**出来的（`describe_in` 认 `description = "…"` / `'…'`、跳过 `--` 注释）——设置程序里不执行脚本，Lua 运行时只在 Server；开关写 `[script] disabled`（`Config::set_array`，缺分节会补出来），删除先用 `rfd` 确认、再顺手把它从名单里摘掉；「新建脚本」在 `Scripts\` 取不重名的文件名（`script.lua` → `script-2.lua`…）、写入 `include_str!` 编进 exe 的 `Scripts\template.lua`，随后 `panel/notepad.rs` 用记事本打开并把模板 `WM_SETTEXT` 塞进它的 `Edit` 子控件（`EnumWindows` 先按我们刚起的进程号认主窗口、机器上本来开着别的记事本时才按类名 `Notepad` 兜底；等窗口与填字都在后台线程轮询，上限 4 秒，不卡界面）。这一页的改动（含开关）都要重启 Server 才生效 —— 「新建脚本」右边那个「重启输入法服务」按钮走 `crate::server::restart()`：连 `\\.\pipe\cloudime` 发一条 `ClientMessage::Indicator { RestartServer }`（设置程序不常驻连接，开一次用完即走），与托盘那条路完全一样（这条消息 Server 不回包，所以发完不等；连不上就把原因写在页面状态里）。
「调试」页在 `pages/debugging.rs`：**原「统计」页整页搬来的输入统计面板**（末尾是「数据与组件」说明）与紧随其后的 `[debugging]` 自动隐藏开关、
原来「高级」页的数据 / 日志入口（打开数据目录 / 打开日志目录 / 打包日志到桌面 / 清空输入日志四个按钮一行）与项目 GitHub 页面 / 帮助手册两个按钮一行、详细日志、学习输入习惯、记录输入日志。
「帮助手册」（`Message::OpenTutorial`）用系统默认程序打开随包的 `tutorial.md`（`component.rs::tutorial_path` = `bundled_root()/tutorial.md`，走 `controls::open_document`；
文件不在只记一条日志）。`open_document` 先用 `AssocQueryStringW` 问这个扩展名有没有默认打开方式（只问长度：`pszout` 给 null 时它把需要的大小写回 `size`，
没关联为 0），**没有就用 `notepad.exe` 打开**——`.md` 在很多机器上没关联，不这样会先弹「你要如何打开这个文件？」。
「通用」页已删（`Shift` + 字母固定进组句，见 `dispatch/key/input.rs::apply_chinese`：字母进缓冲区、`Caps Lock` 亮着的仍直通），
「统计」与「高级」两页并进「调试」；「关于」页已整体删除（版本在「数据与组件」里仍有一份，检查更新只剩 Server 侧（查并写 `update.json`，界面上不再提示），许可与数据署名看 `LICENSE` 与 `docs/design/landscape.md`）。
设置窗口的标题栏图标走 `ViewContext::window_visuals(WindowVisuals::new().icon(path))`（`component.rs::window_icon`）——WinUI 3 不会自动取 exe 里的图标资源，必须显式 `AppWindow.SetIcon`，而那个接口只收 `&'static str`，所以算一次「exe 旁 `cloudime.ico`」的绝对路径再 `Box::leak`；装机包由 `cloudime.iss` 装这份 ico，开发时 `settings/build.rs` 往 exe 旁拷一份。
设置窗口打开时的客户区写死 `WINDOW_CLIENT_SIZE`（`component.rs`：本机系统默认 1912×1028 的「宽取 2/3、高不变」= 1275×1028），再用 `clamp_to_work_area` 夹进主显示器工作区，小屏不顶出屏幕。**这个尺寸必须在第一次 publication 里就给具体值**：框架是「建窗 → 应用 `WindowVisuals` → `Activate`（显示）」三步，晚一步（让窗口先按系统默认显示、再靠 `on_window_size` 缩）用户就会看到「先宽后窄」闪一下；而那一刻窗口还没建出来（第一次 `view` 时枚举本进程窗口，一个都没有），量不到系统默认值，所以只能写死。`client_size` 收 DIP，框架自己按窗口 DPI 换算成像素。
窗口**位置**居中走同文件另一个法子：框架的 `WindowVisuals` 没有位置，等它再到组件里跑一趟（下一次 `view`）窗口已经显示了，挪过去会看到「先左后中」闪一下（实测约 2 帧）。所以 `create` 里装一个**本线程的 CBT 钩子**（`SetWindowsHookExW(WH_CBT, …, GetCurrentThreadId())`，本进程自己的窗口、钩子过程不必进 DLL）：`HCBT_ACTIVATE` 在窗口真正显示**之前**同步回调，在那里 `SetWindowPos` 到所在显示器工作区正中就看不到闪动；`view` 里还留一条「按 pid 找窗口再挪」的兜底（`center_window_once`，`CENTERED` 一次性开关）。
设置程序的 exe 清单由 `windows_reactor_setup::as_self_contained()` 生成（里面插着自包含标记 `<description>windows-reactor-self-contained</description>`，`build.rs` 在这一步之后**直接改 `OUT_DIR\app.manifest` 那一份**，往 `</assembly>` 前补一条 `Microsoft.Windows.Common-Controls 6.0.0.0` 依赖）：`rfd` 的确认框（`MessageBox`）、启动失败提示与系统字体对话框 `ChooseFontW` 这类**系统对话框**只有清单里有这个依赖才用主题控件，否则退回 comctl32 v5、按钮是老式 3D 外观 —— 框架自己的控件是自绘的，不受影响。**别另加一个 `/MANIFESTINPUT`**：两份清单交给链接器合并有丢掉自包含标记的风险（那标记是自包含部署的判据）。

托盘「中 / 英」图标的右键菜单（`tsf/src/com/mode/menu.rs`，`TrackPopupMenuEx` 挂输入框所在窗口）固定几项、不再切中英：灰显的「云朵输入法」标题、分隔线、「设置」（`IndicatorCommand::OpenSettings`）、「查看帮助手册」（就地 `ShellExecuteW` 打开与 DLL 同目录的
`tutorial.md`，不走 Server——那要给协议加枚举变体、还要升版本重装 DLL；`AssocQueryStringW` 问到这个扩展名没有默认打开方式时改用
`notepad.exe`）、「重启输入法服务」（`IndicatorCommand::RestartServer`）。
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

**状态切换提示**（`server/src/ui/status_tip/`，`[input] show_status_change_tip` 缺省开）：中 / 英、Caps Lock、
全 / 半角、简 / 繁、中文 / 西文标点任一变了，就在输入光标附近弹一个停留 1 秒的小条 —— 样式与悬浮状态条同一套图标，
只取排布表的前四个（`ui/status/mod.rs::StatusIcons`，`TIP_BUTTONS = 4`，`Arrangement` 只在本模块可见，提示窗经它拿图标）；
`WS_EX_TRANSPARENT` 点不着（鼠标穿透到下面的应用）、`WS_EX_NOACTIVATE` 不抢焦点，位置贴光标下方、放不下放上方（`place`，
比候选窗那套简单：不用避让高亮行）。触发在 Router（`dispatch/status/mod.rs::check_status_tip`）：与 `reconcile_status` 分开，
只在**用户切状态**的几条路（`handle_mode_changed`、`handle_status_event`（状态条三格与两个内置热键都汇到这里）、每拍 `SyncMode`）上调，
配置热加载**不**走它（改设置不该弹提示）；比较 `TipState`（中英 + Caps + 全半角 + 简繁 + 当前模式的标点），第一次观察只记不弹。
只在 `ime_active && !mode.disabled() && in_text_input` 且拿到过光标矩形时弹 —— 「在不在输入状态」与 Caps 都由 DLL 每拍
`SyncMode` 带上来（`ClientMessage::SyncMode` 的 `in_text_input` / `caps`，v17 加；`in_text_input`：`ITfThreadMgr::GetFocus`
拿不到文档、或上下文被标成 `EMPTYCONTEXT` / `KEYBOARD_DISABLED` 都算不在），因为 Caps 的按键根本不经过 Server。
定位用最近一次的光标矩形（`Router::last_caret`，`position_candidates` 里记，组句结束后仍留着）。

候选窗（`server/src/ui/candidates/`）只在高亮移动时做动画：方向键页内挪高亮、且行内容与拼音行都没变时，`set_content` 按
`Rendered::highlight_rects` 从「上一段的当前视觉矩形」或「上一高亮行矩形」起滑（连按是连续续滑）；高亮没变的新帧（重排、
异步编辑会话补报的组句矩形）不打断正在跑的动画；翻页 / 新查询 / 隐藏直接画或取消。窗口过程按 16 ms 的 `WM_TIMER` 算进度原地重贴，
约 150 ms 的 cubic ease-out 后收尾。
窗口过程按 HWND 从 UI 线程的表里找回窗口（`attach`）。纯展示，不改窗口位置大小、不涉及协议。

「更多候选项」（`Router::show_more`，组句里 `Tab` 切、只活在一次组句里，`recompose` 收组句时清）：展开后一页从
`candidate_count`（5–9）变成**一整屏** `5 × candidate_count`（`Router::page_size`），`CandidateLayout::set_page_size`
就地换页大小；帧里给 `columns`（竖排 5 列、横排 `candidate_count` 列），渲染器见到 `columns > 0` 就走矩阵那条路
（`draw_matrix`），不再看竖排 / 横排——「竖排展开成 5 列」与「横排展开成 5 行」只差 `columns` 取谁。**每格宽度就是
「收起时那条高亮条的宽度」**：壳侧 `ui/candidates` 在收起态记住 `highlight_rects` 里高亮那条的宽，随渲染帧的
`min_cell_width` 下发（窗口隐藏时作废）；比它还长的候选截尾加「…」，短的原样留白，候选不够一屏时不补空列。
还没有这个宽度时（这次组句还没画过收起态）退回「一屏里最长的候选」并封顶 `MAX_CELL_EMS`。
序号在 `RenderData::set` 里按 `columns > 0` 清空（数字键那时是跳页、不再选词），角标照旧。展开态的方向键在网格里走
（`Router::move_highlight_in_grid`：上下换行、左右不越行），拼音光标改用 `[` `]`（`input.rs::apply_printable` 里
先于「上屏候选 + 上屏标点」那条路处理），数字 `1`–`9` / `0` 跳到第 1–9 / 10 页（`goto_page`，越界夹到最后一页），
`Ctrl + 数字` 不杀词、原样交还应用。

候选窗的鼠标（`ui/candidates/` 的 `WM_MOUSEMOVE` / `WM_LBUTTONDOWN` / `WM_RBUTTONDOWN` / `WM_MBUTTONDOWN` / `WM_MOUSEWHEEL`）：`redraw` 记下阴影留白
（内容区坐标 ↔ 客户区坐标），`cell_at` 拿 `last_rects` 命中有哪一格（两种排布都逐行 / 逐项给了矩形，所以**收起态也认单击**）；
单击发 `CandidateEvent::Commit`，悬停发 `CandidateEvent::Hover`、鼠标移出候选窗发 `HoverLeft`（`TrackMouseEvent` 的
`TME_LEAVE`），右键发 `Translate(Option<usize>)`，中键发 `Speak`，滚轮发 `Page(±1)`——两种状态都上报（收起态也跟手），
都走 UI 线程 → 工人线程的 `Work::Candidate`，与状态条同一条路。滚轮翻页走 `Router::scroll`（不是键盘那条 `page`）：
**只换这一页的内容，高亮留在窗口同一格**（原来第几格翻完还第几格，最后一页不满时格号夹到末尾），与「编辑拼音不把高亮
拉回页首」同一个意思。
`Router::handle_candidate_event` 里悬停直接挪高亮，并把鼠标指的那一格记进 `Router::hover_cell`；**它指的那一格又正好
是高亮时**，`recompose` 不把高亮拉回页首（否则每敲一键高亮条都会从页首滑到鼠标那儿），鼠标移出候选窗、组句结束或
窗口收起才清掉。开头还按 `[candidate] mouse_word_selection`（`Router::mouse_selection_allowed`）过一道：
`off` 时**悬停 / 点选**一律不理会（右键翻译、中键朗读、滚轮翻页照旧，那不是「选词」），`more_candidates` 时
只有 `Router::show_more` 展开成网格那会儿才理会；缺省 `off`（新用户不用鼠标选词）。**上屏要等 DLL**：候选窗在 Server 手里、收不到按键，所以文本先攒进 `Router::pending_commit`，DLL 下一拍
`Poll`（组句中 80 ms 一拍）用 `ServerMessage::Update::commit` 带回去，DLL 侧 `apply_poll_commit` 再走一次编辑会话落进
文档。老 DLL（协议 < `CANDIDATE_CLICK_SINCE`）认不出这段文本却又会跟着清组句，所以 Server 见到老协议直接不认
这一下点击（`supports_candidate_click`）。协议因此从 v15 升到 v16。

翻译 Tip（`dispatch/translate/`，配置 `[translate]`）：`Translate` 持有清单、打开着的词典（`.qj` / `.db`）与学习库
（`%APPDATA%\CloudIME\translate.db`），`configure` 挂在配置热加载那条路上（`reload::apply_config`）——换词典、
「重置学习内容」（`[translate] reset_counter` 变了就 `Learning::reset`）都在这里落地；学习库只有 Server 一个写入者，
设置程序只改配置文件，所以不用抢锁、也不会被内存里的旧值盖回去。`self_drawn_frame` 给自绘帧补 `tip`
（只查**高亮候选**这一条，`Router::highlighted_text`）或多释义选择时的 `tip_choices`（`current_frame` 发给 DLL 的那份不带）。
按键在 `input.rs`：`Ctrl + 反引号`（`codes::BACKQUOTE = 0xC0`；DLL 那边所有带 Ctrl 的组合都会先问一趟 Server）查释义，
一条就 `Engine::commit_translation` 上屏、多条进 `begin_choices`；选择态下只有数字与 Esc 有效（`apply_key` 最前面分流到
`apply_choice_key`）。Core 的 `commit_translation` 只把**落进文档 / 日志 / 历史的文本**换成译文，拼音消耗、学习、
个人 n-gram、自动造词仍按候选走（`InputSource::Translation` 在评测里不计分）。

**发音**（`speech`，`Shift + 反引号` 与候选窗**鼠标中键**，`Router::speak`）：一条释义才念（`single_sense`），多条要么进
选择界面后念高亮那条（`apply_choice_key` / 选择界面里的中键）、要么不念；没有译文没动作。开着 `[translate] enabled`
且组句时这一键**一律吃掉**（`Effect::Navigated`，不再输入 `~`），没在组句 / 关掉开关时不认它、照旧打 `~`；中键在
关掉开关时也不出声。
语音走 Windows 的 SAPI（`ISpVoice`，「讲述人」念东西用的也是它）：`windows` crate 只生成接口，coclass GUID
（`CLSID_SpVoice` / `CLSID_SpObjectTokenCategory`）、`SPCAT_VOICES`、`SPF_ASYNC | SPF_PURGEBEFORESPEAK` 都得自己写；
声线按要念的文本认语言（汉字 / 假名 → `0x804`，字母 → `0x409`），COM 对象绑线程所以用 `thread_local` 留一份、
`SetVoice` 按语言去重，`Speak` 异步 + 「新的顶掉旧的」不挡消息循环，念不出来只 `warn`（绝不影响打字）。
测试里 `emit` 换成记下文本（`take_recorded`），不真出声、也不要求机器有音频设备。
鼠标**右键**单击候选走的是同一个 `Router::translate_action`（结果 `TranslateAction::{Commit(Option<String>), Choosing, Nothing}`：
按键那条路把 `Commit` 回成 `Effect::Changed`，鼠标那条路攒进 `pending_commit` 等下一拍 `Poll` 带走），
只在 `[translate] enabled` 开着时响应。
多释义选择（`Translate::choices`，`Choices { index, word, senses, highlight }`）：自绘帧临时变成「顶部一行词条 +
下面每行一条释义」，**不画页码**（壳按 `tip_choices` 去掉底部那一行）、**展开态（Tab）下临时收回单列 / 单行**
（`frame.columns = 0`，退出后 `grid_columns()` 自然恢复），并且**有高亮条**：`highlight` 是「第几条释义」，
鼠标悬停挪它（`set_choice_highlight`）、单击选那一条上屏（与按它的数字键同一条路 `choose_sense`），
方向键也挪它（`move_choice_highlight`；这一屏就一列 / 一行，四个方向都当上一条 / 下一条）、空格选高亮那条
（`codes::SPACE`），数字键与 Esc 照旧；`Shift + 反引号` 在这一屏里念高亮那条（**不选、不退出**）；
这一屏是纯状态改动，动画走候选窗那一套（`Effect::Navigated`
之后照样 `reconcile_candidates(self_drawn_frame())`）；选择界面开着时**不重排**（`rescore` 那边跳过，
否则 `Choices.index` 这个布局下标会对到别的候选上）。**右键（`Translate`，点在窗口任何位置都上报，
没点中格子是 `None`）= `Esc` 取消**。不在选择界面时右键只认点中格子的那一下（`Translate(Some(index))`）。

在线翻译那一行（`dispatch/translate/online.rs`，见 `docs/design/online-translate.md`）：**只是一个显示位**，
内容由脚本给 —— 动作 `online = "文本"` / `{ text, state }` / `false`（清掉）。Server **不碰网络、不认识任何厂商**：
「翻」是脚本自己用 `cloudime.http_post` 做的。这一行归脚本（换候选、改拼音都不动它），
只有组句结束（`reset_composition` / 这一拍之后不在组句）才收；`self_drawn_frame` 把它塞进 `Frame::online`
（只给自绘窗用，不升协议版本）。
它**也参与 `Ctrl + 反引号` 上屏**：`Online` 记着「写这一行时高亮候选是谁」，
`Router::translate_action` 把 `Online::translation_for(word)` 当成一条释义**插在最前面**
（`pos = ONLINE_POS`「在线」，列表里显示成 `(在线)`）—— 只有它一条就直接上屏，和本地释义一起就进释义选择；
上屏它**不**记本地词典的「学会」（`commit_sense` 的 `record` 参数按来源给），陈旧的那一行（挪过候选）不认。
TSF 那边 `key_sink::would_eat` / `eats_key` 在**组句里**（候选窗显示着）把带 Ctrl（不带 Alt / Win）的组合先送进 Server 问一趟
（以前是个白名单：数字 / 回车 / 反引号 / T）—— 脚本才绑得上任意 `Ctrl` 组合；没人绑时 Server 回
`Passthrough`，按键照旧交给应用。**没在组句时**默认一律归应用（`Ctrl+A` / `Ctrl+C` 这类快捷键不能被输入法吃掉），
只有脚本声明过的那几套修饰键（`combination_key`，DLL 的 `wants_script_key`）与那几个具体的键（`key`，
DLL 的 `wants_declared_key`）才先问一趟（见「用户脚本」一节的触发条件）。`Alt` / `Win` 不碰
（AltGr = Ctrl+Alt、Win 是系统键，都在外壳那一层）。
随包在安装目录 `Scripts\lib\` 下给两样东西做这件事：`md5.lua`（纯 Lua MD5，只依赖 LuaJIT 的 `bit`，
拿 RFC 1321 向量回归）与 `example-niutrans.lua`（完整示例）；`lib\` 是子目录，加载器不认，所以不会被执行。
示例拿到译文后会按 `cloudime.candidate.width()`（候选窗内容区宽度，点）折行（最多 5 行、超出的补「…」），
不再改窗口宽度 —— 窗口宽度只由候选决定。

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

`glossary-db`：把青简那套释义表 `.qj` 转成翻译 Tip 用的 `.db`（总表 `words` + 副表 `contents`）：中文词与译词照搬（一个词几条译词就写几条记录），拼音与首字母从 `--word-bank` 指的词库查（云朵 TSV / `.db` / `.qj` 都认，装机目录的 `WordBank\Dict.db` 直接就行），查不到的留空；`--out` 给目录就放进去、缺省与输入同目录。写完用读端重新打开核对词条数 / 译词数，对不上报错。
用法：`cargo run --release -p cloudime-dict-convert -- glossary-db LocalDictionary/glossary-en.qj --word-bank WordBank/Dict.db --out LocalDictionary`。

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