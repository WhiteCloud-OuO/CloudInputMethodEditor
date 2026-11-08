# 含章 ONNX 导出与对拍

把随包的含章·知微模型（`Models/hanzhang-zhiwei-small.qjm`）用 PyTorch 复刻、导出成 ONNX，
再按 candle 的 `CharScorer::score` 规则逐条对拍。目的是先验证“同一份权重、同一个分数”，
再决定要不要把它接进 GPU（DirectML）推理。

这里只做导出与对拍，不改产品运行路径；默认构建仍然走 candle CPU。

## 依赖

建议单独建虚拟环境（在仓库根目录）：

```bash
python -m venv .venv-model-export
.venv-model-export/Scripts/pip install \
  --index-url https://download.pytorch.org/whl/cpu torch
.venv-model-export/Scripts/pip install safetensors onnx onnxruntime numpy
# 只有 --fp16 需要：
.venv-model-export/Scripts/pip install onnxconverter-common
```

DirectML 对拍用 `onnxruntime-directml` 替换 `onnxruntime`（两个包不能装在同一个环境里）：

```bash
python -m venv .venv-dml
.venv-dml/Scripts/pip install numpy onnxruntime-directml
```

## 导出

```bash
.venv-model-export/Scripts/python tools/model-export/export.py \
  --model Models/hanzhang-zhiwei-small.qjm \
  --out target/hanzhang.onnx \
  --cache
```

导出的是一张**完整序列**模型：输入 `idx [batch, seq]`（int64），输出 `logits [batch, seq, vocab]`，
batch / seq 都是动态轴。加 `--cache` 会再导一份 `hanzhang-cache.onnx`：输入 `ids [batch, seq]`、
`past_k` / `past_v [层, batch, 头, past, 头维]`，输出 `logits` 与新的 `new_k` / `new_v`。
权重是 f32；加 `--fp16` 会再转一份 f16（约 73 MB，f32 约 139 MB）。

## 对拍

先生成 candle 参考分（Rust 侧直接用现在的 `CharScorer`）：

```bash
cargo run --release -p cloudime-neural --example model_scores -- \
  Models/hanzhang-zhiwei-small.qjm target/candle-scores.txt
```

再对：

```bash
.venv-model-export/Scripts/python tools/model-export/parity.py \
  --model Models/hanzhang-zhiwei-small.qjm \
  --onnx target/hanzhang.onnx \
  --candle-scores target/candle-scores.txt \
  --bench

# GPU：
.venv-dml/Scripts/python tools/model-export/parity.py \
  --model Models/hanzhang-zhiwei-small.qjm \
  --onnx target/hanzhang.onnx \
  --candle-scores target/candle-scores.txt \
  --providers dml --bench
```

`--providers dml` 默认按每组实际的 `batch` / `seq` 固定维度（DirectML 在固定维度下明显更快）；
加 `--dynamic` 可看动态维度。用缓存模型对拍时加 `--cache-onnx target/hanzhang-cache.onnx`；
它会先跑一次前文缓存（`prefix`），再用 `[last]+候选` 跑 `step`，与 Rust 的 `prefix_cache` /
`log_probs_after` 同一个形态。

## 本机实测（RTX 3060，f32）

四组用例：空前文 3 候选、短前文 3 候选、8 候选、超过 128 token 的长前文 5 候选。
`score` 总分与 candle 的最大差 **5.8e-4**（ONNX logits 最大差 6.5e-5），来自 f32 矩阵乘的求和顺序，
候选之间通常差 0.1 以上，排序不受影响。

| 用例 | batch × seq | candle CPU | ONNX CPU | ONNX DirectML |
| --- | --- | --- | --- | --- |
| 空前文 | 3 × 7 | 15.3 ms | 4.3 ms | 1.9 ms |
| 短前文 | 3 × 7 | 12.5 ms | 4.4 ms | 1.9 ms |
| 8 候选 | 8 × 33 | 77.1 ms | 27.3 ms | 12.9 ms |
| 长前文 | 5 × 123 | 38.7 ms | 59.7 ms | 28.6 ms |

DirectML 会话存活期间显存约 **+230 MB**（f32 权重 + 运行时）。f16 版本约 73 MB、显存更少、更快，
但 `score` 与 candle 的差会到 **2.6e-2**；对排序通常够用，但做“严格对拍”仍建议 f32。

## 缓存模型对拍

`--cache` 导出的缓存模型与完整序列模型做同样的对拍，`score` 最大差 **5.9e-4**；
`past=0` / `past=3` 的 logits、K、V 也都对得上（logits 约 6.5e-5，K/V 约 1e-5）。

CPU 上缓存模型按“前文一次 + 每批候选一小步”拆分：

| 用例 | 完整序列 | 缓存 prefix | 缓存 step | prefix + step |
| --- | --- | --- | --- | --- |
| 空前文 | 4.3 ms | 0.4 ms | 4.3 ms | 4.7 ms |
| 短前文 | 4.4 ms | 3.9 ms | 4.8 ms | 8.7 ms |
| 8 候选 | 26.9 ms | 4.3 ms | 15.7 ms | 20.0 ms |
| 长前文 | 60.0 ms | 13.8 ms | 7.9 ms | 21.6 ms |

长前文时缓存模型优势最大；同一段前文被多次打分时只付 step 的 7.9 ms。空前文 / 短前文反而完整序列更省
（没有值得缓存的前文）。

DirectML 上缓存模型的 `step` 因为 `past` 是动态维度，反而要 26–33 ms；DirectML 完整序列在 8 候选 / 长前文
是 13.2 / 27.8 ms，目前更划算。要让 DML 缓存也快，下一步得把 `past` 也按固定桶分；这次先不展开。

## 还没做的

- 缓存模型还没按 `past` 固定维度分桶；DirectML 上目前用完整序列更简单。
- 还没做 f16 与 f32 的输出对拍阈值、长上下文边界、空候选等回归集。
- 还没接产品配置 / 安装包；这一步只验证数值和性能。
