# Goal — build Berth from PLAN.md

## Refined prompt

> Implement Berth exactly as `docs/PLAN.md` and `docs/TASKS.md` specify, phases 0 to 10, on this Apple Silicon Mac.
> Berth is a Rust + Dioxus 0.7 desktop Docker manager for Colima: folder-first projects, dense tables with visible action buttons, logs, terminal, images, volumes, networks, engines, problems with rule-based fixes, a menu bar item, an assistant that drives Claude Code or muse through Berth's own MCP tools, remote engines, registries, deploy from Git, and Coolify as an out-of-process plugin (brief: `design/product-brief.md`).
> The UI is built to the frames in `design/berth.figcraft` (PNGs in `design/shots/figcraft/screens/`) from the first UI task (ADR 0002). Every UI task ends with a shot of its scene read beside the frame.
>
> Roles (PLAN §3): the Opus session is the architect. It opens each phase, writes the briefs, answers `docs/QUESTIONS.md`, and runs the seven-step review gate at each phase close. Sonnet sessions (or Sonnet subagents under `/orchestrator`) are the workhorses, one task per worker, in task order; the parallel work in PLAN §9.2 uses separate worktrees.
>
> Environment: this Mac only (CLAUDE.md "Environment"). Docker is Colima; tests touch only `berthfx` fixtures labelled `dev.berth.fixture=1`. Nothing is pushed, tagged remotely or released without the owner's approval; signing and notarization wait for the owner's go-ahead (Q4).

## Definition of done

1. Every task in `docs/TASKS.md` has its done-when output pasted in `docs/STATUS.md`.
2. `scripts/check.sh` exits 0 on `main`, and `scripts/it.sh` passes against the fixtures.
3. Each phase's acceptance check (PLAN §9) has passed by hand on this Mac, and local tags `v0.0` to `v0.9` exist.
4. All 34 FigCraft frames are captured from their scenes and signed off in STATUS.md (matched, fixed, or deviation accepted with a reason), plus one line per screen with no frame.
5. The performance budget of brief §1.3 is met and the numbers from `scripts/perf.sh` are in STATUS.md.
6. `scripts/release.sh` produces `Berth-1.0.0-arm64.dmg` under 30 MB; signed and notarized (`spctl` accepted, `stapler validate` ok) when the owner supplied the identity and profile; the clean-install walk of P10-T6 passed.
7. `docs/QUESTIONS.md` has no open entries.
8. The workspace is 1.0.0, `phase/10-ship` is merged into `main`, and `v1.0.0` is tagged locally.

## /goal line

```
/goal Berth is built per docs/GOAL.md: every task in docs/TASKS.md is done with its output in docs/STATUS.md, scripts/check.sh and scripts/it.sh are green on main, all 34 frames are signed off, the release DMG passes the clean-install test, docs/QUESTIONS.md has no open entries, and v1.0.0 is tagged locally. Nothing is pushed or released without the owner's approval.
```
