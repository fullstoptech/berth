# Berth app icon directions

Three hand-authored SVG directions, 1024 canvas, 824 px squircle body (Apple grid), transparent outside, soft drop shadow. No text. See `compare.png` (light and dark backgrounds, 1024 scaled plus 32 pt and 16 pt; sheet is @2x).

| | Concept | Palette (tokens from `design/tokens.json`) |
|---|---|---|
| **A: Slip** (`berth-icon-a.svg`) | A blue container sliding into a light C-shaped slip: the ship entering its berth, on the app's dark chrome. | body `neutral-dark-400` to `neutral-dark-50`; slip `neutral-dark-950` to `neutral-dark-800`; container `blue-400`, `blue-500`, `blue-600`; ridges `blue-950` |
| **B: Hull folder** (`berth-icon-b.svg`) | A container is known by the folder it came from: a white folder whose body narrows into a hull, carrying a navy container. | body `blue-500`, `blue-600`, `blue-800`; folder `neutral-0` to `neutral-300`; container `blue-900`, ridges `blue-400` |
| **C: Quay stack** (`berth-icon-c.svg`) | Three containers stacked on a quay line; the top one is blue, the selected one. Light body for contrast in a dark Dock. | body `neutral-50` to `neutral-300`; blocks `neutral-700` to `neutral-950`; top block `blue-500` to `blue-700`; ridges `blue-950` / white |

Files per direction: `.svg`, `-1024.png`, `-64.png`, `-32.png` (transparent PNGs). Rendered with a small WKWebView snapshot tool (Swift, in the session scratchpad), because `rsvg-convert` is not installed and `qlmanage` gave opaque white backgrounds. Geometry comes from `gen.py` in the same scratchpad; the SVGs are plain and standalone.

## Legibility at 32 and 16 pt

- A: container block and slip read at 32; at 16 it is a blue bar with a grey bracket. On a dark Dock the body edge sinks (hence the 18% top highlight).
- B: clearest silhouette at every size; the folder is instantly legible, the navy container is a 3 px mark at 16.
- C: reads at 32 as a stack; at 16 it is the weakest (ridges and quay line merge). Two ridges per block instead of three after the first check.

## Picked: B, flat

The owner picked B and asked for no gradients, shadows or glows. `berth-icon-b.svg` and its PNGs are now fully flat: one solid fill per shape, no filters, no overlays, geometry unchanged. A and C are kept as drafted (gradient, shadow) for reference only. `compare.png` now shows flat B only, on light and dark backgrounds.

| Shape | Fill | Token |
|---|---|---|
| Squircle body | `#0075d2` | `blue-600` |
| Folder (hull) | `#e5e8ec` | `neutral-200` |
| Container tab | `#003362` | `blue-900` |
| Container ridges | `#7ab5f8` | `blue-400` |

Why it works: the folder reads as a folder, and the navy container with light ridges makes it Berth's hull-folder rather than a plain Finder folder.
