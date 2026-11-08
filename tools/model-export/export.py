"""把含章模型导出成 ONNX（完整序列；可选带 K/V 缓存的版本）。

用法（在仓库根目录）：

    python tools/model-export/export.py \
        --model Models/hanzhang-zhiwei-small.qjm \
        --out target/hanzhang.onnx

    # 再导一份带 past_k / past_v 的缓存模型
    python tools/model-export/export.py \
        --model Models/hanzhang-zhiwei-small.qjm \
        --out target/hanzhang.onnx --cache

依赖：`pip install torch --index-url https://download.pytorch.org/whl/cpu`、`safetensors`、`onnx`、
`onnxruntime`；`--fp16` 还需要 `onnxconverter-common`。
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
import torch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import model as hanzhang
from qjm import read_model


def export_full(model: hanzhang.CharLM, out: Path, opset: int) -> None:
    wrapper = hanzhang.FullModel(model).eval()
    dummy = torch.zeros(1, 8, dtype=torch.long)
    out.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        wrapper,
        (dummy,),
        str(out),
        input_names=["idx"],
        output_names=["logits"],
        dynamic_axes={"idx": {0: "batch", 1: "seq"}, "logits": {0: "batch", 1: "seq"}},
        opset_version=opset,
        do_constant_folding=True,
        dynamo=False,
    )


def export_cache(model: hanzhang.CharLM, config: dict, out: Path, opset: int) -> None:
    wrapper = hanzhang.CacheModel(model).eval()
    layers = config["n_layer"]
    heads = config["n_head"]
    dim = config["n_embd"] // heads
    ids = torch.zeros(1, 5, dtype=torch.long)
    past_keys = torch.zeros(layers, 1, heads, 3, dim)
    past_values = torch.zeros(layers, 1, heads, 3, dim)
    out.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        wrapper,
        (ids, past_keys, past_values),
        str(out),
        input_names=["ids", "past_k", "past_v"],
        output_names=["logits", "new_k", "new_v"],
        dynamic_axes={
            "ids": {0: "batch", 1: "seq"},
            "past_k": {1: "batch", 3: "past"},
            "past_v": {1: "batch", 3: "past"},
            "logits": {0: "batch", 1: "seq"},
            "new_k": {1: "batch", 2: "seq"},
            "new_v": {1: "batch", 2: "seq"},
        },
        opset_version=opset,
        do_constant_folding=True,
        dynamo=False,
    )


def convert_fp16(src: Path, dst: Path) -> None:
    import onnx
    from onnxconverter_common import float16

    model = onnx.load(str(src))
    converted = float16.convert_float_to_float16(model, keep_io_types=True)
    onnx.save(converted, str(dst))


def validate_full(path: Path, model: hanzhang.CharLM, index: dict[str, int]) -> None:
    import onnxruntime as ort

    ids = torch.tensor([[hanzhang.EOS] + hanzhang.encode(index, "今天天气不错")], dtype=torch.long)
    with torch.no_grad():
        reference = model(ids)[0].numpy()
    session = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    got = session.run(["logits"], {"idx": ids.numpy()})[0]
    diff = np.abs(reference - got)
    print(f"  完整序列 logits max={diff.max():.3e} mean={diff.mean():.3e}")


def validate_cache(path: Path, model: hanzhang.CharLM, config: dict, index: dict[str, int]) -> None:
    import onnxruntime as ort

    layers = config["n_layer"]
    heads = config["n_head"]
    dim = config["n_embd"] // heads
    session = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    ids = torch.tensor([[hanzhang.EOS] + hanzhang.encode(index, "今天天气不错")], dtype=torch.long)
    for past_len in (0, 3):
        keys = torch.zeros(layers, 1, heads, past_len, dim)
        values = torch.zeros(layers, 1, heads, past_len, dim)
        with torch.no_grad():
            reference = hanzhang.CacheModel(model)(
                ids, keys, values
            )
        got = session.run(
            None,
            {"ids": ids.numpy(), "past_k": keys.numpy(), "past_v": values.numpy()},
        )
        diffs = [np.abs(a.numpy() - b).max() for a, b in zip(reference, got)]
        print(
            f"  缓存 past={past_len}: logits {diffs[0]:.3e} / k {diffs[1]:.3e} / v {diffs[2]:.3e}"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, help=".qjm 文件或三件套目录")
    parser.add_argument("--out", required=True, help="输出 .onnx")
    parser.add_argument("--opset", type=int, default=17)
    parser.add_argument("--cache", action="store_true", help="再导一份带 past_k / past_v 的缓存模型")
    parser.add_argument("--fp16", action="store_true", help="转 fp16（体积约一半）")
    args = parser.parse_args()

    config, vocab_text, weights_bytes = read_model(args.model)
    from safetensors.torch import load as load_safetensors

    weights = load_safetensors(weights_bytes)
    index = hanzhang_index(vocab_text)
    model = hanzhang.CharLM(config, weights).eval()

    out = Path(args.out)
    export_full(model, out, args.opset)
    print(f"完整序列: {out} ({out.stat().st_size / 1024 / 1024:.1f} MB)")
    validate_full(out, model, index)

    cache_out = None
    if args.cache:
        cache_out = out.with_name(out.stem + "-cache.onnx")
        export_cache(model, config, cache_out, args.opset)
        print(f"缓存: {cache_out} ({cache_out.stat().st_size / 1024 / 1024:.1f} MB)")
        validate_cache(cache_out, model, config, index)

    if args.fp16:
        for path, kind in ((out, "完整序列"), (cache_out, "缓存")):
            if path is None:
                continue
            fp16_path = path.with_name(path.stem + "-fp16.onnx")
            convert_fp16(path, fp16_path)
            print(f"{kind} fp16: {fp16_path} ({fp16_path.stat().st_size / 1024 / 1024:.1f} MB)")


def hanzhang_index(vocab_text: str) -> dict[str, int]:
    from qjm import vocab_index

    return vocab_index(vocab_text)


if __name__ == "__main__":
    main()
