"""Generate design/tokens.json for Berth: OKLCH ramps, light and dark roles, type, space, shape, motion.

Re-run after changing any decision below; the decisions and their reasons live in design/design-system.md.
"""
import json
import math
from pathlib import Path

OUT = Path(__file__).parent / "tokens.json"

# ---------- colour ----------
CHROMATIC_L = [0.975, 0.950, 0.910, 0.860, 0.760, 0.660, 0.560, 0.480, 0.400, 0.320, 0.240]
NEUTRAL_L = [0.985, 0.967, 0.930, 0.885, 0.760, 0.640, 0.540, 0.450, 0.370, 0.260, 0.190]
NEUTRAL_DARK_L = [0.165, 0.195, 0.225, 0.260, 0.300, 0.400, 0.520, 0.640, 0.760, 0.880, 0.960]
NEUTRAL_DARK_LINE = (450, 0.335)   # extra step between 400 and 500: dark hairlines that read on the lifted surfaces
STEPS = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950]

ACCENT_HUE = 252          # signal blue: far from every status hue, so selection never reads as a state
NEUTRAL_CHROMA = 0.006    # near-pure grey with a trace of the accent hue: status colour must carry meaning
HUES = {                  # hue, peak chroma, lightness of the peak
    "blue": (ACCENT_HUE, 0.19, 0.60),
    "green": (150, 0.17, 0.62),
    "amber": (72, 0.17, 0.78),
    "red": (27, 0.20, 0.60),
}
# Categorical hues for container names in merged logs: equal lightness, spaced hues, none equal to a status hue.
SERIES_HUES = [252, 196, 300, 340, 225]   # no greens, ambers or reds: a log name must never read as a status


def oklch_hex(L: float, C: float, H: float) -> str:
    def rgb(c: float) -> tuple[float, float, float]:
        a, b = c * math.cos(math.radians(H)), c * math.sin(math.radians(H))
        l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3
        m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3
        s = (L - 0.0894841775 * a - 1.2914855480 * b) ** 3
        return (4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
                -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
                -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s)

    def in_gamut(c: float) -> bool:
        return all(-1e-4 <= v <= 1 + 1e-4 for v in rgb(c))

    lo, hi = 0.0, C
    if not in_gamut(C):
        for _ in range(30):
            mid = (lo + hi) / 2
            lo, hi = (mid, hi) if in_gamut(mid) else (lo, mid)
        C = lo

    def enc(v: float) -> float:
        return 12.92 * v if v <= 0.0031308 else 1.055 * v ** (1 / 2.4) - 0.055

    return "#" + "".join(f"{round(enc(min(max(v, 0), 1)) * 255):02x}" for v in rgb(C))


def tent(L: float, peak_c: float, peak_l: float) -> float:
    if L >= peak_l:
        return peak_c * (1 - L) / (1 - peak_l)
    return peak_c * (L / peak_l) ** 0.8


def color(value: str) -> dict:
    return {"$value": value, "$type": "color"}


ramps: dict[str, dict] = {"neutral": {"0": color("#ffffff")}}
for step, L in zip(STEPS, NEUTRAL_L):
    ramps["neutral"][str(step)] = color(oklch_hex(L, NEUTRAL_CHROMA, ACCENT_HUE))
ramps["neutral-dark"] = {str(s): color(oklch_hex(L, NEUTRAL_CHROMA + 0.002, ACCENT_HUE))
                         for s, L in sorted([*zip(STEPS, NEUTRAL_DARK_L), NEUTRAL_DARK_LINE])}
for name, (h, c, pl) in HUES.items():
    ramps[name] = {str(s): color(oklch_hex(L, tent(L, c, pl), h)) for s, L in zip(STEPS, CHROMATIC_L)}


def role(ref: str, desc: str) -> dict:
    return {"$value": "{color." + ref + "}", "$type": "color", "$description": desc}


ROLES = {
    # name: (light ref, dark ref, description)
    "bg": ("neutral.100", "neutral-dark.100", "Window ground behind the sidebar and panels. Not for content areas; use surface."),
    "surface": ("neutral.0", "neutral-dark.200", "Main content area: tables, inspector, log viewer. Not for floating layers; use surface-raised."),
    "surface-raised": ("neutral.0", "neutral-dark.300", "Menus, popovers, sheets, the command palette. Pair with a shadow token."),
    "surface-sunken": ("neutral.50", "neutral-dark.50", "Wells inside a surface: code blocks, terminal, input fields. Not for whole panels."),
    "surface-inverse": ("neutral-dark.200", "neutral.100", "Tooltips and the toast. Text on it uses ink-inverse."),
    "surface-hover": ("neutral.100", "neutral-dark.300", "Hovered rows and buttons. Not for selection; use selection."),
    "surface-pressed": ("neutral.200", "neutral-dark.400", "Pressed rows and ghost buttons."),
    "selection": ("blue.100", "neutral-dark.300", "Selected table row and active sidebar item background. Not for hover; use surface-hover."),
    "ink": ("neutral.900", "neutral-dark.900", "Primary text and icons. Not for secondary metadata; use ink-muted."),
    "ink-muted": ("neutral.700", "neutral-dark.700", "Secondary text: paths, timestamps, column headers. Not for disabled; use ink-disabled."),
    "ink-disabled": ("neutral.400", "neutral-dark.500", "Disabled labels only; a disabled control still states why in its tooltip."),
    "ink-inverse": ("neutral.0", "neutral-dark.50", "Text on surface-inverse and on solid status fills."),
    "line": ("neutral.200", "neutral-dark.450", "Hairlines between rows and panels. Not for input borders; use line-strong."),
    "line-strong": ("neutral.500", "neutral-dark.600", "Input and checkbox borders, splitters on hover. Meets 3:1 against surface."),
    "accent": ("blue.700", "blue.300", "Primary button fill, links, selected tab underline, focus. One accent only; never a status."),
    "accent-hover": ("blue.800", "blue.200", "Primary button hover."),
    "accent-pressed": ("blue.900", "blue.100", "Primary button pressed."),
    "on-accent": ("neutral.0", "neutral-dark.50", "Label on an accent fill."),
    "success": ("green.700", "green.300", "Running and healthy: status dots, 'Running' text. Not for buttons."),
    "success-subtle": ("green.50", "green.950", "Background of a running badge or success banner."),
    "warning": ("amber.700", "amber.300", "Restarting, unhealthy, paused, amd64 under emulation."),
    "warning-subtle": ("amber.50", "amber.950", "Background of a warning badge or banner."),
    "danger": ("red.600", "red.300", "Exited with error, failed, and destructive buttons (Kill, Delete)."),
    "danger-subtle": ("red.50", "red.950", "Background of an error badge, banner or marked log line."),
    "danger-hover": ("red.700", "red.200", "Destructive button hover."),
    "on-danger": ("neutral.0", "neutral-dark.50", "Label on a danger fill."),
    "info": ("blue.700", "blue.300", "Created and informational notices. Shares the accent hue but is never a button."),
    "info-subtle": ("blue.50", "blue.950", "Background of an info badge or banner."),
    "stopped-subtle": ("neutral.100", "neutral-dark.300", "Background of a stopped or exited (0) badge. Not for hover; use surface-hover."),
    "stopped": ("neutral.500", "neutral-dark.700", "Stopped and exited-0 status dots: neutral on purpose, stopping is normal."),
    "accent-fill": ("blue.700", "blue.600", "Primary button fill. Saturated in both themes with a white label, the macOS pattern. Not for text; use accent."),
    "accent-fill-hover": ("blue.800", "blue.700", "Primary button hover."),
    "accent-fill-pressed": ("blue.900", "blue.800", "Primary button pressed."),
    "on-accent-fill": ("neutral.0", "neutral.0", "Label on accent-fill."),
    "danger-fill": ("red.600", "red.600", "Destructive button fill (Delete, Kill confirm). Not for text; use danger."),
    "danger-fill-hover": ("red.700", "red.700", "Destructive button hover."),
    "on-danger-fill": ("neutral.0", "neutral.0", "Label on danger-fill."),
    "success-dot": ("green.700", "green.400", "Running and Healthy status dots. Not for text; use success."),
    "warning-dot": ("amber.500", "amber.400", "Starting, Restarting, Unhealthy and Paused dots. Not for text; use warning."),
    "danger-dot": ("red.600", "red.500", "Exited with a non-zero code and Dead dots; the loudest mark on the screen. Not for text; use danger."),
    "info-dot": ("blue.600", "blue.400", "Created dots. Not for text; use info."),
    "log-error-ink": ("red.700", "red.300", "Whole-line text of an error log line."),
    "log-warn-ink": ("amber.700", "amber.300", "Whole-line text of a warning log line."),
    "tooltip": ("neutral-dark.200", "neutral-dark.300", "Tooltip background: inverse in light, raised in dark so it is never the brightest thing on a dark screen."),
    "tooltip-ink": ("neutral.0", "neutral-dark.900", "Tooltip text."),
    "tooltip-line": ("neutral-dark.200", "neutral-dark.450", "Tooltip border (visible in dark only)."),
    "focus": ("blue.600", "blue.400", "Focus ring, 2px outside the control. Meets 3:1 on every surface."),
}

tokens: dict = {"color": {**ramps}, "dark": {"color": {}}}
for name, (light, dark, desc) in ROLES.items():
    tokens["color"][name] = role(light, desc)
    tokens["dark"]["color"][name] = role(dark, desc)
for i in range(1, len(SERIES_HUES) + 1):
    hue, desc = SERIES_HUES[i - 1], "Container name colour in merged logs; categorical, never a status."
    tokens["color"][f"series-{i}"] = {**color(oklch_hex(0.52, 0.14, hue)), "$description": desc}
    tokens["dark"]["color"][f"series-{i}"] = {**color(oklch_hex(0.78, 0.12, hue)), "$description": desc}


def blend(top: str, under: str, alpha: float) -> str:
    """Flatten a translucent tint so every role stays a plain hex."""
    t = [int(top[i:i + 2], 16) for i in (1, 3, 5)]
    u = [int(under[i:i + 2], 16) for i in (1, 3, 5)]
    return "#" + "".join(f"{round(a * alpha + b * (1 - alpha)):02x}" for a, b in zip(t, u))


DARK_SURFACE = ramps["neutral-dark"]["200"]["$value"]
# Dark subtle backgrounds are the 950 step at 40% over the surface, flattened: a tint, never a slab.
for name, hue in (("success", "green"), ("warning", "amber"), ("danger", "red"), ("info", "blue")):
    tokens["dark"]["color"][f"{name}-subtle"] = {**color(blend(ramps[hue]["950"]["$value"], DARK_SURFACE, 0.4)),
                                                 "$description": tokens["dark"]["color"][f"{name}-subtle"]["$description"]}
for kind, hue in (("error", "red"), ("warn", "amber")):
    desc = f"Whole-line background of a {kind} log line, so the eye finds it first."
    tokens["color"][f"log-{kind}-bg"] = role(f"{hue}.50", desc)
    tokens["dark"]["color"][f"log-{kind}-bg"] = {**color(blend(ramps[hue]["950"]["$value"], DARK_SURFACE, 0.4)), "$description": desc}


def dim(v: str) -> dict:
    return {"$value": v, "$type": "dimension"}


TYPE_ROLES = {  # role: (size, line height, weight)
    "caption": ("11px", "16px", 400),
    "label": ("12px", "16px", 500),
    "body": ("13px", "20px", 400),
    "body-lg": ("15px", "22px", 400),
    "h3": ("13px", "20px", 600),
    "h2": ("15px", "20px", 600),
    "h1": ("18px", "24px", 600),
    "display": ("24px", "28px", 600),
    "code": ("12px", "18px", 400),
}
tokens["type"] = {
    "family": {
        "text": {"$value": "'IBM Plex Sans', system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif", "$type": "fontFamily"},
        "display": {"$value": "'IBM Plex Sans', system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif", "$type": "fontFamily"},
        "mono": {"$value": "'JetBrains Mono', ui-monospace, 'SF Mono', Menlo, monospace", "$type": "fontFamily"},
    },
    "size": {k: dim(v[0]) for k, v in TYPE_ROLES.items()},
    "line-height": {k: dim(v[1]) for k, v in TYPE_ROLES.items()},
    "weight": {k: {"$value": v[2], "$type": "fontWeight"} for k, v in TYPE_ROLES.items()},
    "tracking": {"display": dim("-0.01em"), "h1": dim("-0.005em")},
}
tokens["space"] = {str(n): dim(f"{n}px") for n in (0, 2, 4, 6, 8, 12, 16, 20, 24, 32, 40, 48)}
tokens["radius"] = {k: dim(v) for k, v in {"sm": "3px", "md": "5px", "lg": "8px", "full": "999px"}.items()}
tokens["size"] = {
    "control": {"sm": dim("24px"), "md": dim("28px"), "lg": dim("32px")},
    "row": {"md": dim("32px"), "compact": dim("24px")},
    "icon": {"sm": dim("14px"), "md": dim("16px")},
    "target": {"min": dim("24px")},
    "sidebar": dim("220px"),
    "inspector": dim("360px"),
    "titlebar": dim("38px"),
}
# Page gutter: 16 px, 20 px once the window is 1200 px wide (where the inspector docks).
tokens["layout"] = {"gutter": {"default": dim("16px"), "wide": dim("20px")}, "wide-min-width": dim("1200px")}
tokens["density"] = {"compact": {"size": {"row": {"md": dim("24px")}, "control": {"md": dim("24px")}}, "space": {"12": dim("8px")}}}
tokens["shadow"] = {
    "popover": {"$value": "0 4px 12px rgba(10, 14, 20, 0.12), 0 0 0 1px rgba(10, 14, 20, 0.06)", "$type": "shadow"},
    "sheet": {"$value": "0 12px 32px rgba(10, 14, 20, 0.18), 0 0 0 1px rgba(10, 14, 20, 0.08)", "$type": "shadow"},
    "dialog": {"$value": "0 20px 48px rgba(10, 14, 20, 0.24), 0 0 0 1px rgba(10, 14, 20, 0.08)", "$type": "shadow"},
}
tokens["motion"] = {
    "duration": {k: {"$value": v, "$type": "duration"} for k, v in {"instant": "0ms", "micro": "100ms", "short": "150ms", "enter": "150ms", "exit": "100ms"}.items()},
    "dwell": {"toast": {"$value": "5000ms", "$type": "duration"}, "toast-action": {"$value": "10000ms", "$type": "duration"}},
    "easing": {
        "standard": {"$value": "cubic-bezier(0.2, 0, 0, 1)", "$type": "cubicBezier"},
        "exit": {"$value": "cubic-bezier(0.3, 0, 1, 1)", "$type": "cubicBezier"},
    },
}

OUT.write_text(json.dumps(tokens, indent=2) + "\n", encoding="utf-8")
print(f"wrote {OUT}")
