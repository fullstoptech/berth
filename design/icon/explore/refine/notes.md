# MOORING 01 refine, round 2 notes

Owner direction: refine Mooring 01 (open bay holding one block). Owner's pick: **r2-a**. Name 12 was dropped before anything was drawn for it, so there is nothing to label superseded. Source: `design/icon/build_refine.py`, page "Refine" in `design/icon/berth-icon.figcraft`; round 1 (page "Explore") is untouched.

## Variants (design units, mark drawn on a 240 frame)

| | wall | side gap | seat gap | block | floor | wall height | mark W x H | ratio |
|---|---|---|---|---|---|---|---|---|
| r2-a, as drawn, balanced | 20 | 20 | 20 | 56 | 22 | 96 | 136 x 118 | 1.15 |
| r2-b, wider and lower slip | 16 | 18 | 8 | 60 | 18 | 50 | 128 x 86 | 1.49 |
| r2-c, b plus quay overhang 20 each side | 16 | 18 | 8 | 60 | 18 | 50 | 168 x 86 | 1.95 |

- r2-a: wall = side gap = seat gap = 20. The block top sits 76 above the floor top (seat 20 + block 56); the walls are 96 high, which is the block top plus one gap, so the block is held, not shown.
- r2-b: walls (50) stop below the block top (68), so the block clearly rises out of a slip. Ratio is measured on the whole mark (bay plus block); the bay alone is 128 x 68.
- r2-c: the quay overhang is the one added detail. At 16 px the 16-unit-high floor lip thins to a hairline and the mark reads as a table, so it was dropped from the tests and the tiles.

## Optical fixes, r2-a (and the same in b and c)

1. **Floor is 22, walls are 20.** Horizontal bars look thinner than vertical ones at the same weight; +2 on the floor evens the stroke.
2. **Floor and wall meet with a 3 unit inside fillet** (the junction notch). A sharp inside corner clots at 16 and 32 px and looks pinched at 1024. 3 is the largest radius that keeps the 20 unit gap to the block visibly square.
3. **Outer bottom corners 6, wall tops 2.** Flat matte: wall tops are almost square so the walls read as posts, the foot is softened a little so it sits with the block (radius 4). One radius family: 2, 3, 4, 6.
4. **No overshoot.** The bay has straight edges, no round, so there is nothing to overshoot; a +2 overshoot on the foot was tried in the sketch and made the floor look like it sags below the baseline of the wall.
5. **Block centred exactly** (20 each side). It is not nudged: the seat gap (20) equals the side gap, so the block reads as optically centred in the bay.
6. **Armchair read (b).** A first r2-b with 18 unit walls and a heavy floor read as an armchair or sofa (walls as arms). Fix: thinner posts (16), floor 18, seat 8 so the block rests in the slip.
7. **Tile.** The mark's box is fitted to 540 x 440 inside the 824 body and centred on its bounding box, with no optical shift.

## Colour

- Tile: warm cream `#F2EBDD` (no warm-neutral token in `design/tokens.json`; value from the art director).
- Bay: `#121416`, `color.neutral.950`.
- Tile 1 block: coral `#E86156`, `color.red.500`, the nearest token to the suggested `#E5533D`. `color.danger` / `red.600` (`#CC3430`) is the deeper vermilion if the coral is too soft.
- Tile 2 block: `#005EAB`, `color.accent` / `color.blue.700`, Berth's primary.
- Tile geometry: macOS grid, 1024 canvas, 824 body at (100, 100), corner radius 184 with 0.6 smoothing, transparent outside, no gradient, shadow or glow.

## Small sizes (looked at 10x nearest-neighbour renders of the real 16 and 32 px frames)

- r2-a black at 16 px: the block separates from the walls by one pixel row each side and from the floor by one; no counter closes and nothing clots, so no fix was needed. 32 px is clean.
- r2-a tile at 16 px: cream square, black U, a 3 px coloured block. This is the floor of what the mark can do and it holds.
- Standing risk named by the owner (inbox tray, bin, cup): the walls reach above the block, so beside a tray icon r2-a can read as one. r2-b is the answer if that matters.
