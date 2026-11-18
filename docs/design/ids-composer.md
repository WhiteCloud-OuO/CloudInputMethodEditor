# 组字工具（IDS 组字）：设计

> 状态：设计已确认（2026-11-17）。执行步骤见 [`docs/plan/ids-composer.md`](../plan/ids-composer.md)。
>
> 范围：第一版（IDS 精确 / 通配检字 + 互动画布）、第二版（「难输入部件」抽屉）。
> 两版一起定型，实现分阶段落地。

## 1. 目标与非目标

### 1.1 目标

- 用户输入汉字结构（`⿰木木`、`LR木木`）或直接在画布上摆部件，查出对应的 Unicode 字；
- 画布按字统网「互动编辑」的样子做：结构是**可见的嵌套槽位**，不是文本框加一排结构按钮；
- 第二版补「难输入部件」：按笔画 / 位置分类点选那些打不出来的部件；
- 数据全部离线，`ids.qj` 用现有 `.qj` 容器 mmap 打开，启动不解析源文件；
- 独立小工具 `cloudime-ids.exe`，从悬浮状态条「工具 → 组字工具」启动，不进 Server、不改 TSF 协议。

### 1.2 非目标

- 未编码字形、Kage / GlyphWiki 动态造字；
- 联网调用 zi.tools 或任何在线服务；
- 常见部件替换（水→氵、手→扌）——严格按 IDS 匹配；
- 形近字 / 字理匹配；
- 向目标应用注入字符：第一版结果只复制到剪贴板。

## 2. 数据源

### 2.1 BabelStone `IDS.TXT`

| 项 | 值 |
|---|---|
| 地址 | `https://babelstone.co.uk/CJK/IDS.TXT` |
| 版本 | Unicode 16.0（文件日期 2025-06-27） |
| 条数 | 97,680 个字，108,425 条 IDS |
| 大小 | 3,285,669 字节，UTF-8 BOM + CRLF |
| SHA256 | `CC2A0A97E6A51ED6EBE59870EF2AF66E83F3C3C27387D79B4441BB2D791D78B1` |
| 许可 | 文件头明确声明：个人 / 商业用途自由使用，无需许可与署名（IDS 属事实数据） |

实测统计：108,184 个唯一 IDS，10,562 个字有多条来源 IDS，210 个 IDS 对应多个字，2,537 条含 `{n}`，118 条含 `？`。
生成时 `〾` 去掉后会合并少数键，实际输出 108,175 个键、108,403 条记录（97,680 个字不变）。

注意：文件头自相矛盾——顶部写 Unicode 16.0 / 97,680，第 3 节正文仍写 15.1 / 97,058。以实测 97,680 行为准，记录在 `docs/notes/crate-notes.md`。

### 2.2 Unihan `kTotalStrokes`（仅生成期可选）

`data/unihan/Unihan_IRGSources.txt`（`https://www.unicode.org/Public/UCD/latest/ucd/Unihan.zip`），Unicode License。

只用于 `--derive` 候选与新部件的笔画补全；**已收录的部件在 `components.tsv` 里自带笔画**，所以生成 `ids.qj` 不强制需要 Unihan，全新 clone 也能出完整数据。

### 2.3 部件清单 `assets/ids/components.tsv`

```text
# 字符	笔画	分类	启用
𠂇	2		1
龰			1
```

- 第一列必须是单个 `char`；
- 笔画 `1..13`（`13` = 13+），留空则按 Unihan → IDS 递归 → 0（未分类）；
- 分类 `0..4`（围 / 头 / 旁 / 底 / 其它），留空则自动计算；
- 初始 554 项按字统网「难输入部件」的笔画分组整理；其中 186 个有 SVG 字形，
  收进 `component-glyphs.tsv`，其余用系统字体或码位；分类全部交给自动计算。
- `assets/ids/component-glyphs.tsv`：186 个部件的 SVG path 段（`|` 分隔），来自字统网难输入部件
  接口的 `font` 字段；生成时写进 `ids.qj` 的 `CTEXT`，抽屉与画布缺字形时优先使用。
- `--derive` 从 `IDS.TXT` 统计候选（出现字数高、但词库打不出），人工挑选后并入本文件。

## 3. `ids.qj` 格式

### 3.1 容器与数据种类

复用 `crates/cloudime-format`：`Container::open(path, Kind::Ids)` 一次 mmap，`Text` / `Table<T>` 零拷贝。

```rust
pub enum Kind {
    // …既有 1 / 2 / 3 / 5 / 6 / 7
    Ids = 8,
}
```

不复用空号 4，避免和旧文件冲突。容器格式版本不升：只追加可选分节。

### 3.2 分节

```text
ids.qj
├─ META   TOML：来源 / 版本 / 条数
├─ TEXT   UTF-8：规范化 IDS 首尾相接
├─ KEYS   Table<IdsKey>：按 IDS 字节序排序
├─ HASH   Table<u32>：开放寻址，槽里是 KEYS 下标
├─ ENTR   Table<IdsEntry>：IDS → 字
└─ COMP   Table<Component>：难输入部件（可选）
```

`COMP` 缺失时：第一版功能照常，部件抽屉置灰提示「部件库缺失」。

### 3.3 结构

```rust
#[repr(C)]
pub struct IdsKey {
    pub start: u32,        // TEXT 字节偏移
    pub len: u16,          // IDS 字节长度
    pub first_entry: u32,  // ENTR 起始下标
    pub entry_count: u16,
    pub reserved: u16,
}

#[repr(C)]
pub struct IdsEntry {
    pub codepoint: u32,
    pub sources: u16,      // G/H/M/T/J/K/P/V/U/S/B/X/Z 位图；bit13 = UCS2003
    pub flags: u16,        // bit0 〾 / bit1 {n} / bit2 ？
}

#[repr(C)]
pub struct Component {
    pub codepoint: u32,
    pub stroke: u8,        // 1..=13；13 = 13+；0 = 未知
    pub primary: u8,       // 主分类 0..=4
    pub flags: u16,        // bit0 可拼音打出 / bit1 自动候选未确认 / bit2 笔画推算
    pub roles: [u16; 5],   // 围/头/旁/底/其它，总和 1000（千分比）
    pub svg_off: u32,      // CTEXT 偏移；u32::MAX = 没有
    pub svg_len: u16,
    pub reserved2: u16,
}
```

- `IdsKey` 16 字节、`IdsEntry` 8 字节、`Component` 20 字节，都是 4 字节对齐的定长结构；
- `COMP` 按码位严格递增，运行时二分；分类页对全部部件现算排序；
- 同一 IDS 对应多个字时 `entry_count > 1`，字按码位排序；
- 同一字多条来源 IDS → 出现在多个 key 下；同一 key 下同字合并 `sources` / `flags`。

### 3.4 兼容

- 读侧把 `COMP` 当可选，旧 `ids.qj` 不报错；
- `Kind::Ids` 只影响 `cloudime-ids`，其他数据种类不感知；
- 文件只整体替换（`Writer` 写临时文件再改名）。

### 3.5 META

```toml
name = "BabelStone IDS (Unicode 16.0)"
license = "Public Domain"
attribution = "Andrew West (BabelStone); based on data from Kawabata Taichi, John Knightley and IRG submissions"
source = "https://babelstone.co.uk/CJK/IDS.TXT"
version = "2025-06-27"
entries = 97680
generator = "ids-build <version>"
```

部件数量不写进 `Metadata`（不改公共结构），由 `COMP` 表长度得出。

## 4. 解析与规范化

### 4.1 运算符与元数

| 元数 | 运算符 |
|---|---|
| 2 | `⿰ ⿱ ⿴ ⿵ ⿶ ⿷ ⿸ ⿹ ⿺ ⿼ ⿽ ⿻ ㇯` |
| 3 | `⿲ ⿳` |
| 1 | `⿾ ⿿` |

`〾` 是变体前缀标记，解析时去掉并记 `flags`。

### 4.2 缩写

| 缩写 | 运算符 | 缩写 | 运算符 |
|---|---|---|---|
| `LR` | ⿰ | `LL` | ⿲ |
| `UD` | ⿱ | `UU` | ⿳ |
| `OC` | ⿴ | `OD` | ⿵ |
| `OU` | ⿶ | `OR` | ⿷ |
| `RD` | ⿸ | `RU` | ⿺ |
| `LD` | ⿹ | `LU` | ⿽ |
| `OL` | ⿼ | `XX` | ⿻ |
| `MI` | ⿾ | `RO` | ⿿ |
| `SU` | ㇯ | | |

缩写只替换运算符位置（串首与运算符之后），不碰部件本身。

### 4.3 规范化

查询与建索引用同一套：

1. 去首尾空白；
2. 缩写展开；
3. `？` / `?` / `⬚`（U+2B1A）统一成 `？`；
4. 去掉 `〾`（可以出现在任意子树前，数据里有 `⿸{13}〾⿻一㇄`）；
5. `{n}` 解析成未编码叶子（查询输入含 `{n}` 直接报「含未编码部件」）。

不做部件等价替换。

## 5. 查询

### 5.1 精确

```text
normalize(query) → hash::find(HASH, key, |id| key_str(id))
                 → IdsKey → ENTR[first..first+count] → Vec<IdsMatch>
```

命中 0 条 → `Unrecorded`；同一字合并显示。

### 5.2 通配

查询树里含 `？` 时：

1. 解析查询树；
2. 遍历 `KEYS`，取 key 字符串解析成树（`？` 叶子匹配任意子树，运算符与元数必须相同）；
3. 命中后取 entries，按字去重；
4. 最多返回 200 条，同时返回总数，UI 提示「缩小部件或增加结构」。

一次全扫 108k 条在 release 下预期 < 100 ms，放在「查询」按钮 / 回车，不按每键实时查。若实测超标，再在 `ids.qj` 加一节粗指纹子集，不改查询语义。

### 5.3 结果

```text
林   U+6797   ⿰木木   G/T/J/K/P/V
```

- 按码位升序（BMP 在前）；
- 同一字多来源只显示一次；
- 系统字体缺字形时仍显示条目（可复制）。

## 6. 部件目录生成

### 6.1 位置角色表

对 IDS 树里每个叶子，按父运算符 + 子下标判定位置：

| 运算符 | 子 0 | 子 1 | 子 2 |
|---|---|---|---|
| `⿰` `⿲` | 旁 | 旁 | 旁 |
| `⿱` | 头 | 底 | — |
| `⿳` | 头 | 其它（中段） | 底 |
| `⿴` `⿵` `⿶` `⿷` `⿸` `⿹` `⿺` `⿼` `⿽` | 围 | 內 → 其它 | — |
| `⿻` | 其它 | 其它 | — |
| `㇯` | 继承父角色 | 其它（被减部分） | — |
| `⿾` `⿿` | 继承父角色 | — | — |
| 根 | 其它 | | |

原则：

- 「內」不计入围，且权重打 5 折：`包 = ⿹勹巳` 里 `勹` 是围、`巳` 归其它；
- `⿸ / ⿹ / ⿺ / ⿽` 只有子 0 是围；子 1 按其它计（数据无法可靠区分「外框第二笔」与「被围内容」，宁可漏，不要错）。

### 6.2 权重与归一化

```text
w(叶子) = 1 / (1 + 深度) * (內 ? 0.5 : 1.0)

for 每个字 C 的每条 IDS 树:
    for 每个叶子 (c, r):
        sample[c][r] = max(sample[c][r], w)

roles[c][r] = round(1000 * sample[c][r] / sum(sample[c][*]))
```

- 按字去重：同一次统计里 `(C, c, r)` 只取最高权重，避免 `森` / `⿲木木木` 三倍计；
- `？`、`{n}` 叶子跳过；`〾` 只是标记，解析时已经去掉；
- 一个字多条来源 IDS 都参与，但同 `(C, c, r)` 仍只算一次。

### 6.3 主分类

```text
primary = argmax(roles)
if roles[primary] < 300 → primary = 其它
平票顺序：旁 > 头 > 底 > 围 > 其它
```

`components.tsv` 填了分类则覆盖。分类页按 `roles[r] >= 100` 显示，所以一个部件可以同时出现在多个分类下（`木`：旁 + 底）。

实测 554 个部件：`旁 324 / 头 105 / 底 53 / 其它 39 / 围 33`，没有低于 300‰ 的低置信项；
典型值：`氵` 旁 999‰、`艹` 头 997‰、`囗` 围 982‰、`辶` 围 998‰、`灬` 底 946‰。

### 6.4 笔画

优先级：

1. `components.tsv` 的笔画；
2. Unihan `kTotalStrokes`（多区域值取第一个，差异写报告）；
3. IDS 递归推算（CJK Strokes 叶子算 1 画），标记 `笔画推算`；
4. 未知 → `stroke = 0`，UI 放进「未分类」。

### 6.5 可打出判定与派生候选

```text
可打出 = data/generated/dict.tsv 里存在该字的单字词条
难输入 = 出现在 IDS 里但不是单字词条 / 单字频次极低
```

`ids-build --derive --dict data/generated/dict.tsv` 输出 `components-candidates.tsv`（字符、出现字数、笔画、五类千分比、可打出），人工挑选后并入 `components.tsv`，`flags.bit1` 标记未确认。

### 6.6 报告与校验

`--report` 每次输出 `components-report.tsv`：笔画来源、五类千分比、主分类、低置信行、覆盖冲突、所有 `stroke = 0`。

生成中止条件：TSV 行不是单字、码位重复、笔画越界、分类越界、`roles` 无法归一化、同一输入两次生成字节不一致。

## 7. 窗口

### 7.1 布局

```text
┌──────────────────────────────────────────┬──────────────────┐
│ 结构按钮  ⿰ ⿲ ⿱ ⿳ ⿴ ⿵ ⿶ ⿷ ⿸ ⿹ ⿺ ⿼ ⿽ ⿻ ⿾ ⿿ ㇯ │ 难输入部件   [×] │
│                                          │ [笔画] [分类] [最近] │
│        ┌──────────────┬──────────────┐   │ 1 2 3 4 5 6 7   │
│        │      木      │  ┌────────┐  │   │ 8 9 10 11 12 13+│
│        │              │  │   早   │  │   │                  │
│        │              │  ├────────┤  │   │ ┌──┬──┬──┬──┐  │
│        │              │  │   艹   │  │   │ │𠂇│龰│𠂹│𩫏│  │
│        │              │  └────────┘  │   │ ├──┼──┼──┼──┤  │
│        └──────────────┴──────────────┘   │ │丆│匚│刂│冂│  │
│ 表达式：⿰木⿱艹早                         │ └──┴──┴──┴──┘  │
│ 部件输入：[      ] [填入] [退格] [清空]   │ 𠂇 U+20087 · 2画  │
│ [查询]                                   │                  │
│ ┌──────────────────────────────────────┐ │ 最近使用          │
│ │ 林  U+6797  ⿰木木  G/T/J/K/P/V       │ │ [木][艹][氵]…     │
│ └──────────────────────────────────────┘ │                  │
└──────────────────────────────────────────┴──────────────────┘
```

- 默认 980×640，最小 760×520；打开时画布已经是 `⿰`（左右两个空槽），可直接填部件或再拆结构；
- 配色取输入法当前主题：读 `config.toml` 的 `[theme] curr_theme`，把候选窗口的
  背景 / 正文 / 页码 / 高亮色映射成整窗配色；找不到主题就回退内置深色；
- 窗口整窗自绘：`Decorations::None`，顶部 36 点标题栏（标题、最小化 / 关闭），
  拖动与双击最大化交给窗口壳的 `caption_height`；最大化时背景圆角为 0；
- 抽屉展开 300，关闭时隐藏，由工具栏末尾的「部件 / 收起」切换；
- 抽屉正文和结果列表都走 `ScrollHandle` + `ScrollTarget`，滚轮 / 滚动条由窗口壳统一处理。

### 7.2 画布切分

```text
⿰ / ⿲   横向等分 2 / 3 列
⿱ / ⿳   纵向等分 2 / 3 行
⿴       四周内缩
⿵ ⿶ ⿷ ⿼  对应一侧内缩
⿸ ⿹ ⿺ ⿽  两个方向内缩
⿻       编辑时左右分栏，标「重叠」
⿾ ⿿ ㇯  单子区域内缩，标「镜像 / 旋转 / 减除」
```

- 节点间留 6–8px；深一层字号递减；最小节点尺寸限死，防无限嵌套；
- active 槽强调色描边；
- 每个节点写 `Region`，名字 `slot:<path>`（路径用 `.` 连接子下标）。

字号：`min(rect.w, rect.h) * 0.62`，夹在 `10..=96`；结构角标另行小字号。

### 7.3 状态树与插入语义

```rust
enum Node {
    Empty,
    Char(char),
    Wildcard,
    Op { op: IdsOp, children: Vec<Node> },
}

struct Composer {
    root: Node,
    active: Vec<usize>,   // 从根到当前槽的子下标
    input: String,
}
```

结构按钮显示中文结构名（`⿰ 左右结构`、`⿱ 上下结构`…），点击仍然发送 `op:<IdsOp 下标>`，
实际匹配用的 IDS / 缩写不变。

| active 是 | 点结构按钮 |
|---|---|
| `Empty` | 变成 `Op`，子槽全空，active 移到第一个子槽 |
| `Char` / `Wildcard` | 包成 `Op` 的第一个子节点，其余为空 |
| `Op` | 替换为新的 `Op`（不叠层） |

填部件（部件输入框或抽屉点 tile）：

| active 是 | 结果 |
|---|---|
| `Empty` | 填入 |
| `Char` | 替换 |
| `Op` | 填它下面第一个空叶子 |
| 无 active（画布空） | 根变 `Char` |

输入框只保留一个汉字：`TextChanged` 时取最后一个汉字，内容变化就直接写入当前槽，
不再需要点「填入」；单击画布任意槽会把焦点交给输入框。抽屉点 tile 走同一个入口。
插入成功后按深度优先跳到下一个空叶子；没有就停在原地。

退格：current 变 `Empty`；current 是 `Op` 则整棵子树折叠成 `Empty`。清空：根变 `Empty`。

### 7.4 结果列表

复用 `build_text_list` + `ScrollTarget`：

- 行文本 `字  U+XXXX  IDS  来源`；
- 单击选中，双击复制到剪贴板并提示「已复制：林」；
- 剪贴板实现放在 app 内（`OpenClipboard` / `EmptyClipboard` / `GlobalAlloc` / `SetClipboardData`），不依赖 Server；
- 空结果 / 语法错误 / 数据缺失都有明确状态文案。

### 7.5 部件抽屉

| 标签 | 内容 |
|---|---|
| 笔画 | `1 2 3 … 12 13+ 未分类`，记住上次选择 |
| 分类 | 五个角色 chip（`围 / 头 / 旁 / 底 / 其它`，标签带数量），按该角色分值降序、最多 40 个 |
| 最近 | 最近插入的 12 个（去重） |

顶部一个搜索框：按字符、码位十六进制（`2008E`）或 IDS 片段 / 运算符缩写（`LR`、`RD`）过滤当前页。
抽屉顶端与组字画布外框顶端对齐，高度延伸到窗口底边；结构按钮行始终占满窗口宽度，不随抽屉缩窄。
抽屉右上角没有关闭按钮，用工具栏末尾的「收起」关闭。

> 实现说明：分类页最初设想「五组竖排、组头展开」，窗口高度有限，实际改成五个 chip 切换 +
> 单个网格，避免抽屉里再套一层页面滚动；网格支持滚轮与拖动右侧滚动条。

- tile 56×56，网格 4 列，圆角 8；
- 命中区名字 `comp:<tab>:<index>`（同一部件可能出现在多个页，不能用码位当名字）；
- 悬停预览行：`𠂇 · U+20087 · 2画 · 头 78%`；
- `flags.bit0 = 可打出` 的部件默认隐藏，提供「显示可打出部件」开关；
- 点 tile 走 `Composer::insert_component`，不改变窗口焦点。

### 7.6 字形策略

只使用系统字体与随 `ids.qj` 下发的部件 SVG，**不打包整份字体**。

1. 字体族默认「Microsoft YaHei UI」（系统 UI 字体的 GDI 字体链接不覆盖 IDS 运算符），
   可在 `%APPDATA%\CloudIME\ids.toml` 的 `[font] family` 改；
2. 抽屉打开时用 DirectWrite 的 `IDWriteFont::HasCharacter` 探测字形并缓存
   （GDI 的 `GetGlyphIndicesW` 不支持非 BMP，扩展区部件会全部误判）；
3. 部件有内置 SVG（`COMP.svg_off` 指向 `CTEXT`）时优先画 `Element::Svg`：
   抽屉 tile 40px、画布槽位按槽宽等比；字统网 `font` 数据已生成 `assets/ids/component-glyphs.tsv`；
4. 没有 SVG 但系统字体有字形：`Element::Text`；
5. 两者都没有：只显示码位 `U+XXXX`，不再画 `UD丿罒` 这类结构缩写；
6. `show_typeable = true` 时部件抽屉列出「能用拼音打出」的部件，缺省隐藏。

### 7.7 键位

| 键 | 行为 |
|---|---|
| `Tab` | active 槽 → 部件输入框 → 抽屉网格 循环 |
| 抽屉内方向键 | 移动高亮 tile |
| `Enter` / `Space` | 插入高亮 tile（输入框里 Enter = 查询） |
| `Esc` | 折叠抽屉；再按关闭窗口 |
| `Backspace` | 输入框里删字；画布焦点时清当前槽 |

## 8. 错误与降级

| 情况 | 行为 |
|---|---|
| `ids.qj` 不存在 / 打不开 | 整窗提示「IDS 数据缺失」+ 数据路径；不 panic |
| `COMP` 缺失 | 抽屉置灰，第一版功能照常 |
| 表达式不完整 | 状态栏给出具体原因（「⿰ 需要 2 个部件」「多余字符」） |
| 未收录 | 「Unicode IDS 未收录此结构」 |
| 通配结果过多 | 「命中 N 条，显示前 200；请缩小部件或增加结构」 |
| 字体缺字形 | 有内置 SVG 就画 SVG，否则显示码位 `U+XXXX`；仍可插入与复制 |

## 9. 数据与文档同步

- `assets/ids/IDS.TXT` 与 `assets/ids/components.tsv` 进仓库；`data/generated/ids.qj` 在 `.gitignore` 的 `/data` 下，不进仓库；
- `docs/notes/crate-notes.md`：数据版本、SHA、统计、生成命令、分类算法与阈值、限制；
- `docs/notes/release.md`：发布前生成 `ids.qj` 的命令与校验；
- `docs/user/tools/ids-composer.md`：窗口用法、缩写表、缺字形说明、字体设置；
- `tutorial.md`：悬浮状态条「工具」菜单一节补一句。
