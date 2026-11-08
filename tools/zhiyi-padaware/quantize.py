"""把知意 int8 ONNX 的编码器动态量化改成「只按有效 token 算 min/max」。

原模型的 64 个 `DynamicQuantizeLinear` 是按整个激活张量（含 padding）算 scale 的，
所以固定维度 padding 会改掉 scale，即使 attention_mask 正确也会改候选分。
这个脚本只改编码器里形状为 `[1, seq, hidden]` 的那批 DQL 节点：用 `attention_mask`
把 padding 位置排除后再算 scale，scorer 里按 marker / slot gather 出来的 DQL 不动。

输出模型与原模型：
- 不 padding 时输出逐位一致；
- padding 到固定 seq 后，有效位置的输出也逐位一致（logits / char_logits 差为 0）。

用法：

    python tools/zhiyi-padaware/quantize.py \
        --model Models/laya-zhen-long-l12-int8g/laya.int8g.onnx \
        --out target/laya.int8g.padaware.onnx
"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
import onnx
import onnxruntime as ort

ENCODER_PREFIXES = ("/encoder/", "/layers.")
MASK = "attention_mask"


def is_encoder(name: str) -> bool:
    return name.startswith(ENCODER_PREFIXES)


def quantize(model: onnx.ModelProto) -> onnx.ModelProto:
    graph = model.graph
    cast_mask = "padaware_mask_bool"
    mask_3d = "padaware_mask_3d"
    mask_nodes = [
        onnx.helper.make_node(
            "Cast",
            [MASK],
            [cast_mask],
            name="padaware_cast_mask",
            to=onnx.TensorProto.BOOL,
        ),
        onnx.helper.make_node(
            "Unsqueeze",
            [cast_mask, "padaware_axes"],
            [mask_3d],
            name="padaware_unsqueeze_mask",
        ),
    ]
    for name, dtype, value in (
        ("padaware_axes", onnx.TensorProto.INT64, [2]),
        ("padaware_pos_inf", onnx.TensorProto.FLOAT, [np.inf]),
        ("padaware_neg_inf", onnx.TensorProto.FLOAT, [-np.inf]),
        ("padaware_255", onnx.TensorProto.FLOAT, [255.0]),
        ("padaware_zero", onnx.TensorProto.FLOAT, [0.0]),
    ):
        graph.initializer.append(onnx.helper.make_tensor(name, dtype, [len(value)], value))

    nodes = list(mask_nodes)
    counter = 0
    for node in graph.node:
        if node.op_type != "DynamicQuantizeLinear" or not is_encoder(node.input[0]):
            nodes.append(node)
            continue
        source = node.input[0]
        quantized, scale_out, zero_out = node.output
        prefix = f"padaware_{counter}"
        x_min = prefix + "_x_min"
        x_max = prefix + "_x_max"
        low = prefix + "_min"
        high = prefix + "_max"
        scale = prefix + "_scale"
        zero_float = prefix + "_zero_float"
        zero = prefix + "_zero"
        nodes.extend(
            [
                onnx.helper.make_node(
                    "Where",
                    [mask_3d, source, "padaware_pos_inf"],
                    [x_min],
                    name=prefix + "_where_min",
                ),
                onnx.helper.make_node(
                    "Where",
                    [mask_3d, source, "padaware_neg_inf"],
                    [x_max],
                    name=prefix + "_where_max",
                ),
                onnx.helper.make_node(
                    "ReduceMin", [x_min], [low], name=prefix + "_reduce_min", keepdims=0
                ),
                onnx.helper.make_node(
                    "ReduceMax", [x_max], [high], name=prefix + "_reduce_max", keepdims=0
                ),
                onnx.helper.make_node("Sub", [high, low], [prefix + "_range"], name=prefix + "_sub"),
                onnx.helper.make_node(
                    "Div", [prefix + "_range", "padaware_255"], [scale], name=prefix + "_scale_div"
                ),
                onnx.helper.make_node("Neg", [low], [prefix + "_neg_min"], name=prefix + "_neg"),
                onnx.helper.make_node(
                    "Div", [prefix + "_neg_min", scale], [prefix + "_zero_div"], name=prefix + "_zero_div"
                ),
                onnx.helper.make_node("Round", [prefix + "_zero_div"], [zero_float], name=prefix + "_round"),
                onnx.helper.make_node(
                    "Clip",
                    [zero_float, "padaware_zero", "padaware_255"],
                    [prefix + "_clip"],
                    name=prefix + "_clip",
                ),
                onnx.helper.make_node(
                    "Cast", [prefix + "_clip"], [zero], name=prefix + "_cast_zero", to=onnx.TensorProto.UINT8
                ),
                onnx.helper.make_node(
                    "QuantizeLinear", [source, scale, zero], [quantized], name=prefix + "_quant"
                ),
                onnx.helper.make_node("Identity", [scale], [scale_out], name=prefix + "_scale_out"),
                onnx.helper.make_node("Identity", [zero], [zero_out], name=prefix + "_zero_out"),
            ]
        )
        counter += 1
    del graph.node[:]
    graph.node.extend(nodes)
    return model


def sample(seq: int, markers: int = 4, slots: int = 4) -> dict[str, np.ndarray]:
    rng = np.random.default_rng(0)
    return {
        "input_ids": rng.integers(0, 1000, (1, seq), dtype=np.int64),
        "attention_mask": np.ones((1, seq), dtype=np.int64),
        "unit_ids": rng.integers(0, 10, (1, seq), dtype=np.int64),
        "marker_pos": rng.choice(seq, markers, replace=False).astype(np.int64).reshape(1, markers),
        "marker_mask": np.ones((1, markers), dtype=np.bool_),
        "slot_pos": rng.choice(seq, slots, replace=False).astype(np.int64).reshape(1, slots),
    }


def pad(data: dict[str, np.ndarray], bucket: int) -> dict[str, np.ndarray]:
    seq = data["input_ids"].shape[1]
    if seq >= bucket:
        return data
    extra = bucket - seq
    out = dict(data)
    for name in ("input_ids", "unit_ids"):
        out[name] = np.pad(data[name], ((0, 0), (0, extra)), constant_values=0)
    out["attention_mask"] = np.pad(data["attention_mask"], ((0, 0), (0, extra)), constant_values=0)
    return out


def validate(original: Path, quantized: Path) -> None:
    base = ort.InferenceSession(str(original), providers=["CPUExecutionProvider"])
    changed = ort.InferenceSession(str(quantized), providers=["CPUExecutionProvider"])
    for seq in (31, 64, 97):
        data = sample(seq)
        a = base.run(None, data)
        b = changed.run(None, data)
        unpadded = max(np.abs(x - y).max() for x, y in zip(a, b))
        padded_data = pad(data, seq + 17)
        c = changed.run(None, padded_data)
        padded = max(np.abs(x - y).max() for x, y in zip(b, c))
        print(f"seq={seq}: 不 padding vs 原模型 {unpadded:.1e}，padding vs 不 padding {padded:.1e}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--no-validate", action="store_true")
    args = parser.parse_args()

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    model = onnx.load(args.model)
    onnx.save(quantize(model), str(out))
    print(f"写出 {out}（{out.stat().st_size / 1024 / 1024:.1f} MB）")
    if not args.no_validate:
        validate(Path(args.model), out)


if __name__ == "__main__":
    main()
