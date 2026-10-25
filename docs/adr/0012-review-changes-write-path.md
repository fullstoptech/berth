# ADR 0012: One write path through Review changes

Status: accepted (architect, 2026-10-24; PLAN §4 D31, D32, D33)

## Context

Berth writes into the user's repository: generated Dockerfiles and Compose files, template services, rule fixes, and harness proposals. Brief §1.2 says a silent write "breaks trust for good". Brief §2 borrows "show the full effect before commitment": the same numbers appear on the confirm and on the result. YAML libraries that round-trip through serde drop comments and reorder keys.

## Decision

- Every file change is a `GenerationPlan` of `FileDiff`s. Only the Review changes sheet calls `berth_rules::write::apply`.
- Scope (architect, 2026-10-25, AD15): this covers every write Berth proposes to a project's files (compose, `.env`, generated files, rule fixes, harness proposals). A user-initiated export to a path the user picked in the native save panel (log Save, container file Download) is its own click — macOS confirms overwrites — and writes directly.
- `apply` writes through a temp file and rename in the same folder, refuses paths outside the project root, and refuses when a file changed since the plan was made (conflict state with Regenerate).
- Compose edits are text edits that keep comments (`compose_edit`); the result must parse and pass `docker compose config -q` before Write is enabled.
- Every destructive or writing action renders its button, confirm text and result from one `EffectSummary`.

## Consequences

- Harness proposals, recipes, templates and rule fixes share one tested path.
- Text edits cover the cases Berth generates (add a service, change a port, add `.env` lines); anything else is a proposal shown as a full-file diff.
