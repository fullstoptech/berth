# ADR 0003: Engine API through bollard, Colima and Compose through their CLIs

Status: accepted (architect, 2026-10-24; PLAN §4 D27, D28, D29, D47)

## Context

Berth talks to three things: the Docker Engine API on each engine's socket, Colima (profiles, VM resources, start and stop), and Docker Compose (project up, down, build). Bollard 0.21.1 covers the Engine API including exec, archive, events, stats, `system df`, image search, and SSH and TLS transports (features `ssh`, `ssl`; checked in its source 2026-10-24). Neither Colima nor Compose has a Rust library: Colima is a Go CLI; Compose v2 is a Docker CLI plugin whose behaviour (interpolation, profiles, `depends_on`, `develop.watch`) is large and changes with each release.

On this Mac (2026-10-24): Colima 0.10.3 prints `colima list --json` as one object per line and `colima status --json` with `docker_socket`; `rosetta` and `vmType` live in `~/.colima/<profile>/colima.yaml`; Compose 5.5.1 supports `--progress json` and `config --format json`. Compose labels on containers carry `com.docker.compose.project`, `project.working_dir` and `project.config_files` (checked on a running project).

## Decision

- D29: one `berth_engine::discovery` module finds engines: Colima profiles, docker contexts with unix sockets (Ferry ADR 0003 order: `$DOCKER_HOST`, current context, context meta files), and user-added engines. No socket path is hardcoded elsewhere.
- Engine API: `bollard` behind `trait EngineClient` with `BollardEngine` and `MockEngine`.
- D28: Colima through `colima` subprocesses behind `trait ColimaControl`, parsing in pure functions tested on captured output.
- D27: Compose through `docker compose --project-directory … -f … -p … --progress json` with `DOCKER_HOST` set, behind `trait ComposeRunner`.
- D30: binaries are found through `SearchPath` (login-shell PATH merge, Ferry Q-Mac-2).
- D47: remote engines use bollard's `ssh` and `ssl` features; Compose on remote engines uses `DOCKER_HOST=ssh://…`.

## Consequences

- Compose behaviour always matches the user's own `docker compose`; Berth needs the Compose plugin installed (first-run check).
- CLI output formats are a contract: parsers live in one module each, with fixtures captured from real runs, and the IT suite runs them against the live engine.
- containerd and Incus profiles are listed but not managed (D69).
