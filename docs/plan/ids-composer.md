# 组字工具（IDS 组字）实施计划

> 依据：[`docs/design/ids-composer.md`](../design/ids-composer.md)（已确认）。
> 本文只拆执行顺序、文件与验证点；格式、算法与交互细节以设计文档为准。
>
> 执行状态（2026-11-17）：**P0–P3 与第二版增强全部完成**。核心库、生成器、互动画布、部件抽屉、
> 工具菜单与安装脚本都已落地；第二版补了整窗 CloudUI 自绘（自绘标题栏）、抽屉搜索 / 收藏 /
> 可拖滚动条、常见部件替换（水→氵）与结果字形结构预览。真机验证：`push-local.ps1` 覆盖安装、
> Server 正常启动、安装包 ISCC 编译通过（184.7 MB）、安装目录的 `cloudime-ids.exe` 能读取随包 `ids.qj`。
> 改动已提交（2026-11-17）。
> 2026-11-18 调整：结构按钮改中文名、去掉「替换」与结果悬停预览、部件字形改 SVG（`CTEXT`）、
> 输入框限一个汉字并自动同步、点击画布槽自动聚焦输入框。

## 0. 目标与验收

第一版（P0 + P1 + T3.1/T3.2）：

- `cargo run --release -p cloudime-ids-build -- …` 生成 `data/generated/ids.qj`；
- `cargo run -p cloudime-ids --bin ids-query -- "⿰木木"` 输出 `林`；
- 独立窗口能：点结构按钮拆槽 → 填部件 → 查询 → 结果列表滚动 → 双击复制；
- 从悬浮状态条「工具 → 组字工具」打开 `cloudime-ids.exe`；
- `ids.qj` 缺失、表达式错误、未收录、结果过多都有明确提示。

第二版（P2）：

- 右侧抽屉按笔画 `1..13+` 浏览难输入部件，点击填入当前 active 槽并自动跳槽；
- 分类页能按 `圍/頭/旁/底/其它` 浏览，同一部件可出现在多个分类下；
- 系统字体缺字形时显示 `UD丿罒` 这类缩写配方，不出现豆腐块；
- 「最近使用」与「显示可打出部件」开关工作。

## 1. 基线与约束

Requirement Ready Check：`ready`。设计已在 [`docs/design/ids-composer.md`](../design/ids-composer.md) 确认，范围、数据源、格式、算法、交互与非目标都已定。

基线：

- 容器：[`crates/cloudime-format`](../../crates/cloudime-format/src/lib.rs)（`Kind`、`Container`、`Writer`、`Text`、`Table`、`hash`）；
- 独立窗口：[`apps/cloud-ui-demo`](../../apps/cloud-ui-demo/src/main.rs)（`run_native` / `Application` / `Document`）；
- 列表：[`crates/cloud-ui/src/view/text_list.rs`](../../crates/cloud-ui/src/view/text_list.rs)（`build_text_list`、`ScrollHandle`）；
- 工具菜单：[`apps/windows/server/src/ui/status/tools.rs`](../../apps/windows/server/src/ui/status/tools.rs)；
- 冻结命令：`cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`CLOUDIME_UIACCESS=0 cargo test --workspace --locked`；
- 提交信息 `类型(范围): 中文说明`，代码组织与注释按 `docs/contributing.md`。

### 1.1 Change Necessity

- IDS 解析、mmap 查询与部件分类必须是代码：配置或文档替代不了；
- 互动画布是新的窗口状态机，`cloud-ui` 现有的 `View` 拼接表达不了递归槽位；
- `Kind::Ids` 是数据契约的新成员，必须进 `cloudime-format`；
- 最小代码边界：`crates/cloudime-ids`（解析 / 查询 / 部件表）+
  `tools/ids-build`（数据生成）+ `apps/windows/ids`（窗口）+ `Kind::Ids` 一处枚举。

### 1.2 Ripple Signal Triage

- **新 owner**：`crates/cloudime-ids` 拥有 IDS 数据语义；`tools/ids-build` 拥有生成；app 只做表现与交互；
- **数据契约**：`Kind::Ids = 8`、`IdsKey` / `IdsEntry` / `Component` 布局。只增不改，旧 `Kind` 与旧 `.qj` 行为不变；
- **分发面**：新 exe + 新 `ids.qj` 要进 `cargo-build.ps1`、`build-installer.ps1`、`cloudime.iss`、`reconfigure.ps1`、`push-local.ps1`；
- **菜单面**：Server 的「工具」菜单新增固定项；`tools.list` 是 `onlyifdoesntexist`，老安装不会拿到新行，所以**必须走固定项**，不能靠清单；
- **不触碰**：`PROTOCOL_VERSION`、TSF/DLL、Server 候选窗、输入管线。

### 1.3 Plan Pressure Test

- 所有者合适：容器能力留在 `cloudime-format`，IDS 语义独立 crate，生成逻辑独立 tool，UI 只在 app；
- 不需要新架构层，不引入第二套数据读取路径；
- 验证边界清楚：解析 / 分类 / 查询是纯函数测试；窗口是冒烟；安装是真机；
- 任务按依赖排序，单个任务可在一次会话内完成并独立验证。

## 2. TDD Route

- mode：`auto`（仓库无 TDD 配置，用户未要求 strict）；
- decision：
  - **strict**：T0.2、T0.3、T0.4、T0.7、T0.8、T0.9、T0.10、T1.2、T2.1、T2.3——
    解析、索引、分类、状态机属于共享契约或高风险回归面，先写失败测试；
  - **light**：T0.1、T0.5、T0.6、T1.1、T1.3–T1.6、T2.2、T2.4、T2.5、P3——
    窗口绘制与脚本没有纯函数单测收益，用几何纯函数测试 + 真机冒烟覆盖；
- authority：设计文档已冻结数据布局与分类规则；判定由仓库基线（确定性生成、黄金查询）承担；
- verification：每个 strict 任务先跑对应 crate 测试，P3 再跑全 workspace。

## 3. 任务

### P0 数据与核心库

#### T0.1 workspace 骨架

文件：

- `Cargo.toml`（根 members + `[workspace.dependencies]`）
- `crates/cloudime-ids/Cargo.toml`、`src/lib.rs`
- `tools/ids-build/Cargo.toml`、`src/main.rs`
- `apps/windows/ids/Cargo.toml`、`src/main.rs`（空壳，先能编译）

改动：

1. 三个新成员加入 `members`；
2. `cloudime-ids = { path = "crates/cloudime-ids" }` 加入 workspace 依赖；
3. 包名：`cloudime-ids`（lib）、`cloudime-ids-build`（bin `ids-build`）、
   `cloudime-windows-ids`（bin `cloudime-ids`），版本一律 `version.workspace = true`；
4. 三个 `src/main.rs` 先只打印一行，确认 workspace 能编。

验证：`cargo check -p cloudime-ids -p cloudime-ids-build -p cloudime-windows-ids`。

#### T0.2 `Kind::Ids = 8`（strict）

文件：

- `crates/cloudime-format/src/layout/kind.rs`
- `crates/cloudime-format/src/file/container.rs`（测试）

改动：

1. 枚举加 `Ids = 8`，`from_code` / `code` 同步，只加不改；
2. 容器测试：用 `Writer` 写一个最小 `Kind::Ids` 文件，`open` 后 `kind()` 与分节可读。

先写测试：`Kind::from_code(8) == Some(Kind::Ids)`、`Kind::Ids.code() == 8`、`Kind::Ids` 文件往返。

验证：`cargo test -p cloudime-format`。

#### T0.3 IDS 解析器（strict）

文件：

- `crates/cloudime-ids/src/parser/mod.rs`
- `crates/cloudime-ids/src/parser/node.rs`
- `crates/cloudime-ids/src/parser/op.rs`
- `crates/cloudime-ids/src/error.rs`

改动：

1. `IdsOp { op: char, arity: u8 }`，覆盖设计 4.1 的运算符；
2. `IdsNode::{Leaf(char), Unencoded(u8), Wildcard, Op { op, children: Vec<IdsNode> }}`；
3. `parse(input) -> Result<IdsNode, IdsError>`：前缀递归下降、整串消费完；
4. `normalize(input) -> Result<String, IdsError>`：缩写展开、`？/?/⬚` 统一、`〾` 去掉；
5. `to_ids(&self) -> String`：规范化前缀表达式，供生成与断言使用。

先写测试：`⿰木木` 往返、`LR木木` == `⿰木木`、嵌套 `⿻一⿰丨丶`、
`〾` 去标记、`{1}` 解析为 `Unencoded`、缺子节点 / 多余字符 / 未知运算符报错。

验证：`cargo test -p cloudime-ids parser`。

#### T0.4 `IdsDatabase`（strict）

文件：

- `crates/cloudime-ids/src/database/mod.rs`
- `crates/cloudime-ids/src/database/layout.rs`
- `crates/cloudime-ids/src/database/query.rs`
- `crates/cloudime-ids/src/lib.rs`

改动：

1. `IdsKey` / `IdsEntry` 按设计 3.3 定义，`zerocopy` derive，测试断言 `size_of`；
2. `IdsDatabase::open(&Path)`：`Container::open(Kind::Ids)`，取 `TEXT/KEYS/HASH/ENTR`；
3. `key_str(id)`、`entry_slice(key)`、`character(entry)`；
4. `lookup_exact(&self, query) -> Result<Vec<IdsMatch>, IdsError>`；
5. `lookup_wildcard(&self, query, limit) -> Result<WildcardMatches, IdsError>`：
   扫描 `KEYS`、解析每棵树、`？` 匹配任意子树；
6. `IdsMatch { character, ids: String, sources: u16, flags: u16 }`；
7. 错误：`Format(FormatError)`、`Parse(IdsError)`、`UnknownCharacter`、`Unencoded`。

先写测试（用 `Writer` 现场造小 `.qj`）：多字同 IDS、同字多 IDS、精确命中 / 未命中、`？` 通配、`{n}` 不参与精确索引。

验证：`cargo test -p cloudime-ids`。

#### T0.5 查询 CLI（light）

文件：

- `crates/cloudime-ids/src/bin/ids-query.rs`

改动：

1. 参数：`<IDS>` 与可选 `--data <path>`（默认 `data/generated/ids.qj`）；
2. 输出每条结果：`字  U+XXXX  IDS  来源`；
3. 错误走 stderr，退出码非 0。

验证：`cargo run -p cloudime-ids --bin ids-query -- "⿰木木"`。

#### T0.6 数据入库（light）

文件：

- `assets/ids/IDS.TXT`
- `assets/ids/components.tsv`
- `docs/notes/crate-notes.md`

改动：

1. 放入 `IDS.TXT`（3,285,669 字节，SHA256 见设计 2.1）；
2. `components.tsv` 初始 554 项：字符 + 笔画（来自字统网笔画分组）+ 启用，分类留空；
3. `crate-notes.md` 新增「CloudIME 组字数据」一节：URL、版本、SHA、统计、文件头版本矛盾、生成命令。

验证：`Get-FileHash assets\ids\IDS.TXT -Algorithm SHA256` 与设计一致；TSV 每行单字。

#### T0.7 生成 `TEXT / KEYS / HASH / ENTR`（strict）

文件：

- `tools/ids-build/src/main.rs`
- `tools/ids-build/src/args.rs`
- `tools/ids-build/src/read.rs`
- `tools/ids-build/src/write.rs`
- `tools/ids-build/src/tests.rs`（或 `tests/` 黄金）

改动：

1. clap 参数：`--ids`、`--out`、`--components`、`--unihan`、`--dict`、`--report`、`--derive`；
2. 解析 `IDS.TXT`：`U+XXXX <TAB> 字 <TAB> ^IDS$(sources)…`，多来源逐条；
3. 聚合：`normalize` 后的 IDS 为 key，同字合并 `sources/flags`，entries 按码位排序；
4. KEYS 按 IDS 字节序排序；HASH 用 `cloudime_format::hash::build`；
5. META 按设计 3.5；
6. 确定性：同一输入两次生成字节一致（测试里对比两次输出）。

先写测试：20 行 mini `IDS.TXT` → 断言 key 数、entry 顺序、flags、META；两次输出相等。

验证：`cargo test -p cloudime-ids-build`；真数据 `cargo run --release -p cloudime-ids-build -- --ids assets/ids/IDS.TXT --out data/generated/ids.qj`，然后 T0.5 查询冒烟。

#### T0.8 部件表与笔画（strict）

文件：

- `tools/ids-build/src/components.rs`
- `tools/ids-build/src/stroke.rs`
- `crates/cloudime-ids/src/component.rs`（结构 + 分组读取）

改动：

1. 解析 `components.tsv`：单字、笔画 1..=13、分类 0..=4、启用；
2. 笔画优先级：TSV > Unihan `kTotalStrokes` > IDS 递归（CJK Strokes 叶子 = 1 画）> 0；
3. `COMP` 按码位严格递增写出；`stroke = 0` 记 warn 并进报告；
4. `IdsDatabase` 增加 `components()`、`component(codepoint)`、`components_with_stroke()`；
5. `COMP` 缺失时这些方法返回空切片，不报错。

先写测试：TSV 覆盖优先、Unihan 多值取第一个、递归推算 `𠂇`、越界报错、码位递增。

验证：`cargo test -p cloudime-ids-build stroke`、`cargo test -p cloudime-ids components`。

#### T0.9 分类算法与报告（strict）

文件：

- `tools/ids-build/src/classify.rs`
- `tools/ids-build/src/report.rs`

改动：

1. 角色表按设计 6.1，含 `㇯` 继承与「內→其它、5 折」；
2. 遍历所有 IDS 树，按字去重、深度衰减，归一化到 1000；
3. `primary` 阈值 300，平票顺序固定；
4. TSV 分类覆盖，冲突写报告；
5. `--report` 输出 `components-report.tsv`。

先写测试：`氵/艹/宀/囗/辶/灬/一` 主分类；`木` 的 `roles[旁] > roles[底] > 0`；
`包` 的 `勹` 计圍、`巳` 计其它；`⿳` 中段计其它；`⿲木木木` 只产生一次样本；`？/{n}` 不产生样本。

验证：`cargo test -p cloudime-ids-build classify`；真数据生成后 `roles` 总和全为 1000。

#### T0.10 派生候选（strict）

文件：

- `tools/ids-build/src/derive.rs`

改动：

1. 统计每个部件出现在多少个不同的字里；
2. 读 `data/generated/dict.tsv` 的单字词条，标记「可打出」；
3. 输出 `data/generated/components-candidates.tsv`，不直接改 `components.tsv`。

先写测试：mini dict.tsv 下的可打出判定、候选排序、已收录部件不重复输出。

验证：`cargo run --release -p cloudime-ids-build -- --derive …` 生成候选文件。

### P1 互动画布

#### T1.1 窗口骨架与数据定位（light）

文件：

- `apps/windows/ids/Cargo.toml`
- `apps/windows/ids/build.rs`
- `apps/windows/ids/resources/cloudime.ico`（复制 `apps/windows/tsf/resources/cloudime.ico`）
- `apps/windows/ids/src/main.rs`
- `apps/windows/ids/src/app/mod.rs`
- `apps/windows/ids/src/data.rs`

改动：

1. `run_native` + `Application`，默认 980×640、最小 760×520，标题「组字工具」；
2. `data::resolve()`：`--data <path>` → exe 同目录 `data/generated/ids.qj` → cwd `data/generated/ids.qj`；
3. 加载失败显示整窗错误页（路径 + 生成命令），不 panic；
4. `build.rs` 参照 `apps/cloud-ui-demo/build.rs`，manifest `CloudIME.Ids`、图标、`windows_subsystem = "windows"`（release）。

验证：`cargo run -p cloudime-windows-ids` 能开窗；改坏数据路径能看到错误页。

#### T1.2 `Composer` 状态机（strict）

文件：

- `apps/windows/ids/src/composer/mod.rs`
- `apps/windows/ids/src/composer/node.rs`

改动：

1. `Node` / `Composer` 按设计 7.3；
2. `set_active(path)`、`split(op)`、`insert(char)`、`backspace`、`clear`、`to_ids()`、`next_empty()`；
3. `ComposeError::{Incomplete, TooManyParts, Empty}`；
4. 纯逻辑，不引用 `Document` / Win32。

先写测试：空槽/部件/结构三种 active 下 split 的结果；`insert` 四种分支；自动跳槽顺序；
`to_ids` 黄金串；不完整表达式报错。

验证：`cargo test -p cloudime-windows-ids composer`。

#### T1.3 画布布局与绘制（light）

文件：

- `apps/windows/ids/src/canvas/mod.rs`
- `apps/windows/ids/src/canvas/layout.rs`
- `apps/windows/ids/src/canvas/paint.rs`

改动：

1. `layout(node, rect) -> Vec<Placed>`：按设计 7.2 切分，`Placed { path, rect, role }`；
2. 绘制：`RoundRect` 底 + 边框、active 强调色、角标（缩写 + 运算符）、部件文字、`Empty` 的「＋」、`Wildcard` 的「？」；
3. 每个 `Placed` 写 `Region`，名字 `slot:<path>`。

测试（纯函数）：`⿰` 两等分、`⿳` 三等分、包围内缩量、最小尺寸截断、路径字符串唯一。

验证：`cargo test -p cloudime-windows-ids canvas`；窗口里能看到嵌套盒子。

#### T1.4 输入框、键盘与状态（light）

文件：

- `apps/windows/ids/src/input.rs`
- `apps/windows/ids/src/status.rs`

改动：

1. `TextEdit` 控件名 `ids.input`，`TextChanged` 保存文本；
2. 按钮「填入 / 退格 / 清空」；
3. `Tab` 在 input / canvas 之间切换（canvas 用 active 路径表示焦点）；
4. `Status` 枚举：`Idle / Incomplete(&str) / NotFound / TooMany(usize) / DataMissing`；
5. 表达式行实时显示 `to_ids()` 或第一条错误。

验证：手点结构 + 填部件能拼出 `⿰木⿱艹早`。

#### T1.5 结果列表与剪贴板（light）

文件：

- `apps/windows/ids/src/results.rs`
- `apps/windows/ids/src/clipboard.rs`

改动：

1. `TextListState` + `build_text_list`，行文本按设计 7.4；
2. 双击 `RegionDoubleClicked` 行名取原下标，复制字符；
3. `clipboard::set_text`：`OpenClipboard` / `EmptyClipboard` / `GlobalAlloc`（UTF-16 + 结尾 0）/ `SetClipboardData` /
   `CloseClipboard`，失败返回 `Result`，状态栏提示；
4. 结果区固定高度、可滚动，`ScrollTarget` 交给窗口壳。

验证：双击结果后粘贴到记事本得到正确字符。

#### T1.6 查询接线（light）

文件：

- `apps/windows/ids/src/app/query.rs`

改动：

1. 「查询」/ 回车：`to_ids()` → 含 `？` 走 `lookup_wildcard`，否则 `lookup_exact`；
2. 结果刷新、列表滚动归零、状态更新；
3. 未收录 / 结果过多 / 语法错误的文案按设计第 8 节。

验证：`⿰木木` → 林；`⿰木？` → 多条；`⿰木` → 缺部件提示。

### P2 难输入部件

#### T2.1 部件查询 API 补齐（strict）

文件：

- `crates/cloudime-ids/src/component.rs`

改动：

1. `components_with_stroke(stroke)`、`components_in_role(role, min_permille)`；
2. 分类页排序规则（`roles` 降序 → `primary == role` 优先 → 码位升序）；
3. `has_components()`。

先写测试：分组数量、阈值过滤、排序稳定、`COMP` 缺失时返回空。

验证：`cargo test -p cloudime-ids component`。

#### T2.2 抽屉 UI（light）

文件：

- `apps/windows/ids/src/drawer/mod.rs`
- `apps/windows/ids/src/drawer/grid.rs`
- `apps/windows/ids/src/drawer/tabs.rs`

改动：

1. 展开 300 / 折叠 40，右上角 `×`，工具行「部件」开关；
2. 标签页：笔画 / 分类 / 最近；笔画 chip `1..12 13+ 未分类`，记住选择；
3. 网格：4 列、52×52、gap 6，命中区 `comp:<tab>:<index>`；
4. 悬停预览行；
5. 抽屉正文 `ScrollHandle` + `ScrollTarget`；
6. `COMP` 缺失时按钮置灰 + 提示。

测试（纯函数）：网格坐标、tab 切换后的索引映射、折叠宽度。

验证：554 个部件都能翻到；滚轮可用。

#### T2.3 插入交互与最近使用（strict）

文件：

- `apps/windows/ids/src/composer/mod.rs`（追加）
- `apps/windows/ids/src/drawer/recent.rs`

改动：

1. `insert_component(ch) -> InsertOutcome` 四种结果，复用 `insert`；
2. 成功后自动跳槽，记入 MRU（去重、最多 12）；
3. 抽屉点击把 `comp:<tab>:<i>` 映射回码位。

先写测试：`Empty/Char/Op/无 active` 四种 outcome；MRU 去重与容量；插入后 active 位置。

验证：连续点 3 个部件填满 `⿲`。

#### T2.4 字形探测与降级（light）

文件：

- `apps/windows/ids/src/glyph.rs`
- `apps/windows/ids/src/config.rs`
- `apps/windows/ids/src/drawer/paint.rs`

改动：

1. `GlyphProbe`：`GetGlyphIndicesW(GGI_MARK_NONEXISTING_GLYPHS)`，按字体链探测并缓存；
2. 缺字形 → 该部件 IDS 的 ASCII 缩写（最多两层）→ `U+XXXX`；
3. tile 右下角小圆点 + 预览行说明；
4. `%APPDATA%\CloudIME\ids.toml`：`[font] family`、`show_typeable`；读写失败只记日志。

测试（纯函数）：缩写生成（`𠂹` → `UD丿罒`）、递归截断、配置缺省值。

验证：拔掉扩展区字体（或挑一个已知缺字形部件）能看到缩写配方。

#### T2.5 分类页多归属与过滤（light）

文件：

- `apps/windows/ids/src/drawer/roles.rs`

改动：

1. 五组竖排，组头显示数量，默认两行、可展开；
2. 过滤 `roles[r] >= 100`，去重，按该角色排序；
3. `show_typeable` 开关：隐藏 `flags.bit0` 的部件；
4. 组内上限 40，超出显示「更多」。

测试（纯函数）：多归属（`木` 同时进旁 / 底）、阈值、排序、开关。

验证：分类页五组都有内容，`木` 在两处出现。

### P3 集成与发布

#### T3.1 「工具」菜单入口（light）

文件：

- `apps/windows/server/src/ui/status/tools.rs`

改动：

1. 固定命令 `COMPOSER_COMMAND = 0x7001`，菜单项「组字工具」放在分隔线上方；
2. 点击启动 `bundled_root().join("cloudime-ids.exe")`，工作目录为安装根；
3. 文件不存在只记 `warn`，不弹错误。

验证：`cargo test -p cloudime-windows-server`；真机菜单出现并打开工具。

#### T3.2 构建与安装脚本（light）

文件：

- `cargo-build.ps1`
- `build-installer.ps1`
- `apps/windows/installer/cloudime.iss`
- `apps/windows/scripts/reconfigure.ps1`
- `apps/windows/scripts/push-local.ps1`

改动：

1. `cargo-build.ps1`：release 构建列表加 `-p cloudime-windows-ids`，产物检查加 `target\release\cloudime-ids.exe`；
2. `push-local.ps1`：编译列表加 `-p cloudime-windows-ids`；
3. `reconfigure.ps1`：`Copy-BuiltFiles` 覆盖 `cloudime-ids.exe`，同步 `data/generated/ids.qj` 到
   `<安装目录>\data\generated\ids.qj`；
4. `cloudime.iss`：安装 exe 到 `{app}`，`ids.qj` 到 `{app}\data\generated`；卸载段按现有 files 规则带上；
5. `build-installer.ps1`：必需产物加 exe，必需数据加 `data\generated\ids.qj`，缺件报错文案里补生成命令。

验证：`.\cargo-build.ps1` 通过；`build-installer.ps1 -Build` 在缺 `ids.qj` 时报明确错误；补齐后能出包。

#### T3.3 文档（light）

文件：

- `docs/user/tools/ids-composer.md`（新增）
- `docs/user/keys.md`（若新增键位需要）
- `tutorial.md`
- `docs/notes/crate-notes.md`
- `docs/notes/release.md`

改动：

1. 用户页：窗口布局、缩写表、画布操作、结果复制、部件抽屉、缺字形说明、字体设置；
2. `tutorial.md`：悬浮状态条「工具」菜单补「组字工具」；
3. `crate-notes.md`：数据版本 / SHA / 生成命令 / 分类算法与阈值 / 限制；
4. `release.md`：发布前生成 `ids.qj` 的命令与校验。

验证：文档命令能照抄执行。

#### T3.4 全量验证与真机冒烟（light）

命令：

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
$env:CLOUDIME_UIACCESS=0; cargo test --workspace --locked
cargo run --release -p cloudime-ids-build -- --ids assets/ids/IDS.TXT --components assets/ids/components.tsv --out data/generated/ids.qj --report data/generated/components-report.tsv
pwsh -File apps\windows\scripts\push-local.ps1
```

冒烟清单：

1. 悬浮状态条「工具 → 组字工具」能打开；
2. 点 `⿰` → 填 `木` `木` → 查询得到 `林`；
3. `⿰木？` 返回多条，列表可滚；
4. 双击结果，记事本粘贴正确；
5. 抽屉按笔画浏览、点部件插入并自动跳槽；
6. 分类页五组有内容；
7. 缺字形部件显示缩写配方；
8. 删掉 `data/generated/ids.qj` 后工具给出明确错误。

## 4. 依赖与并行

```text
T0.1 → T0.2 → T0.3 → T0.4 → T0.5
                 ↘ T0.7 → T0.8 → T0.9
                          ↘ T0.10
T0.7 → T1.1 → T1.2 → T1.3 → T1.4 → T1.5 → T1.6
T0.8 → T2.1 → T2.2 → T2.3 → T2.5
T1.1, T2.2 → T2.4
T1.6 → T3.1 → T3.2 → T3.4
P3 文档可随任务同步写
```

可并行：

- T0.9 / T0.10 与 T1.x（分类只影响 P2，不挡第一版）；
- T2.4 字形探测与 T2.2 / T2.3 并行；
- T3.3 文档随对应任务先写草稿，T3.4 前合并。

## 5. 验证清单

| 层级 | 内容 |
|---|---|
| 单元 | 解析器往返、精确/通配查询、分类黄金断言、笔画优先级、Composer 状态机、抽屉排序 |
| 数据 | 97,680 字 / 554 部件（可增）、`roles` 总和 1000、两次生成字节一致、SHA 记录 |
| 窗口 | 画布嵌套、active 切换、插入跳槽、结果滚动、双击复制、缺字形配方 |
| 集成 | 工具菜单、安装目录数据定位、缺数据错误页 |
| 发布 | fmt / clippy / workspace test / release 构建 / 安装包 / 真机冒烟 |

## 6. 风险与未决

1. **通配扫描性能**：108k 条逐条解析若超过 100 ms，再加 `SIGN` 粗指纹子集；语义不变，只影响 P1 体验。
2. **分类阈值**：`primary < 300` 归「其它」、分类页 `>= 100‰`，都是经验值；首次生成后用报告抽查，必要时只调常数并补黄金用例。
3. **字体覆盖差异**：探测只认字体链，用户装了全字库字体后表现会变；缩写配方保证功能不受影响。
4. **`㇯` 与 `⿾/⿿` 在真实数据中极少**：解析器必须支持，分类测试用构造用例覆盖，黄金断言不依赖真数据。
5. **`IDS.TXT` 版本矛盾**：以实测行数为准，升级到 Unicode 17 时重跑黄金用例并更新 SHA。
6. **旧安装无 `ids.qj`**：升级包必须带上该文件；`tools.list` 不适用于本工具，菜单项固定，避免老安装看不到入口。
