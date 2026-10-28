#!/usr/bin/env python3
"""Berth icon, round 3: MOORING 01 / r2-a on a WHITE tile (page "White").

Reuses geometry and helpers from build_refine.py. Adds (or rebuilds) page "White" in
design/icon/berth-icon.figcraft; the other pages are untouched.

    python3 design/icon/build_white.py [--no-run]
"""
import json
import subprocess
import sys
from pathlib import Path

import build_refine as br
from build_refine import add, ref, label, vec, xf, geom, WHITE, PURE_BLACK

HERE = Path(__file__).resolve().parent
FILE = HERE / "berth-icon.figcraft"
STEPS = HERE / "explore" / "white" / "steps.json"
RESEARCH = HERE / "research"

BAY = "#121416"      # color.neutral.950
NAVY = "#003362"     # color.blue.900
CORAL = "#E86156"    # color.red.500
RED = "#CC3430"      # color.red.600
EDGE = "#D6D9DD"     # color.neutral.300
VARIANTS = [("w1", "W1 pick", BAY, CORAL), ("w2", "W2 navy bay", NAVY, CORAL), ("w3", "W3 deeper red", BAY, RED)]


def tile(parent, name, x, y, size, bay, block):
    """White macOS tile on the 1024 grid scaled to `size`, with a 4 px (min 1 px) inside edge."""
    s = size / 1024
    params = {"type": "frame", "x": x, "y": y, "width": size, "height": size, "name": name,
              "props": {"fill": "none", "clip": False}}
    if parent is not None:
        params["parent"] = ref(parent)
    f = add("node.create", params)
    add("node.create", {"type": "rectangle", "parent": ref(f), "x": 100 * s, "y": 100 * s, "width": 824 * s, "height": 824 * s,
                        "name": "tile", "props": {"fill": WHITE, "cornerRadius": 184 * s, "cornerSmoothing": 0.6,
                                                  "stroke": EDGE, "strokeWeight": max(4 * s, 1.0), "strokeAlign": "inside"}})
    d, bd, w, hc = geom("a")
    k = min(540 / w, 440 / hc) * s
    dx, dy = (size - w * k) / 2, (size - hc * k) / 2
    vec(f, xf(d, k, dx, dy), "bay", bay)
    vec(f, xf(bd, k, dx, dy), "block", block)
    return f


def build():
    info = json.loads(subprocess.run(["figcraft-cli", "info", str(FILE), "--json"], capture_output=True, text=True).stdout)
    for p in info["pages"]:
        if p["name"] == "White":
            add("page.delete", {"id": p["id"]})
    pg = add("page.add", {"name": "White"})
    add("page.select", {"id": ref(pg)})

    for i, (key, nm, bay, block) in enumerate(VARIANTS):
        tile(None, f"tile-{key}-1024", i * 1104, 0, 1024, bay, block)

    TW, TH = 1800, 2420
    t = add("node.create", {"type": "frame", "x": 0, "y": 1144, "width": TW, "height": TH, "name": "tests",
                            "props": {"fill": "#FAFAFA", "clip": True}})
    label(t, "White tile tests, MOORING 01 / r2-a", 48, 36, 28, "Bold")
    label(t, "W1 coral on near-black  ·  W2 coral on navy  ·  W3 deeper red on near-black  ·  4 px inside edge #D6D9DD (never under 1 px)",
          48, 78, 13)

    # A: size ladder, each tile on white / light grey / dark
    bgs = [("white", "#FFFFFF", PURE_BLACK, "#E5E5E5"), ("light grey dock", "#DADCE2", PURE_BLACK, None), ("dark dock", "#2A2C31", WHITE, None)]
    label(t, "Each tile at 1024 (shown at 256), 128, 32, 16", 48, 120, 14, "Medium")
    for ri, (key, nm, bay, block) in enumerate(VARIANTS):
        for ci, (bn, bg, tc, border) in enumerate(bgs):
            x0 = 48 + ci * 580
            y0 = 150 + ri * 300
            props = {"fill": bg}
            if border:
                props.update({"stroke": border, "strokeWeight": 1})
            cell = add("node.create", {"type": "frame", "parent": ref(t), "x": x0, "y": y0, "width": 560, "height": 280,
                                       "name": f"{key} on {bn}", "props": props})
            label(cell, f"{nm}  ·  {bn}", 14, 8, 11, "Medium", tc)
            tile(cell, f"{key} 256 {bn}", 4, 20, 256, bay, block)
            tile(cell, f"{key} 128 {bn}", 276, 84, 128, bay, block)
            tile(cell, f"{key} 32 {bn}", 420, 170, 32, bay, block)
            tile(cell, f"{key} 16 {bn}", 478, 186, 16, bay, block)
            for lx, txt in ((330, "128"), (426, "32"), (482, "16")):
                label(cell, txt, lx, 255, 10, "Regular", tc)

    # B: dock strips with the category icons
    icons = [("src/orbstack.png", "OrbStack"), ("src/docker-site-icon.png", "Docker"), ("src/podman-desktop.png", "Podman Desktop"),
             ("src/tableplus.png", "TablePlus"), ("local/fork.png", "Fork"), ("local/proxyman.png", "Proxyman")]
    label(t, "In the Dock beside the category icons (Berth: W1, W2, W3)", 48, 1070, 14, "Medium")
    for si, (bn, bg, tc, border) in enumerate(bgs):
        props = {"fill": bg, "cornerRadius": 28}
        if border:
            props.update({"stroke": border, "strokeWeight": 1})
        strip = add("node.create", {"type": "frame", "parent": ref(t), "x": 48, "y": 1100 + si * 300, "width": 1704, "height": 280,
                                    "name": f"dock {bn}", "props": props})
        for size, yy in ((96, 36), (32, 180)):
            x = 32
            seq = icons[:4] + [("w1", "W1"), ("w2", "W2"), ("w3", "W3")] + icons[4:]
            for item, nm in seq:
                if item in ("w1", "w2", "w3"):
                    _, _, bay, block = next(v for v in VARIANTS if v[0] == item)
                    tile(strip, f"dock {bn} {item} {size}", x - 100 * size / 824, yy - 100 * size / 824, size * 1024 / 824, bay, block)
                else:
                    box = add("node.create", {"type": "frame", "parent": ref(strip), "x": x, "y": yy, "width": size, "height": size,
                                              "name": f"{nm} {size}", "props": {"fill": WHITE if "docker" in item else "none",
                                                                                "cornerRadius": size * 0.225, "clip": True}})
                    add("image.place", {"path": str(RESEARCH / item), "parent": ref(box), "x": 0, "y": 0, "width": size,
                                        "height": size, "mode": "fit"})
                x += size + (16 if size == 96 else 10)
        label(strip, "OrbStack, Docker (site icon on white), Podman Desktop, TablePlus, W1, W2, W3, Fork, Proxyman", 32, 146, 11, "Regular", tc)
        label(strip, "same, at 32 px", 32, 228, 11, "Regular", tc)

    # C: coral block vs a red status dot
    label(t, "Does the coral block read as an error dot? W1 / W3 at 32 and 16 px beside an 8 px dot (#E86156, #CC3430)", 48, 2010, 14, "Medium")
    for bi, (bn, bg, tc, border) in enumerate(bgs[::2]):
        props = {"fill": bg}
        if border:
            props.update({"stroke": border, "strokeWeight": 1})
        box = add("node.create", {"type": "frame", "parent": ref(t), "x": 48 + bi * 870, "y": 2040, "width": 850, "height": 330,
                                  "name": f"dot test {bn}", "props": props})
        label(box, bn, 14, 10, 11, "Medium", tc)
        rowy = 40
        for key, dot_col, dot_name in (("w1", CORAL, "#E86156"), ("w3", RED, "#CC3430")):
            _, _, bay, block = next(v for v in VARIANTS if v[0] == key)
            x = 24
            for size in (32, 16):
                tile(box, f"{key} {size} dot {bn}", x - 100 * size / 824 * 0, rowy + (40 - size) / 2, size, bay, block)
                x += size + 14
                add("node.create", {"type": "ellipse", "parent": ref(box), "x": x, "y": rowy + 16, "width": 8, "height": 8,
                                    "name": f"dot {dot_name} {size}", "props": {"fill": dot_col, "stroke": None}})
                x += 8 + 40
            label(box, f"{key.upper()} + dot {dot_name}", x, rowy + 14, 11, "Regular", tc)
            # a sidebar-like row for context
            rx = 24
            label(box, "shop-api-db-1", rx + 20, rowy + 74, 13, "Regular", tc)
            add("node.create", {"type": "ellipse", "parent": ref(box), "x": rx, "y": rowy + 78, "width": 8, "height": 8,
                                "name": "sidebar dot", "props": {"fill": dot_col, "stroke": None}})
            tile(box, f"{key} 32 row {bn}", rx + 180, rowy + 62, 32, bay, block)
            label(box, "failing dot in the list, tile beside it", rx + 226, rowy + 74, 11, "Regular", tc)
            rowy += 140


if __name__ == "__main__":
    build()
    STEPS.parent.mkdir(parents=True, exist_ok=True)
    STEPS.write_text(json.dumps(br.steps))
    print(f"{len(br.steps)} steps -> {STEPS}")
    if "--no-run" not in sys.argv:
        tmp = FILE.with_suffix(".new.figcraft")
        r = subprocess.run(["figcraft-cli", "run", str(STEPS), "--in", str(FILE), "--out", str(tmp)], capture_output=True, text=True)
        bad = [l for l in r.stdout.splitlines() if '"ok":false' in l]
        print(r.stderr.strip()[-600:] or "run ok", *bad[:5], sep="\n")
        if r.returncode == 0:
            tmp.replace(FILE)
        sys.exit(r.returncode)
