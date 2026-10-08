# 云朵 Windows 安装包

用 [Inno Setup](https://jrsoftware.org/isinfo.php) 打的安装包，把 TSF DLL（64 位与 32 位各一份）、Server、设置程序与随包数据一起装进
`C:\Program Files\CloudIME`，注册文本服务，并设登录自启。

## 安装布局

```
C:\Program Files\CloudIME\
    cloudime_tsf_x64.dll          TSF 文本服务（64 位；被加载进每个应用进程；按位数起固定名，不带版本号）
    cloudime_tsf_x86.dll          同上的 32 位版（企业微信 / WPS / 32 位 QQ 这类 32 位应用只能加载它）
    cloudime-server.exe       输入内核 Server（跑在应用进程外）
    cloudime-settings.exe     设置界面
    Microsoft.UI.Xaml.dll …   设置程序自带的 Windows App Runtime（自包含部署，见下节；约 53 MB / 35 个文件）
    cloudime.ico              开始菜单 / 启动项快捷方式的图标（exe 里也嵌了一份）
    tutorial.md               使用手册（键盘、鼠标、候选窗的全部用法；随仓库根目录的 tutorial.md，发版前同步）
    lua.md                    脚本作者的技术文档（清单、事件与载荷、动作、cloudime 表、示例；随仓库根目录的 lua.md）
    SpecialSymbolsInserter.exe 特殊字符输入器（随包带的独立小工具，悬浮状态条「特殊字符」按钮起它）
    LocalDictionary\          本地词典（翻译 Tip 用）：dictionaries.list（一行「显示名=文件名」）+ glossary-*.qj / *.db；
                              「设置 → 翻译 → 本地词典」的选项就是清单里的显示名，Server 与设置程序都从这儿读（约 50 MB，按需要留哪几份）
    tools\                    工具目录：cwt-gui.exe（词库转换工具，GUI）+ cwt.exe（同上的命令行版）+ tools.list（悬浮状态条「工具」按钮的菜单清单）
    data\generated\           lm.qj（语言模型；英文词表已在 Dict.db 里，不再单独装）
    WordBank\                 Dict.db（7 张表：中文普通组 / 稀有组 + 英文）与用户导入的附加词库（本目录对普通用户可写，导入 / 删除词库走它）
    Phrases\Phrase.db         自定义短语库（user 与 cloudime_default 两张表；本目录对普通用户可写，「设置 → 短语」页走它；
                              随仓库带一份成品，升级不覆盖用户短语）
    data\phrase-default.db    内置短语的同步源（与上面同一份文件换个名字，每次升级都覆盖；Server 启动时把它的
                              cloudime_default 同步进 Phrases\Phrase.db，用户短语不受影响）
    Scripts\                  用户 Lua 脚本（一个 .lua 一个脚本；本目录对普通用户可写，「设置 → 脚本」页走它）+
                              template.lua（「新建脚本」的模板，加载器按文件名跳过、不执行；升级会覆盖它，用户脚本不动）
    data\icons-arrangement.cfg 悬浮状态条的按钮排布（哪个按钮、位置、图标）
    data\icons\*.svg          那些按钮的图标（20×20）
    assets\                   sample\
```

Server 与设置程序按 **exe 相对**定位随包资源（`cloudime_platform::resources`）：装机时资源与 exe 同级，
开发时是仓库 `ime\`（exe 在 `target\{debug,release}\` 下往上三层）。相对写法两套布局一致，只有根不同。
词库目录 `WordBank\` 按这个根找（装机时就是程序目录），主词库固定 `Dict.db`。
`data\icons-arrangement.cfg` 与 `data\icons\` 例外：状态条按 exe 同目录找（`server/src/ui/status/arrangement.rs`），
`cargo build` 由 `server/build.rs` 从 `apps\windows\server\src\ui\status\` 拷一份到 `target\{debug,release}\data\`。

`tools\` 是悬浮状态条「工具」按钮的菜单目录：`tools.list` 一行一个 `短路径=名称`（相对 `tools\`，文件不存在的项不显示），
装包只铺一份、升级不覆盖（用户自己加的工具与改过的清单留着）；控制台工具（如 `cwt.exe`）从菜单启动会留在控制台里，
方便看输出、接着敲命令。卸载时 `{app}` 整棵删掉，所以用户自己塞进 `tools\` 的东西也会一起删。

用户数据在 `%APPDATA%\CloudIME`（config.toml、学习数据、统计），用户短语在安装目录的 `Phrases\Phrase.db`，三个进程的日志在 `%LOCALAPPDATA%\CloudIME\logs`（`server.` / `tsf.` / `settings.` 前缀，按天，留 7 天）；
**卸载时这两处随 `[UninstallDelete]` 一并删除**（`{userappdata}` / `{localappdata}`，指运行卸载程序的那个用户；同一台机器上其他账户的数据要各自删）。
只想保留数据的话把 `cloudime.iss` 里那两条 `{userappdata}` / `{localappdata}` 注释掉。
图标由 `regsvr32` 写到 `%ProgramData%\CloudIME\cloudime.ico`（DLL 里 include_bytes 内嵌），反注册时删掉，`[UninstallDelete]` 顺带清掉空目录。

## 安装位置与开始菜单

默认装到 `{sd}\Program Files\CloudIME`（系统盘的 Program Files，64 位安装下与 `{autopf}` 等价）；开始菜单里建的是
**`CloudIME` 目录**（`DefaultGroupName=CloudIME`，不跟显示名「云朵输入法」走），里面是「云朵输入法 设置」与「云朵输入法 卸载」。

安装向导**总是显示选择目录那一页**（`DisableDirPage=no`）让用户能改：Inno 的缺省 `auto` 会在重装 / 升级同一个 AppId 时
自动跳过它，用户想换目录就没机会。

代价：Windows 的 uiAccess 只对装在 `%ProgramFiles%` 下的程序生效，改到别的目录或别的盘后，候选窗口在开始菜单、
任务栏搜索、「设置」这类系统界面里会被盖住（输入与上屏不受影响）。目录页上写了这条提示（`[Messages]` 的 `SelectDirLabel3`）。

**升级不沿用旧值**（`UsePreviousAppDir=no` / `UsePreviousGroup=no`）：改名前后用过的目录/组名不再沿用，一律回到默认目录与新组名。
云朵的 **AppId 是自己的、与青简不同**，两个产品可独立安装、互不覆盖；`[InstallDelete]` 顺带清掉早期云朵版本留在开始菜单与
「启动」文件夹里的项（含改名前的「云朵输入法」组名）。

> 本仓库从青简 fork，但 `AppId`、TSF `CLSID`、语言 profile GUID、组句显示属性 GUID、保留键 GUID 都已换成云朵自己的值：
> 云朵与青简**不是同一个产品**——装云朵不会顶掉青简，反之亦然，两个都装时各占一份输入法（状态条与 Server 各自独立）。

## 安装程序做的几件事

1. **结束旧进程**：`PrepareToInstall` 里 `taskkill` Server 与设置程序（只有这两个 exe 要覆盖）。
2. **应用容器权限**：`icacls` 给安装目录加 `ALL APPLICATION PACKAGES`（SID `*S-1-15-2-1`）读+执行。
   不加的话 UWP/AppContainer 应用（任务栏搜索、设置）读不到 DLL，切不到云朵输入法。
3. **注册文本服务**：64 位 DLL 用 `regsvr32`、32 位 DLL 用 `SysWOW64\regsvr32`，各注册一次（各自写进自己视图的 HKCR，`CTF\TIP` 两边共用；要管理员——安装程序本就提权）。
4. **清旧 DLL**：装完删历次版本留下的 `cloudime_tsf*.dll`，仍被应用占用的登记成重启后删（`RestartReplace`）。
5. **登录自启**：「启动」文件夹放 Server 快捷方式（Explorer 走 ShellExecute 拉起才拿到 uiAccess；计划任务拿不到）。
6. **立即启动**：完成页以当前非提升用户 ShellExecute 起一次 Server，装完就能用，不必先注销。

卸载反向：杀 Server / 设置程序 → 反注册当前版本 DLL（DLL 自己删掉 `%ProgramData%\CloudIME\cloudime.ico`）→ 删文件。
输入法 DLL 被加载在**每个用过输入法的进程**里（连 explorer.exe 都在内），文件锁着删不掉：卸载器对删不掉的
`cloudime_tsf*.dll` 调用 `RestartReplace`（`MoveFileEx` 的 `DELAY_UNTIL_REBOOT`）登记到重启后由系统删除，并在结束前提示重启；
`[UninstallDelete]` 清掉整个安装目录（含用户导入的词库、`Phrases\Phrase.db` 里的用户短语与 `Scripts\` 下的用户脚本）与用户数据（`%APPDATA%\CloudIME`、`%LOCALAPPDATA%\CloudIME`、`%ProgramData%\CloudIME`）。

## 升级：DLL 被占用怎么办

`cloudime_tsf_x64.dll`（32 位那份 `cloudime_tsf_x86.dll`）被加载进每一个有文本框的应用进程，文件锁着覆盖不了；
Inno 缺省的 `CloseApplications` 用 Restart Manager 找出所有占用者要求关闭——对输入法 DLL 就是「关掉一切」，
所以关掉它（`CloseApplications=no`），改成：

- DLL **文件名固定不按版本走**（`cloudime_tsf_x64.dll` / `cloudime_tsf_x86.dll`），同版本重装会撞名，所以装之前先把在用的那份改名让开（`RetireLoadedDll`，改成 `<原名>.old-<随机>.dll`），装完删掉 / 登记重启后删；
- 同名覆盖才需要上面这一步；升级跨版本时文件名不变，同样靠「改名让开」落地，不再有「新旧并排」；
- 只 `regsvr32` 新文件（InprocServer32 指向它）。**不要**对旧 DLL `regsvr32 /u`：那会把整个 CLSID / profile 注销掉；
- 已开着的应用继续用进程里的旧 DLL 直到重启，Server 两个版本都服务（`OpenSession` 带协议版本，对不上只记警告）；
- 装完删旧 DLL（`cloudime_tsf*.dll` 里不是刚装那两个的），删不掉的登记成重启后删。

## 设置程序自带 Windows App Runtime

设置界面用 Windows Reactor（WinUI 3）写，而它的框架依赖引导只有 Windows 11 走得通：要 Windows 11 才有的
AppModel API 把框架包加进进程包图，Windows 10 上没有那两个函数（定位见 `docs\notes\windows-win10.md`）。
所以设置程序用**自包含部署**——`apps\windows\settings\build.rs` 让 `windows-reactor-setup` 把 Windows App Runtime
铺到 `target\release\`，打包时按 `settings-runtime.txt` 挑进 `target\installer\settings-runtime`，本目录的
`cloudime.iss` 再整个目录装到 `{app}` 下、与 `cloudime-settings.exe` 同级。

- 这些文件是运行时必需：少一件（或层级装错）设置窗口就起不来，`build.ps1` 发现缺文件会直接失败。
- **不做多语言**：`settings-runtime.txt` 的语言资源只列简体中文（`zh-cn`，两个 `.mui`），界面文案本身就是中文；
  原先列了 86 个语言目录、185 个文件，现在是 35 个（约 53 MB）。系统语言不是中文时由资源里的默认语言兜底。
- 升级 `windows-reactor` / `windows-reactor-setup` 时，照新版 crate 的 `assets/runtime.txt` 核对 `settings-runtime.txt`。
- Server 与 TSF DLL 不依赖它；装机体积的大头仍是随包数据。
- `windows-reactor-setup` 在 `cargo build` 时用系统 `curl.exe` 从 NuGet 下运行时包（无校验，失败只打印），缓存在 `%LOCALAPPDATA%\windows-reactor-setup`；CI 的 runner 每次都会重下一遍。下载失败的后果由 `build.ps1` 的缺项检查兜住。

## 打包（在编译机上）

数据取自仓库 `data\generated`、`assets`、`WordBank\` 与 `Phrases\`。打包前先确保 `.qj` 与 `Phrases\Phrase.db` 是最新的
（`Phrases\Phrase.db` 是随仓库追踪的：内置短语手写在它的 `cloudime_default` 表里，改它就等于改内置短语；
`Phrases\` 下除 `Phrase.db` 外一律 gitignore；第一次生成空库用
`cargo run --release -p cloudime-dict-convert -- phrase-db Phrases/Phrase.db`，文件已存在时该命令会拒绝覆盖）。

```powershell
# 需要 MSVC 工具链 + Inno Setup
powershell -ExecutionPolicy Bypass -File apps\windows\installer\build.ps1
```

脚本 release 构建三个产物、从 `apps\windows\server\Cargo.toml` 读版本（版本号的源头是仓库根 `version.txt`，由 `cargo-build.ps1` 同步进三个壳）、找 `ISCC.exe`、编 `cloudime.iss`，
成品在 `target\installer\cloudime-<版本>-windows-x86_64-setup.exe`。改了数据 / 脚本但二进制没变时加 `-SkipBuild`；`-Sign` 用自签证书签产物
（uiAccess 要求 Server 签名 + 装 Program Files）。

也可手动：`iscc /DAppVersion=0.0.1 apps\windows\installer\cloudime.iss`。

### 只打包（产物已经编好）

仓库根的 `build-installer.ps1` 面向「不想再跑 cargo build」的场景：从 `target\release` 与仓库数据里挑出
安装用得到的文件，按同样的相对路径铺进 `target\installer\stage`，再用 `/DRepo` 指过去编同一个 `cloudime.iss`；
成品同样落在 `target\installer\`。不编译、不联网，缺产物 / 缺数据 / 缺 ISCC 时列出缺哪一项。

```powershell
powershell -ExecutionPolicy Bypass -File build-installer.ps1                          # 用现有产物打包
powershell -ExecutionPolicy Bypass -File build-installer.ps1 -Build                   # 先编再打
powershell -ExecutionPolicy Bypass -File build-installer.ps1 -Sign                    # 自签并开 uiAccess（仅本机测试）
powershell -ExecutionPolicy Bypass -File build-installer.ps1 -SkipPack               # 只提取，不打包（stage 保留，便于核对）
```

`cloudime.iss` 的源目录与输出目录可用 `/DRepo` / `/DOutputDir` 覆盖（缺省仍是仓库根与 `target\installer`），
所以这个脚本能指到暂存目录而不动仓库里的原始文件。

找 `ISCC.exe` 的顺序：`CLOUDIME_ISCC` 环境变量 → `C:\Program Files[ (x86)]\Inno Setup 7` → PATH 上的 `iscc.exe`
→ `C:\Program Files[ (x86)]\Inno Setup 6`。装在别的盘（如 `D:\Program Files\Inno Setup 7`）时把它加进 PATH 即可，
PATH 里有多份时用 `CLOUDIME_ISCC` 指定要用那份；只改了环境变量的话要开个新终端才看得到。

## 注意

- **Inno 版本**：开发机与 CI 统一用 Inno Setup **7.1.0**（CI 从 jrsoftware/issrc 的 GitHub Release 钉死下载）。它自带简体中文翻译；
  6.x 的安装包不带 `Languages\ChineseSimplified.isl`，Chocolatey 也只有 6.x，别用。`ArchitecturesAllowed=x64compatible` 需 6.3+。
- **签名**：发版证书就绪后在这里加 `SignTool`（对应 mac 的 Developer ID）；开发期用 `-Sign` 的自签证书。
- **没证书的包**（CI 内测版）：先设 `$env:CLOUDIME_UIACCESS = '0'` 再打，Server 不嵌 uiAccess——没签名的 exe 带 uiAccess=true 会起不来。
  代价是候选窗在 UWP 宿主里可能被盖住。`release.yml` 的 `windows` job 就是这么打的。
