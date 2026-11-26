# 五笔码表来源与许可

本目录的 `table.tsv` 由两部分组成。

## 单字与旧短语

- 上游项目：<https://github.com/KyleBing/rime-wubi86-jidian>
- 上游文件：`wubi86_jidian.dict.yaml`（极点五笔主词库）
- 版权：Copyright (c) Kyle Bing
- 许可：MIT License，全文见同目录 [`LICENSE-wubi86-jidian.txt`](LICENSE-wubi86-jidian.txt)

## 新增短语（2–4 字）

本项目的 `table.tsv` 还包含从**万象拼音**的纯中文 2–4 字词条按五笔规则生成的短语：

- 上游项目：<https://github.com/amzxyz/rime-wanxiang-data>
- 版权 / 许可：AMZXYZ / 万象拼音，**CC BY 4.0**，全文见同目录 [`LICENSE-CC-BY-4.0.txt`](LICENSE-CC-BY-4.0.txt)
- 修改：按单字全码重新计算五笔短语编码（二字词取每字前两码；三字词前两字首码 + 第三字前两码；
  四字词前三字首码 + 末字首码），只取权重 ≥1350 的词条、每个编码最多新增 9 条，并保留全部旧短语。

## 本项目的改动

- 从 Rime 的 YAML 词典转换为 `编码<TAB>文字<TAB>权重` 的 TSV；
- 编码统一为小写 `a`–`z`、长度 1–4，去掉上游的 `stem` 列、导入表与文件头元数据；
- 同一「编码 + 文字」只保留权重最高的一条，权重沿用上游数值（新增短语沿用万象权重）。
- 单字与旧短语完整保留；新增短语只取万象 2–4 字纯中文词条（口语词如「看到了」也算）。

`scheme.toml`、`level1.tsv`、`roots.tsv`、`roots_pua.tsv` 是本项目按 86 版五笔规则整理的方案文件；
`zigen.ttf`（字根字体）使用更纱黑体子集，许可见同目录 [`OFL.txt`](OFL.txt)。

分发本目录时请一并保留 `LICENSE-wubi86-jidian.txt`、`LICENSE-CC-BY-4.0.txt` 与 `OFL.txt`。
