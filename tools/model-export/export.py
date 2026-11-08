"""把含章模型导出成 ONNX（完整序列，不做 K/V 缓存）。

用法（在仓库根目录）：

    python tools/model-export/export.py \
        --model Models/hanzhang-zhiwei-small.qjm \
        --out target/hanzhang.onnx

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


def convert_fp16(src: Path, dst: Path) -> None:
    import onnx
    from onnxconverter_common import float16

    model = onnx.load(str(src))
    converted = float16.convert_float_to_float16(model, keep_io_types=True)
    onnx.save(converted, str(dst))


def validate(path: Path, model: hanzhang.CharLM, index: dict[str, int]) -> None:
    import onnxruntime as ort

    ids = torch.tensor([[hanzhang.EOS] + hanzhang.encode(index, "今天天气不错")], dtype=torch.long)
    with torch.no_grad():
        reference = model(ids)[0].numpy()
    session = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    got = session.run(["logits"], {"idx": ids.numpy()})[0]
    diff = np.abs(reference - got)
    print(f"validate: logits max={diff.max():.3e} mean={diff.mean():.3e}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, help=".qjm 文件或三件套目录")
    parser.add_argument("--out", required=True, help="输出 .onnx")
    parser.add_argument("--opset", type=int, default=17)
    parser.add_argument("--fp16", action="store_true", help="先导 f32 再转 fp16（体积约一半）")
    args = parser.parse_args()

    config, vocab_text, weights_bytes = read_model(args.model)
    from safetensors.torch import load as load_safetensors

    weights = load_safetensors(weights_bytes)
    index = hanzhang_index(vocab_text)
    model = hanzhang.CharLM(config, weights).eval()

    out = Path(args.out)
    export_full(model, out, args.opset)

    actual = out
    if args.fp16:
        fp16_out = out.with_name(out.stem + "-fp16.onnx")
        convert_fp16(out, fp16_out)
        print(f"fp16: {fp16_out} ({fp16_out.stat().st_size / 1024 / 1024:.1f} MB)")
        actual = fp16_out

    print(f"onnx: {actual} ({actual.stat().st_size / 1024 / 1024:.1f} MB)")
    validate(actual, model, index)


def hanzhang_index(vocab_text: str) -> dict[str, int]:
    from qjm import vocab_index

    return vocab_index(vocab_text)


if __name__ == "__main__":
    main()
