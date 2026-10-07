use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "cloudime-dict-convert",
    about = "把第三方词库 / 词典转换成云朵输入法的 TSV，或把 TSV 打包成 .qj"
)]
pub struct Args {
    /// 输出目录
    #[arg(long, default_value = "WordBank")]
    pub out_dir: PathBuf,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// 云朵基础词库：从「输入法字词库_分类整理版」数据包 + Unihan 读音建 dict.tsv（两遍跑，见模块文档）
    Lexicon {
        /// 数据包目录（含 01_characters / 02_common / 03_domains），随仓库放在 assets/lexicon
        #[arg(long, default_value = "assets/lexicon")]
        pack: PathBuf,

        /// Unihan_Readings.txt
        #[arg(long, default_value = "data/unihan/Unihan_Readings.txt")]
        unihan: PathBuf,

        /// 多音字词读音标注（JSONL，一行一个词）
        #[arg(long)]
        pinyin: Option<PathBuf>,

        /// 语料词频（lm-unigram.tsv）；没有就按排序号 / 文档频次给底值
        #[arg(long)]
        frequency: Option<PathBuf>,

        /// 把仍靠猜读音的多音字词写到这个文件（一行一个），交给外部标注工具
        #[arg(long)]
        emit_ambiguous: Option<PathBuf>,

        /// 额外并入的词（`词\t次数`，`mine` 挖出来的 oov-candidates.tsv，可给多个）：词库里没有的按次数当词频加进去，读音同领域词
        #[arg(long)]
        extra_words: Vec<PathBuf>,

        /// 领域词在语料里出现不少于这个次数就留在基础词库，否则拆到 dicts/<领域>.qj
        #[arg(long, default_value_t = 50)]
        domain_keep_min: u64,
    },

    /// 英文词表（每行 `词\t编码[\t…]`，带不带表头都行，比如 `assets/lexicon/05_english/00_all_words.tsv`）→ english.tsv
    English {
        /// 输入文件
        #[arg(required = true)]
        inputs: Vec<PathBuf>,

        /// 词频表（`编码\t词频`，`tools/corpus/english_frequency.py` 生成）；给了就写进第三列，前缀补全按它排
        #[arg(long)]
        frequency: Option<PathBuf>,
    },

    /// 纯文本语料（每行一段）→ lm-unigram.tsv + lm-bigram.tsv：按词库分词后统计词级一元 / 二元计数
    Bigram {
        /// 语料文件（UTF-8 纯文本，简体）
        #[arg(required = true)]
        corpus: Vec<PathBuf>,

        /// 分词用的词库（云朵 TSV）；同目录 dicts/ 下的领域词库会一并用于分词（词表与拆分前一致）
        #[arg(long, default_value = "data/generated/dict.tsv")]
        dict: PathBuf,

        /// 短语层文件（assets/lexicon/phrases.tsv，可给多个，人工挑的领域词 domain_words.tsv 也走这条路）：里面的词不参与分词，统计完按成分合成它们的一元 / 二元计数（见 bigram.rs 模块注释）
        #[arg(long)]
        phrases: Vec<PathBuf>,

        /// 品牌词文件（assets/lexicon/brand.tsv，可给多个，中英混杂词 mixed_words.tsv 也走这条路）：语料里没有的词按文件给的次数写进一元表。
        /// 与 --phrases 的区别：合成计数要成分词在语料里，C盘 的 C 不是语料 token，只能直接给
        #[arg(long)]
        brand: Vec<PathBuf>,

        /// 计数低于此值的二元组不输出
        #[arg(long, default_value_t = 3)]
        min_count: u32,

        /// 最多输出多少条二元组（按计数取前 N）
        #[arg(long, default_value_t = 3_000_000)]
        max_bigrams: usize,
    },

    /// 从语料里挖词库没收的词：分词时被拆成连续单字的段按子串计数，出现够多的写到 oov-candidates.tsv（标注读音后交给 `lexicon --extra-words` 并入）
    Mine {
        /// 语料文件（UTF-8 纯文本，简体）；给了 --candidates 就不用扫语料
        #[arg(required_unless_present = "candidates")]
        corpus: Vec<PathBuf>,

        /// 语言模型一元表（`词\t次数`），算相邻字对 PMI 用
        #[arg(long, default_value = "data/generated/lm-unigram.tsv")]
        frequency: PathBuf,

        /// 相邻字对 PMI 的下限；0 不过滤
        #[arg(long, default_value_t = 3.0)]
        min_pmi: f64,

        /// 跳过扫语料，直接过滤上一次写出的 oov-candidates.tsv（调阈值用）
        #[arg(long)]
        candidates: Option<PathBuf>,

        /// 分词用的词库（云朵 TSV）
        #[arg(long, default_value = "assets/lexicon/dict.tsv")]
        dict: PathBuf,

        /// 出现次数低于此值的不要
        #[arg(long, default_value_t = 200)]
        min_count: u32,

        /// 最多几个字
        #[arg(long, default_value_t = 4)]
        max_chars: usize,
    },

    /// 短语层：从 bigram 表的相邻两词与语料的相邻三词里挖 我的 / 不知道 这类人会整块打的组合 → phrases.tsv（人工过一遍后拷进 assets/lexicon/，`lexicon --extra-words` 并入）
    Phrases {
        /// 语料文件
        #[arg(required = true)]
        corpus: Vec<PathBuf>,

        /// 对话语料（corpus 里的一个）：短语在它里面的次数也要够 min_count，维基模板句进不来
        #[arg(long, default_value = "data/corpus/lccc.txt")]
        dialogue: PathBuf,

        /// 分词与成分读音用的词库（云朵 TSV，同目录 dicts/ 一并读）
        #[arg(long, default_value = "data/generated/dict.tsv")]
        dict: PathBuf,

        /// 上一次的 phrases.tsv：词库已并入短语时重跑要给，里面的词先从分词词表里摘掉（否则 我的 是一个词，挖不出 我 + 的）
        #[arg(long)]
        refresh: Option<PathBuf>,

        /// 次数下限
        #[arg(long, default_value_t = 2000)]
        min_count: u32,

        /// 最多几个字
        #[arg(long, default_value_t = 4)]
        max_chars: usize,
    },

    /// 把 TSV 打包成词库存档 / `.qj` 容器：`dict` 读一个 TSV 写单份中文 `.db`，`lm` 读 lm-unigram/bigram.tsv 写 lm.qj；    /// `model` 把训练仓库导出的三件套目录（缺省 data/local_models）打成一个 model.qjm（`--out-dir data/local_models` 就写回原目录，随包带该目录下的全部 `*.qjm`）
    Pack {
        /// 打包哪种数据
        kind: DataKind,

        /// 输入文件；`dict` 一个 TSV，`lm` 两个（一元表、二元表），`model` 一个目录。缺省从输出目录里找同名 TSV（`model` 缺省 data/local_models）
        #[arg(long, num_args = 1..)]
        input: Vec<PathBuf>,

        /// 元数据：名称（必填）
        #[arg(long, default_value = "")]
        name: String,

        /// 元数据：许可证（SPDX 标识，如 GPL-3.0-only、CC-BY-SA-4.0）
        #[arg(long, default_value = "")]
        license: String,

        /// 元数据：署名 / 版权行
        #[arg(long, default_value = "")]
        attribution: String,

        /// 元数据：来源 URL
        #[arg(long, default_value = "")]
        source: String,

        /// 元数据：数据版本（上游版本号或日期）
        #[arg(long, default_value = "")]
        data_version: String,
    },

    /// 合并中文 / 英文 / 品牌 / 中英混杂词源，写出 7 张表的 `WordBank\Dict.db`（引擎按语言与稀有度分流加载）；
    /// 同一 `(词, 拼音, 语言)` 重复的先去重（权重大的胜出）再按判据归表
    WordBank {
        /// 中文词库源（`词\t拼音\t词频`，TSV / `.db` / `.qj` 都认），可给多个；缺省 `data/generated/dict.tsv`
        #[arg(long, value_name = "PATH", num_args = 1..)]
        chinese: Vec<PathBuf>,

        /// 英文词表源（`词\t编码\t词频`），可给多个；缺省 `data/generated/english.tsv`，没有再用 `assets/lexicon/english.tsv`
        #[arg(long, value_name = "TSV", num_args = 1..)]
        english: Vec<PathBuf>,

        /// 品牌词 / 中英混杂词（`词\t次数\t拼音`），可给多个；缺省带上存在的 `assets/lexicon/brand.tsv` 与 `mixed_words.tsv`
        #[arg(long, value_name = "TSV", num_args = 1..)]
        extra: Vec<PathBuf>,

        /// 元数据：名称（必填）
        #[arg(long, default_value = "")]
        name: String,

        /// 元数据：许可证（SPDX 标识）
        #[arg(long, default_value = "")]
        license: String,

        /// 元数据：署名 / 版权行
        #[arg(long, default_value = "")]
        attribution: String,

        /// 元数据：来源 URL
        #[arg(long, default_value = "")]
        source: String,

        /// 元数据：数据版本（上游版本号或日期）
        #[arg(long, default_value = "")]
        data_version: String,
    },

    /// 写出一份空的自定义短语库（`Phrases\Phrase.db`，含 `user` 与 `cloudime_default` 两张空表），
    /// 只用来第一次生成随包的那份。文件已存在时拒绝覆盖：随包那份的 `cloudime_default` 表是手写进去的
    /// 产品数据（内置短语），整份换成空库会把它们抹掉；真要重来加 `--force`
    PhraseDb {
        /// 输出路径
        #[arg(default_value = "Phrases/Phrase.db")]
        path: PathBuf,

        /// 已存在也整份替换成空库（会丢掉里面的内置短语）
        #[arg(long)]
        force: bool,
    },

    /// 本地词典（翻译 Tip 用）：把青简那套释义表 `.qj` 转成 `.db`（总表 `words` + 副表 `contents`）。
    /// 中文词与译词照搬，拼音 / 首字母从词库查（查不到留空）。输出文件名沿用输入的主名，扩展名换成 `.db`
    GlossaryDb {
        /// 输入的释义表 `.qj`（可给多个，各写一份 `.db`）
        #[arg(required = true, num_args = 1..)]
        input: Vec<PathBuf>,

        /// 查拼音 / 首字母的词库（云朵 TSV / `.db` / `.qj` 都认；装机目录的 `WordBank\Dict.db` 就是 `.db`）
        #[arg(long)]
        word_bank: Option<PathBuf>,

        /// 输出到哪儿：给目录就放进去，给路径就当完整文件名；缺省与输入同目录
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// 把改名前的 `.qj`（魔数 `QINGJIAN`）就地改成当前魔数 `CLOUDIME`：容器布局一字未动，只改头 8 字节。
    /// 改之前按容器完整校验一遍、改完再开一遍，坏文件原样报错；已是新魔数的跳过。
    /// `data-v1` / `data-v2` 这类旧数据要重发新号时先跑（`data-bundle.sh` 拒绝旧魔数，见 docs/notes/release.md）
    Rehead {
        /// 改哪一种数据，决定按哪个种类编号校验
        kind: DataKind,

        /// 要改写的 `.qj` 文件（可给多个）
        #[arg(required = true, num_args = 1..)]
        input: Vec<PathBuf>,
    },
}

/// 产品数据的种类：`pack` 打包与 `rehead` 改魔数都按它走。
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DataKind {
    /// 拼音词库
    Dict,

    /// 词级 bigram 语言模型
    Lm,

    /// 本地整句模型（三件套目录 → model.qjm）
    Model,
}

impl DataKind {
    /// 子命令里写的名字（报错文案用）。
    pub fn name(self) -> &'static str {
        match self {
            Self::Dict => "dict",
            Self::Lm => "lm",
            Self::Model => "model",
        }
    }

    /// 按容器打开 `.qj` 时校验的数据种类编号。
    pub fn container_kind(self) -> cloudime_format::Kind {
        match self {
            Self::Dict => cloudime_format::Kind::Dictionary,
            Self::Lm => cloudime_format::Kind::LanguageModel,
            Self::Model => cloudime_format::Kind::Model,
        }
    }
}
