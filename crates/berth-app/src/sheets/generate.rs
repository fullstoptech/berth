//! The Generate sheet (P5-T3): what Berth found in a folder and the few choices a recipe takes
//! (database, other services, ports, Node version), then Review changes. Nothing is written here:
//! the button builds a plan, parks it in the `PlanStore` and opens `Overlay::ReviewChanges`.
//!
//! [`GenerateBody`] is the whole content without the sheet frame, so the Detected sheet (P5-T5) and
//! the New project sheet draw the same thing around their own header.

use std::path::PathBuf;
use std::sync::Arc;

use dioxus::prelude::*;

use berth_core::{Detection, ServiceKind};

use crate::services::plans::{blocking, target_engine, PendingPlan, PlanRun, PlanStore};
use crate::services::ports::taken_ports;
use crate::state::{AppState, Overlay};
use crate::ui::{
    Button, ButtonVariant, CheckState, Checkbox, ControlSize, InlineMessage, InlineTone, Select,
    SelectOption, TextField,
};
use model::{
    build_plan, has_compose, kind_name, options, port_rows, replan, stack_line, with_choices,
    Choices, DATABASES, EXTRAS,
};

pub mod model;

/// The sheet's CSS, rendered with it (tokens only).
pub const GENERATE_CSS: &str = include_str!("../../assets/css/generate.css");

/// What the body shows instead of reading the folder (scenes and tests).
#[derive(Debug, Clone, PartialEq)]
pub struct SeedDetection(pub Detection);

#[derive(Debug, Clone, PartialEq)]
enum Loaded {
    Loading,
    Failed(String),
    Ready(Detection),
}

/// Why Review changes cannot be offered for this folder, or `None`.
#[must_use]
pub fn blocker(d: &Detection) -> Option<String> {
    if has_compose(d) {
        return Some(
            "This folder already has a compose file. Berth does not change it.".to_string(),
        );
    }
    let has_dockerfile = d
        .found
        .iter()
        .any(|f| matches!(f.kind, berth_core::FoundKind::Dockerfile { .. }));
    if d.stack.is_empty() && !has_dockerfile {
        return Some(
            "Berth does not recognise this folder yet. Add a Dockerfile or open a project folder."
                .to_string(),
        );
    }
    None
}

/// The sheet: a card with a title, the body and nothing else.
#[component]
pub fn GenerateSheet(folder: PathBuf) -> Element {
    let mut state = use_context::<AppState>();
    let name = folder
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let close_folder = folder.clone();
    rsx! {
        style { "{GENERATE_CSS}" }
        div {
            class: "gen-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Generate files for {name}",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "gen-title", "Generate files for {name}" }
            GenerateBody {
                folder,
                on_close: move |()| {
                    state.overlays.write().retain(|o| *o != Overlay::Generate(close_folder.clone()));
                },
            }
        }
    }
}

/// What the Detected sheet changes in the body: it shows its own Found list and names the button
/// after what it makes ("Generate compose.yaml").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BodyDress {
    /// The Found card is the sheet's own; leave this one out.
    pub hide_found: bool,
    /// The button's label; "Review changes" when `None`.
    pub action: Option<String>,
    /// The line left of the buttons.
    pub note: Option<String>,
}

/// Detection, choices and the Review changes button, for `folder`. `on_close` is the Cancel button.
#[component]
pub fn GenerateBody(
    folder: PathBuf,
    on_close: EventHandler<()>,
    #[props(default)] dress: BodyDress,
) -> Element {
    let mut state = use_context::<AppState>();
    let store = try_consume_context::<PlanStore>();
    let seed = try_consume_context::<SeedDetection>();

    let mut loaded = use_signal({
        let seed = seed.clone();
        move || seed.map_or(Loaded::Loading, |s| Loaded::Ready(s.0))
    });
    let mut choices = use_signal({
        let seed = try_consume_context::<SeedDetection>();
        move || seed.map_or_else(Choices::default, |s| Choices::from_detection(&s.0, &[]))
    });
    let mut building = use_signal(|| false);
    let mut notice = use_signal(|| None::<String>);
    let mut harness = use_signal(String::new);

    // Read the folder once, off the UI thread. A seed (scene, test) skips it.
    {
        let folder = folder.clone();
        let seeded = seed.is_some();
        use_effect(move || {
            if seeded {
                return;
            }
            let folder = folder.clone();
            spawn(async move {
                let result = blocking(move || berth_rules::detect::detect(&folder)).await;
                match result {
                    Some(Ok(d)) => {
                        // Its services become the starting choices.
                        choices.set(Choices::from_detection(&d, &[]));
                        loaded.set(Loaded::Ready(d));
                    }
                    Some(Err(e)) => loaded.set(Loaded::Failed(e.to_string())),
                    None => loaded.set(Loaded::Failed("The folder could not be read.".to_string())),
                }
            });
        });
    }

    let engine = target_engine(&state.engine_scope.read(), &state.engines.read());
    let taken = taken_for(&state);

    // What the port fields start with. Fields are drawn again when it changes (other services,
    // or the engine's containers arriving), and whatever was typed into the old ones goes.
    let suggestion_key = use_memo(move || {
        let Loaded::Ready(d) = &*loaded.read() else {
            return String::new();
        };
        let picked = choices.read().clone();
        let ports =
            berth_rules::recipes::options_for(&with_choices(d, &picked), &taken_for(&state));
        format!(
            "{:?}|{:?}",
            picked
                .services()
                .iter()
                .map(|k| kind_name(*k))
                .collect::<Vec<_>>(),
            ports.ports
        )
    });
    use_effect(move || {
        let _ = suggestion_key();
        choices.write().ports.clear();
    });

    let harnesses: Vec<berth_core::Harness> = state
        .harnesses
        .read()
        .iter()
        .filter(|h| h.available)
        .cloned()
        .collect();

    let view = loaded.read().clone();
    let name = folder
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let shown_folder = berth_core::fmt::tilde(&folder, &berth_sys::paths::home());

    let body = match view {
        Loaded::Loading => rsx! {
            p { class: "gen-muted", role: "status", "Reading {shown_folder}\u{2026}" }
        },
        Loaded::Failed(message) => rsx! {
            InlineMessage { message: format!("Could not read the folder. {message}"), tone: InlineTone::Danger }
        },
        Loaded::Ready(d) => {
            let current = choices.read().clone();
            let rows = port_rows(&d, &current, &taken, &state.containers.read());
            let port_error = rows.iter().find_map(|r| r.error.clone());
            let block = blocker(&d);
            let harness_name = harnesses
                .iter()
                .find(|h| h.id.0 == *harness.read())
                .map(|h| h.name.clone());
            let disabled_reason = block.clone().or_else(|| port_error.clone()).or_else(|| {
                harness_name
                    .as_ref()
                    .map(|n| format!("Generating with {n} arrives with the assistant (P7-T6)."))
            });
            let can_node = d
                .stack
                .iter()
                .any(|s| matches!(s, berth_core::StackGuess::Node { .. }));
            let node_text = current.node_version.clone().unwrap_or_default();
            let found_services = d.services_needed.clone();
            let db_value = current.database.map_or("none", db_id).to_string();
            let mut db_options = vec![SelectOption::new("none", "No database")];
            db_options.extend(
                DATABASES
                    .iter()
                    .map(|k| SelectOption::new(db_id(*k), kind_name(*k))),
            );
            let detection_for_click = d.clone();
            let folder_for_click = folder.clone();
            let click_store = store.clone();
            let click_engine = engine.clone();
            let click_taken = taken.clone();
            let on_review = move |_| {
                if *building.peek() {
                    return;
                }
                let Some(store) = click_store.clone() else {
                    notice.set(Some("Review changes is not connected.".to_string()));
                    return;
                };
                building.set(true);
                notice.set(None);
                let d = detection_for_click.clone();
                let picked = choices.peek().clone();
                let taken = click_taken.clone();
                let folder = folder_for_click.clone();
                let engine = click_engine.clone();
                spawn(async move {
                    let built = {
                        let d = d.clone();
                        let picked = picked.clone();
                        let taken = taken.clone();
                        blocking(move || {
                            let options = options(&d, &picked, &taken)
                                .map_err(|m| berth_rules::RulesError::Read(PathBuf::new(), m))?;
                            build_plan(&d, &options)
                        })
                        .await
                    };
                    building.set(false);
                    match built {
                        Some(Ok(plan)) if plan.files.is_empty() => {
                            notice.set(Some(plan.notes.join(" ")));
                        }
                        Some(Ok(plan)) => {
                            let title = folder
                                .file_name()
                                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                            let mut pending = PendingPlan::new(plan, title);
                            if let Some(engine) = engine {
                                pending.run = Some(PlanRun::for_folder(
                                    &engine,
                                    &folder,
                                    vec![PathBuf::from("compose.yaml")],
                                ));
                                pending.pin = Some(engine);
                            }
                            let regen_folder = folder.clone();
                            pending.regenerate =
                                Some(Arc::new(move || replan(&regen_folder, &picked, &taken)));
                            let id = store.insert(pending);
                            state.overlays.write().push(Overlay::ReviewChanges(id));
                        }
                        Some(Err(e)) => notice.set(Some(e.to_string())),
                        None => notice.set(Some("The plan did not finish.".to_string())),
                    }
                });
            };
            rsx! {
                p { class: "gen-lede",
                    "Berth read the files in {shown_folder} and nothing else. You review every file before it is written."
                }
                if !dress.hide_found {
                section { class: "gen-card", "aria-label": "What Berth found",
                    h3 { class: "gen-card-title", "Found" }
                    dl { class: "gen-facts",
                        div { class: "gen-fact",
                            dt { "Stack" }
                            dd { "{stack_line(&d)}" }
                        }
                        div { class: "gen-fact",
                            dt { "Needs" }
                            dd {
                                if found_services.is_empty() {
                                    "No services found"
                                } else {
                                    {found_services
                                        .iter()
                                        .map(|s| format!("{} ({})", kind_name(s.kind), s.reason))
                                        .collect::<Vec<_>>()
                                        .join(", ")}
                                }
                            }
                        }
                    }
                }
                }
                if let Some(text) = block.clone() {
                    InlineMessage { message: text, tone: InlineTone::Warning }
                } else {
                    section { class: "gen-card", "aria-label": "Options",
                        h3 { class: "gen-card-title", "Options" }
                        div { class: "gen-field",
                            Select {
                                label: "Database".to_string(),
                                options: db_options,
                                value: db_value,
                                aria_label: "Database".to_string(),
                                onchange: move |e: FormEvent| {
                                    let value = e.value();
                                    choices.with_mut(|c| {
                                        c.set_database(DATABASES.iter().copied().find(|k| db_id(*k) == value));
                                    });
                                },
                            }
                        }
                        div { class: "gen-extras", role: "group", "aria-label": "Other services",
                            for kind in EXTRAS {
                                Checkbox {
                                    key: "{kind_name(kind)}",
                                    state: CheckState::from_bool(current.extras.contains(&kind)),
                                    label: kind_name(kind).to_string(),
                                    aria_label: kind_name(kind).to_string(),
                                    on_toggle: move |on: bool| {
                                        choices.with_mut(|c| c.set_extra(kind, on));
                                    },
                                }
                            }
                        }
                        // Keyed by the services, so a new set of ports starts from its own suggestions.
                        for _ in std::iter::once(()) {
                            div { key: "{suggestion_key}", class: "gen-ports",
                                for row in rows.iter().cloned() {
                                    {
                                        let key = row.key.clone();
                                        rsx! {
                                            div { key: "{row.key}", class: "gen-port",
                                                TextField {
                                                    label: row.label.clone(),
                                                    initial_value: row.value.clone(),
                                                    aria_label: row.label.clone(),
                                                    error: row.error.clone(),
                                                    help: row.warning.clone(),
                                                    oninput: move |e: FormEvent| {
                                                        let text = e.value();
                                                        choices.with_mut(|c| {
                                                            c.ports.insert(key.clone(), text);
                                                        });
                                                    },
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if can_node {
                            div { class: "gen-field",
                                TextField {
                                    label: "Node version".to_string(),
                                    initial_value: node_text,
                                    aria_label: "Node version".to_string(),
                                    oninput: move |e: FormEvent| {
                                        let text = e.value();
                                        choices.with_mut(|c| c.node_version = Some(text));
                                    },
                                }
                            }
                        }
                        if !harnesses.is_empty() {
                            div { class: "gen-field",
                                Select {
                                    label: "Generate with".to_string(),
                                    options: std::iter::once(SelectOption::new("", "Berth recipe"))
                                        .chain(harnesses.iter().map(|h| SelectOption::new(h.id.0.clone(), h.name.clone())))
                                        .collect::<Vec<_>>(),
                                    value: harness.read().clone(),
                                    aria_label: "Generate with".to_string(),
                                    onchange: move |e: FormEvent| harness.set(e.value()),
                                }
                            }
                        }
                    }
                }
                if let Some(text) = notice.read().clone() {
                    InlineMessage { message: text, tone: InlineTone::Danger }
                }
                div { class: "gen-footer",
                    if let Some(note) = dress.note.clone() {
                        p { class: "gen-footer-note", "{note}" }
                    }
                    Button {
                        label: "Cancel".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        onclick: move |_| on_close.call(()),
                    }
                    Button {
                        label: dress.action.clone().unwrap_or_else(|| "Review changes".to_string()),
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Md,
                        loading: *building.read(),
                        disabled: disabled_reason.is_some() || *building.read(),
                        disabled_reason,
                        onclick: on_review,
                    }
                }
            }
        }
    };

    rsx! {
        div { class: "gen-body", "data-folder": "{name}",
            {body}
        }
    }
}

/// The host ports the target engine's running containers hold now.
fn taken_for(state: &AppState) -> Vec<u16> {
    let engine = target_engine(&state.engine_scope.read(), &state.engines.read());
    let containers = state.containers.read();
    let mine: Vec<berth_core::Container> = containers
        .iter()
        .filter(|c| engine.as_ref().is_none_or(|e| c.engine == *e))
        .cloned()
        .collect();
    taken_ports(&mine)
}

fn db_id(kind: ServiceKind) -> &'static str {
    match kind {
        ServiceKind::Postgres => "postgres",
        ServiceKind::Mysql => "mysql",
        ServiceKind::Mongo => "mongo",
        _ => "none",
    }
}

#[cfg(test)]
mod tests;
