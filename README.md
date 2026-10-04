# 云朵输入法 CloudIME

> 输入的不只是文字。

云朵输入法（CloudIME）是一个使用 **Rust** 开发的 Windows 输入法。

https://github.com/user-attachments/assets/d145fde9-a641-4543-8b15-dd7a2685de3d

上面这段话全部由云朵输入法输入：整句拼音一口气敲完，停顿一下由本地小模型重排候选。

- 官网：[cloudime.app](https://cloudime.app)
- 下载：[cloudime.app/download](https://cloudime.app/download)（Windows）
- 文档：[cloudime.app/docs](https://cloudime.app/docs)（安装、按键、设置、数据与隐私）
- 反馈：[GitHub Issues](https://github.com/WhiteCloud-OuO/CloudInputMethodEditor/issues/new/choose)
- QQ 群：[902314603](https://qm.qq.com/q/jBvn2gGTxm)（云朵输入法用户内测体验交流群）

---

## 为什么叫「云朵输入法」

「云」是它仅有的联网：检查更新只读官网的版本列表，数据不经过我们，其余一切都在本机完成。

英文名 **CloudIME** 直译就是「云 + 输入法引擎」。

---

## 核心特性

- **整句输入**：连续拼音一口气上屏，本地语言模型按上下文选路；简拼同样可用。
- **拼写纠错与模糊音**：打错的键能猜回来，方言口音（`z/zh`、`n/l` 等）按需打开。
- **中英混输与英文模式**：`C盘`、`hello` 这类中英混合写法，或 Caps Lock 亮起后的纯英文输入。
- **学习你自己的习惯**：选过的词、词与词的接续记在本机，候选越用越顺手。
- **本地整句模型**：随包的小模型在本机给整句候选重新排序，全程离线。

---

## 平台

云朵输入法只提供 Windows 版：核心输入引擎平台无关，壳负责接入系统输入接口与候选窗口。

```text
Windows  → Text Services Framework (TSF) + 独立的输入引擎进程
```

上游项目还做过 macOS 与 Linux 的壳，本仓库把它们删掉了，只维护 Windows 一条线。

---

## 不打算做什么

输入法首先必须是一个好用的输入法。云朵输入法不会：

- 每输入几个词就弹出测试
- 用复杂 UI 干扰正常输入
- 为了附加功能牺牲输入效率

---

## Philosophy

**输入优先。**

**平台只是壳，Core 才是云朵输入法。**

---

## 隐私

**云朵输入法不上传任何数据。** 拼音转换、词库、学习与整句重排全部在本机完成，没有账号，没有统计上报。
检查更新每天向官网读一次版本列表，请求不带任何标识，可在设置的「关于」页关掉；输入日志只写在本机，可以随时关闭和清空。
细节见文档 [数据与日志](https://cloudime.app/docs/help/data-and-logs)。

---

## 许可

代码以 **GPL-3.0-or-later** 发布（见 [LICENSE](LICENSE)）：可以自由使用、修改与再分发，修改后分发须同样开源。
「云朵输入法」名字与 logo 不在授权范围内。云朵输入法在官方渠道免费；若你为获得它向他人付费，你被骗了。

随包数据（词库、语言模型、英文词表）各自遵循来源的许可证，清单见 [docs/design/landscape.md](docs/design/landscape.md)，设置「关于」页也列了一份。

---

## 参与开发

技术架构、设计决定、路线图与工程记录见 [`docs/`](docs/)；改代码前先看 [开发约定](docs/contributing.md)。
欢迎提 issue 与 PR，PR 模板里有合并前清单。

---

## Status

测试版，自用中，正在给少数测试者打包。API、项目结构和功能设计都可能发生较大变化。

---

<p align="center">
  <strong>云朵输入法 CloudIME</strong><br/>
  输入的不只是文字。
</p>
