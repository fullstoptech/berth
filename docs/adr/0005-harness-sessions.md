# ADR 0005: The assistant drives installed harnesses as sessions

Status: accepted (architect, 2026-10-24; PLAN §4 D43, D71, D77)

## Context

The owner wants the AI "interactive", able to read container errors and fix Compose files, auto-detecting "the Claude harness or the Muse harness" (`.eng/notes.md`). Both CLIs are installed on this Mac in `~/.local/bin` (Claude Code 2.1.309, Muse Code 1.4.12, 2026-10-24).

Verified with `--help` on 2026-10-24: Claude Code has `-p`, `--input-format stream-json`, `--output-format stream-json` (needs `--verbose`), `--include-partial-messages`, `--mcp-config`, `--strict-mcp-config`, `--tools ""`, `--allowedTools`, `--setting-sources`, `--disable-slash-commands`, `--session-id <uuid>`, `--resume`. Muse has `serve` (an MSP v1 session host over stdio, flags `--disable-shell`, `--disable-write`, `--no-session-log`), `schema generate-json-schema` (fingerprint `sha256:cfdb6d2525601b9bac76584ccb0610ab3a5539b5e4f51141f079b99b39fcb4e0`), and `exec --json --workspace --prompt-file`. The MSP schema has `session/start` with `workspaceRoot` and `config.mcpServers` (stdio transport: `command`, `args`, `env`), `turn/start`, `turn/interrupt`, `approval/decide`, and notifications `item/delta`, `item/started`, `item/completed`, `turn/completed`, `approval/requested`.

Ferry learned that Claude Code with the user's settings and plugins loaded costs about 50 times more per prompt (`--setting-sources ""` cut 109k cache-creation tokens to 2k), and that a Finder-launched app cannot find `~/.local/bin` without the login-shell PATH.

## Decision

- Detection searches `SearchPath` (D30) plus user-chosen binaries, and runs `--version`.
- Claude Code: one long-lived `claude -p` process per session with stream-json in and out, Berth's MCP server as the only tool source (`--tools "" --strict-mcp-config --mcp-config <file> --allowedTools mcp__berth`), `--setting-sources "" --disable-slash-commands`, `--session-id` on start and `--resume` on reopen.
- muse: `muse serve --disable-shell --disable-write` per session; `session/start` with the project folder as `workspaceRoot` and Berth's MCP server in `config.mcpServers`; approvals muse raises for `berth` tools are answered "allow" by Berth because Berth gates them itself (ADR 0006); anything else is denied. One-shot fallback: `muse exec --json`.
- The harness never writes files and never runs shell commands; watch mode never applies a change on its own (D71); ⌘. in the Assistant interrupts the turn (D77).

## Consequences

- Assistant quality depends on the harness the user already pays for; Berth sends only the context it shows as chips.
- Flags are constants in one table per adapter; `BERTH_HARNESS_SMOKE=1` tests run the real CLIs.
- Unverified until P7-T3/T4: the exact `--allowedTools` pattern for MCP tools, and muse's approval choices for MCP tools. The tasks verify both first.
