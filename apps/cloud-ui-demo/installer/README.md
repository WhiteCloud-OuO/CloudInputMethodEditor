# Cloud UI Demo 独立应用安装包

`apps/cloud-ui-demo` 的参考安装包，用来验证「独立应用打包」这条路线：
一个普通 exe + 图标 + manifest + Inno Setup 安装 / 卸载，不需要输入法或管理员权限。

它**不随云朵输入法安装包发布**；`apps/windows/installer/` 仍是输入法自己的安装包。

## 产物

```text
target\installer\cloud-ui-demo\
    stage\                               # 打包前铺好的安装内容
        cloud-ui-demo.exe
        cloud-ui-demo.ico                # SetupIconFile / 快捷方式图标来源
        README.txt
    cloud-ui-demo-<版本>-windows-x86_64-setup.exe
```

## 组成

- `apps/cloud-ui-demo/resources/cloud-ui-demo.ico`：多尺寸应用图标（16–256）；示例占位图标，
  产品化时替换。`cloud-ui-demo-256.png` 是生成它的源图。
- `apps/cloud-ui-demo/build.rs`：
  - `embed_manifest` 嵌 PerMonitorV2 DPI 感知 manifest（`CloudUI.Demo`，AsInvoker）；
  - `winresource` 把 `cloud-ui-demo.ico` 嵌进 exe（任务栏 / 资源管理器）。
- `installer/cloud-ui-demo.iss`：Inno Setup 7 脚本，按当前用户安装；
- `installer/build.ps1`：release 构建 → stage → 调 ISCC；
- `installer/README.txt`：装进安装目录的简短说明。

## 构建

```powershell
powershell -ExecutionPolicy Bypass -File apps\cloud-ui-demo\installer\build.ps1
# 已编好 release，只重新打包：
powershell -ExecutionPolicy Bypass -File apps\cloud-ui-demo\installer\build.ps1 -SkipBuild
# 只铺 stage，不调 Inno：
powershell -ExecutionPolicy Bypass -File apps\cloud-ui-demo\installer\build.ps1 -SkipBuild -SkipPack
```

版本号取自 `apps/cloud-ui-demo/Cargo.toml`（`version.workspace = true`，即根 `Cargo.toml` 的
`[workspace.package] version`）。ISCC 查找顺序：`CLOUDIME_ISCC` → Inno Setup 7（Program Files / x86）
→ PATH → Inno Setup 6。

## 安装行为

- 安装目录：`{autopf}\Cloud UI Demo`（`PrivilegesRequired=lowest`，通常是
  `%LocalAppData%\Programs\Cloud UI Demo`），不需要管理员；
- 开始菜单：`Cloud UI Demo` 组（启动 + 卸载），桌面快捷方式可选；
- 不写注册表、不写用户数据；程序关闭所有窗口即退出；
- 卸载删安装目录，不触碰其它位置。

## 作为产品模板

复制 `apps/cloud-ui-demo/` 的 `build.rs` / `resources/` / `installer/` 到新产品，至少改：

1. `cloud-ui-demo.iss`：`AppId` 换成新的 GUID（不能和示例共用，否则两个产品互相覆盖）、
   `AppName` / `AppExe` / `DefaultDirName` / `DefaultGroupName`；
2. `build.ps1`：`$AppDir` / `$Exe` / `$Iss` / `$OutDir` 的包名与路径；
3. `build.rs`：manifest 名（`new_manifest("...")`）与图标路径；
4. 如果产品带数据文件，在 `[Files]` 里补、并决定升级是否覆盖 / 用户数据放哪；
   如果必须装到 Program Files 或需要高权限，把 `PrivilegesRequired` 改成 `admin`。
