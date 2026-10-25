# ADR 0006: Berth's MCP server, the bridge, and gated tools

Status: accepted (architect, 2026-10-24; PLAN §4 D44, §6.9)

## Context

Harnesses reach Docker only through Berth's tools (brief §6.12, §10 item 11): reads run at once, changes need one click, destructive actions need the name typed, and file edits come back as proposed changes. A harness launches MCP servers as its own child processes over stdio, so the MCP server cannot be the app process itself. The app must own every engine call, every approval and every write. `rmcp` 3.7 is the official Rust MCP SDK (features `server`, `macros`, `transport-io`).

## Decision

- `berth mcp --bridge <socket> --session <uuid>` is a mode of the same `berth` binary. It serves MCP over stdio with rmcp and forwards each tool call as one JSON line over a Unix socket in `paths::runtime_dir()` (mode 0700) to the running app. A random token in `BERTH_BRIDGE_TOKEN` authenticates it.
- The app's tool executor looks up the tool's `Risk`: Read runs at once; Change shows an `ApprovalCard` and waits; Destructive needs the item name typed; Deny returns a denial the harness sees. `files.propose_edit` creates a pending `GenerationPlan` and returns "waiting for the user's review"; only Review changes writes.
- Every result is redacted: `.env` values, container env values and Compose `environment` values become `••••` (D79). Every call is an Activity entry.

## Consequences

- One process owns I/O and approvals; harness crashes cannot leave half-applied actions.
- The bridge is a local protocol with its own types (PLAN §6.9); changing it changes both ends in one task.
- When Berth is not running, `berth mcp` fails at once with a clear message, so "Open in terminal" sessions need Berth open.
