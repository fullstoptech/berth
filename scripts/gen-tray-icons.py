#!/usr/bin/env python3
"""Draw the three menu bar template icons (P6-T1, D38): 36x36 px RGBA, black on transparent, so
macOS tints them for the light and dark menu bar. Output: crates/berth-app/assets/tray/*.rgba
(raw bytes, row-major, no header). Run: python3 -I scripts/gen-tray-icons.py"""
import os
import sys

SIZE = 36
SS = 4  # supersampling per axis


def rrect(x, y, x0, y0, x1, y1, r):
    """Is (x, y) inside the rounded rectangle?"""
    if not (x0 <= x <= x1 and y0 <= y <= y1):
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


BARS = [(4.0, 5.0, 32.0, 12.0), (4.0, 14.5, 32.0, 21.5), (4.0, 24.0, 32.0, 31.0)]
BADGE = (27.0, 27.0)


def in_bar(x, y, hollow):
    for (x0, y0, x1, y1) in BARS:
        if rrect(x, y, x0, y0, x1, y1, 2.2):
            if hollow and rrect(x, y, x0 + 1.8, y0 + 1.8, x1 - 1.8, y1 - 1.8, 0.8):
                continue
            return True
    return False


def ink(kind, x, y):
    """Coverage of one sample: True where the icon is drawn."""
    hollow = kind == "stopped"
    drawn = in_bar(x, y, hollow)
    if kind == "failing":
        d2 = (x - BADGE[0]) ** 2 + (y - BADGE[1]) ** 2
        if d2 <= 9.6 ** 2:
            drawn = False  # a clear ring around the badge
        if d2 <= 7.2 ** 2:
            drawn = True
            # the exclamation mark is cut out of the badge
            if abs(x - BADGE[0]) <= 1.1 and BADGE[1] - 4.2 <= y <= BADGE[1] + 0.8:
                drawn = False
            if (x - BADGE[0]) ** 2 + (y - (BADGE[1] + 3.4)) ** 2 <= 1.2 ** 2:
                drawn = False
    return drawn


def render(kind):
    out = bytearray()
    for py in range(SIZE):
        for px in range(SIZE):
            hits = 0
            for sy in range(SS):
                for sx in range(SS):
                    x = px + (sx + 0.5) / SS
                    y = py + (sy + 0.5) / SS
                    hits += 1 if ink(kind, x, y) else 0
            alpha = round(255 * hits / (SS * SS))
            out += bytes((0, 0, 0, alpha))
    return bytes(out)


def main():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
    target = os.path.join(root, "crates", "berth-app", "assets", "tray")
    os.makedirs(target, exist_ok=True)
    for kind, name in (("good", "all-good"), ("failing", "failing"), ("stopped", "engine-stopped")):
        with open(os.path.join(target, name + ".rgba"), "wb") as f:
            f.write(render(kind))
    return 0


if __name__ == "__main__":
    sys.exit(main())
