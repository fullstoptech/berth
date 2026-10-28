//! The New project sheet (P5-T5, ⌘N): three ways to start. Open folder hands over to the folder
//! browser. From editor writes a Compose file in the CodeMirror bundle and saves it into a folder
//! the person chooses, through Review changes (nothing is written without that click). From Git
//! clones a repository (P8-T5, `sheets/from_git.rs`) and goes on to the Detected sheet.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::document;
use dioxus::prelude::*;

use berth_core::{ComposeCommand, GenerationPlan, PlanSource};
use berth_rules::{diff, RulesError};

use crate::bridge::editor::host_id;
use crate::bridge::{parse_event, EditorEvent, EditorOptions, Event, Lang, Theme};
use crate::services::backing::Capability;
use crate::services::folders::display_path;
use crate::services::plans::{blocking, target_engine, PendingPlan, PlanRun, PlanStore};
use crate::state::{AppState, Overlay};
use crate::ui::{Button, ButtonVariant, ControlSize, Icon, IconSize, InlineMessage, InlineTone};

/// The sheet's CSS, rendered with it (tokens only).
pub const NEW_PROJECT_CSS: &str = include_str!("../../assets/css/new_project.css");

/// The file From editor writes.
pub const COMPOSE_FILE: &str = "compose.yaml";

/// What the editor starts with.
pub const STARTER: &str =
    "services:\n  app:\n    image: nginx:alpine\n    ports:\n      - \"8080:80\"\n";

/// The DOM id of the editor ([`host_id`] of it is the element).
const EDITOR_ID: &str = "new-project";

/// Which page of the sheet shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Choose,
    Editor,
    Git,
}

/// A scene or test opens the sheet on this page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedStage(pub Stage);

/// The names Compose looks for; a folder with one already is a project to open, not to create.
const COMPOSE_NAMES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

/// The plan that saves `text` as `compose.yaml` in `folder`, then starts the project.
///
/// # Errors
/// A sentence when the text is empty or the folder already has a Compose file.
pub fn compose_plan(folder: &Path, text: &str) -> Result<GenerationPlan, String> {
    if text.trim().is_empty() {
        return Err("The compose file is empty. Write the services first.".to_string());
    }
    if let Some(existing) = COMPOSE_NAMES.iter().find(|n| folder.join(n).exists()) {
        return Err(format!(
            "This folder already has {existing}. Open the folder instead, or choose another."
        ));
    }
    let mut text = text.trim_end().to_string();
    text.push('\n');
    Ok(GenerationPlan {
        root: folder.to_path_buf(),
        files: vec![diff::file_diff(Path::new(COMPOSE_FILE), None, Some(&text))],
        source: PlanSource::Editor,
        notes: Vec::new(),
        after_write: Some(ComposeCommand::Up {
            services: Vec::new(),
            build: false,
        }),
    })
}

/// Why Review changes cannot be offered yet, or `None`.
#[must_use]
pub fn review_blocker(
    folder: Option<&Path>,
    text: &str,
    backing_reason: Option<String>,
) -> Option<String> {
    if let Some(reason) = backing_reason {
        return Some(reason);
    }
    if folder.is_none() {
        return Some("Choose the folder to save it in.".to_string());
    }
    if text.trim().is_empty() {
        return Some("Write the compose file first.".to_string());
    }
    None
}

/// The script that opens the editor in the page and passes its messages back. It loads the bundle
/// the way the bridge does, taps `berth.emit` for this editor's messages only, and ends on
/// "dispose". `"value"` asks for the text now.
fn editor_script(options: &EditorOptions) -> String {
    let options = serde_json::to_string(options).unwrap_or_else(|_| "{}".to_string());
    let id = serde_json::to_string(EDITOR_ID).unwrap_or_default();
    format!(
        r#"
const berth = (window.berth ??= {{}});
const id = {id};
const options = {options};
const loading = (berth.loading ??= {{}});
function load(name) {{
  loading[name] ??= new Promise((resolve, reject) => {{
    const style = document.createElement("link");
    style.rel = "stylesheet";
    style.href = `/web/${{name}}.css`;
    document.head.appendChild(style);
    const script = document.createElement("script");
    script.src = `/web/${{name}}.js`;
    script.onload = resolve;
    script.onerror = () => reject(new Error(`bundle ${{name}} failed to load`));
    document.head.appendChild(script);
  }});
  return loading[name];
}}
if (!berth.editor) await load("editor");
const previous = berth.emit;
const tap = (message) => {{
  if (message && message.id === id && typeof message.type === "string" && message.type.startsWith("editor.")) {{
    dioxus.send(JSON.stringify(message));
  }}
  if (previous) previous(message);
}};
berth.emit = tap;
berth.editor.create(id, options);
while (true) {{
  const command = await dioxus.recv();
  if (command === "dispose") break;
  if (command === "value") berth.editor.getValue(id);
}}
berth.editor.dispose(id);
if (berth.emit === tap) berth.emit = previous;
"#
    )
}

/// Does the editor keep this key, so the shell's shortcuts never see it? Typing goes to the text
/// (Space, J and K would otherwise toggle the inspector and move the list selection, and the shell
/// would swallow them); ⌘ chords and Escape still reach Berth.
#[must_use]
pub fn editor_keeps_key(key: &str, meta: bool) -> bool {
    !(meta || key == "Escape")
}

fn close(state: &mut AppState) {
    state
        .overlays
        .write()
        .retain(|o| !matches!(o, Overlay::NewProject));
}

/// The sheet.
#[component]
pub fn NewProjectSheet() -> Element {
    let mut state = use_context::<AppState>();
    let seed = try_consume_context::<SeedStage>();
    let mut stage = use_signal(move || seed.map_or(Stage::Choose, |s| s.0));
    let current = *stage.read();

    rsx! {
        style { "{NEW_PROJECT_CSS}" }
        div {
            class: "np-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "New project",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "np-title", "New project" }
            match current {
                Stage::Choose => rsx! {
                    p { class: "np-lede", "How do you want to start?" }
                    div { class: "np-choices", role: "group", "aria-label": "Ways to start",
                        button {
                            class: "np-choice",
                            r#type: "button",
                            onclick: move |_| {
                                let mut overlays = state.overlays.write();
                                overlays.retain(|o| !matches!(o, Overlay::NewProject));
                                overlays.push(Overlay::OpenFolder);
                            },
                            span { class: "np-choice-icon", Icon { name: "folder", size: IconSize::Md } }
                            span { class: "np-choice-text",
                                span { class: "np-choice-name", "Open folder" }
                                span { class: "np-choice-hint",
                                    "Pick a folder you already have. Berth reads it and shows what it finds."
                                }
                            }
                        }
                        button {
                            class: "np-choice",
                            r#type: "button",
                            onclick: move |_| stage.set(Stage::Editor),
                            span { class: "np-choice-icon", Icon { name: "file-text", size: IconSize::Md } }
                            span { class: "np-choice-text",
                                span { class: "np-choice-name", "From editor" }
                                span { class: "np-choice-hint",
                                    "Write a compose file here and save it into a folder you choose."
                                }
                            }
                        }
                        button {
                            class: "np-choice",
                            r#type: "button",
                            onclick: move |_| stage.set(Stage::Git),
                            span { class: "np-choice-icon", Icon { name: "git-branch", size: IconSize::Md } }
                            span { class: "np-choice-text",
                                span { class: "np-choice-name", "From Git" }
                                span { class: "np-choice-hint",
                                    "Clone a repository, then deploy it. Berth can pull new commits later."
                                }
                            }
                        }
                    }
                    div { class: "np-footer",
                        Button {
                            label: "Cancel".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Md,
                            onclick: move |_| close(&mut state),
                        }
                    }
                },
                Stage::Editor => rsx! {
                    EditorStage { on_back: move |()| stage.set(Stage::Choose) }
                },
                Stage::Git => rsx! {
                    crate::sheets::from_git::GitStage { on_back: move |()| stage.set(Stage::Choose) }
                },
            }
        }
    }
}

/// The editor page: the CodeMirror bundle, the folder to save into, and Review changes.
#[component]
fn EditorStage(on_back: EventHandler<()>) -> Element {
    let mut state = use_context::<AppState>();
    let store = try_consume_context::<PlanStore>();
    let mut text = use_signal(|| STARTER.to_string());
    let mut folder = use_signal(|| None::<PathBuf>);
    let mut notice = use_signal(|| None::<String>);
    let mut building = use_signal(|| false);
    // A click on Review changes asks the page for the text, then builds the plan from the answer.
    let mut asked = use_signal(|| false);
    let dev = try_consume_context::<SeedStage>().is_some();

    let eval = use_hook(move || {
        if dev {
            return None;
        }
        let theme: Theme = state.settings.peek().theme.into();
        let options = EditorOptions {
            lang: Lang::Yaml,
            value: STARTER.to_string(),
            read_only: false,
            theme,
        };
        Some(document::eval(&editor_script(&options)))
    });

    // The plan from the text the page just answered with.
    let mut review = {
        let store = store.clone();
        move |typed: String| {
            let Some(chosen) = folder.peek().clone() else {
                return;
            };
            let Some(store) = store.clone() else {
                notice.set(Some("Review changes is not connected.".to_string()));
                return;
            };
            let engine = target_engine(&state.engine_scope.peek(), &state.engines.peek());
            building.set(true);
            notice.set(None);
            spawn(async move {
                let built = {
                    let chosen = chosen.clone();
                    let typed = typed.clone();
                    blocking(move || compose_plan(&chosen, &typed)).await
                };
                building.set(false);
                match built {
                    Some(Ok(plan)) => {
                        let title = chosen
                            .file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        let mut pending = PendingPlan::new(plan, title);
                        if let Some(engine) = engine {
                            pending.run = Some(PlanRun::for_folder(
                                &engine,
                                &chosen,
                                vec![PathBuf::from(COMPOSE_FILE)],
                            ));
                            pending.pin = Some(engine);
                        }
                        let again = chosen.clone();
                        pending.regenerate = Some(Arc::new(move || {
                            compose_plan(&again, &typed)
                                .map_err(|m| RulesError::Read(PathBuf::new(), m))
                        }));
                        let id = store.insert(pending);
                        state.overlays.write().push(Overlay::ReviewChanges(id));
                    }
                    Some(Err(message)) => notice.set(Some(message)),
                    None => notice.set(Some("The plan did not finish.".to_string())),
                }
            });
        }
    };

    // Messages from the page. The text may hold secrets: it is never logged.
    use_hook({
        let mut review = review.clone();
        move || {
            if let Some(mut eval) = eval {
                spawn(async move {
                    while let Ok(raw) = eval.recv::<String>().await {
                        match parse_event(&raw) {
                            Some(Event::Editor(EditorEvent::Change { text: typed, .. })) => {
                                text.set(typed);
                            }
                            Some(Event::Editor(EditorEvent::Value { text: typed, .. })) => {
                                text.set(typed.clone());
                                if *asked.peek() {
                                    asked.set(false);
                                    review(typed);
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
        }
    });
    use_drop(move || {
        if let Some(eval) = eval {
            let _ = eval.send("dispose");
        }
    });

    let home = berth_sys::paths::home();
    let chosen = folder.read().clone();
    let shown = chosen.as_ref().map(|f| display_path(f, &home));
    let backing = *state.backing.read();
    let blocker = review_blocker(
        chosen.as_deref(),
        &text.read(),
        backing.unsupported(Capability::WriteFiles),
    );

    let pick = move |_| {
        spawn(async move {
            let mut dialog = rfd::AsyncFileDialog::new().set_title("Save the compose file in");
            if let Some(start) = folder.peek().clone() {
                dialog = dialog.set_directory(start);
            }
            if let Some(picked) = dialog.pick_folder().await {
                folder.set(Some(picked.path().to_path_buf()));
                notice.set(None);
            }
        });
    };
    let ask = move |_| {
        if *building.peek() || *asked.peek() {
            return;
        }
        match eval {
            Some(eval) => {
                asked.set(true);
                if eval.send("value").is_err() {
                    asked.set(false);
                    notice.set(Some("The editor is not available.".to_string()));
                }
            }
            None => review(text.peek().clone()),
        }
    };

    rsx! {
        p { class: "np-lede", "Write {COMPOSE_FILE}, then choose where it goes." }
        div {
            class: "np-editor",
            onkeydown: move |e| {
                use dioxus::html::ModifiersInteraction;
                let meta = e.data.modifiers().contains(dioxus::html::Modifiers::META);
                if editor_keeps_key(&e.data.key().to_string(), meta) {
                    e.stop_propagation();
                }
            },
            div { id: host_id(EDITOR_ID), class: "berth-editor" }
        }
        div { class: "np-folder",
            span { class: "np-folder-label", "Folder" }
            span { class: "np-folder-path",
                if let Some(shown) = shown {
                    code { "{shown}" }
                } else {
                    span { class: "np-muted", "No folder chosen" }
                }
            }
            Button {
                label: "Choose folder\u{2026}".to_string(),
                variant: ButtonVariant::Secondary,
                size: ControlSize::Md,
                onclick: pick,
            }
        }
        if let Some(message) = notice.read().clone() {
            InlineMessage { message, tone: InlineTone::Danger }
        }
        div { class: "np-footer",
            p { class: "np-footer-note", "Nothing is written until you review the changes." }
            Button {
                label: "Back".to_string(),
                variant: ButtonVariant::Secondary,
                size: ControlSize::Md,
                onclick: move |_| on_back.call(()),
            }
            Button {
                label: "Review changes".to_string(),
                variant: ButtonVariant::Primary,
                size: ControlSize::Md,
                loading: *building.read() || *asked.read(),
                disabled: blocker.is_some(),
                disabled_reason: blocker,
                onclick: ask,
            }
        }
    }
}

#[cfg(test)]
#[path = "new_project_tests.rs"]
mod tests;
