# 输入法词库

这个目录放**英文词库**、几份小规模的额外词表，以及中文词库的来源/许可说明；中文词库本身（77 MB 的万象 YAML）不随仓库。

## 中文词库

内置中文词库来自 **万象拼音**（[`amzxyz/rime-wanxiang-data`](https://github.com/amzxyz/rime-wanxiang-data)）的 14 个中文拼音词库，
按上游“无声调”规则转换后合并成 `data/generated/dict.tsv`，再打包成 `WordBank\Dict.db`：

```bash
cargo run --release -p cloudime-dict-convert -- wanxiang --source <rime-wanxiang-data 的 dicts 目录>
cargo run --release -p cloudime-dict-convert -- word-bank --chinese data/generated/dict.tsv --english data/generated/english.tsv \
  --name 云朵基础词库 --license "CC-BY-4.0 AND MIT AND Unicode-3.0 AND CC-BY-SA-4.0" \
  --attribution "中文词库 © AMZXYZ / 万象拼音（CC BY 4.0，已去除声调、合并词表、权重清洗）；英文词表 ESDB/SCOWL、CSpell、typos、wordfreq；读音 Unihan" \
  --source "https://github.com/amzxyz/rime-wanxiang-data" --data-version "wanxiang-2026-11-24"
```

来源、许可、修改说明见 [WANXIANG.md](WANXIANG.md)，许可证全文见 [LICENSE-CC-BY-4.0.txt](LICENSE-CC-BY-4.0.txt)。
英文词表仍走 `05_english/` 与 `english.tsv`，本次不替换。

## 现有内容

| 路径 | 内容 |
| --- | --- |
| `05_english/` | 英文词库源数据与说明（总表约 11.5 万词形；常用 / 扩展 / 专名缩写 / 词形变化 / 误拼对照 / 显示形式，`05_tech/` 技术词，`sources/` 来源与许可） |
| `english.tsv` | 打包用的英文词表（`词\t编码\t词频`，由 `05_english/` + 词频脚本生成） |
| `english-frequency.tsv` | 英文词频表 |
| `brand.tsv` | 品牌词（云朵输入法）：`lexicon --extra-words` 并入基础词库 |
| `mixed_words.tsv` | 中英混杂词（C盘 / B站 / U盘 / T恤）：`lexicon --extra-words` 并入，语言模型走 `bigram --brand` |

英文的字段、来源与许可见 [英文词库说明](05_english/README.md)。

## 旧中文词库（已停用）

原来的「规范汉字 / 现代汉语常用词 / THUOCL 领域词」链路已停用，源数据不再随仓库；
内置中文词库现在由上面的万象拼音替代。下面只留历史记录，`CLOUDIME.md` 里的旧命令不再使用。

- **源数据**：`01_characters/`（规范汉字）、`02_common/`（现代汉语常用词）、`03_domains/`（11 类领域词）、`04_internet_slang/`、`00_meta/`
- **中文成品**：`dict.tsv`（基础词库）、`dicts/*.tsv`（11 本领域词库）、`mined_words.tsv`（语料挖词）、`phrases.tsv`（短语层）、`domain_words.tsv`（日志挑出的领域词）
- **本地生成产物**（gitignore，随删除一并清掉）：`data/generated/dict.qj`、`data/generated/dicts/*.qj`、`data/generated/lm.qj`、仓库根 `WordBank/`

英文的 `05_english/`、`english.tsv`、`english-frequency.tsv` 与 `assets/sample/`（样例词库）都保留。
