# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools>=4.56"]
# ///
"""把形码方案的 `roots.tsv` 做成一份只含字根的 PUA 字体，并写出配套的 `roots_pua.tsv`。

用法（在仓库根目录跑）：

    uv run tools/root-font/gen_roots_font.py --source C:/Windows/Fonts/SourceHanSansCN-Regular.otf

    uv run tools/root-font/gen_roots_font.py \
        --source C:/Windows/Fonts/SourceHanSansCN-Regular.otf \
        --roots InputSchemes/ShapeCode/wubi/roots.tsv \
        --out-font InputSchemes/ShapeCode/wubi/zigen.otf \
        --out-table InputSchemes/ShapeCode/wubi/roots_pua.tsv \
        --family 云朵字根

做的事：按 `roots.tsv` 里字根第一次出现的顺序编号（从 `U+E000` 起），从源字体里抽出这些字形的
子集，把 `cmap` 改写成「PUA 码位 → 原字形」，`name` 表换成自己的字族名，写出 `zigen.otf` 与
`roots_pua.tsv`；方案的 `scheme.toml` 里填 `RootFont` / `RootFontFamily` 就能用上（见
`docs/design/shape-code.md` §3.4）。

源字体必须是允许改作与再分发的字体（思源黑体 / 思源宋体 / Noto Sans CJK 都是 SIL OFL 1.1）。
生成的字是**源字体的派生作品**：随方案分发时要带上源字体的许可证（OFL.txt）和版权声明；
`--family` 换个名字也顺手避开 OFL 的保留字体名（Reserved Font Name）。
"""

import argparse
import io
import os
import sys

from fontTools import subset
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables._c_m_a_p import CmapSubtable

PUA_BASE = 0xE000
PUA_LAST = 0xF8FF
LICENSE_NOTE = (
    "This Font Software is licensed under the SIL Open Font License, Version 1.1."
)


def read_roots(path):
    """读 `键<TAB>字根 字根…`，返回 [(键, [字根…])]，保持文件里的顺序。"""
    rows = []
    with io.open(path, encoding="utf-8-sig") as file:
        lines = file.read().splitlines()
    for number, raw in enumerate(lines, 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, _, rest = line.partition("\t")
        key = key.strip()
        if len(key) != 1 or not key.isascii() or not key.islower():
            print(f"跳过第 {number} 行：键要正好一个小写 ASCII 字母", file=sys.stderr)
            continue
        roots = []
        for token in rest.split():
            if len(token) != 1:
                # 字体只能按字取字形，连写笔画得先拆成单个字
                print(f"跳过第 {number} 行的字根 `{token}`：只认单个字符", file=sys.stderr)
                continue
            roots.append(token)
        if not roots:
            print(f"跳过第 {number} 行：没有可用字根", file=sys.stderr)
            continue
        rows.append((key, roots))
    return rows


def unique_in_order(rows):
    order = []
    for _, roots in rows:
        for char in roots:
            if char not in order:
                order.append(char)
    return order


def subset_font(source, unicodes):
    font = TTFont(source)
    options = subset.Options()
    options.layout_features = []
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.notdef_outline = True
    options.glyph_names = False
    options.drop_tables += ["DSIG"]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(unicodes=unicodes)
    subsetter.subset(font)
    return font


def remap_to_pua(font, order):
    """把 cmap 从原码位改成 PUA 码位，返回 {字根: 码位}。"""
    source_cmap = font.getBestCmap()
    assign = {}
    pua_cmap = {}
    for index, char in enumerate(order):
        glyph = source_cmap.get(ord(char))
        if glyph is None:
            print(f"源字体里没有 `{char}`（U+{ord(char):04X}），跳过", file=sys.stderr)
            continue
        code = PUA_BASE + index
        if code > PUA_LAST:
            raise SystemExit(f"字根太多，PUA 放不下（{len(order)} 个）")
        pua_cmap[code] = glyph
        assign[char] = code

    cmap = newTable("cmap")
    cmap.tableVersion = 0
    subtable = CmapSubtable.newSubtable(4)
    subtable.platformID = 3
    subtable.platEncID = 1
    subtable.language = 0
    subtable.cmap = pua_cmap
    cmap.tables = [subtable]
    font["cmap"] = cmap
    return assign


def retag(font, family, source_name):
    """换 name 表：字族名、版权、许可证。OS/2 的字符范围跟着 PUA 走。"""
    os2 = font["OS/2"]
    best = font.getBestCmap()
    os2.usFirstCharIndex = min(best)
    os2.usLastCharIndex = max(best)
    os2.ulUnicodeRange1 = os2.ulUnicodeRange2 = os2.ulUnicodeRange3 = os2.ulUnicodeRange4 = 0
    os2.ulCodePageRange1 = os2.ulCodePageRange2 = 0

    notice = f"Derived from {source_name} by tools/root-font/gen_roots_font.py. {LICENSE_NOTE}"
    # PostScript 名要是 ASCII（`云朵字根` 这类字族名过不了这一条）
    ps_name = "".join(ch for ch in family if ch.isascii() and ch.isalnum()) or "CloudIMERoots"
    name = font["name"]
    name.names = []
    for platform, encoding, language in ((3, 1, 0x409), (3, 1, 0x804)):
        name.setName(f"{family}", 1, platform, encoding, language)
        name.setName("Regular", 2, platform, encoding, language)
        name.setName(f"{family} Regular", 4, platform, encoding, language)
        name.setName(f"{ps_name}-Regular", 6, platform, encoding, language)
        name.setName("Version 1.000", 5, platform, encoding, language)
        name.setName(notice, 0, platform, encoding, language)
        name.setName(LICENSE_NOTE, 13, platform, encoding, language)
        name.setName("https://openfontlicense.org", 14, platform, encoding, language)


def write_table(path, rows, assign, source_name):
    with io.open(path, "w", encoding="utf-8", newline="\n") as file:
        file.write("# 形码 PUA 字根（配合字根字体使用）：键<TAB>码位（十六进制，空格分隔）\n")
        file.write(f"# 由 tools/root-font/gen_roots_font.py 依 roots.tsv 的顺序从 {source_name} 生成。\n")
        file.write("# 改了 roots.tsv 要重跑生成脚本，字体和这份表一起换。\n")
        for key, roots in rows:
            codes = [f"{assign[char]:04X}" for char in roots if char in assign]
            if codes:
                file.write(f"{key}\t{' '.join(codes)}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--source", required=True, help="源字体（要有字根的字形，且允许改作再分发）")
    parser.add_argument(
        "--roots",
        default="InputSchemes/ShapeCode/wubi/roots.tsv",
        help="字根表，缺省 InputSchemes/ShapeCode/wubi/roots.tsv",
    )
    parser.add_argument(
        "--out-font",
        default="InputSchemes/ShapeCode/wubi/zigen.otf",
        help="输出的字根字体，缺省 InputSchemes/ShapeCode/wubi/zigen.otf",
    )
    parser.add_argument(
        "--out-table",
        default="InputSchemes/ShapeCode/wubi/roots_pua.tsv",
        help="输出的 PUA 字根表，缺省 InputSchemes/ShapeCode/wubi/roots_pua.tsv",
    )
    parser.add_argument("--family", default="云朵字根", help="字族名，缺省「云朵字根」")
    args = parser.parse_args()

    rows = read_roots(args.roots)
    order = unique_in_order(rows)
    if not order:
        raise SystemExit(f"{args.roots} 里没有可用字根")
    print(f"字根 {len(order)} 个（{len(rows)} 个键），源字体 {os.path.basename(args.source)}")

    font = subset_font(args.source, [ord(char) for char in order])
    assign = remap_to_pua(font, order)
    retag(font, args.family, os.path.basename(args.source))
    font.save(args.out_font)
    write_table(args.out_table, rows, assign, os.path.basename(args.source))

    size = os.path.getsize(args.out_font)
    print(f"写好了 {args.out_font}（{size:,} 字节，{len(assign)} 个字形）和 {args.out_table}")
    print(f"方案里填：RootFont = \"{os.path.basename(args.out_font)}\" / RootFontFamily = \"{args.family}\"")
    print("分发时记得带上源字体的 OFL.txt 与版权声明。")


if __name__ == "__main__":
    main()
