; Cloud UI Demo 独立应用安装脚本（Inno Setup 7）。
;
; 只装一个 exe + 说明文件，按当前用户安装到 {autopf}\Cloud UI Demo（非管理员），
; 建开始菜单快捷方式；程序本身不写注册表 / 用户数据，卸载直接删目录。
; 版本号、暂存目录与输出目录由 build.ps1 用 /D 传进来，也可手动：
;   iscc /DAppVersion=0.0.3 /DStage=... /DOutputDir=... apps\cloud-ui-demo\installer\cloud-ui-demo.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef AppVersionNumeric
  #define AppVersionNumeric AppVersion
#endif
#ifndef Stage
  #define Stage "..\..\..\target\installer\cloud-ui-demo\stage"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\..\target\installer\cloud-ui-demo"
#endif

#define AppName "Cloud UI Demo"
#define AppPublisher "CloudIME"
#define AppExe "cloud-ui-demo.exe"

[Setup]
; 这个 AppId 只属于 Cloud UI Demo，与云朵输入法安装包互不影响。
AppId={{A154419D-B252-4FEF-9891-4E353726BB30}
AppName={#AppName}
AppVerName={#AppName} {#AppVersion}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
VersionInfoVersion={#AppVersionNumeric}
DefaultDirName={autopf}\Cloud UI Demo
DefaultGroupName=Cloud UI Demo
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir={#OutputDir}
OutputBaseFilename=cloud-ui-demo-{#AppVersion}-windows-x86_64-setup
SetupIconFile={#Stage}\cloud-ui-demo.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "chs"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加任务："; Flags: unchecked

[Files]
Source: "{#Stage}\cloud-ui-demo.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\README.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Cloud UI Demo"; Filename: "{app}\{#AppExe}"
Name: "{group}\卸载 Cloud UI Demo"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Cloud UI Demo"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "启动 Cloud UI Demo"; Flags: nowait postinstall skipifsilent
