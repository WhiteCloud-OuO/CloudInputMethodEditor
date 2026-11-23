# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools>=4.56", "skia-pathops>=0.9"]
# ///
"""用开源字体做一份「五笔字根」字体：把字根字形摆到 PUA 码位，顺便把几个不是整字的字根削成理论形状。

用法（在仓库根目录跑）：

    uv run tools/root-font/gen_roots_font.py --source C:/Windows/Fonts/sarasa-gothic-sc-regular.ttf

    uv run tools/root-font/gen_roots_font.py \
        --source C:/Windows/Fonts/sarasa-gothic-sc-regular.ttf \
        --roots InputSchemes/ShapeCode/wubi/roots.tsv \
        --out-font InputSchemes/ShapeCode/wubi/zigen.ttf \
        --out-table InputSchemes/ShapeCode/wubi/roots_pua.tsv \
        --family 云朵字根 --preview zigen-preview.png

做的事：按 `roots.tsv` 里字根第一次出现的顺序编号（从 `U+E000` 起），从源字体取字形（该削的按
`SHAPES` 削掉笔画），`cmap` 指向 PUA 码位、字宽统一成一格，写出字体与配套的 `roots_pua.tsv`；
方案的 `scheme.toml` 里填 `RootFont` / `RootFontFamily` 就能用上（见 `docs/design/shape-code.md` §3.4）。

源字体必须是允许改作与再分发的（更纱黑体、思源黑体、霞鹜文楷都是 SIL OFL 1.1）。仓库里的字根字体
用**更纱黑体 SC Regular**：黑体在 7.5px 这种小字号下比楷体清楚；`SHAPES` 里的坐标也是按它量的。
生成的字是**源字体的派生作品**：随方案分发时要带上源字体的许可证与版权声明；`--family` 换个名字
也顺手避开 OFL 的保留字体名（Reserved Font Name）。
"""

import argparse
import io
import os
import sys

import pathops
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.recordingPen import RecordingPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

PUA_BASE = 0xE000
PUA_LAST = 0xF8FF
LICENSE_NOTE = "This Font Software is licensed under the SIL Open Font License, Version 1.1."

# 五笔字根里有几个「不是整字」的字形，得在源字体字形上削两刀（坐标是字体单位，y 向上、原点在基线）：
#   drop：去掉与判断函数匹配的独立笔画（点之类单独成段的笔画）
#   cut： 切掉一块矩形（横画之类跟主体连在一起的）
# 坐标是按**更纱黑体 SC Regular**（sarasa-gothic-sc-regular.ttf）量的；换源字体要重新量一遍。
# 见 docs/design/shape-code.md §3.3，口诀里的「勺缺点 / 无尾鱼 / 家衣底 / 摘礻衤 / 慈母无心 / 羊字底 / 氏无七」。
SHAPES = {
    # 礻 上面那个点（更纱的礻 是一整段轮廓，只能切）
    "礻": {"cut": [(180, 690, 420, 900)]},
    # 衤 上面那个点（独立的点）
    "衤": {"drop": lambda b: b[3] > 600 and b[2] - b[0] < 300},
    # 无尾鱼：最下面那条横
    "鱼": {"drop": lambda b: b[3] - b[1] < 130 and b[2] - b[0] > 600},
    # 家衣底：豕 顶上那一横
    "豕": {"cut": [(-120, 715, 1120, 900)]},
    # 羊字底：羊 去掉上面两点
    "羊": {"cut": [(-120, 668, 1120, 900)]},
    # 氏无七：去掉中间一横和右下斜钩，只留撇 + 竖提
    "氏": {
        "cut": [
            (248, 375, 1010, 455),
            (532, -120, 1010, 380),
            (520, 455, 640, 680),
        ]
    },
    # 慈母无心：去掉两个点，再切掉中间那一横（三段避开左右两竖）
    "母": {
        "drop": lambda b: b[2] - b[0] < 300 and b[3] - b[1] < 250,
        "cut": [
            (-120, 398, 176, 482),
            (268, 398, 752, 482),
            (835, 398, 1120, 482),
        ],
    },
}


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


def path_parts(path):
    """把路径按 moveTo 切段（只用来量尺寸 / 当布尔运算的操作数，填色规则不受影响）。"""
    record = pathops.Path()
    path.draw(record.getPen())
    parts, current = [], []
    for op, args in record.segments:
        if op == "moveTo" and current:
            parts.append(current)
            current = [(op, args)]
        else:
            current.append((op, args))
    if current:
        parts.append(current)
    return parts


def rectangle(x0, y0, x1, y1):
    path = pathops.Path()
    pen = path.getPen()
    pen.moveTo((x0, y0))
    pen.lineTo((x1, y0))
    pen.lineTo((x1, y1))
    pen.lineTo((x0, y1))
    pen.closePath()
    return path


def carve(path, rule, bounds_pen_factory):
    if "drop" in rule:
        for part in path_parts(path):
            pen = bounds_pen_factory()
            for op, args in part:
                getattr(pen, op)(*args)
            if pen.bounds and rule["drop"](pen.bounds):
                part_path = pathops.Path()
                part_pen = part_path.getPen()
                for op, args in part:
                    getattr(part_pen, op)(*args)
                path = pathops.op(path, part_path, pathops.PathOp.DIFFERENCE, fix_winding=True)
    for box in rule.get("cut", []):
        path = pathops.op(path, rectangle(*box), pathops.PathOp.DIFFERENCE, fix_winding=True)
    return path


def load_source(path):
    font = TTFont(path, lazy=True)
    return font, font.getGlyphSet(), font.getBestCmap()


def build_font(source, order, family):
    tt, glyphs, cmap = load_source(source)
    upem = tt["head"].unitsPerEm
    ascent = tt["hhea"].ascent
    descent = tt["hhea"].descent

    glyph_map = {".notdef": TTGlyphPen(None).glyph()}
    cmap_out = {}
    assign = {}
    missing = []
    shaped = []
    for index, char in enumerate(order):
        name = cmap.get(ord(char))
        if not name:
            missing.append(char)
            continue
        pen = RecordingPen()
        glyphs[name].draw(pen)
        path = pathops.Path()
        out = path.getPen()
        for op, args in pen.value:
            getattr(out, op)(*args)
        path = pathops.simplify(path)
        rule = SHAPES.get(char)
        if rule:
            path = carve(path, rule, lambda: BoundsPen(glyphs))
            shaped.append(char)
        glyph_pen = TTGlyphPen(None)
        path.draw(glyph_pen)
        code = PUA_BASE + index
        if code > PUA_LAST:
            raise SystemExit(f"字根太多，PUA 放不下（{len(order)} 个）")
        glyph_name = f"g{index:03d}"
        glyph_map[glyph_name] = glyph_pen.glyph()
        cmap_out[code] = glyph_name
        assign[char] = code

    builder = FontBuilder(upem, isTTF=True)
    builder.setupGlyphOrder(list(glyph_map))
    builder.setupCharacterMap(cmap_out)
    builder.setupGlyf(glyph_map)
    builder.setupHorizontalMetrics({name: (upem, 0) for name in glyph_map})
    builder.setupHorizontalHeader(ascent=ascent, descent=descent)
    builder.setupNameTable(
        {
            "familyName": family,
            "styleName": "Regular",
            "uniqueFontIdentifier": f"{family} Regular",
            "fullName": f"{family} Regular",
            "psName": "CloudIMEZiGen-Regular",
            "version": "Version 1.000",
            "copyright": (
                f"Derived from {os.path.basename(source)} by "
                f"tools/root-font/gen_roots_font.py; keep OFL.txt with this font. {LICENSE_NOTE}"
            ),
            "licenseDescription": LICENSE_NOTE,
            "licenseInfoURL": "https://openfontlicense.org",
        }
    )
    builder.setupOS2(
        sTypoAscender=ascent,
        sTypoDescender=descent,
        usWinAscent=ascent,
        usWinDescent=-descent,
    )
    builder.setupPost()
    return builder, assign, missing, shaped


def write_table(path, rows, assign, source):
    with io.open(path, "w", encoding="utf-8", newline="\n") as file:
        file.write("# 形码 PUA 字根（配合字根字体使用）：键<TAB>码位（十六进制，空格分隔）\n")
        file.write(
            f"# 由 tools/root-font/gen_roots_font.py 依 roots.tsv 的顺序从 {os.path.basename(source)} 生成。\n"
        )
        file.write("# 改了 roots.tsv 要重跑生成脚本，字体和这份表一起换。\n")
        for key, roots in rows:
            codes = [f"{assign[char]:04X}" for char in roots if char in assign]
            if codes:
                file.write(f"{key}\t{' '.join(codes)}\n")


def write_preview(path, builder, assign, order, source):
    """把生成的字根排成一张对照图（要 PIL，只在手动看效果时用）。"""
    from PIL import Image, ImageDraw, ImageFont

    stream = io.BytesIO()
    builder.save(stream)
    stream.seek(0)
    font = ImageFont.truetype(stream, 64)
    label = ImageFont.truetype(source, 14)
    cell = 96
    cols = 10
    rows = (len(assign) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * cell, rows * cell), "white")
    draw = ImageDraw.Draw(sheet)
    for index, char in enumerate(order):
        if char not in assign:
            continue
        col, row = index % cols, index // cols
        x, y = col * cell, row * cell
        draw.text(
            (x + cell / 2, y + cell / 2 + 6),
            chr(assign[char]),
            font=font,
            fill="black",
            anchor="mm",
        )
        draw.text((x + 4, y + 2), char, fill="#999999", font=label)
    sheet.save(path)


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
        default="InputSchemes/ShapeCode/wubi/zigen.ttf",
        help="输出的字根字体，缺省 InputSchemes/ShapeCode/wubi/zigen.ttf",
    )
    parser.add_argument(
        "--out-table",
        default="InputSchemes/ShapeCode/wubi/roots_pua.tsv",
        help="输出的 PUA 字根表，缺省 InputSchemes/ShapeCode/wubi/roots_pua.tsv",
    )
    parser.add_argument("--family", default="云朵字根", help="字族名，缺省「云朵字根」")
    parser.add_argument("--preview", help="另存一张字根对照图（需要 Pillow）")
    args = parser.parse_args()

    rows = read_roots(args.roots)
    order = unique_in_order(rows)
    if not order:
        raise SystemExit(f"{args.roots} 里没有可用字根")
    print(f"字根 {len(order)} 个（{len(rows)} 个键），源字体 {os.path.basename(args.source)}")

    builder, assign, missing, shaped = build_font(args.source, order, args.family)
    if missing:
        print(f"源字体缺这些字根的字形，已跳过：{''.join(missing)}", file=sys.stderr)
    builder.save(args.out_font)
    write_table(args.out_table, rows, assign, args.source)
    if args.preview:
        write_preview(args.preview, builder, assign, order, args.source)

    print(f"写好了 {args.out_font}（{os.path.getsize(args.out_font):,} 字节，{len(assign)} 个字形）和 {args.out_table}")
    if shaped:
        print(f"削过笔画的字根：{''.join(shaped)}")
    print(f"方案里填：RootFont = \"{os.path.basename(args.out_font)}\" / RootFontFamily = \"{args.family}\"")
    print("分发时记得带上源字体的 OFL.txt 与版权声明。")


if __name__ == "__main__":
    main()
