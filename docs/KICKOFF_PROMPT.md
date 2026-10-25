# Kickoff prompts

Paste the workhorse block into Claude Code (model: Sonnet) at the repository root to start a task. Use the same block for every task, changing only the task id and its branch. Use the architect block (model: Opus) at the start and end of each phase.

---

## Workhorse (task P0-T1)

You are the workhorse for Berth. Read `CLAUDE.md`, then `docs/STATUS.md`, then phase 0 in `docs/TASKS.md` (its "Learned" line first). Your task is **P0-T1**. Read the sections of `docs/PLAN.md` it cites (§5, §6.1, §12 item 4, D54, D62, D63) before writing anything.

Do only P0-T1. Implement exactly what it says, run its "Done when" command, and paste the output into `docs/STATUS.md`. If the task is a UI task, read its frame PNG in `design/shots/figcraft/screens/` before you start, and finish with `scripts/shot.sh` and a side-by-side read as TASKS.md describes. If you hit any of the five escalation conditions in `CLAUDE.md`, stop and write `docs/QUESTIONS.md` instead of improvising.

Finish with `scripts/check.sh` green (until P0-T5 creates it, run `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run --workspace`), update STATUS.md with the next task id, and commit on branch `phase/0-scaffold` as `P0-T1: workspace scaffold`. Do not push.

---

## Architect (phase N open or close)

You are the architect for Berth. Read `docs/PLAN.md` §3, `docs/STATUS.md`, `docs/QUESTIONS.md`, and phase N in `docs/TASKS.md`.

If opening the phase: create `phase/N-<name>` from `main`; answer every open question in QUESTIONS.md with a decision (and an ADR plus a PLAN edit if it changes §4 or §6); write the phase's "Learned" paragraph from STATUS.md (real API names, versions, gotchas, scripts that changed); adjust the phase's tasks only where STATUS.md proves an assumption wrong; commit.

If closing the phase: run the seven-step review gate in PLAN §3 by hand on this Mac, including the phase's acceptance check in PLAN §9; read each UI task's shots beside their frames and sign them off in STATUS.md; fix or send back anything that fails; merge to `main`, tag `v0.N` locally, bump the workspace version to `0.N.0`, and write the next phase's "Learned" paragraph. Never push or tag remotely without the owner's approval.
