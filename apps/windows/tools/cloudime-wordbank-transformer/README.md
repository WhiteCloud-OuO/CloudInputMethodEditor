# cwt —— 云朵词库转换工具

把第三方词库（`yaml` / `tsv` / `dat`）转成云朵的 `.db` 词库，命令行程序 `cwt.exe`。

```text
cwt <输入文件> <输出.db> [--ignore_head] [--exclude_below NUM] [--exclude_lang chinese|english] [--remove_source 0|1]
```

- 输入一律按 **UTF-8** 文本读（带 BOM 会先剥掉）：`.yaml` / `.yml` 按 Rime 词典（先跳过 `---` 开头的 YAML 头），
  `.tsv` / `.dat` 及其它扩展名按制表符文本（`词<TAB>拼音或编码<TAB>权重`）。
- 输出是单张 `words(text, pinyin, language, weight)` 表 + `meta`（`format 2`），与引擎
  `cloudime_dictionary::DictDb::from_path` 的 format 2 分支一致，可以直接丢进 `WordBank\`。

## 参数

| 参数 | 说明 |
|---|---|
| `--ignore_head` | 忽略文件开头的非格式文本（前言、表头等），直到第一条数据行 |
| `--exclude_below NUM` | 排除权重 **严格小于** `NUM` 的条目（`--exclude_below 100` 删掉权重 1..99） |
| `--exclude_lang LANG` | 排除某种语言：`chinese` / `english`（按词里有没有汉字判） |
| `--remove_source 0\|1` | 转换成功后删除源文件：`1` 删除、`0` 保留（缺省） |

## 一行怎么解析

`词<TAB>拼音或编码<TAB>权重`：

- **中文**（第 1 列含汉字）：`text` = 第 1 列，`pinyin` = 第 2 列（`'` 当音节分隔符换成空格、压掉多余空白、转小写）。
- **英文**（否则）：`text` = 第 2 列的小写（查词编码），`pinyin` = 第 1 列（原样写法）。
- **权重**：第 3 列；没给或不是数字时，只有两列且第 2 列本身是数字就当权重（英文词表常见的 `词<TAB>词频`），
  否则用缺省权重 100（普通组的下限，不会被打成生僻）。
- 空行与 `#` 开头忽略；同一 `(词, 拼音)` 去重，权重大的胜出。

## 窗口外壳 cwt-gui

`cwt-gui/` 是同一工具的窗口外壳（VisualFreeBasic 源码；成品 `cwt-gui/release64/cwt-gui.exe` 随仓库带、构建时只拷不编）：
把 `.yaml` / `.tsv` / `.dat` 文件拖进窗口即转换，调用的是**同目录的 `cwt.exe`**、转完自行退出。安装包把它装进
`{app}\tools\`，`tools.list` 登记的就是它——所以两个 exe 必须同目录。
