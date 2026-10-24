# ADR 0002: Design-first UI, mock mode and scenes

Status: accepted (architect, 2026-10-24; PLAN §4 D57, D58, §7.2, §11)

## Context

The design is finished before code: `design/berth.figcraft` has 34 screen frames and 64 components, signed off on 2026-10-24, with PNG exports in `design/shots/figcraft/screens/`. Ferry built its UI to the frames from the first task (Ferry ADR 0007) and still needed a 78-frame audit at the end, partly because reaching each state by hand was slow. Berth wants every frame's state reachable in one command, on real WKWebView, during the task that builds it.

## Decision

- UI tasks build components and screens to the FigCraft frames from the start, using tokens from `design/tokens.json`. No later re-skin.
- D58: `BERTH_MOCK=1` runs the app on `MockEngine`, `MockColima` and `MockCompose` loaded from `fixtures/mock/berth.json`, whose data is the data the frames show. `BERTH_SCENE="<frame name>"` puts the UI into that frame's state.
- Every UI task adds the scenes for its frames to `crates/berth-app/src/scenes.rs`. An SSR test (`dioxus-ssr`) renders each scene and asserts its copy, states and structure.
- D57: the task ends with `scripts/shot.sh <name> --scene "<frame>"` (osascript + `screencapture`, Ferry's macOS precedent) and a side-by-side read of the shot and the frame PNG. Differences are fixed in the task or recorded as a question.

## Consequences

- A UI task's done-when is visual and repeatable; P10 re-captures every scene for the final audit with one loop.
- Mock data is a maintained fixture; changes to core types update it in the same task.
- Screens with no frame are built from the components page and the nearest frame's layout (PLAN §7.2).
