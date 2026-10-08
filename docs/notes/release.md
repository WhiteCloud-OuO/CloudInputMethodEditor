# 发版流程

2026-09-07 搭起来的：GitHub Actions 按标签打包、建 Release、生成官网下载页用的 `releases.json`。
这里记怎么发一版、各环节的依赖，以及官网怎么消费产物。

本仓库从青简 fork、版本号从头开始：当前 `version.txt` / 三个 Windows 壳都是 `0.0.1`（第一次发版前的状态）。

## CI 与自动化

| 文件 | 触发 | 做什么 |
|---|---|---|
| `.github/workflows/ci.yml` | push main、PR | 单个 `windows` job（`windows-latest`）：`cargo fmt --check` / clippy / 全 workspace 测试，外加 `cargo check -p cloudime-windows-tsf --target i686-pc-windows-msvc`（安装包带的 32 位 DLL）。仓库公开，Actions 不计费 |
| `.github/workflows/release.yml` | 推 `v*` 或 `windows-v*` 标签 | `prepare` 门禁并建草稿 Release → `windows` 打包上传 → `publish` 合并摘要、转正、生成 `releases.json`、触发官网构建（见下文） |
| `.github/workflows/audit.yml` | 每周一、Cargo.lock 变动 | `cargo audit`（RustSec 已知漏洞） |
| `.github/dependabot.yml` | 每周一 | Cargo 依赖与钉 commit 的 actions 的更新 PR |

## 一次发版做什么

1. 改版本号，把 `-dev` 去掉：根目录 `version.txt`（三个壳的版本号都从它来），再跑 `.\cargo-build.ps1 -OnlyVersion`
   同步 `apps/windows/{server,tsf,settings}/Cargo.toml`（打包读 `server`，`version.txt` 与三个 `Cargo.toml` 一起提交，别只改一边），
   然后 `cargo update --workspace --offline` 同步 `Cargo.lock`。
   **发版之间版本号一直带 `-dev`**（Rust nightly / Firefox Nightly 那套）：`version.txt` 写 `0.0.1-dev`，打包脚本再接上 git 短哈希，
   本地装的、CI 中间构建的都显示 `0.0.1-dev-1a2b3c4`（工作区有改动加 `+`），测试时一眼知道装的是哪个提交；版本号干净的一定是线上包；带 `-dev` 的标签 CI 直接拒绝。
   Inno 的 `VersionInfoVersion` 只认数字点号，`build.ps1` 去掉后缀再传，文件名保留完整版本。
   本仓库第一次发版就是 `0.0.1`（发版提交里 `version.txt` 是 `0.0.1`，没有可去的 `-dev`）。
2. `CHANGELOG.md` 顶上那一节把「未发布」换成日期：`## <版本> · <日期> · <渠道>`，一行一条、面向用户的措辞（当前只有 Windows 一个平台，条目不加平台前缀）。
   同一提交里更新根目录 **`tutorial.md`（使用手册）**：它随安装包装进 `{app}\tutorial.md` 发给用户，凡是这一版有用户能感知的按键 / 鼠标 / 候选窗改动都要写进去
   （`build-installer.ps1` 在手册比 `CHANGELOG.md` 旧时会提醒一句，但那只是提醒）。
   随包发的还有 **`lua.md`（脚本作者的技术文档）**，装到 `{app}\lua.md`：只在脚本接口（`cloudime` 表、事件、动作、清单字段）变了的那一版动它。
   渠道是**更新渠道**（2026-09-17 定）：定期发的版本标 `stable`、版本号干净（`0.0.1`）；中间放给测试者的版本标 `alpha` / `beta` / `rc`，
   版本号带同名预发布后缀（`0.0.1-beta.1`），`release.yml` 见到后缀就把 GitHub Release 标成预发布、不抢 latest。
   「产品还在测试期」由 0.x 的版本号表达，不占渠道字段；从青简带过来的历史条目不回改。
   **更新日志手写，不由提交自动生成**：发版前按上个标签以来的 `git log` 起草几条，人审一遍再定稿。
   标题下可以写一行 `网盘：[夸克网盘](链接)`（GitHub 下载不方便的用户用）：它不算更新条目，Release 说明把它放在最前面，`releases.json` 单独输出成 `mirrors`，官网显示成按钮。
   网盘文件在 CI 跑完后手动上传。
3. 提交，打注释标签并推：`git tag -a v0.0.1 -m "云朵输入法 0.0.1" && git push origin main v0.0.1`。
   `windows-v<版本>` 也认（Release 标题带「Windows」），当前与 `v<版本>` 打的是同一个包。
4. 标签推出去之后紧接一个普通提交把 `version.txt` 改成下一个开发版（`0.0.2-dev`，再跑 `cargo-build.ps1 -OnlyVersion` 同步三个壳），CHANGELOG 顶上加 `## 0.0.2 · 未发布 · stable`；-dev 版本永远没有标签与 Release。

`release.yml` 的流程：

- `prepare`（`ubuntu-26.04`）：门禁（版本号与标签一致、不带 `-dev`、标签在 main 上、配了索引签名密钥）→ 用 CHANGELOG 那一节加无代码签名提示建**草稿** Release。
- `windows`（`windows-latest`）：下载产品数据 → 装钉死版本的 Inno Setup 7.1.0 → `apps/windows/installer/build.ps1` 打安装包 →
  `tools/release/upload-build.sh` 把安装包和 `SHA256SUMS-windows`、`build-info-windows.json` 片段传到草稿上。
- `publish`：打包成功才跑，`tools/release/finish-release.sh` 把片段合成一份 `SHA256SUMS` 与 `build-info.json`、删掉片段、草稿转正
  → `publish-releases-json.sh` 生成并签名 `releases.json` → `bump-website.sh` 触发官网构建。
  打包失败，Release 就停在草稿、用户看不到；修好后删掉草稿与标签重推，或在 Actions 页重跑失败的 job。

发版后 GitHub Release 上有：`cloudime-<版本>-windows-x86_64-setup.exe`、`SHA256SUMS`、`build-info.json`、`releases.json` 与 `.sig`。

官网由 Cloudflare Workers Builds 按官网仓库的提交自动构建，没有可调用的构建钩子，所以主仓库靠**往官网仓库推一个小提交**来触发：
`tools/release/bump-website.sh` 把版本标签与文档提交号写进官网的 `src/content/upstream.json` 并提交推送（提交者 cloudime-ci）。
配了 `CLOUDIME_WEB_TOKEN`（对 CloudInputMethodEditor-web 有 Contents: read and write 的 fine-grained PAT）release.yml 末尾自动做；
官网文档只随发版更新（`docs/user/` 平时改动不推官网，免得文档领先于用户装到的版本）。没配就在官网仓库随便提交一次（或本地跑这个脚本）。
官网构建时才拉最新 Release 的 `releases.json` 与主仓库 `docs/user`，所以提交内容本身不重要，`upstream.json` 只是留个记录、顺便让文档按记下的提交号拉（版本对得上）。

Rust 工具链由 `rust-toolchain.toml` 钉版本（现在 1.96.0），`ci.yml` 与 `release.yml` 的 `dtolnay/rust-toolchain` 的 `toolchain:` 输入写同一个号（共三处）；升级 Rust 时三处一起改。

## Windows 的特别之处

- 没有代码签名证书时 workflow 设 `CLOUDIME_UIACCESS=0`：没签名的 exe 带 uiAccess=true 起不来（测试二进制同理）。CI 与 release 都设了。
  代价是候选窗在任务栏搜索 / 设置这类 UWP 宿主里可能被盖住，用户文档与 CHANGELOG 已列为已知问题。
  Certum 开源证书办下来后：在 `build.ps1` 加 signtool 一步（`sign-local.ps1` 是本机自签的参考），workflow 去掉那个环境变量；SmartScreen 对无签名安装包的拦截也一并消失。
  Inno Setup 钉 7.1.0（与开发机同版本、钉死 GitHub Release 上的安装程序与哈希；CI 镜像 PATH 上的 Chocolatey 6.x 不带简中翻译，不能让它抢先）。
- **官网**：安装包按文件名识别平台（`ASSET_KINDS`），下载页按访问者平台取「有该平台安装包的最新版本」（`latestFor`）。

## 提交前检查与 CI

本地 `git config core.hooksPath .githooks` 启用一次后，每次提交前 `.githooks/pre-commit` 先拒绝装饰性分隔注释（`// ====` / `// ────`，只做视觉分组不带「为什么」），再跑 `cargo fmt --check` 与 `cargo clippy -D warnings`；
`.githooks/pre-push` 在推之前跑全 workspace 测试。外部 PR 走同一套 `ci.yml`，不过不合。

供应链：workflow 里的 actions 一律钉到 commit（注释写对应标签），`.github/dependabot.yml` 每周一提 Cargo 与 actions 的更新 PR；`audit.yml` 每周与 Cargo.lock 变动时跑 `cargo audit`；
cargo 命令全 `--locked`（含 `build.ps1`）。普通 CI 只有 `contents: read`，checkout 不留凭据；release 的 secrets 不放顶层 env，只注入用它的那一步。

发版门禁（`release.yml` 第一步）：版本号与标签一致且不带 `-dev`；标签指向的提交必须在 `main` 上（`git merge-base --is-ancestor`）；产品数据下载后按 `data` Release 的 `SHA256SUMS` 校验，摘要写进 `build-info.json` 的 `data_sha256`。
**正式版前还欠**：产品数据改成不可变 tag 并在仓库里锁定版本（现在滚动覆盖，同一源码 tag 重跑可能拿到不同数据）、安装包内容验证（词库 / 模型 / 许可齐不齐、签名校验）。

## 产品数据从哪来

词库、语言模型（`WordBank/*.db`、`data/generated/lm.qj`、英文词表）不在 git 里，体积约 90 MB 且由本机数据管道生成。
它们发在仓库里一个个**不可变**的预发布 Release 上：`data-v1`、`data-v2`……每次数据重生成发一个新号、从不覆盖
（预发布不会成为 GitHub 的 latest，官网取 latest 时不会拿到它）。仓库里 `tools/release/data.lock` 钉住当前要用的标签与两个资产的 SHA-256，
跟用到新数据的代码同一个提交进去：checkout 哪个提交就拿到它对应的那版数据，离线自编译的人不会因为我们改了数据而编出坏包。
如果 `data.lock` 里还没有 tag（还没发过 `data-vN`），`data-fetch.sh` 会直接报错并提示先跑 `data-bundle.sh`——构建机不会静默拿到错数据。

- `tools/release/data-bundle.sh`：把 `WordBank/*.db`（词库：主词库与领域词库）、`data/generated/` 里的语言模型与英文词表打成
  `cloudime-data.tar.gz`（包里是**仓库根相对路径**），本地整句模型 `data/local_models/*.qjm`（可能多份；训练仓库导出三件套到
  `data/local_models/`，`tools/release/pack-model.sh` 打成一个 `.qj` 容器，fp16 约 56 MB，元数据也写在那个脚本里）打成
  `cloudime-models.tar.gz`——模型单列一份，只重训模型时不用重传词库；
  连同 LLM 续跑中间产物 `cloudime-llm-intermediates.tar.gz` 发到 `WhiteCloud-OuO/CloudInputMethodEditor` 的下一个 `data-vN`（`--tag` 可指定，已存在就拒绝），然后改写 `data.lock`；数据 tag 指向当前提交，所以那个提交要先 push。
  打包前检查 `lm.qj` 与 `*.qjm` 的魔数必须是 `CLOUDIME`（词库是 SQLite 存档，没有这个头，不查），还是改名前的 `QINGJIAN` 就拒绝（报错里给出下面 `rehead` 的完整命令）。
- `tools/release/data-fetch.sh`：按 `data.lock` 下载（有 gh 用 gh，没有就 curl 直连）、按锁文件里的哈希校验（不信 Release 自己那份 `SHA256SUMS`），
  两份包都解到**仓库根**（`WordBank/`、`data/generated/`、`data/local_models/` 各就各位）。`release.yml` 的打包 job 和离线自编译走同一个脚本；
  `cloudime.iss` 把 `WordBank\*.db`（主词库 `Dict.db`）装进 `{app}\WordBank`、`data\generated` 与 `assets` 里的文件按仓库相对路径装进 `{app}`，模型（`data\local_models\*.qjm`）装进 `{app}\data\local_models`（没有就不装，Server 不重排）。
  标签与两个哈希记进 `build-info.json`（`data_tag` / `data_sha256` / `model_sha256`）。

数据重生成之后（重跑 lexicon / bigram）或模型重训之后跑一次 `data-bundle.sh`（三件套比 `.qjm` 新会自动重打），
把锁文件的改动提交（`chore(data): 数据 data-vN`），否则 CI 打的包还是锁文件指的旧数据。模型文件缺失或哈希不符时 CI 会失败，不会静默地发出错数据的包。

拿**改名前**的数据重发新号（`data-v1` / `data-v2` 的 `.qj` 魔数还是 `QINGJIAN`，读端靠 `cloudime-format::MAGIC_LEGACY` 兼容读）
要把头改成 `CLOUDIME`：`data-fetch.sh` → `cargo run --release -p cloudime-dict-convert -- rehead lm data/generated/lm.qj`
（`data/local_models/*.qjm` 每份再 `rehead model <文件>`）→ `data-bundle.sh --tag data-vN+1`。词库那时还是 `.qj`，用
`tools/dict-convert word-bank --chinese <旧 dict.qj> --english data/generated/english.tsv` 合并成一份 `WordBank\Dict.db` 即可。
`rehead` 只改头 8 字节：改之前按容器完整校验一遍、改完再开一遍，坏文件原样报错不碰，已是新魔数的跳过。
发完新号把 `data.lock` 提交上去，`MAGIC_LEGACY` 那段兼容就可以删了。

2026-09-16 之前用的是滚动覆盖的 `data` Release，已冻结不再更新。

## 版本索引的签名

软件内「检查更新」只认签过名的 `releases.json`（设计见 `docs/design/update.md`）。`publish-releases-json.sh` 生成索引后用 `tools/release-sign` 签出 `releases.json.sig`，
两个文件一起挂到本次 Release 与 GitHub latest；官网构建时原样拷到 `https://cloudime.app/releases.json` 与 `.sig`。

- 私钥是仓库 Secret `CLOUDIME_INDEX_SIGNING_KEY`（base64 的 32 字节）；没配时 `release.yml` 的门禁直接失败。维护者本机留一份在 `~/.config/cloudime/index-signing.key`（不进仓库）。
- 换钥：`cargo run -p cloudime-release-sign -- keygen --out <新文件>`，把打印的公钥加进 `crates/cloudime-update/src/index/signature.rs` 的 `PUBLIC_KEYS`（新旧并列），
  发一两个版本后再换 Secret、去掉旧公钥。直接换 Secret 会让所有已装版本收不到更新。
- 改了 CHANGELOG 后用 `publish-releases-json.sh` 重刷索引同样要带这个环境变量。

## 代码签名（Windows）

安装包与 exe 目前不签名：`sign-local.ps1` 是本机自签的一份参考（自签要满足 uiAccess 对 Server 的代码签名要求），CI 上没有证书，
所以 `CLOUDIME_UIACCESS=0` 关掉 uiAccess、Release 说明里自动加一句 SmartScreen 拦截怎么放行。
拿到 Certum 开源证书后在 `build.ps1` 加 signtool 一步、workflow 去掉 `CLOUDIME_UIACCESS=0`，这条就只剩历史。

## releases.json：官网下载页的数据源

`tools/release/releases_json.py` 从 `CHANGELOG.md`（日期、渠道、更新日志）、GitHub Releases API（附件、地址、大小）
与每次发布的 `SHA256SUMS` / `build-info.json`（每个包的 sha256、提交哈希、构建时间、工具链）生成，挂在每个版本的 Release 上；官网固定取
`https://github.com/<repo>/releases/latest/download/releases.json`（仓库私有期间要带令牌走 API 下载附件）。

结构对应官网 `src/lib/releases.ts` 里的 `Release` / `Asset` 类型：

```json
{
  "schema_version": 1,
  "generated": "2026-09-30T12:00:00Z",
  "repository": "WhiteCloud-OuO/CloudInputMethodEditor",
  "latest": "0.0.1",
  "releases": [
    {
      "version": "0.0.1",
      "date": "2026-09-30",
      "channel": "alpha",
      "notes": ["整句输入：……", "候选窗口：……"],
      "mirrors": [{ "name": "夸克网盘", "url": "https://pan.quark.cn/s/…" }],
      "commit": "869ad00…（40 位）",
      "built_at": "2026-09-30T08:38:12Z",
      "toolchain": "rustc 1.96.0 (ac68faa20 2026-05-25)",
      "assets": [
        { "platform": "windows", "arch": "x64", "cpu": "x86_64", "file": "cloudime-0.0.1-windows-x86_64-setup.exe",
          "url": "https://github.com/WhiteCloud-OuO/CloudInputMethodEditor/releases/download/v0.0.1/cloudime-0.0.1-windows-x86_64-setup.exe",
          "size": 42131552, "sha256": "…" }
      ]
    }
  ]
}
```

- `releases` 从新到旧，`latest` 是第一条的版本号；官网「当前版本」取它，历史版本列表就是整个数组。
- `channel` 是 `alpha` / `beta` / `rc` / `stable`，显示成什么字由官网定；`commit` / `built_at` / `sha256` 给用户核对下载的包，下载页应显示 sha256 与提交短哈希。
- 安装包文件名固定为 `cloudime-<版本>-windows-x86_64-setup.exe`（全小写，2026-09-17 定，包管理器的地址模板与检查更新都靠它稳定），
  平台与架构由文件名判定（`ASSET_KINDS`，历史 Release 上更早的包名也认），`arch` 给人看，`cpu` 给程序比对。
- `schema_version` 现在是 1：只加字段不用动，改了已有字段的含义或结构才加一。
- `mirrors` 来自 CHANGELOG 那一节的「网盘：」行，没有就是空数组；官网在下载按钮旁单独显示。
- `SHA256SUMS` 与 `releases.json` 自己不列进 `assets`。
- 官网侧要做的：构建时下载这个文件替代手写的 `releases` 数组（与拉 `docs/user` 的 `sync-docs.mjs` 同一处、同一个令牌），
  `downloadsOpen` 开关仍由官网自己控制。

## 本机打包

编产物：`.\cargo-build.ps1`（版本号取自 `version.txt`，64 位与 32 位一起编）；
只打包、不重编：`.\build-installer.ps1`；一条命令从源码到安装包：`apps\windows\installer\build.ps1`（CI 走的就是它）。
`powershell -NoProfile -ExecutionPolicy Bypass -File apps\windows\installer\build.ps1`（在仓库根跑）：release 构建 Server / TSF DLL / 设置程序
（另编一份 `i686-pc-windows-msvc` 的 32 位 DLL），再用 Inno Setup 编 `cloudime.iss`，成品在 `target/installer/cloudime-<版本>-windows-x86_64-setup.exe`。
需要 MSVC 工具链与 Inno Setup 7；随包数据取自仓库 `data\generated` 与 `assets`，打包前先确保 `.qj` 是最新的（见 `apps/windows/installer/README.md`）。
