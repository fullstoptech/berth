# Category sheet: the icons Berth sits beside

Loaded 2026-10-28. Sheet: `category-sheet.png` (source `category-sheet.html`, rendered with headless Chrome; every icon at 128 px, as a thresholded black silhouette, and at 32 px). Saved images are in `src/` (from the product's own site or official repo) and `local/` (from the installed .app). Colours are the top pixel buckets measured by the sheet script, not brand-book values. Observations are from the loaded image; "idea" is my reading of it, not a statement from the maker. Internal comparison only, never shipped.

Where an image is the product's logo or a site icon rather than the Dock tile, it says so. Nothing here was checked against the installed Dock icon except Fork and Proxyman.

| # | Product | Image status |
|---|---|---|
| 1 | Docker Desktop | Image of the official whale mark (site favicon). **No image of the Dock tile.** |
| 2 | OrbStack | Image (site icon asset). Installed Dock tile not checked. |
| 3 | Podman Desktop | Image (repo app icon) |
| 4 | Rancher Desktop | Image (repo square logo). Installed Dock tile not checked. |
| 5 | Portainer | Image (repo touch icon). **No Mac app**; web UI only. |
| 6 | Colima | Image of README logo. **No app icon** (CLI). |
| 7 | Lima | Image of site wordmark. **No app icon** (CLI). |
| 8 | Kitematic (legacy) | Image (archived repo) |
| 9 | TablePlus | Image (site touch icon). Installed Dock tile not checked. |
| 10 | Tower | Image (site download-modal icon) |
| 11 | Fork | Image (installed .icns) |
| 12 | Proxyman | Image (installed .icns) |

## Per icon

**1. Docker Desktop.**
- Source: https://www.docker.com/app/uploads/2024/02/cropped-docker-logo-favicon-270x270.png (found on https://www.docker.com/products/docker-desktop/), loaded 2026-10-28. Saved `src/docker-site-icon.png`. Page header image https://www.docker.com/app/uploads/2023/06/meta-image-download-docker-desktop-1110x580.png (read, not saved) shows the same whale in white on blue with outline cubes.
- Shape: a whale in profile, tail up at the right, a stack of square blocks on its back. Silhouette reads as a whale at 32 px.
- Colour: one flat blue, `#1d63ed`, on white.
- Idea: the whale carries containers. The Dock tile itself was not available from an official page, so its tile shape and background are unknown to me.

**2. OrbStack.**
- Source: https://orbstack.dev/img/icon-square256.png (named in the `og:image` of https://orbstack.dev/), loaded 2026-10-28. Saved `src/orbstack.png`.
- Shape: one glossy sphere, centred, on a square pale-blue ground. Silhouette is a circle; the tile adds nothing.
- Colour: violet to magenta sphere (`#af8be7`, magenta shading) on pale blue (`#b6d1fe`, `#adbafc`, `#bee2ff`).
- Idea: an orb (the product's name). Soft, 3D, no glyph.

**3. Podman Desktop.**
- Source: https://raw.githubusercontent.com/containers/podman-desktop/main/buildResources/icon.png (official repo https://github.com/containers/podman-desktop), loaded 2026-10-28. Saved `src/podman-desktop.png`.
- Shape: dark squircle, a hexagon outline in the centre, a shipping-container block with four vertical slots inside it.
- Colour: near-black purple (`#150c29`, `#290f45`, `#08051c`); hexagon edge gradient from sky blue through violet to magenta; pale lilac container.
- Idea: a container in a hexagon. The hexagon plus container is the whole mark.

**4. Rancher Desktop.**
- Source: https://raw.githubusercontent.com/rancher-sandbox/rancher-desktop/main/resources/icons/logo-square-512.png (official repo https://github.com/rancher-sandbox/rancher-desktop), loaded 2026-10-28. Saved `src/rancher-desktop.png`. The site also links `/images/icon-rancher-desktop.svg` at https://rancherdesktop.io/ (not saved).
- Shape: a landscape rounded rectangle (not a square), white glyph cut out: a rounded box in the middle, a bracket-like curve at each upper side, a bar below. Reads as a horned animal head or a bracketed box.
- Colour: one flat blue `#0081be`, white glyph.
- Idea: Rancher's horned-head logo reduced to boxes.

**5. Portainer.**
- Source: https://raw.githubusercontent.com/portainer/portainer/develop/app/assets/ico/apple-touch-icon.png (official repo https://github.com/portainer/portainer), loaded 2026-10-28. Saved `src/portainer.png`.
- Shape: a tall near-black rounded rectangle with a white letter P and a square full stop. Typographic.
- Colour: charcoal `#2e2f33`, off-white `#f7f6f3`.
- Idea: the letter. Portainer is a web UI served from a container; I found no macOS app icon for it.

**6. Colima.**
- Source: https://raw.githubusercontent.com/abiosoft/colima/main/colima.png (README of https://github.com/abiosoft/colima), loaded 2026-10-28. Saved `src/colima.png`. No app icon exists; Colima is a command-line tool.
- Shape: a line-drawn llama carrying crates, beside the wordmark "Colima". Wide, not square.
- Colour: black on white, with green crates and a green dot in the "i".
- Idea: a llama with cargo (Colima = "container on Lima", llama pun).

**7. Lima.**
- Source: https://raw.githubusercontent.com/lima-vm/lima/master/website/static/images/logo.svg (official repo https://github.com/lima-vm/lima), loaded 2026-10-28. Saved `src/lima.svg`. No app icon exists; Lima is a command-line tool.
- Shape: the word "Lima" in a heavy geometric sans, the dot of the "i" a circle. Wordmark only.
- Colour: black with a lime-green dot `#a9cf38`.
- Idea: a type logo with one green dot.

**8. Kitematic (legacy).**
- Source: https://raw.githubusercontent.com/docker-archive-public/docker.kitematic/HEAD/util/kitematic.png (archived repo https://github.com/docker-archive-public/docker.kitematic), loaded 2026-10-28. Saved `src/kitematic.png`. (The older URL github.com/docker-archive/kitematic returned 404 on this date.)
- Shape: a flat iOS-style rounded square with a bold white "K" and a second tilted shape that looks like an "I" or a cube edge.
- Colour: flat sky blue `#3cb5e8`, white glyph.
- Idea: monogram.

**9. TablePlus.**
- Source: https://tableplus.com/resources/favicons/apple-icon-180x180.png (linked from https://tableplus.com/), loaded 2026-10-28. Saved `src/tableplus.png`. This is the site's touch icon; I did not confirm that it matches the Mac Dock tile.
- Shape: an origami elephant in profile, no tile. The silhouette breaks apart under threshold (fold shading is the shape).
- Colour: yellow to orange (`#ffcd2d`, `#feb017`, `#fc9212`).
- Idea: a faceted elephant (the PostgreSQL elephant, in folded paper).

**10. Tower.**
- Source: https://www.git-tower.com/assets/img/icons/downloads-modal-app-icon.1792594336.png (on https://www.git-tower.com/mac), loaded 2026-10-28. Saved `src/tower.png`.
- Shape: blue squircle, a glass control-tower cab on a pale shaft, a black and yellow chequered band at the waist.
- Colour: blue (`#6c97fc`, `#5377e7`, `#3b59cd`) with black and yellow.
- Idea: a control tower (the name, "Tower"). Silhouette is a mess at 32 px; the checker band carries it.

**11. Fork.**
- Source: local `/Applications/Fork.app/Contents/Resources/AppIcon.icns`, read 2026-10-28 with `sips`. Saved `local/fork.png`.
- Shape: macOS squircle, one white four-tine fork, vertical, centred.
- Colour: cyan to cerulean (`#0fb5e9`, `#0192ca`), white fork.
- Idea: a fork (the git fork), drawn as cutlery. One object, very readable.

**12. Proxyman.**
- Source: local `/Applications/Proxyman.app/Contents/Resources/AppIcon.icns`, read 2026-10-28 with `sips`. Saved `local/proxyman.png`.
- Shape: macOS squircle with an inner outline, an astronaut helmet with a dark visor, three-quarter view.
- Colour: bright blue (`#0fadee`, `#0f6de2`, `#0f91e8`), white helmet, grey visor.
- Idea: a helmet (a person looking through a window). Glossy, 3D.

## Taken (avoid)

Shapes and ideas:
- The whale, and any whale-like animal carrying boxes (Docker). The llama carrying crates is also taken (Colima).
- The generic container box or cube, and a stack of square blocks (Docker's blocks, Podman's container in a hexagon, Rancher's boxed glyph).
- A hexagon holding a container (Podman Desktop).
- A single glossy sphere or orb (OrbStack).
- A monogram letter in a tile: K (Kitematic), P (Portainer).
- A single everyday object in a squircle: fork (Fork), helmet (Proxyman), control tower (Tower).
- A faceted or origami animal (TablePlus).
- A pure wordmark with a green dot (Lima).

Colours:
- Flat or gradient blue is the most crowded: Docker `#1d63ed`, Rancher `#0081be`, Kitematic `#3cb5e8`, Fork and Proxyman cyan to cerulean, Tower periwinkle. Five of twelve icons are blue-led.
- Dark purple to violet to magenta: Podman Desktop and OrbStack.
- Yellow and orange: TablePlus (and Tower's band).
- Charcoal with off-white: Portainer.
- Black and white line work with one green accent: Colima, Lima.

## Open space (what no one here uses)

Observed gaps in these twelve only; I did not survey the wider app market, so each needs a check before it is called free.
- Warm non-yellow hues: red, vermilion, coral. None of the twelve is red-led.
- Green as a tile ground (green appears only as a dot or accent in Colima and Lima).
- A near-white or cream tile with a dark single-colour glyph (only Portainer inverts, in charcoal).
- A mark that is not a container, an animal, a letter or a tool: for example a place (a berth, a mooring, a dock slip, a bollard or a harbour cleat), a ship's cleat, a line and loop, a plain geometric shape.
- Flat, single-colour glyphs with no gloss. Only Kitematic, Rancher and Portainer are flat; most of the Mac-native tiles (Fork, Proxyman, Tower, OrbStack) are glossy 3D.
- A horizontal or "slot" form (a berth is a place to tie up) instead of a vertical stack or a cube. Podman's container is horizontal, but inside a hexagon.
- A folder idea ("a container is known by the folder it came from"): no icon in this set shows a folder or a tab shape.

## Not done

- Docker Desktop's real Dock tile was not obtained from an official page; the whale mark stands in for it.
- OrbStack, Rancher Desktop and TablePlus images are site or repo assets, not checked against the installed app.
- No trademark or look-alike clearance has been done on any of this.
- Portainer, Colima and Lima have no macOS app icon of their own; they are on the sheet as category neighbours.
