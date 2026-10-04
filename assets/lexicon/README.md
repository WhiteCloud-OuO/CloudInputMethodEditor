# 输入法词库

这个目录现在只放**英文词库**和几份小规模的额外词表；原来的**中文**词库（规范汉字、常用词、领域词及其生成产物）已整体移除，
等新的中文词库文件到位后再补回来（生成链路见 [CLOUDIME.md](CLOUDIME.md)）。

## 现有内容

| 路径 | 内容 |
| --- | --- |
| `05_english/` | 英文词库源数据与说明（总表约 11.5 万词形；常用 / 扩展 / 专名缩写 / 词形变化 / 误拼对照 / 显示形式，`05_tech/` 技术词，`sources/` 来源与许可） |
| `english.tsv` | 打包用的英文词表（`词\t编码\t词频`，由 `05_english/` + 词频脚本生成） |
| `english-frequency.tsv` | 英文词频表 |
| `brand.tsv` | 品牌词（云朵输入法）：`lexicon --extra-words` 并入基础词库 |
| `mixed_words.tsv` | 中英混杂词（C盘 / B站 / U盘 / T恤）：`lexicon --extra-words` 并入，语言模型走 `bigram --brand` |

英文的字段、来源与许可见 [英文词库说明](05_english/README.md)。

## 已移除（等新中文词库替换）

- **源数据**：`01_characters/`（规范汉字）、`02_common/`（现代汉语常用词）、`03_domains/`（11 类领域词）、`04_internet_slang/`、`00_meta/`
- **中文成品**：`dict.tsv`（基础词库）、`dicts/*.tsv`（11 本领域词库）、`mined_words.tsv`（语料挖词）、`phrases.tsv`（短语层）、`domain_words.tsv`（日志挑出的领域词）
- **本地生成产物**（gitignore，随删除一并清掉）：`data/generated/dict.qj`、`data/generated/dicts/*.qj`、`data/generated/lm.qj`、仓库根 `WordBank/`

英文的 `05_english/`、`english.tsv`、`english-frequency.tsv` 与 `assets/sample/`（样例词库）都保留。
