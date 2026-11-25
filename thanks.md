# 鸣谢

云朵输入法站在很多人的劳动之上。下面按类别记录直接使用的项目、数据、模型与字体；
新增随包数据或有特殊许可的依赖时，请同步更新本文件，并在对应目录保留许可证原文。

## 特别鸣谢

- **青简输入法**（<https://qingjian.app>）：云朵输入法从青简输入法独立而来，界面与交互的许多基础来自它。
- **[Panic~大佬](https://github.com/yenharvey)**：对云朵输入法的全力支持。

## 输入方案与词库

- **极点五笔社区词库**（[KyleBing/rime-wubi86-jidian](https://github.com/KyleBing/rime-wubi86-jidian)，MIT，Copyright (c) Kyle Bing）：
  五笔 86 主码表 `table.tsv`；许可全文见 `InputSchemes/ShapeCode/wubi/LICENSE-wubi86-jidian.txt`。
- **Rime / 小狼毫**（<https://rime.im/>）：输入法「Core + 薄壳」架构与方案数据的先行者。
- **《通用规范汉字表》**（教育部、国家语委）：汉字字表；iDvel 转录，shengdoushi 分级核对。
- **《现代汉语常用词表（草案）》**（教育部 2008）：常用词与读音；liuxilu 校对版。
- **THUOCL**（清华大学自然语言处理实验室，MIT）：领域词。
- **Unihan**（Unicode Consortium，Unicode License v3）：汉字读音。
- **中文维基百科**（CC BY-SA 4.0）：语言模型语料之一。
- **LCCC**（清华大学 CoAI，MIT）：语言模型语料之一。
- **ESDB / SCOWL**（[en-wl/wordlist](https://github.com/en-wl/wordlist)，© Kevin Atkinson，MIT-like）：英文词表。
- **CSpell software-terms**（[cspell-dicts](https://github.com/streetsidesoftware/cspell-dicts)，MIT）：英文技术词。
- **typos**（[crate-ci/typos](https://github.com/crate-ci/typos)，MIT / Apache-2.0，本包采用 MIT）：误拼对照。
- **wordfreq**（[rspeer/wordfreq](https://github.com/rspeer/wordfreq)，代码 MIT，词频数据按 CC BY-SA）：英文词频。

## 字体

- **更纱黑体 Sarasa Gothic**（[be5invis/Sarasa-Gothic](https://github.com/be5invis/Sarasa-Gothic)，OFL 1.1）：
  字根字体 `zigen.ttf` 由它的子集裁剪而来，许可见 `InputSchemes/ShapeCode/wubi/OFL.txt`。

## 本地模型

- **[知意输入法](https://github.com/dd1000001000/zhiyi_ime)的作者**：知意 1.4V2-Long-L12 模型权重（Apache-2.0），用于增强 / 混合档的候选重排与按音节出字。
- **ONNX Runtime**（[microsoft/onnxruntime](https://github.com/microsoft/onnxruntime)，MIT）：知意模型推理。
- **Candle**（[huggingface/candle](https://github.com/huggingface/candle)，MIT / Apache-2.0）：含章·知微模型推理。

## 开源软件

- **Rust** 与 **crates.io** 上的所有依赖维护者。
- **cosmic-text**（[pop-os/cosmic-text](https://github.com/pop-os/cosmic-text)，含 [qingjian-team](https://github.com/qingjian-team/cosmic-text) 的光学字号补丁）：文字整形与排版。
- **tiny-skia**、**resvg**：绘制与 SVG 栅格化。
- **OpenCC**（[BYVoid/OpenCC](https://github.com/BYVoid/OpenCC)，经 `ferrous-opencc`）：简繁转换。
- **SQLite**（公有领域）：词库存档。
- **LuaJIT** 与 **mlua**：用户脚本运行时。
- **Windows API 绑定**（[microsoft/windows-rs](https://github.com/microsoft/windows-rs)）：Win32 / COM / TSF 调用。

## 还有

- 所有测试、反馈、提 issue、写脚本与帮助传播云朵输入法的朋友。

维护约定：只用于理解思路、没有随包分发的内容不列入本清单；随包数据与许可策略以
[`docs/design/landscape.md`](docs/design/landscape.md) 为准。
