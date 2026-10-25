use dioxus::prelude::*;

use super::engine_switcher::engine_dot;
use super::icon::{Icon, IconSize};
use crate::state::{AppState, BottomTab, EngineScope};

const GIB: u64 = 1 << 30;

/// VM memory as the profile was configured: Colima reports whole GiB
/// ("8 GB" in the UI), so an exact GiB multiple prints as such; anything
/// else uses the decimal formatter.
fn vm_memory(vm: &berth_core::VmResources) -> String {
    let bytes = vm.memory.as_u64();
    if bytes > 0 && bytes.is_multiple_of(GIB) {
        format!("{} GB", bytes / GIB)
    } else {
        berth_core::fmt::bytes(vm.memory)
    }
}

/// "colima/default", "ssh/hetzner-1": the engine as the status bar names it.
fn engine_title(engine: &berth_core::Engine) -> String {
    use berth_core::{EngineKind as K, RemoteTransport as T};
    match &engine.kind {
        K::Colima { profile } => format!("colima/{profile}"),
        K::Remote {
            transport: T::Ssh { .. },
        } => format!("ssh/{}", engine.name),
        K::Remote {
            transport: T::Tcp { .. },
        } => format!("tcp/{}", engine.name),
        K::Socket { .. } => engine.name.clone(),
    }
}

/// The status bar's left text (PLAN 7.1): engine, Docker version, VM size
/// and running count for the scoped engine, e.g.
/// "colima/default · Docker 28.4.0 · 4 CPU · 8 GB · 15 running". An engine
/// that is not running says its state instead of numbers it cannot know.
pub fn engine_summary(engines: &[berth_core::Engine], scope: &EngineScope) -> String {
    if engines.is_empty() {
        return "Looking for engines\u{2026}".to_string();
    }
    match scope {
        EngineScope::All => {
            let running: u32 = engines
                .iter()
                .filter(|e| e.state == berth_core::EngineState::Running)
                .map(|e| e.counts.containers_running)
                .sum();
            format!(
                "All engines \u{b7} {} engines \u{b7} {running} running",
                engines.len()
            )
        }
        EngineScope::One(id) => {
            let Some(engine) = engines.iter().find(|e| e.id == *id) else {
                return format!("{} \u{b7} Not found", id.0);
            };
            let mut parts = vec![engine_title(engine)];
            if engine.state == berth_core::EngineState::Running {
                if let Some(version) = &engine.docker_version {
                    parts.push(format!("Docker {version}"));
                }
                if let Some(vm) = &engine.resources {
                    parts.push(format!("{} CPU", vm.cpus));
                    parts.push(vm_memory(vm));
                }
                parts.push(format!("{} running", engine.counts.containers_running));
            } else {
                parts.push(engine_dot(&engine.state).label.to_string());
            }
            parts.join(" \u{b7} ")
        }
    }
}

/// "3 problems", "1 problem", "No problems".
pub fn problems_label(count: usize) -> String {
    match count {
        0 => "No problems".to_string(),
        1 => "1 problem".to_string(),
        n => format!("{n} problems"),
    }
}

/// The problems count in the title bar and the status bar; click opens
/// the Problems tab of the bottom panel.
#[component]
pub fn ProblemsChip() -> Element {
    let mut state = use_context::<AppState>();
    let count = state.problems.read().len();
    let label = problems_label(count);
    let icon_name = if count == 0 {
        "circle-check"
    } else {
        "alert-circle"
    };
    let mut class = "problems-chip".to_string();
    if count == 0 {
        class.push_str(" problems-chip-none");
    }
    rsx! {
        button {
            class: "{class}",
            r#type: "button",
            "aria-label": "{label}. Open Problems",
            onmousedown: move |e| e.stop_propagation(),
            ondoubleclick: move |e| e.stop_propagation(),
            onclick: move |_| {
                let mut bottom = state.bottom.write();
                bottom.open = true;
                bottom.tab = BottomTab::Problems;
            },
            span { class: "problems-chip-icon",
                Icon { name: icon_name, size: IconSize::Md }
            }
            span { "{label}" }
        }
    }
}

#[component]
pub fn StatusBar() -> Element {
    let state = use_context::<AppState>();
    let summary = engine_summary(&state.engines.read(), &state.engine_scope.read());
    rsx! {
        footer { class: "statusbar",
            span { class: "statusbar-summary", "{summary}" }
            ProblemsChip {}
        }
    }
}
