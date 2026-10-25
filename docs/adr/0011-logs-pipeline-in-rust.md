# ADR 0011: Logs pipeline in Rust, batched per frame

Status: accepted (architect, 2026-10-24; PLAN §4 D25, D26, §6.6, §7.6)

## Context

Brief §1.3 asks for 10,000 log lines a second without dropped frames, a capped buffer, and search that runs in Rust. A webview that receives one message per line, or holds every line in the DOM, cannot do that.

## Decision

- `berth-logs` holds a ring buffer of 100,000 lines per view. Level, error marks, JSON summaries and stack-trace folding are computed once per line on push.
- `batch()` merges container streams and yields at most one batch per 16 ms tick. The view publishes one version bump per batch; the UI renders only the visible rows plus overscan, asking `LogView::rows(range)`.
- Search (plain or regex), filters, Next error and the text for Copy, Save and Ask about these lines run in Rust.
- Stats follow the same rule: only visible rows subscribe (D26).

## Consequences

- A throughput test (`berth-logs`, release build) and the follow-at-10k-lines check in the running app are both phase 3 acceptance checks.
- Lines older than the buffer are dropped with a visible notice; Save writes what the buffer holds.
