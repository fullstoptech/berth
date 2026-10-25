# ADR 0009: Plain CSS generated from tokens; no Tailwind

Status: accepted (architect, 2026-10-24; PLAN §4 D4, D5, D7, §7.7)

## Context

`.eng/stack.md` says "CSS variables generated from design/tokens.json". Ferry used Tailwind v4 through dx and hit two problems: dx 0.7.10 runs Tailwind on its own when a `tailwind.css` file exists in the crate, and during `dx bundle` it rewrote the committed CSS with a smaller file, so the shipped binary did not embed the reviewed CSS until the release script disabled dx's Tailwind (Ferry STATUS, Mac session §2). Ferry also found that `data-theme` on an inner element cannot override a dark OS preference.

## Decision

- `scripts/gen-tokens.py` turns `design/tokens.json` into `crates/berth-app/assets/css/tokens.css` with role variables, dark values under the media query and `:root[data-theme="dark"]`, density and reduced-motion overrides (PLAN §7.7). `check.sh` fails when the committed file is stale.
- Each component has its own CSS file next to the others in `assets/css/`; `assets.rs` concatenates them with `include_str!` into the custom head.
- No hex colours in component CSS (`scripts/css-lint.sh`). No file named `tailwind.css` anywhere in the app crate.
- Theme and density attributes live on `document.documentElement`.

## Consequences

- No build step between source and shipped CSS; what is reviewed is what ships.
- More CSS written by hand, but every rule maps to a component in the design.
