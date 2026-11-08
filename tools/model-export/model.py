"""把含章（`CharLm`）用 PyTorch 复刻出来，导出成 ONNX，并按 candle 的 `score` 规则算分。

结构对应 `crates/cloudime-neural/src/model.rs` / `scorer.rs`：
逐字嵌入 + 位置嵌入、8 层 pre-LN 自注意力、GELU(erf) MLP、权重绑定的输出层；
`score` 的规则也照抄（`<eos>` 开头、前文截断、候选拼 batch、末尾补 0）。
"""

from __future__ import annotations

import math

import torch
import torch.nn as nn
import torch.nn.functional as F

from qjm import EOS, encode

MASKED = -1.0e4


class Block(nn.Module):
    def __init__(self, hidden: int):
        super().__init__()
        self.ln1 = nn.LayerNorm(hidden, eps=1e-5)
        self.qkv = nn.Linear(hidden, 3 * hidden)
        self.proj = nn.Linear(hidden, hidden)
        self.ln2 = nn.LayerNorm(hidden, eps=1e-5)
        self.fc = nn.Linear(hidden, 4 * hidden)
        self.mlp_proj = nn.Linear(4 * hidden, hidden)


class CharLM(nn.Module):
    def __init__(self, config: dict, weights: dict[str, torch.Tensor]):
        super().__init__()
        self.config = config
        self.hidden = config["n_embd"]
        self.heads = config["n_head"]
        self.tok_emb = nn.Embedding(config["vocab_size"], self.hidden)
        self.pos_emb = nn.Embedding(config["context"], self.hidden)
        self.blocks = nn.ModuleList(
            [Block(self.hidden) for _ in range(config["n_layer"])]
        )
        self.ln_f = nn.LayerNorm(self.hidden, eps=1e-5)

        def put(module: nn.Module, key: str) -> None:
            module.weight.data.copy_(weights[f"{key}.weight"].float())
            if module.bias is not None:
                module.bias.data.copy_(weights[f"{key}.bias"].float())

        self.tok_emb.weight.data.copy_(weights["tok_emb.weight"].float())
        self.pos_emb.weight.data.copy_(weights["pos_emb.weight"].float())
        put(self.ln_f, "ln_f")
        for index, block in enumerate(self.blocks):
            prefix = f"blocks.{index}"
            put(block.ln1, f"{prefix}.ln1")
            put(block.qkv, f"{prefix}.attn.qkv")
            put(block.proj, f"{prefix}.attn.proj")
            put(block.ln2, f"{prefix}.ln2")
            put(block.fc, f"{prefix}.mlp.fc")
            put(block.mlp_proj, f"{prefix}.mlp.proj")

    def _mask(self, length: int, past: int, device: torch.device) -> torch.Tensor:
        rows = torch.arange(length, device=device).unsqueeze(1)
        columns = torch.arange(past + length, device=device).unsqueeze(0)
        fill = torch.full((), MASKED, device=device)
        empty = torch.zeros((), device=device)
        return torch.where(columns > past + rows, fill, empty)

    def forward(
        self,
        ids: torch.Tensor,
        past_keys: list[torch.Tensor] | None = None,
        past_values: list[torch.Tensor] | None = None,
    ) -> tuple[torch.Tensor, list[torch.Tensor], list[torch.Tensor]]:
        batch, length = ids.shape
        past = 0 if not past_keys else past_keys[0].shape[2]
        positions = torch.arange(past, past + length, device=ids.device)
        x = self.tok_emb(ids) + self.pos_emb(positions).unsqueeze(0)
        mask = self._mask(length, past, ids.device)
        head_dim = self.hidden // self.heads
        keys = []
        values = []
        for index, block in enumerate(self.blocks):
            qkv = block.qkv(block.ln1(x))
            q, k, v = qkv.chunk(3, dim=-1)
            q = q.view(batch, length, self.heads, head_dim).transpose(1, 2)
            k = k.view(batch, length, self.heads, head_dim).transpose(1, 2)
            v = v.view(batch, length, self.heads, head_dim).transpose(1, 2)
            if past_keys is not None:
                previous_k = past_keys[index].expand(batch, -1, -1, -1)
                previous_v = past_values[index].expand(batch, -1, -1, -1)
                k_all = torch.cat([previous_k, k], dim=2)
                v_all = torch.cat([previous_v, v], dim=2)
            else:
                k_all, v_all = k, v
            attention = (q @ k_all.transpose(2, 3)) * (1.0 / math.sqrt(head_dim))
            attention = torch.softmax(attention + mask, dim=-1)
            y = (attention @ v_all).transpose(1, 2).reshape(batch, length, self.hidden)
            x = x + block.proj(y)
            x = x + block.mlp_proj(F.gelu(block.fc(block.ln2(x))))
            keys.append(k)
            values.append(v)
        logits = self.ln_f(x) @ self.tok_emb.weight.T
        return logits, keys, values


class FullModel(nn.Module):
    """没有 K/V 缓存、一次吃完整序列的导出包装。"""

    def __init__(self, model: CharLM):
        super().__init__()
        self.model = model

    def forward(self, ids: torch.Tensor) -> torch.Tensor:
        logits, _, _ = self.model(ids)
        return logits


@torch.no_grad()
def score(
    model: CharLM,
    config: dict,
    index: dict[str, int],
    context: str,
    candidates: list[str],
) -> list[float]:
    """与 `CharScorer::score` 同样的截断 / 补 0 / 前文缓存规则。"""
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
    batch = len(tails)
    flat = torch.zeros(batch, width, dtype=torch.long)
    targets = torch.zeros(batch, width, dtype=torch.long)
    for row, tail in enumerate(tails):
        flat[row, 0] = last
        flat[row, 1 : 1 + len(tail)] = torch.tensor(tail, dtype=torch.long)
        targets[row, : len(tail)] = torch.tensor(tail, dtype=torch.long)
    if head:
        _, past_keys, past_values = model(torch.tensor(head, dtype=torch.long).unsqueeze(0))
        logits, _, _ = model(flat, past_keys, past_values)
    else:
        logits, _, _ = model(flat)
    log_probs = torch.log_softmax(logits.float(), dim=-1)
    picked = log_probs.gather(2, targets.unsqueeze(2)).squeeze(2)
    return [float(picked[row, : len(tail)].sum()) for row, tail in enumerate(tails)]
