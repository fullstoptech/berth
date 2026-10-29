#!/usr/bin/env python3
"""Berth icon, round 1: black-only explorations in FigCraft.

Writes design/icon/explore/steps.json (a FigCraft command script) and runs it
headless to produce design/icon/berth-icon.figcraft. Re-runnable: the file is
rebuilt from scratch every time. Marks are defined as plain SVG path strings in a
200x200 box; the 32 px copies are the same paths scaled by 0.16.

    python3 design/icon/build_explore.py
"""
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "berth-icon.figcraft"
STEPS = HERE / "explore" / "steps.json"
BLACK, WHITE = "#000000", "#FFFFFF"


# ---------------------------------------------------------------- path helpers
def n(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def rect(x, y, w, h, ccw=False):
    return rrect(x, y, w, h, 0, ccw)


def rrect(x, y, w, h, r, ccw=False):
    tl, tr, br, bl = (r, r, r, r) if not isinstance(r, tuple) else r
    a = lambda rad, sw, px, py: f"A{n(rad)} {n(rad)} 0 0 {sw} {n(px)} {n(py)}"
    if not ccw:
        d = [f"M{n(x + tl)} {n(y)}", f"H{n(x + w - tr)}"]
        if tr: d.append(a(tr, 1, x + w, y + tr))
        d.append(f"V{n(y + h - br)}")
        if br: d.append(a(br, 1, x + w - br, y + h))
        d.append(f"H{n(x + bl)}")
        if bl: d.append(a(bl, 1, x, y + h - bl))
        d.append(f"V{n(y + tl)}")
        if tl: d.append(a(tl, 1, x + tl, y))
    else:
        d = [f"M{n(x + tl)} {n(y)}"]
        if tl: d.append(a(tl, 0, x, y + tl))
        d.append(f"V{n(y + h - bl)}")
        if bl: d.append(a(bl, 0, x + bl, y + h))
        d.append(f"H{n(x + w - br)}")
        if br: d.append(a(br, 0, x + w, y + h - br))
        d.append(f"V{n(y + tr)}")
        if tr: d.append(a(tr, 0, x + w - tr, y))
    d.append("Z")
    return "".join(d)


def circ(cx, cy, r, ccw=False):
    sw = 0 if ccw else 1
    return (f"M{n(cx - r)} {n(cy)}A{n(r)} {n(r)} 0 1 {sw} {n(cx + r)} {n(cy)}"
            f"A{n(r)} {n(r)} 0 1 {sw} {n(cx - r)} {n(cy)}Z")


def poly(pts, hole=False):
    """Closed polygon; winding forced clockwise (solid) or counter-clockwise (hole)."""
    area = sum(pts[i][0] * pts[(i + 1) % len(pts)][1] - pts[(i + 1) % len(pts)][0] * pts[i][1]
               for i in range(len(pts)))
    cw = area > 0
    if cw == hole:
        pts = pts[::-1]
    return "M" + "L".join(f"{n(x)} {n(y)}" for x, y in pts) + "Z"


def F(*ds):
    return ("F", "".join(ds))


def S(d, w, cap="round", join="round", color=BLACK):
    return ("S", d, w, cap, join, color)


def WS(d, w, cap="none", join="miter"):
    return S(d, w, cap, join, WHITE)


# transform (scale + translate) every coordinate of an absolute-command path
ARGS = {"M": 2, "L": 2, "H": 1, "V": 1, "C": 6, "Q": 4, "A": 7, "Z": 0}


def xf(d, s=1.0, dx=0.0, dy=0.0):
    tokens = re.findall(r"[MLHVCQAZ]|-?\d*\.?\d+", d)
    out, i = [], 0
    while i < len(tokens):
        c = tokens[i]
        i += 1
        out.append(c)
        k = ARGS[c]
        vals = [float(t) for t in tokens[i:i + k]]
        i += k
        if c == "H":
            vals = [vals[0] * s + dx]
        elif c == "V":
            vals = [vals[0] * s + dy]
        elif c == "A":
            rx, ry, rot, lg, sw, x, y = vals
            vals = [rx * s, ry * s, rot, lg, sw, x * s + dx, y * s + dy]
        else:
            vals = [v * s + (dx if j % 2 == 0 else dy) for j, v in enumerate(vals)]
        out.append(" ".join(n(v) for v in vals))
    return "".join(out)


# ------------------------------------------------------------------- MOORING
def bollard(x0, y0, sc=1.0):
    """Bollard silhouette: cap, neck, flared base; origin top-left of a 56x106 box."""
    parts = [rrect(0, 0, 56, 22, 11), rect(14, 20, 28, 56), poly([(14, 70), (42, 70), (56, 106), (0, 106)])]
    return [xf(p, sc, x0, y0) for p in parts]


mooring = []
# 01 slip + block
mooring.append([F(poly([(36, 40), (54, 40), (54, 142), (146, 142), (146, 40), (164, 40), (164, 160), (36, 160)]),
                  rrect(72, 76, 56, 56, 6))])
# 02 bollard, rope, block
mooring.append([F(*bollard(30, 50)), F(rrect(114, 100, 56, 56, 6)),
                S("M58 86C92 86 96 128 114 128", 8)])
# 03 cleat on its plate
mooring.append([F(poly([(16, 84), (44, 62), (156, 62), (184, 84), (156, 106), (122, 106), (122, 140), (78, 140), (78, 106), (44, 106)]),
                  rrect(58, 140, 84, 18, 4))])
# 04 ring with an open mooring gap, block inside
mooring.append([S("M80 40H52A12 12 0 0 0 40 52V148A12 12 0 0 0 52 160H148A12 12 0 0 0 160 148V52A12 12 0 0 0 148 40H120", 14, "none", "miter"),
                F(rrect(72, 72, 56, 56, 6))])
# 05 chocks
mooring.append([F(rrect(64, 54, 72, 78, 6), poly([(18, 132), (54, 96), (54, 132)]), poly([(182, 132), (146, 96), (146, 132)]),
                  rect(18, 144, 164, 14))])
# 06 funnel slip
mooring.append([F(poly([(30, 38), (48, 38), (72, 146), (128, 146), (152, 38), (170, 38), (142, 164), (58, 164)]),
                  rrect(76, 94, 48, 48, 5))])
# 07 padeye
mooring.append([F(rrect(46, 80, 108, 80, 8)), S(circ(100, 54, 22), 14)])
# 08 piles
mooring.append([F(rect(28, 30, 20, 140), rect(152, 30, 20, 140), rect(48, 94, 24, 12), rect(128, 94, 24, 12),
                  rrect(70, 70, 60, 60, 6))])
# 09 quay corner
mooring.append([F(poly([(32, 32), (64, 32), (64, 136), (168, 136), (168, 168), (32, 168)]), rrect(82, 70, 58, 58, 6))])
# 10 bay in a slab
mooring.append([F(poly([(28, 36), (68, 36), (68, 106), (132, 106), (132, 36), (172, 36), (172, 164), (28, 164)]),
                  rrect(80, 60, 40, 36, 5))])
# 11 bollard wrapped
mooring.append([F(rrect(52, 28, 96, 28, 14), rect(76, 54, 48, 70), poly([(76, 114), (124, 114), (152, 164), (48, 164)])),
                WS("M68 76L132 92", 9), WS("M68 98L132 114", 9)])
# 12 hull in slip
mooring.append([F(poly([(30, 40), (48, 40), (48, 148), (152, 148), (152, 40), (170, 40), (170, 166), (30, 166)]),
                  "M100 46C124 66 132 92 132 116V132Q132 138 126 138H74Q68 138 68 132V116C68 92 76 66 100 46Z")])
# 13 coil
mooring.append([S("M110 104A20 20 0 0 1 70 104A30 30 0 0 1 130 104A40 40 0 0 1 50 104A50 50 0 0 1 150 104", 12, "round", "round"),
                F(rrect(86, 90, 28, 28, 4))])
# 14 line between two dots, block hung low
mooring.append([F(circ(34, 50, 12), circ(166, 50, 12)), S("M34 50Q100 168 166 50", 8),
                F(rrect(76, 98, 48, 48, 6))])
# 15 arch berth
mooring.append([S("M44 166V104A56 56 0 0 1 156 104V166", 16, "none", "miter"), F(rrect(74, 110, 52, 56, 6))])
# 16 marina fingers
mooring.append([F(rect(26, 30, 148, 18), rect(26, 30, 22, 140), rect(89, 30, 22, 140), rect(152, 30, 22, 140),
                  rrect(54, 96, 30, 54, 5))])

# -------------------------------------------------------------------- ORIGIN
origin = []
# 01 slats: top one short is the tab
origin.append([F(rrect(30, 30, 60, 26, 6), rrect(30, 68, 140, 26, 6), rrect(30, 106, 140, 26, 6), rrect(30, 144, 140, 26, 6))])
# 02 side tab + ribbed body
origin.append([F(rrect(56, 30, 116, 140, 12), rrect(28, 52, 40, 48, 8),
                 rect(88, 56, 10, 88, True), rect(110, 56, 10, 88, True), rect(132, 56, 10, 88, True))])
# 03 hull folder
origin.append([F("M30 44Q30 36 38 36H78Q84 36 88 41L96 54H170V100C170 140 140 164 100 168C60 164 30 140 30 100Z")])
# 04 bow folder
origin.append([F(poly([(28, 70), (130, 70), (176, 112), (130, 160), (28, 160)]), rrect(28, 42, 56, 30, (6, 6, 0, 0)))])
# 05 container in the pocket
origin.append([F(rrect(44, 26, 112, 90, 8), rect(64, 40, 10, 22, True), rect(86, 40, 10, 22, True), rect(108, 40, 10, 22, True), rect(130, 40, 10, 22, True)),
               WS(rrect(26, 80, 148, 92, 12), 14, "none", "miter"), F(rrect(26, 80, 148, 92, 12))])
# 06 slash split block with tab on the left half
origin.append([F(poly([(30, 26), (72, 26), (80, 40), (114, 40), (82, 168), (30, 168)]), poly([(130, 40), (170, 40), (170, 168), (98, 168)]))])
# 07 chamfered ribbed tab
origin.append([F(poly([(46, 38), (112, 38), (124, 54), (154, 54), (170, 70), (170, 152), (154, 168), (46, 168), (30, 152), (30, 54)]),
                 rect(56, 84, 10, 62, True), rect(80, 84, 10, 62, True), rect(104, 84, 10, 62, True), rect(128, 84, 10, 62, True))])
# 08 iso hexagon with tab
iso_tab = poly([(38, 64), (72, 44), (64, 30), (30, 50)])
origin.append([F(poly([(100, 30), (162, 66), (162, 138), (100, 174), (38, 138), (38, 66)]), iso_tab),
               WS("M100 102L38 66M100 102L162 66M100 102V174", 7)])
# 09 capsule with tab and port
origin.append([F(rrect(26, 62, 148, 96, 48), rrect(44, 40, 54, 40, 10), circ(138, 110, 12, True))])
# 10 tree: folder -> container
origin.append([F(poly([(28, 30), (58, 30), (66, 40), (96, 40), (96, 78), (28, 78)]), rrect(110, 110, 62, 58, 6)),
               S("M50 78V139H110", 10, "none", "miter")])
# 11 floating tab above ribbed block
origin.append([F(rrect(34, 30, 52, 24, 6), rrect(34, 64, 132, 106, 10),
                 rect(62, 90, 10, 56, True), rect(86, 90, 10, 56, True), rect(110, 90, 10, 56, True), rect(134, 90, 10, 56, True))])
# 12 half folder, half ribs
origin.append([F(poly([(28, 38), (78, 38), (88, 52), (104, 52), (104, 168), (28, 168)]),
                 rect(116, 52, 14, 116), rect(138, 52, 14, 116), rect(160, 52, 14, 116))])
# 13 tab right, chamfer left, window
origin.append([F(poly([(48, 52), (112, 52), (122, 36), (172, 36), (172, 168), (28, 168), (28, 72)]), rrect(56, 80, 88, 58, 6, True))])
# 14 stepped index
origin.append([F(rrect(28, 38, 144, 28, 5), rrect(28, 24, 40, 18, 4), rrect(28, 88, 144, 28, 5), rrect(80, 74, 40, 18, 4),
                 rrect(28, 138, 144, 28, 5), rrect(132, 124, 40, 18, 4))])
# 15 slab with path slash, notched corner
origin.append([F(poly([(58, 28), (172, 28), (172, 172), (28, 172), (28, 58)])), WS("M118 62L82 138", 14, "round", "round")])
# 16 folder outline with a block inside
origin.append([S("M32 164V48H80L92 62H168V164Z", 12, "none", "miter"), F(rrect(62, 96, 76, 48, 6))])

# ---------------------------------------------------------------------- NAME
name = []
# 01 stacked D blocks on a stem
name.append([F(rect(32, 30, 26, 140), rrect(58, 30, 84, 62, (0, 31, 31, 0)), rrect(58, 106, 98, 64, (0, 32, 32, 0)))])
# 02 monoline rope B
name.append([S("M44 32V168M44 32H106A34 34 0 0 1 106 100H44M44 100H116A34 34 0 0 1 116 168H44", 16)])
# 03 B on its side: a waterline and two hulls
name.append([F(rect(24, 62, 152, 24), "M28 86A34 34 0 0 0 96 86Z", "M104 86A34 34 0 0 0 172 86Z")])
# 04 bollard-stem B
name.append([F(rrect(20, 20, 60, 26, 13), rect(36, 44, 28, 116), rrect(20, 152, 60, 26, 4)),
             S("M64 62H106A26 26 0 0 1 106 114H64M64 114H116A28 28 0 0 1 116 170H64", 14, "none", "miter")])
# 05 negative B in a slab
name.append([F(rrect(28, 28, 144, 144, 18), xf(rect(48, 48, 18, 104, True), 1, 11),
               xf("M74 48V94H98A23 23 0 0 0 98 48Z", 1, 11), xf("M74 104V152H106A24 24 0 0 0 106 104Z", 1, 11))])
# 06 ribbed B
name.append([F(rect(32, 30, 26, 140), rrect(58, 30, 84, 66, (0, 33, 33, 0)), rrect(58, 104, 98, 66, (0, 33, 33, 0)),
               rect(76, 42, 8, 42, True), rect(94, 42, 8, 42, True), rect(112, 42, 8, 42, True),
               rect(76, 116, 8, 42, True), rect(96, 116, 8, 42, True), rect(116, 116, 8, 42, True))])
# 07 B with a pointer notch at the waist
name.append([F(rect(32, 30, 26, 140),
               "M58 30H108A34 34 0 0 1 126 94L112 100L128 106A38 38 0 0 1 112 170H58Z",
               rrect(70, 48, 36, 32, 8, True), rrect(70, 118, 44, 34, 8, True))])
# 08 outline B, block docked in the lower bowl
name.append([S("M40 98H116A35 35 0 0 1 116 168H40V32H106A33 33 0 0 1 106 98H40Z", 12, "none", "miter"),
             F(rrect(70, 118, 34, 34, 4))])
# 09 lowercase b: bollard stem + rope ring
name.append([F(rrect(30, 24, 52, 24, 12), rect(43, 44, 26, 120), rrect(30, 150, 52, 20, 4)),
             S(circ(114, 120, 34), 16)])
# 10 faceted B
name.append([F(poly([(32, 32), (110, 32), (136, 50), (136, 76), (118, 98), (146, 118), (146, 148), (122, 168), (32, 168)]),
               poly([(62, 56), (100, 56), (110, 64), (110, 74), (100, 84), (62, 84)], True),
               poly([(62, 114), (108, 114), (118, 124), (118, 140), (108, 146), (62, 146)], True))])
# 11 inline B
name.append([F(rect(30, 28, 32, 144), rrect(62, 28, 84, 70, (0, 35, 35, 0)), rrect(62, 102, 100, 70, (0, 35, 35, 0))),
             WS("M46 42V158M46 42H100A18 18 0 0 1 100 90H46M46 112H112A22 22 0 0 1 112 156H46", 5, "none", "miter")])
# 12 lowercase b, bowl is a slip holding a block
name.append([F(rect(30, 24, 26, 146)), S(rrect(56, 86, 96, 84, 14), 14, "none", "miter"), F(rrect(86, 116, 34, 34, 4))])
# 13 capsule B
name.append([F(rect(32, 30, 28, 140), rrect(32, 30, 112, 66, 33), rrect(32, 104, 128, 66, 33))])
# 14 block above, hull below
name.append([F(rect(32, 30, 26, 140), rrect(58, 30, 90, 66, 6),
               "M58 108H158C158 148 134 170 108 170H58Z")])
# 15 rectilinear B with square counters
name.append([F(rrect(32, 30, 100, 74, (0, 16, 0, 0)), rrect(32, 98, 122, 72, (0, 0, 16, 0)),
               rect(58, 52, 50, 32, True), rect(58, 120, 72, 30, True))])
# 16 pixel B from a 3x5 stack of blocks
cells = ["110", "101", "110", "101", "110"]
pix = []
for r, row in enumerate(cells):
    for c, ch in enumerate(row):
        if ch == "1":
            pix.append(rrect(40 + c * 44, 30 + r * 28, 40, 24, 4))
name.append([F(*pix)])

ROUTES = [
    ("mooring", "MOORING", "the place a container rests", mooring),
    ("origin", "ORIGIN", "a container is known by the folder it came from", origin),
    ("name", "NAME", "a B built from Berth's parts", name),
]

# --------------------------------------------------------------------- script
steps = []


def add(command, params):
    steps.append({"command": command, "params": params})
    return len(steps) - 1


def ref(i):
    return {"$step": i}


def label(parent, text, x, y, size, weight="Regular", color=BLACK):
    props = {"fontSize": size, "fill": color, "fontStyle": weight}
    return add("node.create", {"type": "text", "text": text, "parent": ref(parent), "x": x, "y": y, "props": props, "name": text})


def mark(parent, name_, items, scale, x, y):
    size = 200 * scale
    f = add("node.create", {"type": "frame", "parent": ref(parent), "x": x, "y": y, "width": size, "height": size,
                            "name": name_, "props": {"fill": "none", "clip": False}})
    for k, it in enumerate(items):
        if it[0] == "F":
            add("node.create", {"type": "vector", "parent": ref(f), "path": xf(it[1], scale), "name": f"shape-{k + 1}",
                                "props": {"fill": BLACK, "stroke": None}})
        else:
            _, d, w, cap, join, color = it
            add("node.create", {"type": "vector", "parent": ref(f), "path": xf(d, scale), "name": f"line-{k + 1}",
                                "props": {"fill": "none", "stroke": color, "strokeWeight": w * scale,
                                          "strokeCap": cap, "strokeJoin": join}})
    return f


add("file.new", {"name": "Berth icon"})
add("page.rename", {"id": 1, "name": "Explore"})

COLS, PAD, GAP, CELL, LABEL = 4, 48, 32, 200, 28
SHEET_W = PAD * 2 + COLS * CELL + (COLS - 1) * GAP  # 992
SHEET_GAP = 120
for ri, (key, title, idea, marks) in enumerate(ROUTES):
    rows = (len(marks) + COLS - 1) // COLS
    top = 120
    strip_y = top + rows * (CELL + LABEL + 20) + 8
    sheet_h = strip_y + 120
    sheet = add("node.create", {"type": "frame", "x": ri * (SHEET_W + SHEET_GAP), "y": 0, "width": SHEET_W, "height": sheet_h,
                                "name": f"{key} / contact sheet", "props": {"fill": WHITE, "clip": True}})
    label(sheet, title, PAD, 36, 30, "Bold")
    label(sheet, f"{idea}  ·  {len(marks)} explorations, black only", PAD, 78, 14)
    for i, items in enumerate(marks):
        c, r = i % COLS, i // COLS
        x = PAD + c * (CELL + GAP)
        y = top + r * (CELL + LABEL + 20)
        add("node.create", {"type": "rectangle", "parent": ref(sheet), "x": x, "y": y, "width": CELL, "height": CELL,
                            "name": f"cell-{i + 1:02d}", "props": {"fill": "none", "stroke": "#0000001F", "strokeWeight": 1}})
        mark(sheet, f"{key}-{i + 1:02d}", items, 1.0, x, y)
        label(sheet, f"{i + 1:02d}", x, y + CELL + 8, 12, "Medium")
    strip = add("node.create", {"type": "frame", "parent": ref(sheet), "x": 0, "y": strip_y, "width": SHEET_W, "height": 112,
                                "name": f"{key} / strip 32", "props": {"fill": WHITE}})
    label(strip, "32 px", PAD, 0, 12, "Medium")
    sp = 56
    for i, items in enumerate(marks):
        x = PAD + i * sp
        mark(strip, f"{key}-{i + 1:02d} @32", items, 0.16, x, 28)
        label(strip, f"{i + 1:02d}", x + 6, 68, 9)

STEPS.parent.mkdir(parents=True, exist_ok=True)
STEPS.write_text(json.dumps(steps))
print(f"{len(steps)} steps -> {STEPS}")
if "--no-run" not in sys.argv:
    r = subprocess.run(["figcraft-cli", "run", str(STEPS), "--out", str(OUT)], capture_output=True, text=True)
    bad = [l for l in r.stdout.splitlines() if '"ok":false' in l]
    print(r.stderr.strip() or "run ok", *bad[:5], sep="\n")
    sys.exit(r.returncode)
