//! The Detected sheet (P5-T5, "New project / detected / 1440"): what Berth found in a folder that
//! was just opened or dropped on the window. A folder with a Compose file offers Start project; any
//! other folder shows the Generate body (P5-T3) under the same Found list and Review changes
//! comes from its button. Nothing is written here.
//!
//! The folder was read when the sheet opened, once, off the UI thread (D36): Berth never scans in
//! the background.

use std::path::{Path, PathBuf};

use dioxus::prelude::*;

use berth_core::{ComposeCommand, Detection, FoundKind};

use crate::services::actions::ActionsService;
use crate::services::backing::Capability;
use crate::services::folders::{display_path, folder_name, found_rows, service_label, stack_label};
use crate::services::plans::{blocking, target_engine, PlanRun};
use crate::sheets::generate::{BodyDress, GenerateBody, SeedDetection, GENERATE_CSS};
use crate::state::{AppState, Overlay, Section};
use crate::ui::{Button, ButtonVariant, ControlSize, Icon, IconSize, InlineMessage, InlineTone};

/// The sheet's CSS, rendered with it (tokens only).
pub const DETECTED_CSS: &str = include_str!("../../assets/css/detected.css");

/// The frame's line left of the buttons.
pub const NOTE: &str = "Nothing is written until you review the changes.";

#[derive(Debug, Clone, PartialEq)]
enum Loaded {
    Loading,
    Failed(String),
    Ready(Detection),
}

/// One row of the Found list: `(file, found, text)`.
#[must_use]
pub fn found_list(detection: &Detection) -> Vec<(&'static str, bool, String)> {
    found_rows(detection)
        .into_iter()
        .map(|row| {
            let yes = row.detail.is_some();
            (
                row.file,
                yes,
                row.detail.unwrap_or_else(|| "not found".to_string()),
            )
        })
        .collect()
}

fn found_icon(found: bool) -> &'static str {
    if found {
        "circle-check"
    } else {
        "x"
    }
}

/// The stack line: "Node 22 · pnpm · Next.js 15", or why there is none.
#[must_use]
pub fn stack_text(detection: &Detection) -> String {
    let lines: Vec<String> = detection.stack.iter().map(stack_label).collect();
    if lines.is_empty() {
        "No known stack".to_string()
    } else {
        lines.join(", ")
    }
}

/// The Compose files the detection found, as names relative to the folder.
#[must_use]
pub fn compose_files(detection: &Detection) -> Vec<PathBuf> {
    detection
        .found
        .iter()
        .filter(|f| matches!(f.kind, FoundKind::Compose))
        .filter_map(|f| f.path.file_name().map(PathBuf::from))
        .collect()
}

/// "2 more folders are waiting.", or `None`.
#[must_use]
pub fn waiting_note(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 more folder is waiting.".to_string()),
        n => Some(format!("{n} more folders are waiting.")),
    }
}

fn close(state: &mut AppState, folder: &Path) {
    state
        .overlays
        .write()
        .retain(|o| *o != Overlay::Detected(folder.to_path_buf()));
}

/// The sheet: a card with the folder's name and path, what was found, and the way on.
#[component]
pub fn DetectedSheet(folder: PathBuf) -> Element {
    let state = use_context::<AppState>();
    let queue = try_consume_context::<crate::ui::drop_overlay::DropQueue>();
    let seed = try_consume_context::<SeedDetection>();
    let mut loaded = use_signal({
        let seed = seed.clone();
        move || seed.map_or(Loaded::Loading, |s| Loaded::Ready(s.0))
    });
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
                    Some(Ok(d)) => loaded.set(Loaded::Ready(d)),
                    Some(Err(e)) => loaded.set(Loaded::Failed(e.to_string())),
                    None => loaded.set(Loaded::Failed("The folder could not be read.".to_string())),
                }
            });
        });
    }

    let home = berth_sys::paths::home();
    let name = folder_name(&folder);
    let shown = display_path(&folder, &home);
    let on_close = {
        let folder = folder.clone();
        move |()| {
            let mut state = state;
            close(&mut state, &folder);
        }
    };
    let waiting_count = queue.map_or(0, |q| q.0.read().len());
    let waiting = waiting_note(waiting_count);

    let body = match loaded.read().clone() {
        Loaded::Loading => rsx! {
            p { class: "dt-muted", role: "status", "Reading {shown}\u{2026}" }
        },
        Loaded::Failed(message) => rsx! {
            InlineMessage { message: format!("Could not read the folder. {message}"), tone: InlineTone::Danger }
            div { class: "dt-footer",
                Button {
                    label: "Close".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: {
                        let on_close = on_close.clone();
                        move |_| on_close(())
                    },
                }
            }
        },
        Loaded::Ready(d) => {
            let found = found_list(&d);
            let stack = stack_text(&d);
            let services: Vec<(String, String)> = d
                .services_needed
                .iter()
                .map(|g| (service_label(g.kind).to_string(), format!("({})", g.reason)))
                .collect();
            let has_compose = !compose_files(&d).is_empty();
            rsx! {
                h3 { class: "dt-label", "Found" }
                ul { class: "dt-found", "aria-label": "Found",
                    for (file, yes, text) in found {
                        li {
                            key: "{file}",
                            class: if yes { "dt-found-row dt-found-yes" } else { "dt-found-row dt-found-no" },
                            span { class: "dt-found-icon",
                                Icon { name: found_icon(yes), size: IconSize::Sm }
                            }
                            code { class: "dt-found-file", "{file}" }
                            span { class: "dt-found-detail", "{text}" }
                        }
                    }
                }
                h3 { class: "dt-label", "Stack" }
                p { class: "dt-stack", "{stack}" }
                if !services.is_empty() {
                    h3 { class: "dt-label", "Services needed" }
                    ul { class: "dt-services", "aria-label": "Services needed",
                        for (service, reason) in services {
                            li { key: "{service}-{reason}", class: "dt-service",
                                Icon { name: "box", size: IconSize::Sm }
                                span { class: "dt-service-name", "{service}" }
                                span { class: "dt-service-reason", "{reason}" }
                            }
                        }
                    }
                }
                if has_compose {
                    StartProject {
                        folder: folder.clone(),
                        files: compose_files(&d),
                        note: waiting.clone(),
                        on_close: on_close.clone(),
                    }
                } else {
                    SeededBody {
                        folder: folder.clone(),
                        detection: d.clone(),
                        note: waiting.clone().map_or_else(
                            || NOTE.to_string(),
                            |w| format!("{NOTE} {w}"),
                        ),
                        on_close: on_close.clone(),
                    }
                }
            }
        }
    };

    rsx! {
        style { "{GENERATE_CSS}" }
        style { "{DETECTED_CSS}" }
        div {
            class: "dt-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "{name} detected",
            onclick: move |e| e.stop_propagation(),
            div { class: "dt-head",
                h2 { class: "dt-title", "{name}" }
                if let (Some(queue), true) = (queue, waiting_count > 0) {
                    Button {
                        label: "Skip the rest".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Md,
                        onclick: move |_| {
                            let mut waiting = queue.0;
                            waiting.write().clear();
                        },
                    }
                }
            }
            p { class: "dt-path", "{shown}" }
            {body}
        }
    }
}

/// The Generate body for a folder already read: what it asks for is the one detection.
#[component]
fn SeededBody(
    folder: PathBuf,
    detection: Detection,
    note: String,
    on_close: EventHandler<()>,
) -> Element {
    use_context_provider(|| SeedDetection(detection));
    rsx! {
        GenerateBody {
            folder,
            on_close,
            dress: BodyDress {
                hide_found: true,
                action: Some("Generate compose.yaml".to_string()),
                note: Some(note),
            },
        }
    }
}

/// A folder with a Compose file: Start project runs `compose up -d` for it, as the Projects list
/// does for any project. Berth does not change the file.
#[component]
fn StartProject(
    folder: PathBuf,
    files: Vec<PathBuf>,
    note: Option<String>,
    on_close: EventHandler<()>,
) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let mut notice = use_signal(|| None::<String>);
    let backing = *state.backing.read();
    let unavailable = backing.unsupported(Capability::CreateContainers);
    let engine = target_engine(&state.engine_scope.read(), &state.engines.read());
    let reason = unavailable.or_else(|| {
        engine
            .is_none()
            .then(|| "No engine is running. Start one first.".to_string())
    });
    let start = {
        let folder = folder.clone();
        let files = files.clone();
        move |_| {
            let (Some(actions), Some(engine)) = (actions.clone(), engine.clone()) else {
                notice.set(Some("No engine is connected.".to_string()));
                return;
            };
            let run = PlanRun::for_folder(&engine, &folder, files.clone());
            let command = ComposeCommand::Up {
                services: Vec::new(),
                build: false,
            };
            match actions.run_for_folder(&mut state, &run, command) {
                Ok(_) => {
                    close(&mut state, &folder);
                    state.section.set(Section::Projects);
                }
                Err(message) => notice.set(Some(message)),
            }
        }
    };
    let names = files
        .iter()
        .map(|f| f.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let text = note.map_or_else(
        || format!("Berth does not change {names}."),
        |n| format!("Berth does not change {names}. {n}"),
    );
    rsx! {
        if let Some(message) = notice.read().clone() {
            InlineMessage { message, tone: InlineTone::Danger }
        }
        div { class: "dt-footer",
            p { class: "dt-footer-note", "{text}" }
            Button {
                label: "Cancel".to_string(),
                variant: ButtonVariant::Secondary,
                size: ControlSize::Md,
                onclick: move |_| on_close.call(()),
            }
            Button {
                label: "Start project".to_string(),
                variant: ButtonVariant::Primary,
                size: ControlSize::Md,
                disabled: reason.is_some(),
                disabled_reason: reason,
                onclick: start,
            }
        }
    }
}

#[cfg(test)]
#[path = "detected_tests.rs"]
mod tests;
