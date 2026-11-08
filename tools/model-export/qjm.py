"""读取 `.qjm`（`cloudime-format` 容器）里的三节内容。

容器布局见 `crates/cloudime-format/src/layout/`：32 字节文件头 + 每条 24 字节的分节表；
模型固定有 `CONF`（config.json）、`VOCB`（vocab.json）、`SAFT`（model.safetensors）三节。
也接受训练仓库直接导出的三件套目录。
"""

from __future__ import annotations

import json
import struct
from pathlib import Path

_HEADER = struct.Struct("<8sHHI16s")
_ENTRY = struct.Struct("<4sIQQ")
_MAGIC = (b"CLOUDIME", b"QINGJIAN")

UNK = 1
EOS = 2


def encode(index: dict[str, int], text: str) -> list[int]:
    return [index.get(char, UNK) for char in text]


def _read_container(path: Path) -> dict[bytes, bytes]:
    data = path.read_bytes()
    magic, _version, _kind, count, _reserved = _HEADER.unpack_from(data, 0)
    if magic not in _MAGIC:
        raise ValueError(f"{path} 不是 .qj / .qjm 容器")
    sections: dict[bytes, bytes] = {}
    for index in range(count):
        tag, _reserved, offset, length = _ENTRY.unpack_from(
            data, _HEADER.size + index * _ENTRY.size
        )
        sections[tag] = data[offset : offset + length]
    return sections


def read_model(path: str | Path) -> tuple[dict, str, bytes]:
    """返回 `(config, vocab 原文, safetensors 字节)`。"""
    path = Path(path)
    if path.is_dir():
        config = json.loads((path / "config.json").read_text(encoding="utf-8"))
        vocab = (path / "vocab.json").read_text(encoding="utf-8")
        weights = (path / "model.safetensors").read_bytes()
        return config, vocab, weights

    sections = _read_container(path)
    try:
        config = json.loads(sections[b"CONF"].decode("utf-8"))
        vocab = sections[b"VOCB"].decode("utf-8")
        weights = sections[b"SAFT"]
    except KeyError as missing:
        raise ValueError(f"{path} 缺少 {missing.args[0]!r} 分节") from missing
    return config, vocab, weights


def vocab_index(vocab_text: str) -> dict[str, int]:
    """`vocab.json` 的 `tokens` 前三个是特殊 token，之后一个字符一个 token。"""
    tokens = json.loads(vocab_text)["tokens"]
    return {token: index for index, token in enumerate(tokens[3:], start=3) if len(token) == 1}
