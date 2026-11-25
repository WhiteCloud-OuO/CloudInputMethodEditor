# 五笔码表来源与许可

本目录的 `table.tsv` 使用极点五笔社区词库：

- 上游项目：<https://github.com/KyleBing/rime-wubi86-jidian>
- 上游文件：`wubi86_jidian.dict.yaml`（极点五笔主词库）
- 版权：Copyright (c) Kyle Bing
- 许可：MIT License，全文见同目录 [`LICENSE-wubi86-jidian.txt`](LICENSE-wubi86-jidian.txt)

## 本项目的改动

- 从 Rime 的 YAML 词典转换为 `编码<TAB>文字<TAB>权重` 的 TSV；
- 编码统一为小写 `a`–`z`、长度 1–4，去掉上游的 `stem` 列、导入表与文件头元数据；
- 同一「编码 + 文字」只保留权重最高的一条，权重沿用上游数值；
- 未包含上游的用户词库、扩展词库与行政区域词库。

`scheme.toml`、`level1.tsv`、`roots.tsv`、`roots_pua.tsv` 是本项目按 86 版五笔规则整理的方案文件；
`zigen.ttf`（字根字体）使用更纱黑体子集，许可见同目录 [`OFL.txt`](OFL.txt)。

分发本目录时请一并保留 `LICENSE-wubi86-jidian.txt` 与 `OFL.txt`。
