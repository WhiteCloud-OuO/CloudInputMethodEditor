# 车牌一览（原始数据）

`车牌一览.xlsx` 是用户提供的全国车牌简称与对应地名表：第一张表 `Table` 两列

| 车牌 | 对应地名 |
|---|---|
| `京A、C、E…` | `北京市` |
| `冀A` | `河北省石家庄市` |

一行可以写多个字母（`苏E、苏U`、`京A、C、E`）。生成时拆成一条条「省份简称 + 字母」，
按省份简称的拼音 + 字母建 key（`苏A` → `sua`），并派生出短地名（`江苏省南京市` → `江苏南京`）。

生成命令（输出 `data/generated/plates.qj`，安装包再把它放到安装目录 `data\plates.qj`）：

```powershell
cargo run --release -p cloudime-dict-convert -- plate --out data/generated/plates.qj
```

文件格式见 `crates/cloudime-plate`；读取端是 `cloudime_plate::PlateTable`（mmap，`Kind::Plate`）。
原始数据由用户提供，未附来源许可证；如后续要随安装包分发，请在这里补上来源与许可。
