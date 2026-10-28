#!/usr/bin/env python3
"""Berth icon, round 2: refine MOORING 01 (open bay holding one block).

Adds (or rebuilds) the page "Refine" in design/icon/berth-icon.figcraft and leaves
page "Explore" untouched. Run build_explore.py first if the file does not exist.

    python3 design/icon/build_refine.py [--no-run]
"""
import json
import math
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
FILE = HERE / "berth-icon.figcraft"
STEPS = HERE / "explore" / "refine" / "steps.json"
RESEARCH = HERE / "research"

BLACK = "#121416"      # color.neutral.950 in design/tokens.json
PURE_BLACK = "#000000"
WHITE = "#FFFFFF"
CREAM = "#F2EBDD"      # no warm-neutral token exists
CORAL = "#E86156"      # color.red.500
BLUE = "#005EAB"       # color.accent / color.blue.700
ONE_COLOUR = BLUE


# ------------------------------------------------------------------ path tools
def n(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def rrect(x, y, w, h, r):
    return (f"M{n(x + r)} {n(y)}H{n(x + w - r)}A{n(r)} {n(r)} 0 0 1 {n(x + w)} {n(y + r)}V{n(y + h - r)}"
            f"A{n(r)} {n(r)} 0 0 1 {n(x + w - r)} {n(y + h)}H{n(x + r)}A{n(r)} {n(r)} 0 0 1 {n(x)} {n(y + h - r)}"
            f"V{n(y + r)}A{n(r)} {n(r)} 0 0 1 {n(x + r)} {n(y)}Z")


def rpoly(verts):
    """Clockwise polygon (y down) with a fillet radius per vertex: [(x, y, r), ...]."""
    out, m = [], len(verts)
    for i, (x, y, r) in enumerate(verts):
        px, py, _ = verts[i - 1]
        nx, ny, _ = verts[(i + 1) % m]
        v1 = (px - x, py - y)
        v2 = (nx - x, ny - y)
        l1, l2 = math.hypot(*v1), math.hypot(*v2)
        u1, u2 = (v1[0] / l1, v1[1] / l1), (v2[0] / l2, v2[1] / l2)
        if r <= 0:
            out.append(("L", (x, y)))
            continue
        cosang = u1[0] * u2[0] + u1[1] * u2[1]
        theta = math.acos(max(-1, min(1, cosang)))
        d = r / math.tan(theta / 2)
        a = (x + u1[0] * d, y + u1[1] * d)
        b = (x + u2[0] * d, y + u2[1] * d)
        cross = (x - px) * (ny - y) - (y - py) * (nx - x)
        out.append(("A", a, b, r, 1 if cross > 0 else 0))
    d = ""
    for i, seg in enumerate(out):
        if seg[0] == "L":
            d += ("M" if i == 0 else "L") + f"{n(seg[1][0])} {n(seg[1][1])}"
        else:
            _, a, b, r, sw = seg
            d += ("M" if i == 0 else "L") + f"{n(a[0])} {n(a[1])}A{n(r)} {n(r)} 0 0 {sw} {n(b[0])} {n(b[1])}"
    return d + "Z"


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


# -------------------------------------------------------------------- geometry
def bay(t, floor_t, side, block, seat, wall_h, ext=0, r_top=2, r_in=3, r_out=6, r_q=2, r_blk=4):
    """Bay silhouette + block. Origin top-left of the composition, y down.
    t wall, floor_t floor, side = wall-to-block gap, seat = block-to-floor gap,
    wall_h = wall height above the floor, ext = floor overhang each side (quay edge)."""
    top_of_block = seat + block
    hc = max(wall_h, top_of_block) + floor_t
    yft = hc - floor_t
    ywt = yft - wall_h
    xl = ext
    xli = xl + t
    xri = xli + block + 2 * side
    xr = xri + t
    w = xr + ext
    v = [(xl, ywt, r_top), (xli, ywt, r_top), (xli, yft, r_in), (xri, yft, r_in), (xri, ywt, r_top), (xr, ywt, r_top)]
    if ext:
        v += [(xr, yft, r_in), (w, yft, r_q), (w, hc, r_out), (0, hc, r_out), (0, yft, r_q), (xl, yft, r_in)]
    else:
        v += [(xr, hc, r_out), (xl, hc, r_out)]
    block_d = rrect(xli + side, yft - seat - block, block, block, r_blk)
    return rpoly(v), block_d, w, hc


VARIANTS = {
    # a: as drawn, balanced. Wall = side gap = seat gap = 20; wall top = block top + one gap.
    "a": dict(desc="as drawn, balanced (owner pick)", t=20, floor_t=22, side=20, block=56, seat=20, wall_h=22 + 0 + 76),
    # b: landscape slip. Walls lower than the block top.
    "b": dict(desc="wider and lower slip", t=16, floor_t=18, side=18, block=60, seat=8, wall_h=50, r_out=3, r_top=1),
    # c: b plus a quay overhang.
    "c": dict(desc="b plus quay edge", t=16, floor_t=18, side=18, block=60, seat=8, wall_h=50, ext=20, r_out=3, r_top=1),
}
# a: block top sits seat+block = 76 above the floor top; wall top is one gap higher.
VARIANTS["a"]["wall_h"] = 20 + 56 + 20


def geom(key):
    p = dict(VARIANTS[key])
    p.pop("desc")
    return bay(**p)


# ---------------------------------------------------------------------- script
steps = []


def add(command, params):
    steps.append({"command": command, "params": params})
    return len(steps) - 1


def ref(i):
    return {"$step": i}


def label(parent, text, x, y, size=12, weight="Regular", color=PURE_BLACK):
    return add("node.create", {"type": "text", "text": text, "parent": ref(parent), "x": x, "y": y, "name": text,
                               "props": {"fontSize": size, "fill": color, "fontStyle": weight}})


def vec(parent, d, name, fill):
    return add("node.create", {"type": "vector", "parent": ref(parent), "path": d, "name": name,
                               "props": {"fill": fill, "stroke": None}})


def mark(parent, key, name, x, y, size, ink, block_ink, frame_fill="none", fit=0.70):
    """A bay mark centred in a size x size frame; the composition fills `fit` of the longer side."""
    d, bd, w, hc = geom(key)
    k = size * fit / max(w, hc)
    dx, dy = (size - w * k) / 2, (size - hc * k) / 2
    f = add("node.create", {"type": "frame", "parent": ref(parent), "x": x, "y": y, "width": size, "height": size,
                            "name": name, "props": {"fill": frame_fill, "clip": False}})
    vec(f, xf(d, k, dx, dy), "bay", ink)
    vec(f, xf(bd, k, dx, dy), "block", block_ink)
    return f


def tile(parent, key, name, x, y, size, accent):
    """macOS icon tile on the 1024 grid, scaled to `size`: 824 body, continuous corner, transparent outside."""
    s = size / 1024
    f = add("node.create", {"type": "frame", "parent": ref(parent) if parent is not None else None, "x": x, "y": y,
                            "width": size, "height": size, "name": name, "props": {"fill": "none", "clip": False}})
    if parent is None:
        del steps[f]["params"]["parent"]
    add("node.create", {"type": "rectangle", "parent": ref(f), "x": 100 * s, "y": 100 * s, "width": 824 * s, "height": 824 * s,
                        "name": "tile", "props": {"fill": CREAM, "cornerRadius": 184 * s, "cornerSmoothing": 0.6, "stroke": None}})
    d, bd, w, hc = geom(key)
    k = min(540 / w, 440 / hc) * s
    dx, dy = (size - w * k) / 2, (size - hc * k) / 2
    vec(f, xf(d, k, dx, dy), "bay", BLACK)
    vec(f, xf(bd, k, dx, dy), "block", accent)
    return f


def build(best):
    # clean slate for the page
    info = json.loads(subprocess.run(["figcraft-cli", "info", str(FILE), "--json"], capture_output=True, text=True).stdout)
    for p in info["pages"]:
        if p["name"] == "Refine":
            add("page.delete", {"id": p["id"]})
    pg = add("page.add", {"name": "Refine"})
    add("page.select", {"id": ref(pg)})

    # ---- 1. variants
    W1, H1 = 1100, 520
    sheet = add("node.create", {"type": "frame", "x": 0, "y": 0, "width": W1, "height": H1, "name": "refine / variants",
                                "props": {"fill": WHITE, "clip": True}})
    label(sheet, "MOORING 01, refined", 48, 36, 28, "Bold")
    label(sheet, "open bay holding one block  ·  black on white  ·  Name 12 dropped by owner direction (nothing drawn)", 48, 78, 13)
    for i, key in enumerate("abc"):
        x = 48 + i * 340
        mark(sheet, key, f"r2-{key}", x, 120, 240, PURE_BLACK, PURE_BLACK)
        label(sheet, f"r2-{key}  ·  {VARIANTS[key]['desc']}", x, 372, 13, "Medium")
        for j, sz in enumerate((64, 32, 16)):
            mark(sheet, key, f"r2-{key} @{sz}", x + [0, 80, 128][j], 410, sz, PURE_BLACK, PURE_BLACK)
        # specs
        d, bd, w, hc = geom(key)
        p = VARIANTS[key]
        label(sheet, f"wall {p['t']}  side gap {p['side']}  seat {p['seat']}  block {p['block']}  floor {p['floor_t']}  walls {p['wall_h']}", x, 470, 10)
        label(sheet, f"mark {n(w)} x {n(hc)} = {w / hc:.2f}:1", x, 486, 10)

    # ---- 2. tiles (best variant only)
    tile(None, best, "tile-coral-1024", 0, 600, 1024, CORAL)
    tile(None, best, "tile-blue-1024", 1104, 600, 1024, BLUE)

    # ---- 3. tests sheet
    TW, TH = 1360, 1500
    ty = 600 + 1024 + 120
    t = add("node.create", {"type": "frame", "x": 0, "y": ty, "width": TW, "height": TH, "name": "tests",
                            "props": {"fill": WHITE, "clip": True}})
    label(t, f"Render tests, MOORING 01 / r2-{best}", 48, 36, 28, "Bold")
    # A: one mark, black / one colour / reversed at 16 32 128
    label(t, "Marks: black, one colour (accent blue), reversed", 48, 96, 14, "Medium")
    modes = [("black", WHITE, PURE_BLACK), ("one colour", WHITE, ONE_COLOUR), ("reversed", BLACK, WHITE)]
    for mi, (mn, bg, ink) in enumerate(modes):
        x0 = 48 + mi * 430
        bgf = add("node.create", {"type": "frame", "parent": ref(t), "x": x0, "y": 130, "width": 400, "height": 190,
                                  "name": f"mark-{mn}", "props": {"fill": bg, "stroke": "#0000001F", "strokeWeight": 1}})
        tc = BLACK if bg == WHITE else WHITE
        for szi, sz in enumerate((16, 32, 128)):
            xx = 24 + [0, 60, 140][szi]
            mark(bgf, best, f"{mn} {sz}", xx, 24 + (128 - sz) / 2 if sz < 128 else 24, sz, ink, ink, fit=0.90)
            label(bgf, f"{sz} px", xx, 164, 11, "Regular", tc)
        label(bgf, mn, 24, 4, 11, "Medium", tc)
    # B: tiles at 1024 (shown 256), 128, 32, 16
    label(t, "Tiles: 1024 master (shown at 256), 128, 32, 16", 48, 350, 14, "Medium")
    for ti, (tn, acc) in enumerate((("coral", CORAL), ("blue", BLUE))):
        x0 = 48 + ti * 650
        pad = add("node.create", {"type": "frame", "parent": ref(t), "x": x0, "y": 384, "width": 620, "height": 300,
                                  "name": f"tiles-{tn}", "props": {"fill": "#ECECEE"}})
        tile(pad, best, f"{tn} 256", 12, 22, 256, acc)
        tile(pad, best, f"{tn} 128", 290, 22 + 64, 128, acc)
        tile(pad, best, f"{tn} 32", 440, 22 + 160, 32, acc)
        tile(pad, best, f"{tn} 16", 500, 22 + 176, 16, acc)
        label(pad, "1024 / 4", 100, 276, 11)
        label(pad, "128", 340, 276, 11)
        label(pad, "32", 442, 276, 11)
        label(pad, "16", 502, 276, 11)
    # C: dock strips
    icons = [("src/orbstack.png", "OrbStack"), ("src/docker-site-icon.png", "Docker"), ("src/podman-desktop.png", "Podman Desktop"),
             ("src/tableplus.png", "TablePlus"), ("local/fork.png", "Fork"), ("local/proxyman.png", "Proxyman")]
    label(t, "In the Dock, beside the category icons (Berth: coral then blue)", 48, 716, 14, "Medium")
    for si, (sn, bg, tc) in enumerate((("light", "#DADCE2", PURE_BLACK), ("dark", "#2A2C31", WHITE))):
        strip = add("node.create", {"type": "frame", "parent": ref(t), "x": 48, "y": 750 + si * 330, "width": 1264, "height": 300,
                                    "name": f"dock-{sn}", "props": {"fill": bg, "cornerRadius": 28}})
        # large row (96 px) and small row (32 px)
        for size, yy in ((96, 36), (32, 190)):
            x = 32
            seq = [(i, nm) for i, nm in icons[:4]] + [("ours-coral", "Berth"), ("ours-blue", "Berth")] + icons[4:]
            for item, nm in seq:
                if item == "ours-coral":
                    tile(strip, best, f"dock {sn} berth coral {size}", x - 100 * size / 824, yy - 100 * size / 824, size * 1024 / 824, CORAL)
                elif item == "ours-blue":
                    tile(strip, best, f"dock {sn} berth blue {size}", x - 100 * size / 824, yy - 100 * size / 824, size * 1024 / 824, BLUE)
                else:
                    box = add("node.create", {"type": "frame", "parent": ref(strip), "x": x, "y": yy, "width": size, "height": size,
                                              "name": f"{nm} {size}", "props": {"fill": WHITE if "docker" in item else "none",
                                                                                "cornerRadius": size * 0.225, "clip": True}})
                    add("image.place", {"path": str(RESEARCH / item), "parent": ref(box), "x": 0, "y": 0, "width": size,
                                        "height": size, "mode": "fit"})
                x += size + (16 if size == 96 else 10)
            if size == 96:
                label(strip, "OrbStack, Docker (site icon on white), Podman Desktop, TablePlus, [Berth coral, Berth blue], Fork, Proxyman",
                      32, 150, 11, "Regular", tc)
            else:
                label(strip, "same, at 32 px", 32, 232, 11, "Regular", tc)


if __name__ == "__main__":
    best = "b"
    for a in sys.argv[1:]:
        if a.startswith("--best="):
            best = a.split("=", 1)[1]
    build(best)
    STEPS.parent.mkdir(parents=True, exist_ok=True)
    STEPS.write_text(json.dumps(steps))
    print(f"{len(steps)} steps -> {STEPS}")
    if "--no-run" not in sys.argv:
        tmp = FILE.with_suffix(".new.figcraft")
        r = subprocess.run(["figcraft-cli", "run", str(STEPS), "--in", str(FILE), "--out", str(tmp)], capture_output=True, text=True)
        bad = [l for l in r.stdout.splitlines() if '"ok":false' in l]
        print(r.stderr.strip()[-600:] or "run ok", *bad[:5], sep="\n")
        if r.returncode == 0:
            tmp.replace(FILE)
        sys.exit(r.returncode)
