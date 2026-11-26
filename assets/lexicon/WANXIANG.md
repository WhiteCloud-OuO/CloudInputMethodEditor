# 万象拼音词库

云朵输入法的内置中文词库来自万象拼音：

- 上游：<https://github.com/amzxyz/rime-wanxiang-data>
- 使用文件：`dicts/` 下 14 个中文拼音词库
  - 核心：`zi` / `jichu` / `cuoyin` / `duoyin` / `diming` / `mingren` / `yiren`
  - 分类：`yixue` / `huaxue` / `yaopin` / `wuzhong` / `taifeng` / `fangyan` / `shici`
- 不使用的文件：`en`（英文）、`mixed`（中英混合字面码）、`abbrev` / `t9_abbrev`（简码 / T9 码）。
  这三个的“编码”不是拼音，不能进中文词库；英文词表继续用 `05_english/`。
- 快照：2026-11-24 下载的 `dicts/`（所有文件头部 `version: "LTS"`）。上游分支在 2026-11-25 还有一次小更新，
  正式发版前应重新拉取并把 commit / 文件 SHA256 记进发布记录。

## 许可

万象的**原创词库与拼音数据**按 **CC BY 4.0** 发布，许可证全文见 [LICENSE-CC-BY-4.0.txt](LICENSE-CC-BY-4.0.txt)；
万象仓库里的程序代码不随数据授权，云朵只使用数据，没有复制它的代码。

按 CC BY 4.0 的要求：

- 署名：`AMZXYZ / 万象拼音`，来源链接 <https://github.com/amzxyz/rime-wanxiang-data>；
- 注明修改：去除声调、统一 `ü → v`、合并 14 个词库、权重清洗与去重；
- 不暗示万象为云朵输入法背书。

这些信息同时写进 `WordBank\Dict.db` 的 `META`（`license` / `attribution` / `source`）。

## 转换规则

`cargo run --release -p cloudime-dict-convert -- wanxiang --source <dicts 目录>`：

- 去调：`āáǎà` 等 → 无调元音，`ǖǘǚǜü` → `v`；
- **`j/q/x/y` 后的 `u` 实际是 ü，统一写成 `v`**（`jū → jv`、`jué → jve`、`juān → jvan`、`jūn → jvn`，`q/x/y` 同理）；
  解析器同时接受标准写法 `ju` 与 `v` 写法 `jv`，查询都归一成 `jv`；`you` / `ya` 这类不是 ü 的音节不受影响。
- 叹词：`ń/ň/ǹ → en`，`ńg/ňg/ǹg → ng`，`ḿ/m̀ → me`；
- 生僻音节：`tei` / `fiao` / `biang` / `cei` / `ng` / `hng` / `hm` 已加入解析器；
- 权重取前导数字（`76N → 76`），缺省 1，0 抬到 1；
- 同一 `(文字, 拼音)` 取最大权重合并；`cuoyin` 的第 4 列提示文字不写入词库；
- 补一条 `哼 hng`（上游只有 `heng`），权重沿用 `哼 heng`。

## 重建

```bash
cargo run --release -p cloudime-dict-convert -- wanxiang --source <dicts 目录> --out data/generated/dict.tsv
cargo run --release -p cloudime-dict-convert -- word-bank \
  --chinese data/generated/dict.tsv --english data/generated/english.tsv \
  --name 云朵基础词库 \
  --license "CC-BY-4.0 AND MIT AND Unicode-3.0 AND CC-BY-SA-4.0" \
  --attribution "中文词库 © AMZXYZ / 万象拼音（CC BY 4.0，已去除声调、合并词表、权重清洗）；英文词表 ESDB/SCOWL、CSpell、typos、wordfreq；读音 Unihan" \
  --source "https://github.com/amzxyz/rime-wanxiang-data" \
  --data-version "wanxiang-2026-11-24"
```

`word-bank` 默认还会并入 `assets/lexicon/brand.tsv` 与 `mixed_words.tsv`，英文仍来自 `data/generated/english.tsv`。
