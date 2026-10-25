# ADR 0010: Menu bar item and the macOS platform layer

Status: accepted (architect, 2026-10-24; PLAN §4 D3, D8, D38, D39, D42, D67)

## Context

The owner said yes to a menu bar item (brief §6.14). Closing the window keeps it running. dioxus-desktop 0.7.10 ships `trayicon` (tray-icon 0.21, native muda menus), `WindowCloseBehaviour::WindowHides`, and re-exports `wry`, `tao` and `muda` (checked in its source 2026-10-24). Ferry on this Mac needed an integrated title bar with wry's `set_traffic_light_inset` (tao's own inset is lost because wry replaces the content view) and a native Edit menu so Cmd+C and Cmd+V work in inputs.

## Decision

- All macOS-only API calls live in `crates/berth-app/src/platform/macos.rs`: title bar, app menu, tray item, dock icon policy, Open in Terminal app. Other files call the `platform` module only.
- The menu's content is a pure `menu_model(&AppSnapshot) -> MenuModel` (engines with Start/Stop, pinned and running projects, failing containers with Show logs, Open Berth, Quit), unit-tested. The platform layer turns it into muda items and routes clicks to the same actions the buttons use.
- The menu is native, not a webview popover; status uses small template icons plus words (D80).

## Consequences

- The 320 px frames guide content and order, not pixels.
- A richer popover would need a second webview window; that is Later.
- A Linux port later replaces one file.
