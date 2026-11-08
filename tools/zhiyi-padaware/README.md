# 知意固定维度：padding 感知的动态量化

知意模型（`laya.int8g.onnx`）里的 64 个 `DynamicQuantizeLinear` 是按**整个激活张量**算
scale / zero point 的。直接把它 padding 到固定 `seq_len`，即使 `attention_mask` 标对、编码器
mask 也确实是 `-inf`，量化 scale 仍会被 padding 拉开，候选分会变。

## 先试过静态量化，不够用

把 64 个 DQL 全部换成固定 scale（80 个校准样本、20 个验证样本，min/max 与 99.9% / 99% 两种截断）：

| 方案 | logits 最大差 | logits 平均差 | char_logits 最大差 | top1 翻转 |
| --- | --- | --- | --- | --- |
| 全局 min/max | 0.91 | 0.49 | 1.65 | 6 / 20 |
| 99.9% 截断 | 1.05 | 0.46 | 1.67 | 7 / 20 |
| 99% 截断 | 0.91 | 0.42 | 2.19 | 5 / 20 |

静态 per-tensor scale 比原来的 per-input 动态 scale 粗太多，排序会改；不接。

## 现在用的办法：只按有效 token 算 DQL 的 min/max

`quantize.py` 只改**编码器**里形状为 `[1, seq, hidden]` 的 DQL 节点（`/encoder/...` 与 `/layers.*`）：

1. 用 `attention_mask` 把 padding 位置换成 `+inf` / `-inf`；
2. `ReduceMin` / `ReduceMax` 只看有效 token；
3. 用这个 min/max 算 scale / zero point，再 `QuantizeLinear`。

scorer 里按 marker / slot gather 出来的 DQL（形状 `[1, markers, hidden]`）本来就不含 padding，保持原样。

结果：

- 不 padding：输出与原模型**逐位一致**
- padding 到固定 seq 后：有效位置输出与不 padding 也**逐位一致**
  （logits / char_logits 差均为 0）

## 性能（RTX 3060，固定 seq，markers/slots 动态）

| seq | 原模型 CPU | padaware CPU | 原模型 DML 固定 | padaware DML 固定 |
| --- | --- | --- | --- | --- |
| 64 | 9.4 ms | 12.2 ms | 5.7 ms | 6.6 ms |
| 128 | 16.6 ms | 21.7 ms | 7.5 ms | 8.5 ms |
| 256 | 35.5 ms | 44.4 ms | 14.3 ms | 13.3 ms |
| 320 | 47.4 ms | 60.9 ms | 14.7 ms | 16.6 ms |

padaware 多了一点 mask + Reduce 的开销，但 DirectML 固定维度仍然比 CPU 快 2–3 倍；
现在 padding 是安全的，可以按 `seq_len` 建桶。

## 用法

```bash
python tools/zhiyi-padaware/quantize.py \
  --model Models/laya-zhen-long-l12-int8g/laya.int8g.onnx \
  --out target/laya.int8g.padaware.onnx
```

脚本默认会跑三组随机输入验证“不 padding 对原模型”和“padding 对不 padding”。

## 还没接的

- Rust 侧还没按 `seq_len` 建桶、padding、按桶缓存 DML 会话；这一步只把模型改成 padding 安全。
- DirectML 版 `onnxruntime.dll` 还没换进安装包；目前只是可行性验证。
