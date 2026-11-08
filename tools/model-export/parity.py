"""对拍：同一份含章模型，candle（Rust）与导出后的 ONNX 算出的候选总分。

先生成 candle 参考分：

    cargo run --release -p cloudime-neural --example model_scores -- \
        Models/hanzhang-zhiwei-small.qjm target/candle-scores.txt

再跑：

    python tools/model-export/parity.py --model Models/hanzhang-zhiwei-small.qjm \
        --onnx target/hanzhang.onnx --candle-scores target/candle-scores.txt

加 `--providers dml` 走 DirectML；`--dynamic` 不固定 batch / seq（默认按每组实际形状固定，
DirectML 在固定形状下明显更快）。
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))

from qjm import EOS, encode, read_model, vocab_index


def parse_candle(path: Path) -> list[tuple[str, list[tuple[str, float]]]]:
    cases: list[tuple[str, list[tuple[str, float]]]] = []
    context = ""
    current: list[tuple[str, float]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("CONTEXT\t"):
            context = line.split("\t", 1)[1]
            current = []
        elif line.startswith("SCORE\t"):
            _, candidate, value = line.split("\t")
            current.append((candidate, float(value)))
        elif line == "END":
            cases.append((context, current))
    return cases


def build_batch(
    config: dict,
    index: dict[str, int],
    context: str,
    candidates: list[str],
) -> tuple[np.ndarray, int, list[list[int]]]:
    limit = config["context"]
    full = [EOS] + encode(index, context)
    tails = []
    for candidate in candidates:
        ids = encode(index, candidate)
        if len(ids) > limit - 1:
            ids = ids[-(limit - 1) :]
        tails.append(ids)
    width = max((len(tail) for tail in tails), default=0) + 1
    prefix = full[max(0, len(full) - (limit - width + 1)) :]
    rows = [prefix + tail + [0] * (width - 1 - len(tail)) for tail in tails]
    return np.array(rows, dtype=np.int64), len(prefix), tails


def log_softmax(values: np.ndarray) -> np.ndarray:
    shifted = values - values.max(axis=-1, keepdims=True)
    return shifted - np.log(np.exp(shifted).sum(axis=-1, keepdims=True))


def open_session(path: Path, rows: np.ndarray, providers: str, dynamic: bool):
    import onnxruntime as ort

    options = ort.SessionOptions()
    options.log_severity_level = 3
    if providers == "dml" and not dynamic:
        for name, value in (("batch", rows.shape[0]), ("seq", rows.shape[1])):
            options.add_free_dimension_override_by_name(name, value)
    names = (
        ["DmlExecutionProvider", "CPUExecutionProvider"]
        if providers == "dml"
        else ["CPUExecutionProvider"]
    )
    return ort.InferenceSession(str(path), sess_options=options, providers=names)


def scores(session, rows: np.ndarray, prefix_len: int, tails: list[list[int]]) -> list[float]:
    logits = session.run(["logits"], {"idx": rows})[0]
    log_probs = log_softmax(logits.astype(np.float64))
    result = []
    for row, tail in enumerate(tails):
        total = 0.0
        for offset, token in enumerate(tail):
            total += log_probs[row, prefix_len - 1 + offset, token]
        result.append(float(total))
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, help=".qjm 或三件套目录（只用来取 config / vocab）")
    parser.add_argument("--onnx", required=True)
    parser.add_argument("--candle-scores", required=True)
    parser.add_argument("--providers", choices=["cpu", "dml"], default="cpu")
    parser.add_argument("--dynamic", action="store_true", help="不固定 batch / seq")
    parser.add_argument("--bench", action="store_true", help="每组再跑 10 次取平均")
    args = parser.parse_args()

    config, vocab_text, _ = read_model(args.model)
    index = vocab_index(vocab_text)
    cases = parse_candle(Path(args.candle_scores))
    worst = 0.0
    for context, expected in cases:
        candidates = [candidate for candidate, _ in expected]
        rows, prefix_len, tails = build_batch(config, index, context, candidates)
        session = open_session(Path(args.onnx), rows, args.providers, args.dynamic)
        got = scores(session, rows, prefix_len, tails)
        print(f"CONTEXT {context[:24]!r} batch={rows.shape[0]} seq={rows.shape[1]}")
        for (candidate, want), value in zip(expected, got):
            diff = abs(value - want)
            worst = max(worst, diff)
            print(f"  {candidate:14s} candle={want: .6f} onnx={value: .6f} diff={diff:.2e}")
        if args.bench:
            for _ in range(3):
                session.run(None, {"idx": rows})
            elapsed = []
            for _ in range(10):
                started = time.perf_counter()
                session.run(None, {"idx": rows})
                elapsed.append((time.perf_counter() - started) * 1000)
            print(f"  bench: {np.mean(elapsed):.2f} ms（min {np.min(elapsed):.2f}）")
    print(f"worst |diff| = {worst:.2e}")


if __name__ == "__main__":
    main()
