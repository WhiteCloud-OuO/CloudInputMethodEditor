; 云朵 Windows 输入法安装脚本（Inno Setup）。
;
; 装到 Program Files\CloudIME（64 位），把 TSF DLL（64 位与 32 位各一份，见 README「安装布局」）、Server、设置程序与随包数据装在一起，
; 然后：① 给安装目录加 ALL APPLICATION PACKAGES 读+执行权限（UWP/AppContainer 应用——任务栏搜索、
; 设置——才能加载 DLL）；② regsvr32 注册文本服务，64 位与 32 位各注册一次（图标落到 %ProgramData%\CloudIME）；
; ③ 在「启动」文件夹放 Server 快捷方式（登录时由 Explorer 走 ShellExecute 拉起，uiAccess 才生效——
;    计划任务直接拉起拿不到 uiAccess）；④ 装完点 Finish 立即以原用户 ShellExecute 起一次 Server，免得先注销。
; 卸载反向：删旧任务（若有）、杀 Server、反注册 DLL，删文件与用户数据（%APPDATA%\CloudIME、%LOCALAPPDATA%\CloudIME）；
; DLL 被应用占用时登记成重启后删并提示重启；启动快捷方式 Inno 自动删。
;
; 升级：DLL 被加载进每个应用进程，文件锁着覆盖不了，所以 DLL 按版本起名（cloudime_tsf-<版本>.dll）并排装，
; 注册新的，旧的装完后删（删不掉的登记成重启后删）；已开着的应用继续用旧 DLL 直到重启，Server 两个版本都服务。
; Inno 的 CloseApplications 会用 Restart Manager 找出所有加载了 *.dll 的进程要求关闭——对输入法 DLL 就是关一切，故关掉；
; 只有 Server / 设置程序两个 exe 要覆盖，安装前自己 taskkill。
; Server 杀掉后，正在打字的应用里 DLL 连不上会自己把它拉起来（见 tsf 的 launch.rs），又占住 exe 和 mmap 着的数据：
; 安装 / 卸载期间持有互斥体 Global\CloudIMEInstaller，新 DLL 看到它就不拉；旧版 DLL 不认得它，
; 所以 Server exe 先改名腾位再杀、[Files] 里排最后装，新 exe 落地前 DLL 拉不起任何 Server。
;
; 版本号由打包脚本用 /DAppVersion=... 传入，缺省 0.0.1。用法见本目录 README.md。

#ifndef AppVersion
  #define AppVersion "0.0.1"
#endif
; VersionInfoVersion 只认 a.b.c.d 数字；版本带预发布后缀（0.0.1-alpha.1）时由打包脚本传去掉后缀的数字版本。
#ifndef AppVersionNumeric
  #define AppVersionNumeric AppVersion
#endif
#define AppName "云朵输入法"
#define Publisher "云朵输入法"
#define WebsiteUrl "https://cloudime.im"
; 脚本相对仓库根（ime/）：installer → windows → apps → ime。
; 打包脚本可用 /DRepo=<目录> 指到另一份「同布局」的暂存目录（如 target\installer\stage），
; 只把它当作根去取文件；输出目录同理可用 /DOutputDir 覆盖。
#ifndef Repo
  #define Repo "..\..\.."
#endif
#ifndef OutputDir
  #define OutputDir Repo + "\target\installer"
#endif
; 按位数起固定名的 TSF DLL（不带版本号；Cargo 输出的是 cloudime_tsf.dll，编完由脚本改名，见文件头「升级」）。
#define TsfDll "cloudime_tsf_x64.dll"
#define TsfDll32 "cloudime_tsf_x86.dll"

[Setup]
AppId={{0E048967-806B-4724-80DD-989308F71342}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#Publisher}
AppSupportURL={#WebsiteUrl}
VersionInfoVersion={#AppVersionNumeric}
; 默认装到系统盘的 Program Files\CloudIME（64 位安装下与 {autopf} 等价，显式写出来是为了明确「系统盘」）。
DefaultDirName={sd}\Program Files\CloudIME
; 目录页始终显示：Inno 缺省是 auto，重装 / 升级同一个 AppId 时会自动跳过它，用户想换目录就没机会。
; 注意 uiAccess 只对装在 %ProgramFiles% 下的程序生效：改到别的目录或别的盘后，候选窗在开始菜单、
; 任务栏搜索这类系统界面里会被盖住（输入与上屏不受影响），所以目录页上给了提示（见 [Messages]）。
DisableDirPage=no
; 开始菜单里建一个 CloudIME 目录（不跟显示名「云朵输入法」走）。
DefaultGroupName=CloudIME
DisableProgramGroupPage=yes
; 升级不沿用上次的安装目录与程序组：一律回到默认目录与新组名（改名前后用过的目录不再沿用）。
; 云朵的 AppId 与青简不同，两个产品独立共存、互不覆盖；旧组由 [InstallDelete] 清掉。
UsePreviousAppDir=no
UsePreviousGroup=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 1809 起（Windows App Runtime 的下限，见 docs\notes\windows-win10.md）。
MinVersion=10.0.17763
PrivilegesRequired=admin
; 别让 Restart Manager 去关所有加载了 DLL 的应用（那是每一个有文本框的应用）。
CloseApplications=no
OutputDir={#OutputDir}
OutputBaseFilename=cloudime-{#AppVersion}-windows-x86_64-setup
SetupIconFile={#Repo}\apps\windows\tsf\resources\cloudime.ico
UninstallDisplayIcon={app}\cloudime.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "chs"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Messages]
; 目录页的说明：改目录会让 uiAccess 失效（候选窗盖不住系统界面），在选目录这一步就讲清楚。
SelectDirLabel3=云朵输入法 将被安装到下面指定的文件夹中。
; 完成页：DLL 会装进每个应用进程，装之前就开着的应用要用新版必须重启（见文件头「升级」）。
; 这是最容易被误解成「设置/新版没生效」的一点，所以放在完成页明说。
FinishedLabel=云朵输入法 已安装在你的计算机上。建议重新启动计算机以便 云朵输入法 的服务程序正常运行。
[Files]
; —— 二进制 ——
; DLL 按版本起名并排装；卸载时若仍被占用，登记成重启后删。
Source: "{#Repo}\target\release\cloudime_tsf_x64.dll";      DestDir: "{app}"; DestName: "{#TsfDll}"; Flags: ignoreversion uninsrestartdelete
Source: "{#Repo}\target\i686-pc-windows-msvc\release\cloudime_tsf_x86.dll"; DestDir: "{app}"; DestName: "{#TsfDll32}"; Flags: ignoreversion uninsrestartdelete
Source: "{#Repo}\target\release\cloudime-settings.exe"; DestDir: "{app}"; Flags: ignoreversion
; 设置程序自带一份 Windows App Runtime（自包含部署：Windows 10 上机器装的框架包用不了，见 docs\notes\windows-win10.md）；
; 文件由 build.ps1 按 settings-runtime.txt 从 target\release 挑进 target\installer\settings-runtime，必须与 exe 同级。
Source: "{#Repo}\target\installer\settings-runtime\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Repo}\apps\windows\tsf\resources\cloudime.ico"; DestDir: "{app}"; Flags: ignoreversion
; 特殊字符输入器：随仓库带的 VFB 成品（apps\windows\ssi\release64\，构建时只拷不编），装在 {app} 根目录。
; Server 按自己 exe 旁的固定名起它（悬浮状态条「特殊字符」按钮，ui/mod.rs 的 open_spec_chars）。
Source: "{#Repo}\apps\windows\ssi\release64\SpecialSymbolsInserter.exe"; DestDir: "{app}"; Flags: ignoreversion
; —— 本地词典（翻译 Tip 用）：词典文件（`.qj` / `.db`）与清单 `dictionaries.list` 一起装进 {app}\LocalDictionary\ ——
; 文件随仓库带（与 cwt-gui、SpecialSymbolsInserter 一样，构建时只拷不编）；Server 与设置程序都从这儿读，
; 设置页「翻译 → 本地词典」的选项就是清单里的显示名。体积不小（三份约 50 MB），换词典直接替换这里的文件即可。
Source: "{#Repo}\LocalDictionary\*"; DestDir: "{app}\LocalDictionary"; Flags: ignoreversion
; —— 命令行工具：exe 与「工具」菜单清单装进 {app}\tools；清单只在没有时铺一份，用户自己加的工具与改过的清单升级时留着 ——
Source: "{#Repo}\target\release\cwt.exe"; DestDir: "{app}\tools"; Flags: ignoreversion
; cwt-gui.exe 是随仓库带的可执行文件（VB6 编的成品，放在 cwt-gui\release64\，构建时只拷不编）
Source: "{#Repo}\apps\windows\tools\cloudime-wordbank-transformer\cwt-gui\release64\cwt-gui.exe"; DestDir: "{app}\tools"; Flags: ignoreversion
Source: "{#Repo}\apps\windows\installer\tools.list"; DestDir: "{app}\tools"; Flags: onlyifdoesntexist
; —— 随包生成数据（只装运行时要的 .qj，不装 dev 中间产物）——
Source: "{#Repo}\data\generated\lm.qj";          DestDir: "{app}\data\generated";       Flags: ignoreversion
; —— 词库：主词库 Dict.db（中文 + 英文合一份）与用户导入的附加词库都放 WordBank\ ——
; Excludes 挡掉开发机自己的 UserWordBank.db（用户自造词库，运行时生成，绝不能进包）。
Source: "{#Repo}\WordBank\*.db";   DestDir: "{app}\WordBank"; Flags: ignoreversion; Excludes: "UserWordBank.db"
; —— 短语库：安装目录 Phrases\Phrase.db（user 与 cloudime_default 两张表）——
; 用户脚本目录：只装「新建脚本」用的模板（用户在设置页里建的脚本不随包走；升级会覆盖模板）。
; lib\ 里放脚本用的工具（纯 Lua 的 md5.lua）与几份完整示例（example-niutrans.lua、example-theme-by-app.lua、
; example-auto-number.lua）—— 它们在子目录里，加载器只认 Scripts\ 根部的 *.lua，所以不会被当成脚本执行。
Source: "{#Repo}\Scripts\template.lua"; DestDir: "{app}\Scripts"; Flags: ignoreversion
Source: "{#Repo}\Scripts\lib\*"; DestDir: "{app}\Scripts\lib"; Flags: ignoreversion

; —— 主题：安装目录 Themes\ 只放**随包默认**主题（只读）；用户自己编辑 / 新建的主题在
; `%APPDATA%\CloudIME\Themes\`（可写，设置页「主题」页写那里）——
Source: "{#Repo}\Themes\*"; DestDir: "{app}\Themes"; Flags: ignoreversion
; onlyifdoesntexist：升级别覆盖用户自己的短语；文件随仓库带（内置短语写在 cloudime_default 表里）。
Source: "{#Repo}\Phrases\Phrase.db"; DestDir: "{app}\Phrases"; Flags: onlyifdoesntexist
; 内置短语的同步源：同一份文件换个名字装进 data\，每次升级都覆盖；Server 启动时把它的 cloudime_default
; 同步进 Phrases\Phrase.db（只在不同时替换，user 表不动），升级也能拿到新的内置短语。
Source: "{#Repo}\Phrases\Phrase.db"; DestDir: "{app}\data"; DestName: "phrase-default.db"; Flags: ignoreversion
; —— 本地整句模型（data\local_models\ 下的 *.qjm；没有就不装，Server 不重排）——
; 目录里可以有多份（不同用途），Server 自己优先词表含汉字的字级模型（见 cloudime-neural 的 find_model），所以整目录带上。
Source: "{#Repo}\data\local_models\*.qjm"; DestDir: "{app}\data\local_models"; Flags: ignoreversion skipifsourcedoesntexist
; —— 随 git 的资源 ——
Source: "{#Repo}\tutorial.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Repo}\lua.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Repo}\assets\sample\dict.tsv";        DestDir: "{app}\assets\sample"; Flags: ignoreversion
; 悬浮状态条的图标按钮：排布表与 icons\ 装在 data\ 下，Server 按 exe 位置读 {app}\data\icons-arrangement.cfg
Source: "{#Repo}\apps\windows\server\src\ui\status\icons-arrangement.cfg"; DestDir: "{app}\data"; Flags: ignoreversion
Source: "{#Repo}\apps\windows\server\src\ui\status\icons\*.svg";           DestDir: "{app}\data\icons"; Flags: ignoreversion
; —— Server 放最后：它一落地，旧版 DLL 就能把它拉起来并占住数据文件（见文件头）——
Source: "{#Repo}\target\release\cloudime-server.exe";   DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\云朵输入法 设置"; Filename: "{app}\cloudime-settings.exe"; IconFilename: "{app}\cloudime.ico"
Name: "{group}\云朵输入法 卸载"; Filename: "{uninstallexe}"
; 登录自启：登录时 Explorer 走 ShellExecute 拉起本快捷方式 → AppInfo 授予 uiAccess，候选窗才能盖过商店 / 任务栏搜索。
; 用 {commonstartup}（所有用户「启动」文件夹）而非 {userstartup}：本安装器是 admin 机器级安装，
; admin 模式下写每用户区会落到「谁提权就写谁」的 profile（Inno 会告警且可能不是目标用户）；
; 机器级「启动」项对每个登录用户都在其会话里由该用户的 Explorer 拉起，仍是 per-user 运行、仍授予 uiAccess。
; （计划任务直接拉起拿不到 uiAccess，故不用 schtasks。）
Name: "{commonstartup}\云朵 Server"; Filename: "{app}\cloudime-server.exe"; WorkingDir: "{app}"; IconFilename: "{app}\cloudime.ico"

[Run]
; ① UWP/AppContainer 应用要能读安装目录才能加载 DLL（*S-1-15-2-1 = ALL APPLICATION PACKAGES，按 SID 与语言无关）。
Filename: "{sys}\icacls.exe"; Parameters: """{app}"" /grant *S-1-15-2-1:(OI)(CI)RX /T /C /Q"; \
  Flags: runhidden waituntilterminated; StatusMsg: "配置应用容器权限…"
; ①.5 词库目录对普通用户可写：导入 / 删除词库要走这里（Program Files 默认只有管理员能写）。S-1-5-32-545 = BUILTIN\Users。
Filename: "{sys}\icacls.exe"; Parameters: """{app}\WordBank"" /grant *S-1-5-32-545:(OI)(CI)M /T /C /Q"; \
  Flags: runhidden waituntilterminated; StatusMsg: "配置词库目录权限…"
; ①.6 短语目录对普通用户可写：「设置 → 短语」页的增删改要写 Phrases\Phrase.db（同样在 Program Files 下）。
Filename: "{sys}\icacls.exe"; Parameters: """{app}\Phrases"" /grant *S-1-5-32-545:(OI)(CI)M /T /C /Q"; \
  Flags: runhidden waituntilterminated; StatusMsg: "配置短语目录权限…"
; ①.7 脚本目录对普通用户可写：「设置 → 脚本」页的新建 / 编辑 / 删除都要写 Scripts\（同样在 Program Files 下）。
Filename: "{sys}\icacls.exe"; Parameters: """{app}\Scripts"" /grant *S-1-5-32-545:(OI)(CI)M /T /C /Q"; \
  Flags: runhidden waituntilterminated; StatusMsg: "配置脚本目录权限…"
; ② 注册文本服务（写 HKCR + 图标到 %ProgramData%\CloudIME\cloudime.ico）。注册的是本版本的 DLL，
;    InprocServer32 指向新文件；旧版本的 DLL **不能** regsvr32 /u（那会把整个 CLSID 注销掉）。
Filename: "{sys}\regsvr32.exe"; Parameters: "/s ""{app}\{#TsfDll}"""; \
  Flags: runhidden waituntilterminated; StatusMsg: "注册输入法…"
; 32 位那份用 SysWOW64 里的 32 位 regsvr32 注册，InprocServer32 才落到 WOW6432Node 下给 32 位进程用。
Filename: "{syswow64}\regsvr32.exe"; Parameters: "/s ""{app}\{#TsfDll32}"""; \
  Flags: runhidden waituntilterminated; StatusMsg: "注册输入法（32 位）…"
; ④ 装完立即起一次 Server 见 [Code] 的 StartServerOnce：装完（ssPostInstall）先无条件起一次，
;    完成页点 Finish 再试一次（已经起过就不重复）。uiAccess=true 的 exe 不能用
;    CreateProcess / runasoriginaluser 拉起（报 740），必须走 ShellExecute（等同双击），
;    所以不能用 [Run] 条目，只能在 [Code] 里 ShellExecAsOriginalUser。

[UninstallRun]
; 反向：先删登录任务、杀 Server / 设置程序、反注册 DLL，Inno 再删文件（DLL 若仍被占用，重启后删）。
Filename: "{sys}\schtasks.exe"; Parameters: "/delete /tn ""CloudIME Server"" /f"; \
  Flags: runhidden; RunOnceId: "DelLogonTask"
Filename: "{sys}\taskkill.exe"; Parameters: "/im cloudime-server.exe /f"; \
  Flags: runhidden; RunOnceId: "KillServer"
Filename: "{sys}\taskkill.exe"; Parameters: "/im cloudime-settings.exe /f"; \
  Flags: runhidden; RunOnceId: "KillSettings"
Filename: "{sys}\regsvr32.exe"; Parameters: "/u /s ""{app}\{#TsfDll}"""; \
  Flags: runhidden; RunOnceId: "UnregDll"
Filename: "{syswow64}\regsvr32.exe"; Parameters: "/u /s ""{app}\{#TsfDll32}"""; \
  Flags: runhidden; RunOnceId: "UnregDll32"

[InstallDelete]
; 早期云朵版本（含改名前后用过的开始菜单目录名）与登录自启快捷方式都清掉，免得新旧两份并存（两条状态条、两个 Server）。卸载新版时会自动清掉新目录。
Type: filesandordirs; Name: "{commonprograms}\云朵输入法"
Type: filesandordirs; Name: "{userprograms}\云朵输入法"
Type: files; Name: "{commonstartup}\CloudIME Server.lnk"
; 更早版本装在当前用户「启动」文件夹里的自启快捷方式：与机器级那份并存会起两个 Server（两条状态条）。
Type: files; Name: "{userstartup}\CloudIME Server.lnk"
; 两个快捷方式都改过名（原「设置 云朵输入法」「卸载 云朵输入法」）：升级时删掉旧的，免得开始菜单里留两份。
Type: files; Name: "{commonprograms}\CloudIME\设置 云朵输入法.lnk"
Type: files; Name: "{userprograms}\CloudIME\设置 云朵输入法.lnk"
Type: files; Name: "{commonprograms}\CloudIME\卸载 云朵输入法.lnk"
Type: files; Name: "{userprograms}\CloudIME\卸载 云朵输入法.lnk"
; 更早版本把模型装在 data\model\（三件套或单个 model.qjm）：现在搬到 data\local_models\，旧目录整棵清掉，免得多占几十 MB。
Type: filesandordirs; Name: "{app}\data\model"
; 0.0.1 开发版的随包码表旧位置
Type: filesandordirs; Name: "{app}\codes"
; 词库搬进 WordBank\ 之前的旧位置（主词库与领域词库）
Type: files; Name: "{app}\data\generated\dict.qj"
Type: filesandordirs; Name: "{app}\data\generated\dicts"
; 词库启用清单 List.dat 已取消：升级时清掉旧版留下的那份
Type: files; Name: "{app}\WordBank\List.dat"

[UninstallDelete]
; 历次升级留下的旧版本 DLL（正常在升级时就删了；仍被占用的会留到这里）。
Type: files; Name: "{app}\cloudime_tsf-*.dll"
Type: files; Name: "{app}\cloudime-server.old-*.exe"
; 用户导入的词库、生成数据这类不在卸载记录里的文件：整目录清掉。
; 仍被应用加载着的 DLL 删不掉，由 [Code] 的 RetireLockedDlls 登记到重启后删。
Type: filesandordirs; Name: "{app}"
; 用户数据：配置、短语（Phrase.db）、学习数据都在 %APPDATA%\CloudIME，日志在 %LOCALAPPDATA%\CloudIME。
; 卸载时一并删除，重装不会恢复；只想留着数据的话把这两条注释掉。
; 这里是机器级安装（PrivilegesRequired=admin），{userappdata} / {localappdata} 指**运行卸载程序的那个用户**
; （也就是本机编译 / 使用输入法的人，UAC 提权不换用户）；同一台机器上其他账户的数据要各自删除。
; ISCC 会为这几条 per-user 路径报 UsedUserAreasWarning，属预期。
Type: filesandordirs; Name: "{userappdata}\CloudIME"
Type: filesandordirs; Name: "{localappdata}\CloudIME"
; 语言栏 / 输入法图标：regsvr32 反注册时 DLL 自己已经删了 cloudime.ico，这里把空目录也清掉。
Type: filesandordirs; Name: "{commonappdata}\CloudIME"

[Code]
var
  { 卸载时是否有输入法 DLL 因为还被应用占用而登记成了「重启后删除」。 }
  RestartNeededForDlls: Boolean;

function CreateMutex(Attributes: Longint; InitialOwner: BOOL; Name: String): THandle;
  external 'CreateMutexW@kernel32.dll stdcall';

{ 安装 / 卸载期间持有的互斥体，名字与 tsf 的 launch.rs 一致；句柄不关，进程退出时系统收回。 }
procedure HoldInstallerMutex;
begin
  if CreateMutex(0, False, 'Global\CloudIMEInstaller') = 0 then
    Log('建安装互斥体失败');
end;

function InitializeSetup: Boolean;
begin
  HoldInstallerMutex;
  Result := True;
end;

function InitializeUninstall: Boolean;
begin
  HoldInstallerMutex;
  Result := True;
end;

{ 运行中的 exe 不能覆盖但能改名：改成 cloudime-server.old-<随机>.exe 腾出名字，旧版 DLL 就拉不起它（见文件头）。
  装完由 DeleteStaleFiles 删掉。 }
procedure RetireServerExe;
var
  Path: String;
begin
  Path := ExpandConstant('{app}\cloudime-server.exe');
  if FileExists(Path) then
    if not RenameFile(Path, ExpandConstant('{app}\cloudime-server.old-') + IntToStr(Random(1000000)) + '.exe') then
      Log('改名旧 Server 失败: ' + Path);
end;

procedure KillProcess(const Image: String);
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/im ' + Image + ' /f', '',
    SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

{ 同版本重装（开发期反复装）：目标文件名与已加载的 DLL 撞名，覆盖不了但 Windows 允许改名，
  先把它改成 <原名>.old-<随机>.dll 腾出名字，装完由 DeleteStaleDlls 删掉 / 登记重启后删。 }
procedure RetireLoadedDll(const Name: String);
var
  Path, Retired: String;
begin
  Path := ExpandConstant('{app}\') + Name;
  if FileExists(Path) then
  begin
    Retired := Path + '.old-' + IntToStr(Random(1000000)) + '.dll';
    if not RenameFile(Path, Retired) then
      Log('改名旧 DLL 失败: ' + Path);
  end;
end;

{ 覆盖前先结束 Server 与设置程序（只有这两个 exe 要覆盖；DLL 按版本并排装，不用关应用）。
  没在跑时 taskkill 返回非 0，忽略。 }
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  RetireServerExe;
  KillProcess('cloudime-server.exe');
  KillProcess('cloudime-settings.exe');
  RetireLoadedDll('{#TsfDll}');
  RetireLoadedDll('{#TsfDll32}');
  Result := '';
end;

{ 清掉旧版本建的「登录自启」计划任务（现在改用「启动」文件夹快捷方式，见 [Icons]）。
  计划任务直接拉起 Server 拿不到 uiAccess，升级安装时删掉它，免得它在登录时抢先以非 uiAccess 方式
  起 Server 并占住命名管道，让快捷方式那份起不来。没有旧任务时 schtasks 返回非 0，忽略即可。 }
procedure DeleteLegacyLogonTask;
var
  ResultCode: Integer;
begin
  Exec('schtasks.exe', '/delete /tn "CloudIME Server" /f', '',
    SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

{ 删掉改名腾位的旧 Server；删不掉的登记成重启后删。 }
procedure DeleteRetiredServers;
var
  Dir, Path: String;
  Found: TFindRec;
begin
  Dir := ExpandConstant('{app}');
  if FindFirst(Dir + '\cloudime-server.old-*.exe', Found) then
  begin
    try
      repeat
        Path := Dir + '\' + Found.Name;
        if not DeleteFile(Path) then
          RestartReplace(Path, '');
      until not FindNext(Found);
    finally
      FindClose(Found);
    end;
  end;
end;

{ 删掉旧版本的 DLL（含没带版本号的最早那份）。仍被某个应用加载着的删不掉，登记成重启后删：
  那些应用重启前继续用旧 DLL，Server 两个版本都服务。 }
procedure DeleteStaleDlls;
var
  Dir, Path: String;
  Found: TFindRec;
begin
  Dir := ExpandConstant('{app}');
  if FindFirst(Dir + '\cloudime_tsf*.dll', Found) then
  begin
    try
      repeat
        if (CompareText(Found.Name, '{#TsfDll}') <> 0) and (CompareText(Found.Name, '{#TsfDll32}') <> 0) then
        begin
          Path := Dir + '\' + Found.Name;
          if not DeleteFile(Path) then
            RestartReplace(Path, '');
        end;
      until not FindNext(Found);
    finally
      FindClose(Found);
    end;
  end;
end;

var
  ServerStarted: Boolean;

{ 以原（非提升）用户身份 ShellExecute 起一次 Server（等同双击）。uiAccess=true 的 exe 不能用
  CreateProcess / runasoriginaluser 拉起（报 740），AppInfo 只在 ShellExecute 时才授予 uiAccess 高 z-band 权限。
  装完（ssPostInstall）与完成页点 Finish 都会调，第二次是空操作——避免起出两个 Server。 }
procedure StartServerOnce;
var
  ErrorCode: Integer;
begin
  if ServerStarted then
    exit;
  ServerStarted := True;
  if not ShellExecAsOriginalUser(
      '', ExpandConstant('{app}\cloudime-server.exe'), '', '',
      SW_SHOWNORMAL, ewNoWait, ErrorCode) then
    Log('启动 Server 失败（错误码 ' + IntToStr(ErrorCode) + '）：登录时会由自启快捷方式拉起。');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    DeleteLegacyLogonTask;
    DeleteStaleDlls;
    DeleteRetiredServers;
    { 覆盖安装时安装前会先杀掉 Server（PrepareToInstall），完成页那步又可能被静默安装 / 提前关窗跳过，
      所以文件一落位就先无条件起一次，保证装完立刻能用。 }
    StartServerOnce;
  end;
end;

{ 卸载时输入法 DLL 还被加载在每个用过输入法的进程里（连 explorer.exe 都在内），文件锁着删不掉：
  删不掉的用 RestartReplace 登记到重启后由系统删除（就是 MoveFileEx 的 DELAY_UNTIL_REBOOT），
  返回是否需要提示用户重启。不先做这一步的话，[UninstallDelete] 只会删失败、文件一直留在安装目录。 }
function RetireLockedDlls: Boolean;
var
  Dir, Path: String;
  Found: TFindRec;
begin
  Result := False;
  Dir := ExpandConstant('{app}');
  if FindFirst(Dir + '\cloudime_tsf*.dll', Found) then
  begin
    try
      repeat
        Path := Dir + '\' + Found.Name;
        if not DeleteFile(Path) then
        begin
          Log('输入法模块仍被占用，登记到重启后删除: ' + Path);
          RestartReplace(Path, '');
          Result := True;
        end;
      until not FindNext(Found);
    finally
      FindClose(Found);
    end;
  end;
end;

{ 卸载：改名旧 Server（[UninstallRun] 的 taskkill 与 regsvr32 /u 都排在后面，不能先把 DLL 删掉，
  否则反注册找不到文件）；文件都处理完再登记占用中的 DLL，最后提示重启。 }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RetireServerExe
  else if CurUninstallStep = usPostUninstall then
    RestartNeededForDlls := RetireLockedDlls
  else if CurUninstallStep = usDone then
  begin
    if RestartNeededForDlls and (not UninstallSilent) then
      MsgBox('输入法模块还在被其它程序占用，已安排在下次重启时删除。' + #13#10 +
             '请重启计算机以完成卸载。设置、短语与学习数据已经删除。',
             mbInformation, MB_OK);
  end;
end;

{ 装完在完成页点 Finish 后立即起一次 Server。
  uiAccess=true 的 exe 不能用 CreateProcess / runasoriginaluser 拉起（报 740），
  必须以原（非提升）用户身份 ShellExecute（等同双击），AppInfo 才会授予 uiAccess 高 z-band 权限。 }
function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpFinished) and (not WizardSilent) then
    StartServerOnce;
end;
