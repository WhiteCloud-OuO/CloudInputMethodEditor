#!/usr/bin/env bash
# 按 tools/release/data.lock 下载产品数据并校验：cloudime-data.tar.gz（WordBank/ 与 data/generated/）
# 与 cloudime-models.tar.gz（data/local_models/）都解到**仓库根**。
#
#   tools/release/data-fetch.sh            # 下载 + 校验 + 解开
#   tools/release/data-fetch.sh --verify   # 只校验 target/release-data/ 里已下载的文件
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LOCK="$ROOT/tools/release/data.lock"
OUT="$ROOT/target/release-data"
REPO="WhiteCloud-OuO/CloudInputMethodEditor"
ASSETS=(cloudime-data.tar.gz cloudime-models.tar.gz)
cd "$ROOT"

[[ -f "$LOCK" ]] || { echo "缺少 $LOCK" >&2; exit 1; }
# 锁文件里没有这一项时返回空串（`grep` 无匹配会让管道非零，加了 `|| true` 才不会被 `set -e` 静默打断）。
lock_value() { grep -E "^$1 *= *" "$LOCK" | head -1 | sed -E 's/^[^=]*= *//' | tr -d '[:space:]' || true; }
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }

TAG="$(lock_value tag)"
if [[ -z "$TAG" ]]; then
  {
    echo "$LOCK 里没有 tag：本仓库还没发过 data-vN。"
    echo "先跑 tools/release/data-bundle.sh 发一版（预发布，发到 $REPO），它会填好 data.lock；提交后再取数据。"
  } >&2
  exit 1
fi
mkdir -p "$OUT"

if [[ "${1:-}" != "--verify" ]]; then
  for f in "${ASSETS[@]}"; do
    rm -f "$OUT/$f"
    failed=0
    if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
      gh release download "$TAG" --repo "$REPO" --pattern "$f" --dir "$OUT" || failed=1
    else
      curl -fL --retry 3 -o "$OUT/$f" "https://github.com/$REPO/releases/download/$TAG/$f" || failed=1
    fi
    if [[ "$failed" == 1 ]]; then
      echo "下载失败：$REPO 的 $TAG 里没有资产 $f（Release 还没发，或资产名对不上）。" >&2
      echo "数据流水线见 docs/notes/release.md；仓库当前状态见 $LOCK。" >&2
      exit 1
    fi
  done
fi

for f in "${ASSETS[@]}"; do
  expected="$(lock_value "$f")"
  actual="$(sha256 "$OUT/$f")"
  [[ -n "$expected" && "$actual" == "$expected" ]] || { echo "$f 与 data.lock 不符（$TAG）：期望 $expected，实际 $actual" >&2; exit 1; }
  echo "$f  $actual"
done
[[ "${1:-}" == "--verify" ]] && exit 0

mkdir -p data/generated data/local_models
# 包里的路径就是仓库根相对路径（WordBank/、data/generated/、data/local_models/）
tar -xzf "$OUT/cloudime-data.tar.gz" -C .
tar -xzf "$OUT/cloudime-models.tar.gz" -C .
# 解出来的 mtime 比 checkout 出来的 TSV 旧，data-bundle.sh 会以为要重打
find WordBank data/generated data/local_models -type f -exec touch {} +
echo "产品数据 $TAG 已就位"

if [[ -n "${GITHUB_ENV:-}" ]]; then
  {
    echo "DATA_TAG=$TAG"
    echo "DATA_SHA256=$(lock_value cloudime-data.tar.gz)"
    echo "MODEL_SHA256=$(lock_value cloudime-models.tar.gz)"
  } >> "$GITHUB_ENV"
fi
