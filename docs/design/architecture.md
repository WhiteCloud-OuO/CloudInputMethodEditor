# 架构

## 总体结构

```text
                     CloudIME Core
                          │
                          ▼
          Windows Adapter（TSF DLL ↔ Server 进程）
                          │
                          ▼
                    Candidate UI
```

词库、拼音解析、候选生成、排序、用户词频学习全部属于 Core。
平台层只做两件事：把系统输入事件翻译成 Core 的输入，把 Core 返回的候选画到候选窗口。

判断标准：换一种输入法框架接 Core（TSF 换成别的），不应该需要改 Core 的任何一行。

## 架构约束

这些是核心设计决定，不要违反。

1. **Core 平台无关。** `cloudime-core` 及其兄弟 crate 不允许依赖任何平台 API。
   平台层里不允许出现排序逻辑或词库访问。
2. **输入优先于学习。** 任何为学习功能增加的延迟、弹窗、UI 干扰都是设计错误；
   附加查询不能阻塞候选生成，Core 必须能在附加结果尚未就绪时先返回候选。

## Workspace 结构

```text
cloudime/
├── crates/
│   ├── cloudime-core/          # composition / parser / correction / candidate / ranking / sentence / engine …（下面单列）
│   ├── cloudime-dictionary/    # 词库加载与查询
│   ├── cloudime-learning/      # 用户词频、用户词、自造词库（UserWordBank.db，缺省在安装目录 WordBank\ 下）、个人英文词、个人 n-gram、个人敲错表（user.tsv / user-words.tsv / user-english.tsv / user-ngram.tsv / user-typos.tsv）、输入日志（input-log.jsonl）、输入统计（usage.tsv）、词汇记录（user-vocab.tsv）
│   ├── cloudime-lm/            # 整句转换的 bigram 语言模型：LanguageModel 的实现
│   ├── cloudime-neural/        # 字级 Transformer 的本地推理（candle）：SentenceScorer 的实现，给整句前几条路径重打分
│   ├── cloudime-format/        # .qj 数据容器：mmap 打开、零拷贝视图、写入器、可落盘的哈希索引（dictionary / lm 依赖它）
│   └── cloudime-platform/      # 平台层的公共部分：配置文件、协议类型
│
├── apps/
│   ├── cli/                    # 测试工具：查询、逐键计时、输入日志回放评测、整句评测
│   └── windows/                # Windows 壳：server（Server 进程：IPC 分派 + Engine + 命名管道 + 自绘候选窗）、settings（设置程序）、tsf（TSF 文本服务 DLL，cdylib）
│
├── tools/
│   ├── dict-convert/           # 产品数据生成：lexicon / bigram / mine / english / pack
│   └── corpus/                 # 语料预处理脚本（uv）
│
├── assets/                     # 随仓库的产品数据源：词库源、图标、样例
├── data/                       # gitignore：语料、Unihan、生成物 data/generated/
├── docs/
└── README.md
```

`cloudime-core` 内部模块：

```text
cloudime-core
├── composition     # 输入状态机：未选拼音缓冲、已选文本段、光标；选中只并进组句，整段转换完才整体上屏
├── parser          # 拼音切分（全拼 / 简拼 / 模糊音）
├── candidate       # 候选数据模型（Candidate / CandidateKind）；layout 是分页排布，壳与设置程序共用
├── ranking         # 候选排序
├── shortcut        # 快捷候选：日期 / 时间 / 星期、v 表达式模式（四则运算、中文数字），不查词库
├── english         # 英文候选：词表精确词 / 前缀补全 / 一处编辑纠正（edit.rs），大小写跟着敲的走；主要用在中文模式的中英混输
├── sentence        # 离线整句转换：词图 + bigram Viterbi + 束搜索，LanguageModel trait（cloudime-lm 实现，缺省退化为一元），UserNgram 个人 n-gram（二元 + 三元），Context 上文（前两个词），
│               #   SentenceScorer trait（cloudime-neural 实现）：convert_paths 出前 K 条路径，Engine（engine/rescoring）按 路径分 + λ·(神经分 − 静态分) 重排，异步时后台线程打分、壳停顿后取
├── fuzzy           # 模糊音：FuzzyRules（配置 [input] mo_hu_yin_list，一位一条规则）把每个音节扩展成多种写法，Expanded 借出给词库多写法查询
├── engine          # 对外门面：Engine，以及 Learner trait 与空实现；表达式前缀固定 v（`shortcut::EXPRESSION_PREFIX`），问字与删候选已下线
└── storage         # 小文件落盘原语：write_atomic（临时文件 + fsync + 改名）、read_text_lossy；学习 crate 与配置都用它
```

词库内存布局（`cloudime-dictionary`）：词文本与拼音键各放一个连续 arena，词目只存 `u32` 偏移 + 词频，
键按字节序排好（当年 89 万条的测试词库约 150 MB RSS，比 `String` + `Vec<String>` 的朴素布局省三分之二；现在产品词库 8.7 万条，
且 `.qj` 是 mmap 直接映射，见「数据文件」）。
查询接口是 `lookup_pattern(&[SyllablePattern])`（命中音节数 ≥ 模式长度）与 `lookup_exact`（正好等长），每个位置可以是
完整音节或前缀 / 声母。实现是逐级前缀收窄：「以某段前缀开头的键」在排好序的索引里总是连续区间，完整音节直接二分到
`前缀 + 音节 + 空格`，简拼位置按区间里实际出现的音节跳块（每块看第一条键就能二分出块尾），区间小于 48 条就改线性比对；
代价与匹配到的音节组合数成正比，与首音节下有多少键无关。每个位置可以给多种写法（`lookup_pattern_alt` / `lookup_exact_alt`，
模糊音用），同一位置的写法在每级逐个走、代价相加不相乘；调用方保证同一位置的写法互不覆盖。Engine 侧同一次查询里相同的前缀模式只查一遍，
命中远超 500 条时先按便宜的预选键（词频 × 选择次数 × 纠错折扣 × 联想折扣）`select_nth_unstable` 砍到 1000，再算上下文与同输入串选择次数、按权重排，候选最多给壳 500 条。

拼音切分（`parser`）：按位置做动态规划，每个位置只保留最优 8 种前缀切分，token 可以是完整音节、
声母（简拼）或末尾未打完的前缀。排序键：音节少 > 不完整音节少 > 前面的音节长。

中英混输（`Engine::push_english`）：整段输入（不含 `'`）在英文词表里就加一个 `CandidateKind::English` 候选，
上屏吃掉整段输入。英文候选与中文词、整句候选进同一个权重池：权重是英文词频 × (1 + 用户选过次数)，
没有词频按 1.0；拼音不像话时顺带出前缀补全。词表 `WordList` 在 dictionary crate。

代码组织约定（2026-09-03 起）：一个 struct / enum / trait 及其 impl 单独一个文件，
模块文件只做 `mod` 声明、re-export 和自由函数；结构体字段逐条 `///` 注释并用空行分隔；
`thiserror` 的 `#[error]` 文案用英文，日志与 UI 文案用中文。

`Engine` 的会话 API：`set_input / push / backspace` 喂拼音，`query()` 返回不带译文的
`Query { segmentations, candidates, timings }`，`annotate(&mut CandidateList)` 补译文。
`commit(&Candidate) -> Option<String>` **选中**一个候选并立刻喂给 Learner（词频、词转移、自动造词，时机与以前一致）：
候选只吃掉一部分拼音时返回 `None`，这一选择并进组句的已选段（`Composition::selected`），组句显示成
「已选文本 + 未选拼音」，候选只对剩余拼音出，退格先撤回最后一次选择；整段转换完（未选拼音为空）时返回
`Some(已选文本 + 本次)`，壳才一次性把整段交给应用。回车 / 组句中的标点走 `take_raw`（已选文本 + 剩余拼音原样）。
`Learner::flush()` 由壳在退出 / 停用时调用，
失败只记日志不返回错误；壳停用时还调 `break_chain()`，之后上屏的词按句首记；
`Engine::flush_learning()` 把学习数据与输入日志一起落盘且不作废格子缓存，壳激活期间也定时调它。

### 崩溃不丢：原子写、损坏容忍、panic 隔离

输入法进程随时会被系统杀掉或自己崩掉，用户攒的学习数据和正在打的字都不能因此没了：

- **原子写**（`cloudime_core::storage::write_atomic`）：学习 crate 的六张 TSV、输入统计 `usage.tsv`、词汇记录 `user-vocab.tsv`、`config.toml`、`.env` 都先写同目录的临时文件，
  flush + fsync 后改名覆盖；任何时刻磁盘上要么是旧文件要么是新文件。输入日志 `input-log.jsonl` 是追加写不走这条路，
  崩溃最多留半行，回放工具按行跳过坏行并计数。
- **损坏容忍**：学习数据各文件按行解析，格式不对的行记一条警告跳过（下次落盘就清掉了），编码坏掉的字节按替换字符读进来；
  只有权限、坏盘这类真正的 io 错误才算读失败，这时壳退回只在内存里学习（不带路径，不会拿空表覆盖用户的文件），输入法照常启动。
- **panic 隔离**：Core 与 Engine 跑在 Server 进程里，它崩了不带倒任何一个应用——应用进程里只有连它的管道客户端。
  DLL 发现管道断开就按没组句处理（字母照常送到应用），并顺带 `ShellExecuteW` 把 Server 拉起来（见下「TSF DLL」），
  最多丢掉正在打的那几个字母。DLL 自己跑在应用进程里，panic 会带走应用，所以起组句的编辑会话、轮询、
  键盘布局这几个边界都用 `catch_unwind` 拦住（`com/edit/update.rs`、`com/poll/mod.rs`、`com/key/layout.rs`），
  拦下后这一拍不写、进程不倒。
- **有界丢失**：学习数据除了会话结束时保存，Server 工人循环还按 `LEARNING_FLUSH_INTERVAL`（60 秒）定时 flush
  （没有新数据时是空操作），被杀最多丢一分钟的学习。

实际结构会随开发调整，调整后同步更新这里。

## crate 依赖方向

```text
cloudime-dictionary        （纯数据加载与查询，不依赖任何兄弟 crate）
        ▲
cloudime-core              （定义 Learner trait，依赖 dictionary）
        ▲           ▲            ▲
cloudime-learning  cloudime-lm  cloudime-neural   （实现 core 的 trait，依赖 core）
        ▲           ▲            ▲
cloudime-platform          （配置文件 Config：general / shortcut / fuzzy / model 分节，toml_edit 原地改键保留注释；协议类型，可序列化；依赖 core）
        ▲
apps/*                     （组装：Engine::new(dict).with_learner(..).with_language_model(..)）
```

`apps/cli` 是 Phase 1 的测试壳：`cargo run -p cloudime-cli -- kaifa` 直接查询，
不带参数进入交互模式（拼音查询、序号上屏、`:q` 退出），`--user-dict` 指定用户词频文件。
`--typing` 是性能测试模式：把输入当一键一键敲进去，每个前缀查一次，一行一键打印各阶段耗时
（这是输入法每键的真实工作量，联想在后台线程不算），启动日志里带各数据文件的加载耗时。
性能改动要用 release 构建跑它看数字，目标每键 10 ms 以内。

- Core 只依赖 dictionary，不依赖 learning。学习通过 trait 注入（`Learner` / `InputLogger` / `UsageMeter` / `VocabularyTracker`，
  缺省实现都是空操作），这样 Core 的单元测试和 CLI 工具不需要真实词典也能跑。
- `cloudime-platform` 里的类型必须可序列化（serde）：Core 在独立的 Server 进程里，TSF DLL 走协议跟它说话；
  设置程序改的 `config.toml` 由 Server 热加载读回，不另开一条通道。
- `storage` 只放 Core 自己的持久化原语，用户词频的数据模型归 `cloudime-learning`。

## Core 的关键技术决定

### 多词库

Engine 查词的词库是一个列表，**按优先级从高到低**：用户词（Learner 持有）、主词库（随包 `WordBank\Dict.db`，按语言与稀有度拆成 7 张表；这里指普通中文组）、
稀有词库（同一份 `Dict.db` 的稀有组，缺省关）、附加词库（`Engine::set_extra_dictionaries`，用户导入的第三方词库，**按添加顺序**）。
它们一起进词级查询和整句词图，且**跨词库按词文本去重、靠前优先**：同一个词在靠前的词库里命中后，后面的词库不再产出这条
（同一本词库内部的重复照旧全收，交给排序按名次去重）。附加词库不带语言模型，它的词在路径上按词频兜底打分（`sentence::fallback_log_prob`），
所以导入的第三方词库只影响「有没有这个词」和它的词频，不改变语言模型的尺度。
主词库装配成三份：普通中文（`Engine::new` 的主词库）、稀有中文（`Engine::with_rare`，`Engine::set_rare_enabled` 整组开关，缺省关，装配后按 `[word_bank] rare_items` 设置）、
英文（`Engine::with_english`）给中英混输的英文候选与句末英文段用；中文两份仍是原来的内存二分词库。
壳负责装配：词库都在随包根的 `WordBank\` 下，主词库 `Dict.db`；用户导入的第三方词库是同目录下另外的 `.db`，
目录里有的全部加载（没有 List.dat 这种启用清单）。导入 = `cloudime_dictionary::import` 只收现成的 `.db`（读一遍校验后原样复制进目录），
移除 = 文件挪到 `WordBank\removed\`，
之后 Server 的热加载（`dispatch/reload`）按目录快照重新装配。设置 → 词库页只列导入的第三方词库，内置的 `Dict.db`（随包）与
`UserWordBank.db`（用户自造词库，缺省也在 `WordBank\`，位置由 `[word_bank] user_file` 决定）没有开关、始终加载、不出现在列表里。这也是第三方词库带着自己许可证单独分发的落点：
词库元数据里有名称与许可证，设置 → 词库页里直接显示。用户短语另存安装目录的 SQLite（`Phrases\Phrase.db`，`user` / `cloudime_default` 两张表，见 `crates/cloudime-platform/src/phrase.rs`）。

### 数据文件：`.qj` 容器

词库、语言模型这类常驻数据用自己的二进制容器 `.qj`（`crates/cloudime-format`），原则是**内存布局就是文件布局**：
从 TSV 解析出来的几段连续数组（词库的词文本 arena、拼音键 arena、键索引、词目；语言模型的词 arena、词表、哈希索引、
CSR 偏移与后继）原样落盘，打开时 mmap 整个文件、校验一遍头与分节边界，不反序列化。启动从 0.9 s 降到 50 ms。

- 文件 = 32 字节头（魔数 `CLOUDIME`、格式版本、数据种类 `Kind`、分节数）+ 分节表（4 字节标签 + 偏移 + 长度，正文 8 字节对齐）
  + 各分节。第一节固定是 `META`：TOML 的 `Metadata`（名称、许可证 SPDX、署名、来源、版本、条数、生成者），
  设置 → 词库页的列表直接显示它，第三方词库各带各的许可证靠的就是这一节。
- 数据种类：词库、语言模型、英文词表，以及本地整句模型 `Kind::Model`——扩展名换成 `.qjm`，
  三节 `CONF` / `VOCB` / `SAFT` 原样装训练仓库导出的 `config.json` / `vocab.json` / `model.safetensors`（safetensors 是不透明载荷，
  mmap 后切片给 candle，张量搬上设备后容器即丢；`META.entries` 记参数量）。`cloudime-neural::find_model(dir)` 先找 `.qjm`、没有再认三件套目录，
  所以开发直接加载训练直出的目录，随包与用户目录只有一个文件；`dict-convert pack model`（`tools/release/pack-model.sh` 带元数据调它）打包。
- 数值小端、原生对齐，crate 在大端机器上拒绝编译。字符串分节打开时校验一次 UTF-8，之后 `Text::deref` 走 unchecked
  （曾经每次 deref 都重新校验 30 MB，CLI 直接卡死）。定长结构体用 `zerocopy` 派生，`#[repr(C)]` 且手工排字段消灭填充
  （`KeyIndex` 16 字节、`Slot` 12 字节、`WordEntry` 12 字节、`Successor` 8 字节）。
- 两种视图：`Table<T>`（`Owned(Vec<T>)` / `Mapped`，`Deref<Target = [T]>`）与 `Text`（`Owned(String)` / `Mapped`，
  `Deref<Target = str>`）。解析路径与映射路径产出同一种结构，查询代码不区分。
- 文件里的哈希索引（`cloudime_format::hash`）：开放寻址、槽里放条目编号、键留在 arena；哈希函数必须跨进程、跨版本稳定
  （写文件的进程和读文件的进程算出来要一样），用 FNV-1a 64 加 fmix64 终混（FNV 低位对 UTF-8 中文这种字节模式相近的短串分布差，
  只用低位选槽会长链）。**为什么手写而不是用库**：标准库与 foldhash 的哈希器带随机种子，不能用；blake3 / SHA 这类密码学哈希
  一次几百纳秒、且是为抗碰撞设计的，这里每键要算几千次、只要分布均匀；xxh3 / wyhash 这类非密码学库能用，但任何依赖升级
  悄悄改了算法（或换了默认种子）就会让用户机器上所有 `.qj` 失效，而这个函数总共 12 行、有测试钉死输出值，
  自己写风险最小。若以后碰撞或分布出问题，换成 xxh3 并把 `FORMAT_VERSION` 加一。
- 写文件先写同目录 `.qj.tmp` 再改名；数据文件只整体替换，从不就地修改（mmap 的安全前提）。
- 生成：`cargo run --release -p cloudime-dict-convert -- word-bank --name … --license … --source …` → `WordBank\Dict.db`（7 张表的 SQLite 存档，`meta.format = 3`），
  `pack lm --name … --license …` → `data/generated/lm.qj`（`tools/release/data-bundle.sh` 打包时校验这些文件齐全、不自己重打）。`Dictionary::from_path` / `DictDb::from_path` 按文件头与表结构自动选
  SQLite / `.qj` / TSV 路径（SQLite 旧 `format` 1 / 2 也认），`BigramModel::from_path`（`.qj`）与 `from_paths`（TSV）分开；输入法与 CLI 优先用打包好的那份。
  英文词表已并入 `Dict.db` 的 `words_english`，不再单独分发。
- 格式版本不兼容时 `FORMAT_VERSION` 加一，读旧版的代码按需保留；`Kind` 编号只增不改。

### 候选生成需要整句转换

词级候选（trie 查词库）只能做到「能用」。日常可用的门槛是整句转换：
bigram 语言模型 + Viterbi，加上简拼、模糊音。没有整句输入，开发者自己都不会切换过来用，
学习功能就没有承载体。

词库与语言模型不自造，见 [landscape.md](landscape.md) 的数据源一节。

### 联想与个人模型的边界

- 整句转换里最贵的一步是词图格子查词（每个简拼位置都要在词库里逐音节块收窄）。敲键是增量的，第 n+1 键只新增以它结尾的
  最多 8 个格子，所以格子候选放在 `sentence::SpanCache`（键是格子模式含模糊写法，值是排好截好的 `SpanWord`）跨按键复用；
  候选与词库、用户词、选择次数、个人出现次数有关，Engine 在 commit / `learner_mut` / 换 Learner 时整个清掉。
  拼写纠错的上千个变体先过无分配的 `parser::is_fully_segmentable`，剩下几个才做真正的切分。
- 整句转换的语言模型通过 `LanguageModel` trait 注入（`sentence/language_model.rs`）：`log_prob(previous, word)`，
  模型不认识的词返回 `None`，Core 用词库词频兜底并扣分。`cloudime-lm::BigramModel` 从 `lm.qj`（或 `lm-unigram.tsv` / `lm-bigram.tsv`）
  加载：词表是 arena + 定长条目 + 文件里的开放寻址哈希索引；二元按前词分组成 CSR（`offsets[v]..offsets[v+1]` 是 v 的后继段，
  段内按后词编号二分，一次查找落在一两个缓存行里），P(w|v) = 0.8·c(v,w)/c(v) + 0.2·c(w)/N。
  数据由 `dict-convert bigram` 统计：用云朵词库做一元最大概率分词（与词图同一套词表），连续汉字段为句，`<s>` 句首标记。
  词图每格只留词频前 6 个词（有简拼位置的格子留 20 个：`h` 下几十个常用字，留少了句子里要的那个进不来），每个位置束宽 8。
  简拼位置就是前缀模式（`SyllablePattern.complete = false`），词库层不区分；每条路径覆盖的简拼位置相同，不需要额外罚分。
- **个人 n-gram**（`sentence/user_ngram.rs` 的 `UserNgram`，由 Learner 持有、`Learner::user_ngram()` 暴露）：
  上屏的词序列转移计数，二元 (前词, 后词) 与三元 (前二词, 前词, 后词) 一起记（整句按路径上的词逐条记，连续选词也记；
  标点、透传、回车上屏拼音、切应用打断链，下一个词按句首 `<s>` 记，句首词不记三元）。上文是 `sentence::Context`（前一个词 + 再前一个词），
  Engine 的 `CommitChain` 记最近两个上屏的词；Viterbi 不扩状态，前二词取前驱节点的回指（它那条最优路径上的前一个词），是近似。
  打分时与静态模型（只看前一个词）插值：P = (1−μ)·P_静态 + μ·P_个人，μ = c(v)/(c(v)+8) 封顶 0.5，前词没见过就不插值。
  P_个人 先算二元 P₂ = 0.8·c(v,w)/c(v) + 0.2·c(w)/N；这对上文 (u,v) 见过时再套一层绝对折扣的三元
  P₃ = max(c(u,v,w) − D, 0)/c(u,v) + D·N₁₊(u,v,·)/c(u,v)·P₂（D = 0.75），没见过的接续只拿回退的份额，(u,v) 没见过就是 P₂。
  三元不训练、不平滑参数，就是在线计数；它分辨的是二元混在一起的接续（「我想 → 去」与「不想 → 要」）。
  封顶保证没见过的接续最多打折、不会被压死；K = 8 让一次误选翻不过强 bigram，选两次才翻。
  个人出现次数也参与词图每格的前 6 选择，保证用户常用的同音词进得了格子。二元 + 三元超过 20 万条时所有计数减半。
  持久化在 `cloudime-learning` 的 `user-ngram.tsv`：三列 `前词\t后词\t次数` 是二元，四列 `前二词\t前词\t后词\t次数` 是三元，旧的三列文件照读。
  撤销（退格删光重选）、删词（`forget_word`）、减半都同时覆盖二元与三元。
- **自动造词**：用户自己连着选出的两个词（不是整句路径里的），合起来不超过 4 个字、词库与用户词里都没有，
  且这条转移已记够次数（同一段拼音里连着选的两次、分两段打的也两次），就记成用户词（`UserWordBank.db`，
  初始权重 = 各字在词库里的词频最大值）并记一次选择。
  同一段拼音里自选一个词之后剩下的部分走整句候选（`jidiaole` 选 挤、剩下 掉了 按空格）时，接缝处第一个词的转移按自选记双份，
  但不参与两词造词（我 + 的… 这种接缝太常见、转移计数早就够了，回放里会把 我的 造成用户词，之后它就得和 沃德 按选择次数比）。
  另外一段拼音分几次选完（`CommitChain` 记着这段里上屏的每个词）时，合起来的文本按「整段字母 → 合成词」记一次选择（`user-choices.tsv`），
  记到两次且词库里没有、不超过 4 个字就造成用户词，下次整段打出来它靠权重直接排前：这是「用户手动拼了一遍整句」最直接的信号，
  比等个人 n-gram 一份一份累到翻过静态模型快得多（「挤掉了」靠 n-gram 要选四次）。
  注意这只是**记学习**的时机没变：用户选中间某个词时不再立刻把它落进文档，而是并进组句的已选段（`云朵shurufa`），
  等整段转换完、或按回车 / 敲标点时，才把「已选文本 + 剩余部分」一次性交给应用（见上文 `commit`）。
  词级排序只有一条规则：按权重降序，权重相同按文本升序。一个词的权重是
  `词频 × 用户权重因子 × (1+同输入串选择次数) × 上下文系数 × 纠错折扣 × 联想折扣`：上下文系数是
  `clamp(exp(log P(词 | 上一个上屏词) − 词频兜底), e^-4, e^4)`（个人 n-gram 插值，模型不认识时系数 1），
  所以 `ba` 在「做了」后面出 吧、句首看模型；同一输入串下选过的词（`user-choices.tsv`，键是候选覆盖的那段字母）自然排前，
  `mgs` 选过 美国式 下次就是首选；查前缀带出的更长词按多出的字数打 `0.8^k` 联想折扣，模糊音命中打 0.5 折。
  用户权重因子（`Learner::rank_weight`）替代了原来的 `1+全局选择次数`：自造词就是它在用户词库里的词频（初始 = 各字词库词频最大值、每次重选 ×1.2、撤销退回一次），
  其余词按全局重复次数每次 ×1.15。
  拼写纠错（Core `correction`）只保留一处编辑的额外读法：拼音「不像话」（切不动 / 非末尾简拼，或末尾落单字母）时，把换位与
  **键盘相邻键替换**能整段切分的纠正读法作为额外候选加进同一个池子，按权重打折（换位 0.75、替换 0.70；原样输入本身对不上一整段词 /
  短语时再乘 1.2 / 1.15）。原样读法的候选一直保留，不再整段替换切分；删除 / 插入两类不做。整句词图不再有音节级敲错边；
  `Query::correction` 从此恒为 `None`，拼音行也不画删除线。上屏纠错读法的词时按那处编辑换算消耗的原串长度与 (敲的, 要的) 音节对
  （`Learner::record_typo`，`user-typos.tsv`），敲错对只记录、不再参与打分。候选音节对回敲的字母（消耗、记敲错）用 `Engine::align`，模糊音命中也走它。
  退格撤销（`LastCommit`，Engine 留最近 4 次上屏 `recent_commits`）：上屏后壳把组句外的退格告诉 Engine（`note_backspace`），退格从最近一次往前数，
  一次上屏的字删光了就候着；接着重打其中一段拼音（或其前缀）选了别的词，就把那次记的选择次数、输入串选择、词转移、整段合成词的选择全部退回（`Learner::unrecord*`）。
  组句里退格同理：有已选文本、光标又在未选拼音末尾时，`Engine::backspace` 先撤回最后一次选择（把中文还原成它的拼音，`Composition::unselect_last`），
  并把那条上屏记录标成「已被删掉」；紧接着改选别的词由同一条 `apply_retraction` 退回它的学习，光标停在未选拼音中间时照旧删一个字符。
  删掉「沃德 书」两个词重打成「我的 书」也认得出（日志里这种错法一天十几次，以前只看最后一次上屏，一次都没撤回，错词越选越靠前）；
  重打后选的还是同一个词只把记录丢掉；重打的拼音谁都对不上就当在改别处，全忘掉；删得比记着的几次加起来还多也全忘掉。
  标点、英文词、原样上屏这些没学习的上屏也留一条只有长度的记录，退格数过它们才能数到更早的词。
  整句候选与词级候选、英文候选进同一个按权重排的池子（不再无条件插到最前）：整句权重是路径词权重（`词频 × (1+选择次数) × 联想折扣`）的几何平均，
  再乘模型系数（神经重打分的 `clamp(exp(λ·(神经分 − 静态分)), e^-4, e^4)`，没拿到神经分时 1.0）。与词候选同文本同读音时不重复插、词留在原位；
  同文本不同读音的去掉词级那条、整句顶上。用户点选的转移记双份（`EXPLICIT_TRANSITION_WEIGHT`），整句路径里顺带的记一份：整句是模型自己算的，
  按空格接受会把它喂回模型形成回声，用户明确改选一次就要能压过去。
- **个人英文词**（`Learner::learn_english` / `user_english`，`user-english.tsv`）：回车 / 英文模式直通原样上屏的、像英文词的字母串
  （中文模式下要求切不成完整拼音）和选中的英文候选，组成一张小 `WordList`，与随包英文词表一起出英文候选、排在前面。见 candidate-ui.md。
- 个人化优先用在线 n-gram，神经模型只做重排，且要过评测门槛（见 roadmap Phase 7）。
- 本地输入历史与个人数据可查看、可清除。

## 平台层的技术决定

### Windows：TSF

- TSF DLL 会被加载进每一个应用进程，核心逻辑必须放在进程外。
  采用 Weasel（WeaselServer）和水杉（Server 进程）相同的结构：DLL 只做 IPC，Rust Core 跑在独立进程里。
- 使用 `windows` crate 的 COM `implement` 宏。
- TSF 是公认最难的输入法 API，工时预期要按整个项目一半来估。

**已落地（骨架）：**

- **IPC 协议**：`cloudime-platform::protocol`，Server ↔ DLL 两端共用、全部 serde。`ClientMessage`（DLL → Server：
  开 / 关会话、按键、上屏、回上下文、回选区、报中英模式）与 `ServerMessage`（Server → DLL：按键结果、上屏结果、异步重绘、请求上下文、请求选区）；
  失焦 / 停用时 DLL 发 `Commit`，Server 回 `Committed { text }`（缓冲区原样交出），
  DLL 用最近收键记下的 `ITfContext` 经编辑会话落进文档；应用强行终止组句（`OnCompositionTerminated`）时拼音已被框架定成普通文本，
  DLL 只记「Server 缓冲过期」，下次说话前先 `Commit` 并丢掉交出的文本，不再插一次。
  中英模式按本地习惯，单击切换键在中 / 英间翻转，
  切换键写死为单击 `shift`（Server 恒发 `SwitchKeys::default()`，设置里不再提供勾选项；协议仍带 `SwitchKeys`
  这一项，类型只为协议而留，缺省值就是 shift）、内置英文模式也不再可关（`[general] english_mode` 曾是它的开关，
  issue #81，现恒为开）。**模式全局一份、存在 Server**（`Router.english`，与搜狗一致）：
  DLL 里用户切了（切换键、语言栏按钮、任务栏转换模式）用 `ModeChanged` 报上去；激活、线程得到焦点（`com/focus.rs` 的
  `ITfThreadFocusSink`，切窗口时 `ITfKeyEventSink::OnSetFocus` 不触发）和每隔几拍的轮询用 `SyncMode` 取回并跟上
  （`service/mode.rs::adopt_mode`，不回报）；悬浮状态条上点「中 / 英」直接改 Server 那份。有 DLL 来取模式也就说明云朵输入法是当前输入法，
  状态条据此显示，`ImeSwitched` 收起。之前模式各应用各记一份、Server 只采纳「前台会话」的上报，后台线程激活、新应用继承都会让
  各处对不上，改成全局后这些都不存在了。
  单击判定在**击键 sink** 里（`com/key/tap.rs`，喂 `OnTestKeyDown` / `OnTestKeyUp`：按下切换键到抬起之间没有别的键插进来就是一次单击；
  微软 SampleIME 的 `OnTestKeyDown` 同样处理 VK_SHIFT，sink 收得到独立修饰键）。之前用线程级 `WH_KEYBOARD` 钩子判定，但钩子**看不到被 TSF 吃掉的键**
  （msctf 在队列层把它们改成 WM_NULL），被 TSF 吃掉的可打印键（组句中的字母、标点）会被误判成单击而切换模式，2026-09-11 真机确认 sink 收得到这些键后钩子已删。
  「Shift 字母进组句」由 **Server 读配置、经协议下发**（`protocol::InputSettings`：`OpenSession` 的回包
  `SessionOpened` 带一次，之后每拍 `SyncMode` 跟着走，值变了就地应用，**设置改完约 320 ms 内生效**，不必切走再切回输入法）；
  同一份 `InputSettings` 里的切换键与英文模式两项，Server 现在恒发固定值（`SwitchKeys::default()` 与 `english_mode: true`，
  设置里没有对应选项了）。DLL 不读配置文件——它跑在每个应用进程里，AppContainer 里的商店应用连 `%APPDATA%` 都读不到。
  `ctrl+alt+space` 是组合键、属系统键不经击键 sink，登记成 TSF 保留键（`com/key/preserved.rs` 的 `GUID_SWITCH_MODE`），
  协议里仍支持（`SwitchKeys::ctrl_alt_space`），缺省值不启用、设置里也没有勾选项。
  系统热键 Ctrl + Space：中文 Windows 把它绑成系统的「输入法/非输入法切换」（`IME_CHOTKEY_IME_NONIME_TOGGLE`），系统先截走、保留键收不到。
  但我们**适配**这条系统热键：它翻的是「输入法开 / 关」compartment（`GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`），
  `com/mode/sink.rs` 监听它（`sync_from_keyboard_open`）：关 = **禁用**、开 = 回到禁用前的中 / 英，照样报给 Server 成为全局状态。反向由 `refresh_mode_indicator`
  每次把开关写成与状态一致（`open = !disabled`；新线程里它缺省是关），系统热键下一次按下才总是真的切换、不白按；关着时按键照样送到 TIP（真机日志验过）。
  它的 Space 被系统截走，击键 sink 只看到 Ctrl 按下又抬起，会被当成单击 Ctrl，所以开关一变就作废正按着的单击（`KeyTap::cancel`）。
  装了别的键盘布局 / 输入法时 Windows 可能改用这条热键换输入法，那由系统决定。
  **三条固定内置热键**（不落配置）：`Shift + Space` 翻全角 / 半角（击键 sink 里拦，发 `IndicatorCommand::ToggleCharWidthType`）、
  `Ctrl + Alt + .` 简繁与 `Ctrl + Alt + ,` 标点（`com/key/preserved.rs` 里两个保留键，发 `ToggleSimpTrad` / `TogglePunctuation`）；
  后两个带 Alt、属系统键不经击键 sink，只能走保留键，且会在应用自己的快捷键之前被接走。
  **四条切换入口**——单击切换键、语言栏按钮、悬浮状态条、任务栏转换模式 compartment——都汇到 `com/service/mode.rs::set_mode`（`InputMode` 三态），
  下发的内置英文模式为关时在那里一并拦住（Server 侧状态条点击按同一项拦，免得两边显示不一致），此时连语言栏的「中 / 英」按钮都不登记；
  协议值恒为开，这条分支只是那一项仍占着协议。禁用 = 不接管：`would_eat` 直接放行、悬浮状态条收起（`reconcile_status` 只在 `ime_active && !disabled` 时显示）、
  任务栏图标用 `mode/off-*.alpha`。
  DLL 记 `english_mode` 持久状态；任务栏的中 / 英指示器靠 `GUID_LBI_INPUTMODE` 语言栏按钮渲染
  （`com/mode/button.rs`，图标是设计稿 SVG 预栅格化的四档 alpha 蒙版，`mode/icon.rs` 按系统 DPI 挑档、按任务栏 `SystemUsesLightTheme` 填黑或白，Caps Lock 亮着显示「A」、禁用时显示「禁」；第三方 TIP 单写转换模式 compartment 不出这个指示器），另外顺带写一份转换模式 compartment
  （`GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION` 的 `TF_CONVERSIONMODE_NATIVE` 位，`com/mode/mod.rs`）。这条 compartment 还**反向同步**：激活时对它挂
  `ITfCompartmentEventSink`（`com/mode/sink.rs`，与「输入法开 / 关」共用一个 sink），用户点任务栏中 / 英（或别的输入指示器途径）改了转换模式时 `OnChange` 读回 `NATIVE` 位、与当前
  `english_mode` 不同才翻转（相同即我们自己写的那次，忽略以防回环），翻转只刷语言栏按钮与悬浮状态条、**不回写 compartment**
  （`follow_system_mode`：在它自己的 `OnChange` 里写它会被拒、报 0x8000FFFF，要写的值也本来就等于当前值）；
  **激活后的最初一瞬除外**（`service/mode.rs` 的 `CONVERSION_RESTORE_GUARD`，约 1.2 s）：msctf 会在 TIP 激活后 200–300 ms 把 profile 存的
  转换模式写回 compartment，那不是用户操作，采纳了会被当成用户切换、改掉全局模式。Caps Lock **不参与模式判定**，
  它只是大小写位：亮着时字母连同大小写位一起交给应用（应用自己按 `shift XOR caps` 算），中英模式与标点的全半角都不看它；
  亮着时**单击 Shift** 会先 `SendInput` 补一次 Caps Lock 清掉锁定、再切英文（与微软拼音一致），撤销锁定后回到中 / 英由用户后续操作决定。
  `KeyModifiers` 因此带 `caps`（大小写）与 `english_mode`（持久模式）两个非物理位。Router（`dispatch/key/input.rs`）里
  `english = english_mode`，英文模式就是**纯直通**：字母与标点都归应用，不组句、不出候选，终端与代码编辑器里的
  补全因此占不掉（连「按应用关候选」的名单都不需要了）。「先上屏、再把这个键交给应用」的做法在 Windows 上会乱序
  （放行是同步的、上屏走异步编辑会话），所以组句中的空格 / 标点改成吃掉、连同上屏文本一起插入；Shift 大写字母交 [`Engine::push`] 进缓冲区
  （Core 按小写匹配、原样上屏时还原大写，见 [`Composition`]），不再走「先上屏再放行」；
  组句外没有全角映射的字符里，`-` `=` 也改由壳自己插入（`dispatch/key/input.rs::apply_punctuation`）：
  放行要等宿主把键交回应用，实测在部分宿主（Edge / QQ 等）里这个键到不了，用户看到的是「按了没反应」；
  其余（`@` 数字等）继续放行，宿主连不上 Server 时也只吃「可能是在打拼音」的字母（`service/key_sink.rs`），
  免得断连窗口里标点 / 数字跟着一起没反应；
  应用标识仍随 `OpenSession { app }` 报一次（宿主进程的 exe 文件名，`GetModuleFileNameW(NULL)` 取的，DLL 加载在应用进程里），
  Server 每会话记下、写进输入日志。
  一次要绘制的状态是 `Frame`（preedit 分段 + 候选页 + 可选的提示 `notice`；当前没有来源使用 `notice`，字段保留），preedit 用 `PreeditSegment`（Core `MarkedSegment` 的可序列化镜像，
  协议不耦合 Core 内部枚举），候选直接嵌 `cloudime_core::CandidateList`。同词干类型收进子目录：`key/{event,outcome}`、`frame/preedit/{kind,segment}`。
- **Server 进程**：`apps/windows/server`（package `cloudime-windows-server`，bin `cloudime-server`）。`dispatch::Router` 按 `SessionId` 分派多会话（Windows 一个 Server 服务多个应用进程，
  每会话各持组句状态）。会话开 / 关、按键与上屏、Engine 装配、命名管道传输（`\\.\pipe\cloudime`）都已跑通，端到端测过。
- **候选窗口（Server 进程自绘 + uiAccess）**：候选窗从前在**应用进程内的 DLL** 自绘，普通置顶窗被微软商店 / 任务栏搜索这些**更高 z-band** 的宿主盖住。现改由 **Server 进程**自绘（`server/src/ui/`：一条专用 UI 线程注册窗口类 + 建分层窗 + 跑消息循环，HWND 只在该线程碰；工人线程经 `Sender<UiCommand>` + `PostThreadMessageW(WM_APP)` 把「显示(`Frame`+屏幕矩形) / 隐藏」marshal 过去；进程级 `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` 按物理像素对齐应用报来的矩形）。DLL 只量光标屏幕矩形（`GetTextExt`，退鼠标）发 `PositionCandidates{rect}`，并在组句于 DLL 侧结束（应用终止组句 / 断线，`OnCompositionTerminated` 这条 Server 无从知晓）时发 `HideCandidates`；Server 握着 `Frame` 直接自绘，本地整句重排到达也直接刷自己的窗、不回传 DLL（渲染代码——候选 + 分页 + 柔和阴影——整块从 DLL 搬到 Server）。**盖过高 z-band 宿主**靠 Server exe 的 `uiAccess="true"` manifest（`server/build.rs` 用 embed-manifest 嵌）+ 代码签名 + 装 Program Files 三者齐备（`SetWindowPos(HWND_TOPMOST)` 才自动升进 UIAccess 高带）：开发自签 + 本机受信任根（`installer/sign-local.ps1`），发版换 Certum 开源代码签名证书；uiAccess exe 不能 CreateProcess 拉起（报 740），装完 / 登录都走 ShellExecute（安装器完成页 `ShellExecAsOriginalUser` + `{commonstartup}` 启动快捷方式由 Explorer 拉起才授 uiAccess，故不用计划任务）。候选窗每显示一页，Server 调 `Engine::note_displayed`（收窗传空）告知当前页。
- **悬浮状态条（Server 进程自绘，可拖动 / 记位置）**：桌面上常驻的小浮窗，显示当前中 / 英，与任务栏的中 / 英指示器（语言栏按钮）并存。跟候选窗**同一条 UI 线程**、复用同一套分层窗口贴图（`server/src/ui/layered/`）与云朵渲染器（字体 / 配色 / DPI / 深浅，候选窗与状态条共用一份）；自己一个窗口类与窗口过程（`server/src/ui/status/`）：一排图标按钮（顺序、显隐与图标来自 exe 旁 `data\icons-arrangement.cfg`，`ui/status/arrangement.rs` 解析；Caps Lock 亮灭不在 Server 手上，状态条自己每 250 ms 读一次 `GetKeyState`，变了重画）：按下鼠标先 `DragDetect`，挪出阈值就交给系统移动循环（`WM_NCLBUTTONDOWN` + `HTCAPTION`，结束时 `WM_EXITSIZEMOVE` 报新位置），没挪就是点击、按 x 落进哪格；`WM_MOUSEACTIVATE` 回 `MA_NOACTIVATE` 点它不抢应用焦点；窗口过程按 HWND 从 thread_local 表查到对象。点格 / 拖动结束经 `StatusEvent`（`dispatch/status/`）投回工人线程（工人循环收的是 `ipc::Work`：DLL 消息或状态条事件），Router 写回配置（`[status_bar] x/y`，热加载再读回；「中文标点 / 英文标点」「全角 / 半角」只改会话内状态、不落盘，重启回缺省，「简 / 繁」写回 `[input] simp_trad_chinese_chars_toggle`）；设置按钮由 UI 线程直接起设置程序，工具页弹 exe 旁 `tools\tools.list` 登记的工具菜单、特殊字符页起随包带的「特殊字符输入器」（`SpecialSymbolsInserter.exe`，与 Server 同目录）。中英模式全局一份、存在 Server（见上文「中英模式」）：DLL 里用户切了用 `ModeChanged` 报来，状态条上点「中 / 英」直接改 Server 那份，各 DLL 激活、得到焦点时和轮询定时器（没组句、本线程前台时每几拍）用 `SyncMode` 取走跟上；有 DLL 来取模式就说明云朵输入法是当前输入法，状态条据此显示。会话号用线程 id（`com::session_id`）——TSF 的 client id 各进程都是同样那几个值，拿它当会话号会撞。前台线程没连着 Server 时（登录后 Server 起得比第一个应用晚、Server 重启过）轮询那一拍顺带补连，连上报一次当前模式（重启过的 Server 不知道）。状态条**常驻桌面**，只跟「当前输入法是不是云朵输入法」走：第一次 `ModeChanged` 显示，DLL 挂 `ITfActiveLanguageProfileNotifySink`（`com/profile.rs`）在别的 TIP 被激活时用一条临时连接发 `ImeSwitched` 收起（此时自己已被停用、会话连接已关），应用退出（`CloseSession`）不收。开关与记住的位置：状态条常开（只跟「当前输入法是不是云朵输入法」走），位置在 `[status_bar]`（`x` / `y`），热加载即时生效；前台全屏时是否自动收起由 `[debugging] auto_hide_float_tool_bar` 定（缺省关，`ui/status/fullscreen.rs` 每秒查一次前台窗口是不是盖满所在显示器；关着就不查、全屏也显示，切输入法仍收）；uiAccess 高 z-band 与候选窗同进程天然继承。参考微软水杉的 FTB 形态（它用 D2D + DirectComposition 且不记位置），落地时选沿用本项目已有的分层窗那套以保持视觉语言一致、并加了位置持久化。
- **帧编解码**：长度前缀 JSON 帧的 `read_message` / `write_message` 与缺省管道名放在 `cloudime-platform::protocol`，Server 与 DLL 共用（DLL 不必依赖整个 Server 库）。
- **TSF DLL**：`apps/windows/tsf`（package `cloudime-windows-tsf`，`cdylib`，产物 `cloudime_tsf.dll`，依赖官方 `windows` crate 的 COM `implement` 宏）。「引擎层」不是 Engine 而是连 Server 的**管道客户端** `EngineClient`（平台无关、可端到端测）；
  连不上 Server 时 DLL 自己把它拉起来（`com/service/launch.rs`）：Server 只在登录时由「启动」文件夹的快捷方式拉起，中途挂了以前只能等下次登录、期间静默吞键。
  用 `ShellExecuteW`（`uiAccess=true` 的 exe 用 `CreateProcess` 报 740），与 DLL 同目录的 `cloudime-server.exe`；
  进程内 5 秒冷却 + 跨进程命名互斥体（`Local\CloudIMEServerLaunch`）保证多个应用同时发现 Server 不在时只起一个。
  COM 层：`DllGetClassObject` → `IClassFactory` → `#[implement(ITfTextInputProcessor, ITfKeyEventSink, ITfDisplayAttributeProvider)]` → `Activate` 挂击键 sink + 语言栏中英按钮 + 连管道 → `OnKeyDown` 转发按键、经异步编辑会话（`TF_ES_READWRITE`，不带 SYNC）写组句 / 上屏；`DllRegisterServer` 写 InprocServer32 并经 `ITfInputProcessorProfiles` / `ITfCategoryMgr` 注册文本服务与各能力类别。
  组句拼音的**内联下划线**（组句文字底下的细实线）走 TSF 显示属性协议（`com/display_attribute/`）：注册 `GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER` 类别 + 一个自定义显示属性 GUID（细实线、`TF_ATTR_INPUT`），
  `ITfDisplayAttributeProvider`（实现在 TextService 上）把 GUID 对应的 `TF_DISPLAYATTRIBUTE` 交给系统；收键写组句时用 `ITfCategoryMgr::RegisterGUID` 把 GUID 换成 atom，`SetValue` 进组句范围的 `GUID_PROP_ATTRIBUTE` 属性，宿主据此在拼音底下画线。
  收键与运行细节记进 `%LOCALAPPDATA%\CloudIME\logs\tsf.<日期>.log`（与 Server / 设置程序同目录，按天一个文件、留 7 天；多进程追加同一文件）；候选窗口不再由 DLL 自绘（已搬到 Server 进程，见上「候选窗口」），DLL 侧只做 preedit 内联 + 上报光标矩形。
- **交叉编译验证**：`cloudime-core` / `-dictionary` / `-format` / `-lm` / `-platform` / `apps/windows/{server,tsf}` 已能
  `cargo check --target x86_64-pc-windows-gnu` 通过（借此修掉 `cloudime-format` 里 unix 专有的 `Mmap::advise` 未 `cfg` 的移植 bug）；
  本机只 `check`，真正编译在 Windows 机器上做（`cloudime-neural` 的 candle 后端在 Windows 走 CPU，已接进 Server，见下「本地整句模型」）。
- **本地整句模型（Server 进程）**：`server/src/dispatch/rescore/`。启动时 `find_model`（用户目录 `%APPDATA%\CloudIME\local_models\` 优先，否则随包 `data\local_models\`；`.qjm` 单文件或三件套目录，目录里有多份时优先词表含汉字的字级模型）；`[candidate] use_local_sentence_organization_model` 开着就起线程加载并预热（`ModelLoader`），下一次按键 / tick 接上 `set_async_sentence_scorer`。
  Server 没有定时器：缓冲变化后 `schedule_rescoring` 起防抖，工人循环 `recv_timeout(router.next_tick())` 按 `RescoreState` 的节拍醒来（防抖 80 ms → `request_rescoring`；然后 20 ms 一次 `poll_rescoring`，最多等 2 s），DLL 组句期间每 80 ms 的 `Poll` 也顺带 `tick`。分到了重查一次、重建候选布局、由 Server 自绘的候选窗直接重画，DLL 下一次 `Poll` 拿到新帧更新内联 preedit；翻过页 / 动过高亮不动。热加载那一项变了才重载 / 卸载。
  前文：DLL 在**起组句的那次读写编辑会话**里顺手读选区起点前 64 个 UTF-16 单元（`com/edit/surrounding.rs::text_before_caret`，拼音还没插进去、不用再开一次会话），随 `ClientMessage::Surrounding` 单向送来。**密码框与私密输入**（2026-09-12 查了微软文档 / SampleIME / Chromium 源码后定）：
  TSF 规定键盘类 TIP 必须看上下文的 `GUID_COMPARTMENT_KEYBOARD_DISABLED`（微软文档明说密码框应禁用文本服务、`IS_PASSWORD` 只是标注不提供保护；Chromium 给密码框的上下文设的就是它），
  DLL 在 `OnTestKeyDown` / `OnKeyDown` / 保留键里没在组句时先查它（连同 `EMPTYCONTEXT`，`com/context.rs`），非零整键放行、不组句（密码框里根本不进输入法）；
  输入范围（`GUID_PROP_INPUTSCOPE`）只在起组句那次编辑会话里读一次（`com/edit/surrounding.rs::input_context`）：含 `IS_PRIVATE` / 密码 / PIN 之一算**私密**——Chromium 源码里密码框与不学习的输入框映射成 `IS_PRIVATE`（含义「别学」；2026-09-12 box 实测 Edge InPrivate 的网页文本框报的仍是 `IS_SEARCH`，`IS_PRIVATE` 只在密码框见过，这条是兜底）——私密时不读前文，并随 `ClientMessage::Privacy` 告诉 Server（客户端只在变了时发；记事本等不支持该属性的应用 `GetValue` 失败按不私密）。
  Server 按会话记 `private`、焦点切换时重设，Core `Engine::set_private`：学习器与输入日志外面各套一层 `Muted*`（写吞掉、读照常，排序不变），不学不记。
- **版本与发布**：`apps/windows/{server,tsf,settings}/Cargo.toml` 各写死自己的 `version`（三个一起改，打包读 `server`），
  发布标签见 `docs/notes/release.md`（一个 `v<版本>` 标签一个 GitHub Release）。`cloudime-windows-tsf` 是同一 Windows 产品的另一半（各自 `Cargo.toml` 记版本；`apps/windows/` 下的 server / tsf / settings 是同一个产品的三个产物，不合成一个 crate——DLL 不能带 Engine 的依赖树）。
