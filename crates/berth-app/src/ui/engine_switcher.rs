use dioxus::prelude::*;

use super::icon::{Icon, IconSize};
use super::sidebar_item::{Dot, DotShape, DotSpec};
use crate::state::{AppState, EngineScope};

/// Does an engine belong to the current scope? (D81: one engine or all.)
pub fn scope_includes(scope: &EngineScope, engine: &berth_core::EngineId) -> bool {
    match scope {
        EngineScope::All => true,
        EngineScope::One(id) => id == engine,
    }
}

/// The dot for an engine's state (Starting/Stopping are rings).
pub fn engine_dot(state: &berth_core::EngineState) -> DotSpec {
    use berth_core::{EngineState as E, Tone};
    let (tone, shape, label) = match state {
        E::Running => (Tone::Success, DotShape::Solid, "Running"),
        E::Stopped => (Tone::Stopped, DotShape::Solid, "Stopped"),
        E::Starting => (Tone::Warning, DotShape::Ring, "Starting"),
        E::Stopping => (Tone::Warning, DotShape::Ring, "Stopping"),
        E::Unreachable { .. } => (Tone::Danger, DotShape::Solid, "Unreachable"),
        E::Unsupported { .. } => (Tone::Stopped, DotShape::Hollow, "Unsupported"),
    };
    DotSpec { tone, shape, label }
}

fn kind_label(kind: &berth_core::EngineKind) -> &'static str {
    use berth_core::{EngineKind as K, RemoteTransport as T};
    match kind {
        K::Colima { .. } => "Colima",
        K::Remote {
            transport: T::Ssh { .. },
        } => "SSH",
        K::Remote {
            transport: T::Tcp { .. },
        } => "TCP",
        K::Socket { .. } => "Socket",
    }
}

fn kind_icon(kind: &berth_core::EngineKind) -> &'static str {
    match kind {
        berth_core::EngineKind::Remote { .. } => "cloud",
        _ => "server",
    }
}

/// What the switcher button names: the engine, or "All engines".
pub fn scope_label(engines: &[berth_core::Engine], scope: &EngineScope) -> String {
    match scope {
        EngineScope::All => "All engines".to_string(),
        EngineScope::One(id) => engines
            .iter()
            .find(|e| e.id == *id)
            .map_or_else(|| id.0.clone(), |e| e.name.clone()),
    }
}

/// Title-bar engine picker (D81): shows the scoped engine with its state
/// dot; the menu lists every engine with a dot and "All engines".
/// `initially_open` exists for the "open menu" state in tests and scenes.
#[component]
pub fn EngineSwitcher(#[props(default)] initially_open: bool) -> Element {
    let mut state = use_context::<AppState>();
    let mut open = use_signal(move || initially_open);
    let engines = state.engines.read().clone();
    let scope = state.engine_scope.read().clone();
    let label = scope_label(&engines, &scope);
    let current = match &scope {
        EngineScope::One(id) => engines.iter().find(|e| e.id == *id),
        EngineScope::All => None,
    };
    let dot = current.map(|e| engine_dot(&e.state));
    let icon = current.map_or("server", |e| kind_icon(&e.kind));
    let stopped_word = current.and_then(|e| {
        (e.state != berth_core::EngineState::Running).then(|| engine_dot(&e.state).label)
    });
    let is_open = open();
    rsx! {
        div {
            class: "engine-switcher",
            onkeydown: move |e| {
                if e.data.key() == Key::Escape && open() {
                    e.stop_propagation();
                    open.set(false);
                }
            },
            button {
                class: "engine-switcher-button",
                r#type: "button",
                "aria-haspopup": "menu",
                "aria-expanded": is_open.to_string(),
                "aria-label": "Engine: {label}",
                onmousedown: move |e| e.stop_propagation(),
                ondoubleclick: move |e| e.stop_propagation(),
                onclick: move |_| open.set(!open()),
                span { class: "engine-switcher-icon",
                    Icon { name: icon, size: IconSize::Md }
                }
                span { class: "engine-switcher-name", "{label}" }
                if let Some(word) = stopped_word {
                    span { class: "engine-state", "{word}" }
                }
                if let Some(spec) = dot {
                    Dot { spec }
                }
                span { class: "engine-switcher-chevron",
                    Icon { name: "chevron-down", size: IconSize::Sm }
                }
            }
            if is_open {
                div {
                    class: "engine-backdrop",
                    onmousedown: move |e| e.stop_propagation(),
                    onclick: move |_| open.set(false),
                }
                div { class: "engine-menu", role: "menu", "aria-label": "Engines",
                    for engine in engines.iter() {
                        {
                            let id = engine.id.clone();
                            let checked = scope == EngineScope::One(engine.id.clone());
                            rsx! {
                                button {
                                    key: "{engine.id.0}",
                                    class: "engine-menu-item",
                                    r#type: "button",
                                    role: "menuitemradio",
                                    "aria-checked": checked.to_string(),
                                    onclick: move |_| {
                                        state.engine_scope.set(EngineScope::One(id.clone()));
                                        open.set(false);
                                    },
                                    Dot { spec: engine_dot(&engine.state) }
                                    span { class: "engine-menu-name", "{engine.name}" }
                                    span { class: "engine-menu-kind", "{kind_label(&engine.kind)}" }
                                }
                            }
                        }
                    }
                    div { class: "engine-menu-separator", role: "separator" }
                    button {
                        class: "engine-menu-item",
                        r#type: "button",
                        role: "menuitemradio",
                        "aria-checked": (scope == EngineScope::All).to_string(),
                        onclick: move |_| {
                            state.engine_scope.set(EngineScope::All);
                            open.set(false);
                        },
                        span { class: "engine-menu-name", "All engines" }
                    }
                }
            }
        }
    }
}
