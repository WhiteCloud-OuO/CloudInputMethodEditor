#!/usr/bin/env bash
# 把本机运行时数据发成一版不可变的数据 Release（data-vN，预发布），并写 tools/release/data.lock。
# 两份资产：cloudime-data.tar.gz（WordBank/*.db 词库、data/generated/lm.qj 语言模型与英文词表）
# 与 cloudime-models.tar.gz（data/local_models/*.qjm 本地整句模型；单列一份，只重训模型时不用重传词库）。
# CI 与自编译按锁文件取数据（data-fetch.sh）；改了数据发新号，锁文件与用到新数据的代码同一个提交。
#
#   tools/release/data-bundle.sh                 # 发到下一个 data-vN
#   tools/release/data-bundle.sh --tag data-v7   # 指定标签；已存在就拒绝
#   tools/release/data-bundle.sh --pack          # 只打包到 target/release-data/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/target/release-data"
LOCK="$ROOT/tools/release/data.lock"
# 发到哪个仓库：与 data-fetch.sh 同一个。显式指定，别跟着本地 remote 走——本地 clone 的 origin
# 可能还指着改名前的仓库。
REPO="WhiteCloud-OuO/CloudInputMethodEditor"
cd "$ROOT"

MODE=upload
TAG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --pack) MODE=pack; shift ;;
    --tag) TAG="$2"; shift 2 ;;
    *) echo "未知参数 $1" >&2; exit 1 ;;
  esac
done

# 运行时数据（安装包用得到的）：语言模型与英文词表（CLI 也读）。english-frequency.tsv 只是打英文词表的
# 构建输入，正本在 assets/lexicon/english-frequency.tsv，不进包。
PRODUCT_FILES=(lm.qj english.tsv)
LLM_FILES=(pinyin-llm.jsonl)
# 词库：主词库 Dict.db（中文 + 英文合一份）与用户导入的附加词库都在 WordBank/ 下（SQLite 存档）
WORDBANK_FILES=()
for f in WordBank/*.db; do [[ -f "$f" ]] && WORDBANK_FILES+=("$f"); done

for f in "${PRODUCT_FILES[@]}"; do
  [[ -f "data/generated/$f" ]] || { echo "缺少 data/generated/$f，先按 assets/lexicon/CLOUDIME.md 生成" >&2; exit 1; }
done
[[ ${#WORDBANK_FILES[@]} -gt 0 ]] || { echo "缺少 WordBank/*.db（主词库 Dict.db），先跑 tools/dict-convert word-bank" >&2; exit 1; }

# 本地整句模型：data/local_models/ 下的 *.qjm（可能多份，Server 自己挑词表含汉字的字级模型）。
# 有训练仓库导出的三件套就先打成 .qjm（重训了没重打）；没有三件套、手上已有 *.qjm 也行。
[[ -f data/local_models/model.safetensors ]] && tools/release/pack-model.sh
MODEL_FILES=()
for f in data/local_models/*.qjm; do [[ -f "$f" ]] && MODEL_FILES+=("$f"); done
[[ ${#MODEL_FILES[@]} -gt 0 ]] || { echo "缺少 data/local_models/*.qjm（本地整句模型），见 docs/notes/release.md" >&2; exit 1; }

# 魔数必须是 CLOUDIME：改名前的 QINGJIAN 读端虽然兼容，但发出去的 data.lock 就一直钉着要靠兼容垫着的旧数据。
# 词库是 SQLite 存档，没有这个头，不查。
legacy=()
for f in data/generated/lm.qj "${MODEL_FILES[@]}"; do
  [[ -f "$f" ]] || continue
  [[ "$(head -c 8 "$f")" == "CLOUDIME" ]] || legacy+=("$f")
done
if [[ ${#legacy[@]} -gt 0 ]]; then
  {
    echo "还是改名前的魔数（QINGJIAN）：${legacy[*]}"
    echo "先就地改写（布局没变，只改头 8 字节）再发，见 docs/notes/release.md「产品数据从哪来」："
    for f in "${legacy[@]}"; do
      if [[ "$f" == data/generated/lm.qj ]]; then
        echo "  cargo run --release -p cloudime-dict-convert -- rehead lm $f"
      else
        echo "  cargo run --release -p cloudime-dict-convert -- rehead model $f"
      fi
    done
  } >&2
  exit 1
fi

rm -rf "$OUT" && mkdir -p "$OUT"
# 路径相对仓库根：CI 解到仓库根就与本机一样（WordBank/、data/generated/、data/local_models/）
PRODUCT_PATHS=()
for f in "${PRODUCT_FILES[@]}"; do PRODUCT_PATHS+=("data/generated/$f"); done
tar -czf "$OUT/cloudime-data.tar.gz" -C . "${PRODUCT_PATHS[@]}" "${WORDBANK_FILES[@]}"
# 模型单独一份：只重训模型时不用重传词库（.qjm 内部已是 fp16，不再压）
tar -czf "$OUT/cloudime-models.tar.gz" -C . "${MODEL_FILES[@]}"
present=()
for f in "${LLM_FILES[@]}"; do [[ -f "data/generated/$f" ]] && present+=("$f"); done
[[ ${#present[@]} -gt 0 ]] && tar -czf "$OUT/cloudime-llm-intermediates.tar.gz" -C data/generated "${present[@]}"
# 与 data-fetch.sh 同款回退：git-bash / Linux 有 sha256sum，macOS 才只有 shasum
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
(cd "$OUT" && sha256 ./*.tar.gz | tee SHA256SUMS)
du -h "$OUT"/*.tar.gz

[[ "$MODE" == "pack" ]] && exit 0

if [[ -z "$TAG" ]]; then
  # 一个 data-vN 都还没有时 grep 会空手而归、管道非零，被 set -e 静默打断——`|| true` 兜住（首次发布就踩这个）。
  last="$(gh release list --repo "$REPO" --limit 200 --json tagName --jq '.[].tagName' | grep -E '^data-v[0-9]+$' | sed 's/data-v//' | sort -n | tail -1 || true)"
  TAG="data-v$(( ${last:-0} + 1 ))"
fi
[[ "$TAG" =~ ^data-v[0-9]+$ ]] || { echo "标签要写成 data-vN：$TAG" >&2; exit 1; }
gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1 && { echo "$TAG 已存在，数据版本不覆盖" >&2; exit 1; }

# 数据 tag 指向当前提交：这个提交必须已经在目标仓库（先 push），否则 tag 指不到东西。
HEAD_SHA="$(git rev-parse HEAD)"
gh api "repos/$REPO/commits/$HEAD_SHA" --silent >/dev/null 2>&1 || {
  echo "$REPO 上没有当前提交 $(git rev-parse --short HEAD)：先把代码 push 上去再发数据（数据 tag 指向它）。" >&2
  exit 1
}

# SHA256SUMS 的行格式随平台而异：GNU 文本模式是 `<hash>  ./file`，二进制模式是 `<hash> *./file`，
# 老写法 `grep " ./file\$"` 在 Windows 的 Git Bash 上匹配不到（前面是 `*` 不是空格）。
sha_of() { awk -v name="$1" '{ n = $2; sub(/^\*/, "", n); sub(/^\.\//, "", n); if (n == name) { print $1; exit } }' "$OUT/SHA256SUMS"; }
gh release create "$TAG" --repo "$REPO" --prerelease --target "$HEAD_SHA" --title "产品数据 $TAG" \
  --notes "词库 / 语言模型（cloudime-data.tar.gz）、本地整句模型（cloudime-models.tar.gz）、LLM 续跑中间产物（cloudime-llm-intermediates.tar.gz）。不可变；仓库 tools/release/data.lock 钉住要用哪一版。" \
  "$OUT"/*.tar.gz "$OUT/SHA256SUMS"

cat > "$LOCK" <<EOF
# 产品数据版本，data-bundle.sh 写、data-fetch.sh 读；不要手改
tag = $TAG
cloudime-data.tar.gz = $(sha_of cloudime-data.tar.gz)
cloudime-models.tar.gz = $(sha_of cloudime-models.tar.gz)
EOF
echo "已发 ${TAG}，锁文件已更新（记得提交）"
