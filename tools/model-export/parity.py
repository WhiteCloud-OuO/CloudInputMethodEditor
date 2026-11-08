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


def build_cache_batch(
    config: dict,
    index: dict[str, int],
    context: str,
    candidates: list[str],
) -> tuple[list[int], np.ndarray, np.ndarray, list[list[int]]]:
    """Rust `score` 的输入拆分：共享前文 `head` + 每行 `[last]+候选` 与目标 token。"""
    limit = config["context"]
    full = [EOS] + encode(index, context)
    tails = []
    for candidate in candidates:
        ids = encode(index, candidate)
        if len(ids) > limit - 1:
            ids = ids[-(limit - 1) :]
        tails.append(ids)
    width = max((len(tail) for tail in tails), default=0) + 1
    last = full[-1]
    head = full[:-1]
    head = head[max(0, len(head) - (limit - width)) :]
    flat = np.zeros((len(tails), width), dtype=np.int64)
    targets = np.zeros((len(tails), width), dtype=np.int64)
    for row, tail in enumerate(tails):
        flat[row, 0] = last
        flat[row, 1 : 1 + len(tail)] = tail
        targets[row, : len(tail)] = tail
    return head, flat, targets, tails


def open_cache_session(path: Path, providers: str):
    import onnxruntime as ort

    options = ort.SessionOptions()
    options.log_severity_level = 3
    names = (
        ["DmlExecutionProvider", "CPUExecutionProvider"]
        if providers == "dml"
        else ["CPUExecutionProvider"]
    )
    return ort.InferenceSession(str(path), sess_options=options, providers=names)


def cache_scores(
    session,
    config: dict,
    head: list[int],
    flat: np.ndarray,
    targets: np.ndarray,
    tails: list[list[int]],
) -> list[float]:
    layers = config["n_layer"]
    heads = config["n_head"]
    dim = config["n_embd"] // heads
    empty = np.zeros((layers, 1, heads, 0, dim), dtype=np.float32)
    if head:
        ids = np.array([head], dtype=np.int64)
        _, keys, values = session.run(
            None, {"ids": ids, "past_k": empty, "past_v": empty}
        )
    else:
        keys = values = empty
    logits = session.run(
        None, {"ids": flat, "past_k": keys, "past_v": values}
    )[0]
    log_probs = log_softmax(logits.astype(np.float64))
    result = []
    for row, tail in enumerate(tails):
        total = 0.0
        for offset in range(len(tail)):
            total += log_probs[row, offset, targets[row, offset]]
        result.append(float(total))
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, help=".qjm 或三件套目录（只用来取 config / vocab）")
    parser.add_argument("--onnx", required=True, help="完整序列模型")
    parser.add_argument("--cache-onnx", help="带 past_k / past_v 的缓存模型")
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
        if args.cache_onnx:
            head, flat, targets, tails = build_cache_batch(config, index, context, candidates)
            session = open_cache_session(Path(args.cache_onnx), args.providers)
            got = cache_scores(session, config, head, flat, targets, tails)
            print(f"CONTEXT {context[:24]!r} head={len(head)} step={flat.shape[1]}")
        else:
            rows, prefix_len, tails = build_batch(config, index, context, candidates)
            session = open_session(Path(args.onnx), rows, args.providers, args.dynamic)
            got = scores(session, rows, prefix_len, tails)
            print(f"CONTEXT {context[:24]!r} batch={rows.shape[0]} seq={rows.shape[1]}")
        for (candidate, want), value in zip(expected, got):
            diff = abs(value - want)
            worst = max(worst, diff)
            print(f"  {candidate:14s} candle={want: .6f} onnx={value: .6f} diff={diff:.2e}")
        if args.bench:
            if args.cache_onnx:
                layers = config["n_layer"]
                heads = config["n_head"]
                dim = config["n_embd"] // heads
                empty = np.zeros((layers, 1, heads, 0, dim), dtype=np.float32)
                ids = np.array([head], dtype=np.int64) if head else np.zeros((1, 0), dtype=np.int64)
                for _ in range(3):
                    session.run(None, {"ids": ids, "past_k": empty, "past_v": empty})
                    session.run(None, {"ids": flat, "past_k": empty, "past_v": empty})
                prefix_times = []
                step_times = []
                for _ in range(10):
                    started = time.perf_counter()
                    _, keys, values = session.run(
                        None, {"ids": ids, "past_k": empty, "past_v": empty}
                    )
                    prefix_times.append((time.perf_counter() - started) * 1000)
                    started = time.perf_counter()
                    session.run(None, {"ids": flat, "past_k": keys, "past_v": values})
                    step_times.append((time.perf_counter() - started) * 1000)
                print(
                    f"  bench: prefix {np.mean(prefix_times):.2f} ms，"
                    f"step {np.mean(step_times):.2f} ms"
                )
            else:
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
