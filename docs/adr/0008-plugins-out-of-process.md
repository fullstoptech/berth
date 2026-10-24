# ADR 0008: Out-of-process plugins; Coolify as the first

Status: accepted (architect, 2026-10-24; PLAN §4 D45, D46, D76, §6.10)

## Context

The owner wants Coolify "as the first plugin, not built in" and the project is open source, so outside authors will write plugins. A plugin holding a Coolify token can deploy to production (brief §1.2 risks). The brief's default is a separate process with a manifest and declarative views (brief §9). Coolify's OpenAPI spec at tag v4.5.0 has 212 paths and includes every endpoint brief §6.11 uses (`/version`, `/health`, `/resources`, `/servers`, `/deployments`, `/deploy`, `/applications/{uuid}/logs`, `/applications/{uuid}/restart`, `/deployments/{uuid}/cancel`, `/deployments/applications/{uuid}`; checked 2026-10-24).

Alternatives: WASM plugins (sandboxed, but a large host API and one language toolchain per author), or Coolify built in (rejected by the owner).

## Decision

- Plugins are executables started by `berth-plugin-host`, speaking JSON-RPC 2.0, one JSON object per line on stdio (`berth-plugin-protocol`, which has no internal dependencies so authors can use it).
- A manifest declares permissions (`network` hosts, `read_engines`, `run_engine_actions`, `secrets` keys, `mcp`) and contributions (sidebar section, project badge, palette commands). The user sees them on enable.
- Plugins draw no UI. They return `View` values (table, detail, log stream, message, form) that the app renders with its own components.
- Secrets reach a plugin only through `host/secret.get` for declared and granted keys.
- Coolify is `plugins/coolify` (binary `berth-coolify`), a hand-written reqwest client for the endpoints in brief §6.11, with the spec vendored and pinned.

## Consequences

- Any language can write a plugin; a crash cannot take Berth down (the host restarts it with backoff).
- Network hosts are declared, not enforced, in v1 (a later sandbox ADR).
- New view kinds are protocol changes and need a protocol version bump.
