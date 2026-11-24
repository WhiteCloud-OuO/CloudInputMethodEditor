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
import math
import os
import sys

import pathops
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.recordingPen import RecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

PUA_BASE = 0xE000
PUA_LAST = 0xF8FF
LICENSE_NOTE = "This Font Software is licensed under the SIL Open Font License, Version 1.1."

# 五笔字根里有几个「不是整字」的字形，得在源字体字形上擦掉多余的笔画（坐标是字体单位，y 向上、原点在基线）：
#   drop： 去掉与判断函数匹配的独立笔画（点之类单独成段的笔画）
#   erase：圆头擦除，`(起点, 终点, 半径)` 是一条「胶囊」：沿线段擦一条两头半圆的槽，
#          切口是圆的，斜笔、弯笔不会像矩形那样留下直角和细碎残条；弯曲的笔画用几段折线接起来擦。
#   cut：  矩形切，只在「整块区域夹在两条竖画之间」这种地方用（圆头反而会啃出尖角），比如 母 的中间横。
# 坐标是按**更纱黑体 SC Regular**（sarasa-gothic-sc-regular.ttf）量的，换源字体要重新量一遍。
# 见 docs/design/shape-code.md §3.3，口诀里的「勺缺点 / 无尾鱼 / 家衣底 / 摘礻衤 / 慈母无心 / 羊字底 / 氏无七」。
SHAPES = {
    # 礻 去掉右下角那个点（示字旁「摘」掉末笔）
    "礻": {"erase": [((425, 320), (510, 185), 55)]},
    # 衤 去掉右下角的两笔（衣字旁「摘」掉末两笔）
    "衤": {"erase": [((525, 460), (470, 300), 60), ((470, 320), (545, 170), 60)]},
    # 无尾鱼：最下面那条横
    "鱼": {"drop": lambda b: b[3] - b[1] < 130 and b[2] - b[0] > 600},
    # 家衣底：豕 顶上那一横
    "豕": {"erase": [((-40, 753), (980, 753), 36)]},
    # 羊字底：羊 去掉上面两点（一条横贯的胶囊，两头的圆在字外，切口是直的）
    "羊": {"erase": [((-100, 760), (1100, 760), 95)]},
    # 舟字底：舟 去掉顶上那一撇（大胶囊擦掉，再用一条横贯的薄胶囊把撇根削平，顶横不会缺）
    "舟": {
        "erase": [((500, 880), (455, 745), 48), ((-60, 748), (1060, 748), 26)]
    },
    # 丘 去掉底下一横
    "丘": {"erase": [((20, 13), (980, 13), 42)]},
    # 牛 去掉中间那一竖：竖画横穿两条横，分三段切（上、两条横之间、下），切边正好压在两条横的边上，
    # 所以横不会被啃出缺口，竖也不会留下残茬
    "牛": {
        "cut": [
            (455, 657, 570, 900),
            (455, 347, 570, 582),
            (455, -90, 570, 272),
        ]
    },
    # 兰 去掉下面两横（它们是独立的两段轮廓）
    "兰": {"drop": lambda b: b[3] - b[1] < 120 and b[2] - b[0] > 500},
    # 流 的右下角（川字底，右竖带钩）：从整字里裁下来，放大到字根大小并居中
    "流": {"keep": (355, -120, 1000, 385), "fit": (640, 500, 340)},
    # 下面这些是「从整字里裁一块部件」：keep 矩形 + fit 放大居中
    "衣": {"keep": (30, -120, 970, 585), "fit": (600, 500, 320)},   # 衣底：去掉顶上一点一横
    "且": {"keep": (150, 75, 860, 900), "fit": (540, 500, 330)},    # 且 去掉最下一横
    "具": {"keep": (30, 180, 970, 900), "fit": (600, 500, 330)},    # 具 去掉底下的八
    "兴": {"keep": (40, 220, 950, 900), "fit": (600, 500, 330)},    # 兴 去掉底下八
    "亦": {"keep": (30, -120, 970, 590), "fit": (600, 500, 330)},   # 亦 去掉顶上一点一横
    "拜": {"keep": (20, -120, 340, 860), "fit": (520, 500, 330)},   # 拜 左半
    "足": {"keep": (30, -120, 970, 440), "fit": (520, 500, 330)},   # 足 去掉上面的口
    "师": {"keep": (80, -120, 350, 860), "fit": (500, 500, 330)},   # 师 左半
    "临": {"keep": (410, -120, 960, 300), "fit": (560, 500, 300)},  # 临 右下
    "眉": {"keep": (30, 470, 890, 820), "fit": (520, 500, 330)},    # 眉 去掉目
    "虍": {"keep": (30, 290, 950, 860), "fit": (560, 500, 330)},    # 虍 去掉里面的七
    "皮": {"keep": (30, 280, 960, 860), "fit": (520, 500, 330)},    # 皮 去掉里面的又
    "业": {"keep": (30, 95, 970, 860), "fit": (560, 500, 330)},     # 业 去掉最下一横
    "革": {"keep": (30, -120, 970, 690), "fit": (560, 500, 330)},   # 革 去掉上面的廿
    # 氏无七：切掉中间一横（左侧接在竖画上，用矩形切），斜钩用圆头擦
    "氏": {
        "cut": [(248, 375, 1010, 455)],
        "erase": [
            ((565, 500), (565, 680), 38),
            ((576, 490), (594, 350), 45),
            ((594, 350), (617, 250), 45),
            ((617, 250), (640, 190), 45),
            ((640, 190), (660, 140), 45),
            ((660, 140), (680, 100), 45),
            ((680, 100), (700, 60), 45),
            ((700, 60), (725, 25), 45),
            ((725, 25), (760, -5), 45),
            ((760, -5), (800, -32), 45),
            ((800, -32), (850, -45), 45),
            ((850, -45), (890, -20), 45),
            ((890, -20), (920, 40), 45),
            ((920, 40), (935, 190), 45),
        ]
    },
    # 慈母无心：去掉两个点，再切掉中间那一横（竖画的接缝用矩形切，圆头在这种地方反而会啃出星形）
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


def capsule(start, end, radius, steps=12):
    """圆头擦除用的「胶囊」：沿 start→end 的一条线段，两头各一个半圆。

    用折线逼近半圆（12 段，1000 单位的 em 里误差 0.5 单位以内），布尔运算里够用。
    """
    (x0, y0), (x1, y1) = start, end
    angle = math.atan2(y1 - y0, x1 - x0)
    points = []
    for index in range(steps + 1):
        theta = angle - math.pi / 2 + math.pi * index / steps
        points.append((x1 + radius * math.cos(theta), y1 + radius * math.sin(theta)))
    for index in range(steps + 1):
        theta = angle + math.pi / 2 + math.pi * index / steps
        points.append((x0 + radius * math.cos(theta), y0 + radius * math.sin(theta)))
    path = pathops.Path()
    pen = path.getPen()
    pen.moveTo(points[0])
    for point in points[1:]:
        pen.lineTo(point)
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
    for start, end, radius in rule.get("erase", []):
        eraser = capsule(start, end, radius)
        path = pathops.op(path, eraser, pathops.PathOp.DIFFERENCE, fix_winding=True)
    for box in rule.get("cut", []):
        path = pathops.op(path, rectangle(*box), pathops.PathOp.DIFFERENCE, fix_winding=True)
    if "keep" in rule:
        path = pathops.op(path, rectangle(*rule["keep"]), pathops.PathOp.INTERSECTION, fix_winding=True)
    path = drop_specks(path, bounds_pen_factory)
    if rule.get("center_x"):
        path = center_horizontally(path, bounds_pen_factory)
    if "fit" in rule:
        path = fit_to(path, *rule["fit"], bounds_pen_factory)
    return path


def fit_to(path, target_height, cx, cy, bounds_pen_factory):
    """把字形缩放到指定高度、并把中心挪到 (cx, cy)（从整字里裁出来的部件用）。"""
    pen = bounds_pen_factory()
    path.draw(pen)
    if not pen.bounds:
        return path
    x0, y0, x1, y1 = pen.bounds
    scale = target_height / (y1 - y0)
    tx = cx - scale * (x0 + x1) / 2
    ty = cy - scale * (y0 + y1) / 2
    moved = pathops.Path()
    transform = TransformPen(moved.getPen(), (scale, 0, 0, scale, tx, ty))
    path.draw(transform)
    return moved


def center_horizontally(path, bounds_pen_factory):
    """把字形横向挪到 em 框中间（从整字里裁出来的部件要用）。"""
    pen = bounds_pen_factory()
    path.draw(pen)
    if not pen.bounds:
        return path
    shift = 500 - (pen.bounds[0] + pen.bounds[2]) / 2
    moved = pathops.Path()
    transform = TransformPen(moved.getPen(), (1, 0, 0, 1, shift, 0))
    path.draw(transform)
    return moved


def drop_specks(path, bounds_pen_factory):
    """擦完可能留下亚像素级的碎渣（断开的一小段轮廓），按面积清掉。"""
    kept = []
    for part in path_parts(path):
        pen = bounds_pen_factory()
        for op, args in part:
            getattr(pen, op)(*args)
        if pen.bounds and (pen.bounds[2] - pen.bounds[0]) * (pen.bounds[3] - pen.bounds[1]) < 4000:
            continue
        kept.append(part)
    if len(kept) == len(path_parts(path)):
        return path
    rebuilt = pathops.Path()
    pen = rebuilt.getPen()
    for part in kept:
        for op, args in part:
            getattr(pen, op)(*args)
    return rebuilt


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
