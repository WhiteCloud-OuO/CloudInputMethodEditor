# 形码输入：五笔 86 的方案目录与码表（2026-11-23）

## 0. 前提

- 输入方案由 `[input] input_schemes` **单选**决定（全键拼音 / 双键拼音 / 形码输入），**永不允许混合输入**：
  挂着形码码表时按键只按形码解释，不再走拼音切分、整句转换，`v` / `u` / `i` 也不是表达式 / 区位 / 符号面板
  （要用那三种面板走 `Ctrl + Alt + V` / `U` / `I`，见 `docs/design/input-modes.md`）。
- **临时规则**：形码方案下本地模型不参与（全键 / 双键拼音才放行），见 `docs/user/input/local-model.md`；
  形码测试完成后撤掉这条判断。
- 形码数据与方案都是**本机开发数据**：仓库根 `InputSchemes/` 在 `.gitignore` 内，不进版本库；
  `InputSchemes/ShapeCode/wubi/table.tsv` 是样例，装机时由安装包或用户自己放。

## 1. 目录与文件

| 项 | 约定 |
|---|---|
| 方案目录 | **安装目录**下的 `InputSchemes\ShapeCode\`（`bundled_root()`；与双键拼音的 `InputSchemes\DoubleKeysPinyin\` 同级） |
| 一个方案 | 一个子目录：`wubi\`、`cangjie\`、`shengbi\`；**子目录名就是方案 id** |
| 方案文件 | 子目录里的 `scheme.toml`；没有 `scheme.toml` 的子目录不算方案（扫描时直接跳过） |
| 码表 | 方案文件 `[Shape] Table` 指的 TSV（缺省 `table.tsv`），相对方案目录 |
| 当前方案 | 方案目录下的 `use_input_scheme.cfg`，键值行 `scheme = wubi`（与双键拼音同一套文件格式与文件名） |
| 缺省方案 | `.cfg` 没写 / 指不到时取**目录名升序第一个读得出来的方案**，并把 `scheme` 行重建过去 |

```text
InputSchemes/ShapeCode/
  use_input_scheme.cfg      scheme = wubi
  wubi/
    scheme.toml
    table.tsv
  cangjie/                  以后的方案（现在空目录，不算方案）
  shengbi/
```

## 2. `scheme.toml`

```toml
[Info]
Name = "五笔86"
Version = "1.0"
Author = "极点五笔社区数据 / 云朵输入法"

[Shape]
Table = "table.tsv"
MaxCodeLength = 4
```

| 节 / 键 | 必填 | 类型 | 说明 |
|---|---|---|---|
| `[Info] Name` | 是 | 字符串 | 方案名（设置页 / 日志显示） |
| `[Info] Version` | 是 | 字符串 | 方案版本（只记录，暂无迁移逻辑） |
| `[Info] Author` | 否 | 字符串 | 作者 |
| `[Shape] Table` | 否 | 字符串 | 码表文件，相对方案目录；缺省 `table.tsv` |
| `[Shape] MaxCodeLength` | 否 | 整数 | 最长编码长度，1–8；缺省 4（五笔 4 键） |

- 节名、键名**大小写不敏感**（`[info]` / `name` 也认）。
- `Name` / `Version` 缺失、`MaxCodeLength` 不是 1–8 的整数、码表读不出来或一条能用的条目都没有 → 该方案读不出来
  （日志记原因；设置页以后用红字列出）。一个方案读不出来不影响别的方案，最坏情况退回全键拼音。

## 3. 码表 `table.tsv`

一行一条，三列，UTF-8（容忍 BOM，LF / CRLF 都行）：

```text
编码<TAB>文字<TAB>权重
wqvb	你好	20
wqvb	您好	10
ggll	一	20
```

- **编码**：小写 `a`–`z`，长度 1–`MaxCodeLength`；大写 / 超长 / 空编码的行跳过并记一条 `warn`。
- **文字**：单字或词组，空的行跳过。
- **权重**：非负整数，越大越靠前；不写按 `1`；不是整数跳过。
- `#` 开头的行与空行忽略。
- 单字与词组共用一张表：五笔的一级 / 二级 / 三级简码与全码就是同一批字的多个编码。

**当前样例**：`wubi/table.tsv`，89,250 行（单字 26,601 行、词组 62,649 行；不同编码 77,168 个；
编码长度 1 位 50 / 2 位 656 / 3 位 5,426 / 4 位 83,118；权重 5–890）。
来源 `KyleBing/rime-wubi86-jidian` 的 `wubi86_jidian.dict.yaml`（极点五笔主词库，数据许可已由维护者确认）：
`--- … ...` 头之后每行 `文字<TAB>编码<TAB>权重`，转成我们的「编码 / 文字 / 权重」并把同码同词去重。
`WindInput` 的 `wubi86_chaizi.txt`（单字拆字 + 完整 86 码，27,823 字）留作字根提示 / 罕用字来源，暂不并进主表；
`wubi86_jidian_extra.dict.yaml`（877 条 emoji / 网络词）留作可选第二词库。

## 4. 查询与排序

敲 1–4 键都拿**已经敲的整串**当前缀，命中码表里所有以它开头的编码（`Core` 的 `shape::Table::candidates`）：

1. 前缀范围靠二分定位（条目按编码升序、同码按权重降序存）；
2. 同一页里按**文字去重**：一个字 / 词在好几个编码下命中（`工` 有 `a` / `aaa` / `aaaa`）只留一条 ——
   优先权重高的，权重相同取**源文件里靠前的**（一级简码通常在前）；
3. 候选排序：**权重降序** → 编码短者优先 → 源文件顺序；
4. 最多回 `SHAPE_MAX_CANDIDATES = 500` 条（一键的前缀能命中几千条，够翻很多页了）。

没有任何候选（例如敲了 `z`，极点词库已去掉 `^z` 的编码）→ `ParseError::NoSegmentation`，
壳按「切不动的串」把预编辑原样画出来，退格照常。

## 5. 上屏与学习

- 形码候选是 `CandidateKind::Chinese`，`syllables` 只有一项：**候选命中的那条完整编码**
  （前缀命中时上屏按「音节是敲的键的前缀」对齐，整段敲的键都会被吃掉）。
- 选中、空格 / 数字上屏、退格撤回、已选文本拼成组句、输入日志与统计都复用拼音那条 Chinese 路径；
  输入日志里的 `keys` / `pinyin` 都是形码编码本身。
- 学习按通用规则记（选择次数、`码 → 文字` 的选择），但**形码的候选排序目前只按静态权重**，
  用户选择不影响下一次的顺序（见「还没做」）。

## 6. 与拼音方案的边界

`Engine::query_inner` 的顺序：英文模式 → 双键拼音 → **形码** → 表达式 / 英文直输 / 全拼。

- 形码与双键拼音不会同时挂（`[input] input_schemes` 只有一个值）；都挂着时双键拼音优先。
- `Engine::expression_mode` 见到形码码表返回 `false`；`v` 是普通编码键。
- 拼音行（预编辑）显示**敲的编码原文**：Shift / Caps Lock 敲的大写按原样显示，匹配按小写算。
- 特殊输入模式（`/` 逐字选）只给全 / 双拼用；`RouterConfig::special_input` 对形码是 `false`。
- 简拼、模糊音、整句转换、中英混输、英文前后缀补全、辅助码与键位映射参考都不参与形码查询。

## 7. 还没做

- **`z` 通配**：极点词库本身排除了 `^z` 的编码，现在敲 `z` 没有候选；以后要按 `z` = 任意一位做码表匹配。
- **字根提示**：`wubi86_chaizi.txt` 的拆字列可以做成候选右侧的字根提示（需要 `HeiTiZiGen.ttf`）。
- **扩展 / 用户词库**：`wubi86_jidian_extra.dict.yaml` 与用户自造码表；现在只有方案目录里的主码表。
- **设置页面板**：形码输入那块还是占位；换方案现在只能改 `use_input_scheme.cfg` 的 `scheme`。
- **仓颉 / 声笔**：目录已留好，码表与方案文件还没做。
- **学习排序**：形码候选没接 `Learner` 的用户权重。
