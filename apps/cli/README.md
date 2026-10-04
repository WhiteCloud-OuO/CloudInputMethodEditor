# cloudime-cli

云朵输入法 Core 的命令行壳：不依赖任何平台 API，用来查候选、看排序、量性能、回放评测。改 Core 的任何东西都先在这里验证，再去碰输入法。

```bash
cargo run --release -p cloudime-cli -- kaifa            # 查一个拼音
cargo run --release -p cloudime-cli -- kaifa nihao      # 查几个
cargo run --release -p cloudime-cli                     # 交互模式
```

性能相关的（`--typing`、`--replay`）一律用 `--release`，debug 构建慢十倍以上，数字没有意义。

## 数据文件

缺省按优先级找：`data/generated/` 里打包好的 `.qj` → 那里的 TSV → 仓库自带的产品数据（`assets/lexicon/`）→ `assets/sample/` 的样例。
语言模型 `data/generated/lm.qj`（或 `lm-unigram.tsv` / `lm-bigram.tsv`）有就加载，没有就退化成一元词频整句。

| 参数 | 作用 |
|---|---|
| `--dict <路径>` | 主词库；`.qj` / TSV / SQLite `.db` 都认。输入法的随包主词库是 `WordBank\Dict.db`（7 张表：中文普通组 / 稀有组 + 英文，按语言与稀有度分流） |
| `--extra-dict <路径>` | 附加词库，可给多个，与主词库一起查 |
| `--english <路径>` | 英文词表（中英混输的英文候选与补全）；随包英文已在 `WordBank\Dict.db` 里，单独给这份只给 CLI 用 |
| `--user-dict <路径>` | 用户词频文件（`user.tsv`，同目录的 `user-words.tsv` / `UserWordBank.db` / `user-choices.tsv` / `user-ngram.tsv` / `user-english.tsv` 一起读）；给了就在退出时写回，不给则只在本次会话内学习 |
| `--config <路径>` | 配置文件，缺省与输入法共用 `~/Library/Application Support/CloudIME/config.toml` |

## 行为开关

| 参数 | 作用 |
|---|---|
| `--fuzzy z-zh,n-l,…` / `--fuzzy all` | 模糊音，覆盖配置里的 `[input] mo_hu_yin_list` |
| `--english-mode` | 英文模式：字母不当拼音，候选来自英文词表。只给回放与评测还原旧日志用——产品里英文模式是纯直通、不给候选 |
| `--limit N` | 只显示前 N 个候选（缺省 9） |

## 输出

每个查询打印：切分（多种切分用 `\|` 分开，`…` 表示简拼 / 残缺音节）、纠错（`纠正: nihooma → ni hao ma`）、候选列表（序号、候选、`[句]` 是整句候选、右侧词性与译文）、各阶段耗时。

## 交互模式

不给拼音参数就进入交互模式：

| 输入 | 作用 |
|---|---|
| 拼音 | 查候选 |
| 序号 | 上屏上一次查询的第 N 个候选并记入学习 |
| `:raw` | 上一次输入原样上屏（相当于回车） |
| `:del N` | 删掉第 N 个候选：用户词整个删，词库词清掉对它的学习 |
| `:q` | 退出 |

## 性能：`--typing`

把每个输入当作一键一键敲进去，每个前缀都查一次，打印每键各阶段耗时与最慢 / 平均值。目标每键 10 ms 以内。

```bash
cargo run --release -p cloudime-cli -- --typing jintianwanshangwomenquchifan
```

## 回放评测：`--replay`

读输入法记的输入日志（`~/Library/Application Support/CloudIME/input-log.jsonl`，每次上屏一行），把每行当时的键重新喂给引擎，看现在的排序会不会把当时选的词放在首选：

```bash
cargo run --release -p cloudime-cli -- --replay ~/Library/Application\ Support/CloudIME/input-log.jsonl --misses 20
```

- 按来源（词 / 整句 / 英文 / 其他）算首选命中率、前五命中率、平均名次、不在候选里的条数；原样上屏不评分只计数。
- 只在内存里学习，不写任何文件；按日志顺序回放，命中的候选照样上屏让上文往前走。
- 加 `--user-dict …/user.tsv` 就带上现有的学习数据（回放分支不落盘）。不带是「冷引擎」，量的是词库与模型本身；带上量的是实际体验，两者之差是学习功能的贡献。
- `--misses N` 打印前 N 条没命中首选的例子（作用域、当时选的、现在的前三）。

排序、整句、纠错的任何改动，改前改后各跑一次比差值。

## 日志

终端里日志走 stderr，`RUST_LOG=debug` 看逐键细节；设置 `CLOUDIME_LOG_DIR` 时再按天写文件。
