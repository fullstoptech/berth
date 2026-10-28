//! The Build image sheet (P8-T3, AD43; no frame, so it follows the Create
//! volume sheet and the pull card) and the file-panel helpers behind
//! Import image, Export and Push.
//!
//! Three pages in one card. The form takes a context (a folder, or a
//! Dockerfile written in the editor), tags, build arguments (values masked)
//! and a platform. While the build runs, the page shows the step progress,
//! the live log and Cancel. Afterwards it shows the size of the first tag
//! and Push next, or the failing step marked in the log, the error and a
//! link to the Problems tab. The page reads the [`ImageStore`]'s build list
//! (D23); closing the sheet leaves a running build running, and opening it
//! again picks up the newest build.
//!
//! A [`berth_core::BuildSpec`] is built when the button is clicked and
//! handed to the store. It is never logged and never kept in a signal:
//! build arguments can be secrets, so the rows only live as form state.

use std::path::{Path, PathBuf};

use dioxus::document;
use dioxus::prelude::*;

use berth_core::{BuildContext, BuildSpec, Engine, EngineId, Image};

use crate::bridge::editor::host_id;
use crate::bridge::{parse_event, EditorEvent, EditorOptions, Event, Lang, Theme};
use crate::screens::images::{pull_target, PLATFORMS};
use crate::services::images::{
    check_build_spec, check_reference, with_default_tag, BuildKey, BuildLine, BuildLineKind,
    BuildPhase, BuildState, ImageStore, PushPhase, PushState, EXPORT_ACTION,
};
use crate::services::Services;
use crate::sheets::new_project::editor_keeps_key;
use crate::state::{AppState, BottomTab, Overlay, PendingKey, Toast, ToastTone};
use crate::ui::progress_bar::{ProgressBar, PROGRESS_BAR_CSS};
use crate::ui::{
    Button, ButtonVariant, ControlSize, Icon, IconButton, IconSize, InlineMessage, InlineTone,
    Select, SelectOption, TextField,
};

/// The sheet's CSS (also drawn by the image inspector for its push card).
pub const BUILD_IMAGE_CSS: &str = include_str!("../../assets/css/build_image.css");

/// What a Dockerfile written in the editor starts as.
pub const STARTER: &str = "FROM alpine:3\nCMD [\"echo\", \"hello\"]\n";

/// The DOM id of the sheet's editor ([`host_id`] of it is the element).
const EDITOR_ID: &str = "build-image";

/// How many log lines the page draws; the store keeps more.
pub const LOG_SHOWN: usize = 300;

// ---------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------

/// Where the build context comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// A folder on disk, tarred with its `.dockerignore` applied.
    #[default]
    Folder,
    /// A Dockerfile written in the editor; the context is that file alone.
    Dockerfile,
}

/// A scene or test opens the sheet on this source. The editor page is not
/// started when it is set (an SSR render has no page to run it in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedSource(pub Source);

/// One build-argument row of the form. Debug leaves the value out.
#[derive(Clone, PartialEq, Eq)]
pub struct ArgRow {
    pub id: u32,
    pub name: String,
    pub value: String,
}

impl std::fmt::Debug for ArgRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArgRow")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// The tags typed in the field: separated by spaces or commas.
#[must_use]
pub fn parse_tags(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// The first thing wrong with the typed tags, or `None`.
#[must_use]
pub fn tags_problem(text: &str) -> Option<String> {
    parse_tags(text).iter().find_map(|tag| {
        if tag.contains('@') {
            return Some("A new image is named with a tag, not a digest".to_string());
        }
        check_reference(tag).err()
    })
}

/// The rows as `(name, value)` pairs; a row with nothing typed is skipped.
/// A value with no name stays so the check can say so.
#[must_use]
pub fn build_args(rows: &[ArgRow]) -> Vec<(String, String)> {
    rows.iter()
        .filter(|r| !(r.name.trim().is_empty() && r.value.is_empty()))
        .map(|r| (r.name.trim().to_string(), r.value.clone()))
        .collect()
}

/// The spec the form describes. A missing folder becomes an empty path;
/// [`form_blocker`] and the service's check refuse it.
#[must_use]
pub fn form_spec(
    source: Source,
    folder: Option<&Path>,
    dockerfile: &str,
    inline: &str,
    tags: &str,
    rows: &[ArgRow],
    platform: &str,
) -> BuildSpec {
    let dockerfile = dockerfile.trim();
    BuildSpec {
        context: match source {
            Source::Folder => {
                BuildContext::Folder(folder.map(Path::to_path_buf).unwrap_or_default())
            }
            Source::Dockerfile => BuildContext::Inline(inline.to_string()),
        },
        dockerfile: (source == Source::Folder
            && !dockerfile.is_empty()
            && dockerfile != "Dockerfile")
            .then(|| dockerfile.to_string()),
        tags: parse_tags(tags),
        build_args: build_args(rows),
        platform: (!platform.is_empty()).then(|| platform.to_string()),
    }
}

/// Why Build is off, or `None`.
#[must_use]
pub fn form_blocker(
    source: Source,
    folder: Option<&Path>,
    inline: &str,
    tags: &str,
    has_engine: bool,
) -> Option<String> {
    if !has_engine {
        return Some("No running engine to build on".to_string());
    }
    match source {
        Source::Folder if folder.is_none() => {
            return Some("Choose the folder to build".to_string());
        }
        Source::Dockerfile if inline.trim().is_empty() => {
            return Some("Write the Dockerfile first".to_string());
        }
        _ => {}
    }
    if parse_tags(tags).is_empty() {
        return Some("Give the image a name such as myapp:dev".to_string());
    }
    tags_problem(tags)
}

/// The index of the log line that started the failing step: the last
/// `Step` line before the first `Failed` line.
#[must_use]
pub fn failing_step_line(lines: &[BuildLine]) -> Option<usize> {
    let first_failed = lines.iter().position(|l| l.kind == BuildLineKind::Failed)?;
    lines[..first_failed]
        .iter()
        .rposition(|l| l.kind == BuildLineKind::Step)
}

/// Everything a page of the sheet needs from one build, small enough to
/// copy on every event (the state itself holds up to thousands of lines).
#[derive(Debug, Clone, PartialEq)]
pub struct BuildView {
    pub key: BuildKey,
    pub engine: EngineId,
    pub tags: Vec<String>,
    pub context: String,
    pub phase: BuildPhase,
    pub step: Option<(u32, u32)>,
    pub fraction: Option<f32>,
    pub failure: Option<String>,
    /// How many older lines the page does not draw.
    pub hidden: usize,
    /// The newest [`LOG_SHOWN`] lines.
    pub lines: Vec<BuildLine>,
    /// The failing step's line, as an index into `lines`, when it is drawn.
    pub failing: Option<usize>,
}

impl BuildView {
    #[must_use]
    pub fn of(build: &BuildState) -> Self {
        let start = build.lines.len().saturating_sub(LOG_SHOWN);
        Self {
            key: build.key(),
            engine: build.engine.clone(),
            tags: build.tags.clone(),
            context: build.context.clone(),
            phase: build.phase.clone(),
            step: build.step,
            fraction: build.fraction(),
            failure: build.failure.clone(),
            hidden: start,
            lines: build.lines[start..].to_vec(),
            failing: failing_step_line(&build.lines).and_then(|i| i.checked_sub(start)),
        }
    }

    /// The first tag, as the image list names it.
    #[must_use]
    pub fn first_tag(&self) -> String {
        with_default_tag(self.tags.first().map_or("", String::as_str))
    }
}

/// The built image's size as the engine listed it, once it has.
#[must_use]
pub fn built_size(images: &[Image], engine: &EngineId, tag: &str) -> Option<String> {
    images
        .iter()
        .find(|i| i.engine == *engine && i.repo_tags.iter().any(|t| t == tag))
        .map(|i| berth_core::fmt::bytes(i.size))
}

/// The file an export offers: `postgres:17-alpine` is `postgres-17-alpine.tar`.
#[must_use]
pub fn export_file_name(reference: &str) -> String {
    let base: String = reference
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let base = base.trim_matches('-');
    format!("{}.tar", if base.is_empty() { "image" } else { base })
}

/// The names an import reports, for a toast.
#[must_use]
pub fn imported_text(names: &[String], file: &str) -> String {
    match names {
        [] => format!("Imported {file}"),
        [one] => format!("Imported {one}"),
        [first, second] => format!("Imported {first} and {second}"),
        [first, second, rest @ ..] => {
            format!("Imported {first}, {second} and {} more", rest.len())
        }
    }
}

// ---------------------------------------------------------------------
// Entry points (the Images controls, the image inspector)
// ---------------------------------------------------------------------

fn toast(state: &mut AppState, text: String, tone: ToastTone) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone,
        action: None,
    });
}

/// Open the sheet (once).
pub fn open(state: &mut AppState) {
    let mut overlays = state.overlays.write();
    if !overlays.iter().any(|o| matches!(o, Overlay::BuildImage)) {
        overlays.push(Overlay::BuildImage);
    }
}

fn close(state: &mut AppState) {
    state
        .overlays
        .write()
        .retain(|o| !matches!(o, Overlay::BuildImage));
}

/// Ask for an image archive in an open panel and load it into `engine`.
/// The file is the person's own pick, so this is not a Review changes write.
pub fn import_with_panel(
    mut state: AppState,
    store: ImageStore,
    services: Services,
    engine: EngineId,
) {
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Import image")
            .add_filter("Image archive", &["tar", "tgz", "gz"])
            .pick_file()
            .await
        else {
            return;
        };
        let name = file.file_name();
        store.import(
            &services,
            &engine,
            file.path().to_path_buf(),
            move |result| match result {
                Ok(names) => toast(&mut state, imported_text(&names, &name), ToastTone::Success),
                Err(reason) => toast(
                    &mut state,
                    format!("Could not import {name}: {reason}"),
                    ToastTone::Danger,
                ),
            },
        );
    });
}

/// Ask where to save `image` (a reference, or an id when it has no tag) in a
/// save panel and write its archive there.
pub fn export_with_panel(
    mut state: AppState,
    store: ImageStore,
    services: Services,
    engine: EngineId,
    image: String,
) {
    spawn(async move {
        let Some(target) = rfd::AsyncFileDialog::new()
            .set_title(format!("Export {image}"))
            .set_file_name(export_file_name(&image))
            .save_file()
            .await
        else {
            return;
        };
        let file = target.file_name();
        let name = image.clone();
        store.export(
            &services,
            &engine,
            &image,
            target.path().to_path_buf(),
            move |result| match result {
                Ok(bytes) => toast(
                    &mut state,
                    format!(
                        "Exported {name} to {file} ({})",
                        berth_core::fmt::bytes(bytesize::ByteSize::b(bytes))
                    ),
                    ToastTone::Success,
                ),
                Err(reason) if reason == "Cancelled" => {
                    toast(
                        &mut state,
                        format!("Cancelled the export of {name}"),
                        ToastTone::Neutral,
                    );
                }
                Err(reason) => toast(
                    &mut state,
                    format!("Could not export {name}: {reason}"),
                    ToastTone::Danger,
                ),
            },
        );
    });
}

// ---------------------------------------------------------------------
// The sheet
// ---------------------------------------------------------------------

/// The form's values. They live as long as the sheet, so a failed build
/// can be edited and run again.
#[derive(Clone, Copy, PartialEq)]
struct Form {
    source: Signal<Source>,
    folder: Signal<Option<PathBuf>>,
    dockerfile: Signal<String>,
    inline: Signal<String>,
    tags: Signal<String>,
    args: Signal<Vec<ArgRow>>,
    next_arg: Signal<u32>,
    platform: Signal<String>,
}

fn use_form() -> Form {
    let seeded = try_consume_context::<SeedSource>().map(|s| s.0);
    Form {
        source: use_signal(move || seeded.unwrap_or_default()),
        folder: use_signal(|| None),
        dockerfile: use_signal(|| "Dockerfile".to_string()),
        inline: use_signal(|| STARTER.to_string()),
        tags: use_signal(String::new),
        args: use_signal(Vec::new),
        next_arg: use_signal(|| 0),
        platform: use_signal(String::new),
    }
}

/// The Build image sheet.
#[component]
pub fn BuildImageSheet() -> Element {
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let form = use_form();
    // The build this sheet shows: the newest one, which may have started
    // from an earlier opening of the sheet.
    let started = use_signal(|| store.builds.peek().last().map(BuildState::key));
    let view = started.read().clone().and_then(|key| {
        store
            .builds
            .read()
            .iter()
            .find(|b| b.key() == key)
            .map(BuildView::of)
    });
    rsx! {
        style { "{BUILD_IMAGE_CSS}" }
        style { "{PROGRESS_BAR_CSS}" }
        div {
            class: "bi-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Build image",
            onclick: move |e| e.stop_propagation(),
            match view {
                None => rsx! { FormPage { form, started } },
                Some(view) if view.phase == BuildPhase::Running => rsx! { RunningPage { view } },
                Some(view) => rsx! { FinishedPage { view, started } },
            }
        }
    }
}

#[component]
fn FormPage(form: Form, started: Signal<Option<BuildKey>>) -> Element {
    let mut state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let mut refused = use_signal(|| None::<String>);
    let Form {
        source,
        mut folder,
        mut dockerfile,
        mut tags,
        mut args,
        mut next_arg,
        mut platform,
        ..
    } = form;

    let engine: Option<Engine> = pull_target(&state.engines.read(), &state.engine_scope.read());
    let current = *source.read();
    let chosen = folder.read().clone();
    let typed_tags = tags.read().clone();
    let blocker = form_blocker(
        current,
        chosen.as_deref(),
        &form.inline.read(),
        &typed_tags,
        engine.is_some(),
    );
    let tag_error = tags_problem(&typed_tags);
    let shown_folder = chosen
        .as_ref()
        .map(|f| crate::services::folders::display_path(f, &berth_sys::paths::home()));
    let platforms: Vec<SelectOption> = PLATFORMS
        .iter()
        .map(|(value, label)| SelectOption::new(*value, *label))
        .collect();
    let rows = args.read().clone();
    let engine_note = engine.as_ref().map(|e| format!("Builds on {}.", e.name));

    let build = {
        let engine = engine.clone();
        move |_| {
            let (Some(engine), Some(services)) = (engine.as_ref(), services.as_ref()) else {
                return;
            };
            let spec = form_spec(
                *source.peek(),
                folder.peek().as_deref(),
                &dockerfile.peek(),
                &form.inline.peek(),
                &tags.peek(),
                &args.peek(),
                &platform.peek(),
            );
            let checked = match check_build_spec(spec) {
                Ok(checked) => checked,
                Err(reason) => {
                    refused.set(Some(reason));
                    return;
                }
            };
            let key = (engine.id.clone(), checked.tags[0].clone());
            match store.build(services, &engine.id, checked) {
                Ok(()) => {
                    refused.set(None);
                    started.set(Some(key));
                }
                Err(reason) => refused.set(Some(reason)),
            }
        }
    };

    rsx! {
        h2 { class: "bi-title", "Build image" }
        p { class: "bi-lede", "Build an image from a folder with a Dockerfile, or from one you write here." }
        div { class: "bi-form",
            SegmentedPick { source }
            match current {
                Source::Folder => rsx! {
                    div { class: "bi-source",
                        span { class: "bi-source-path",
                            if let Some(shown) = shown_folder {
                                code { "{shown}" }
                            } else {
                                span { class: "bi-muted", "No folder chosen" }
                            }
                        }
                        Button {
                            label: "Choose folder\u{2026}".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Md,
                            onclick: move |_| {
                                spawn(async move {
                                    let mut dialog = rfd::AsyncFileDialog::new().set_title("Build context");
                                    if let Some(start) = folder.peek().clone() {
                                        dialog = dialog.set_directory(start);
                                    }
                                    if let Some(picked) = dialog.pick_folder().await {
                                        folder.set(Some(picked.path().to_path_buf()));
                                        refused.set(None);
                                    }
                                });
                            },
                        }
                    }
                    TextField {
                        label: "Dockerfile".to_string(),
                        initial_value: dockerfile.peek().clone(),
                        placeholder: "Dockerfile".to_string(),
                        aria_label: "Dockerfile path in the folder".to_string(),
                        oninput: move |e: FormEvent| dockerfile.set(e.value()),
                    }
                },
                Source::Dockerfile => rsx! { DockerfileEditor { text: form.inline } },
            }
            TextField {
                label: "Tags".to_string(),
                initial_value: typed_tags.clone(),
                placeholder: "myapp:dev".to_string(),
                aria_label: "Image tags".to_string(),
                help: "One or more names, separated by spaces. Push uses the registry of the first.".to_string(),
                error: tag_error,
                oninput: move |e: FormEvent| {
                    refused.set(None);
                    tags.set(e.value());
                },
            }
            div { class: "bi-args", role: "group", "aria-label": "Build arguments",
                div { class: "bi-args-head",
                    span { class: "bi-args-label", "Build arguments" }
                    Button {
                        label: "Add argument".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        icon: "plus",
                        onclick: move |_| {
                            let id = *next_arg.peek();
                            next_arg.set(id + 1);
                            args.write().push(ArgRow { id, name: String::new(), value: String::new() });
                        },
                    }
                }
                for row in rows {
                    ArgField { key: "{row.id}", row: row.clone(), args }
                }
            }
            Select {
                label: "Platform".to_string(),
                options: platforms,
                value: platform.read().clone(),
                aria_label: "Platform".to_string(),
                onchange: move |e: FormEvent| platform.set(e.value()),
            }
        }
        if let Some(reason) = refused.read().clone() {
            div { class: "bi-form",
                InlineMessage { message: reason, tone: InlineTone::Danger }
            }
        }
        div { class: "bi-actions",
            if let Some(note) = engine_note {
                p { class: "bi-actions-note", "{note}" }
            }
            Button {
                label: "Cancel".to_string(),
                variant: ButtonVariant::Secondary,
                size: ControlSize::Md,
                onclick: move |_| close(&mut state),
            }
            Button {
                label: "Build".to_string(),
                variant: ButtonVariant::Primary,
                size: ControlSize::Md,
                icon: "box",
                disabled: blocker.is_some(),
                disabled_reason: blocker,
                onclick: build,
            }
        }
    }
}

#[component]
fn SegmentedPick(source: Signal<Source>) -> Element {
    use crate::ui::{SegmentedControl, SegmentedItem};
    let mut source = source;
    let selected = match *source.read() {
        Source::Folder => "folder",
        Source::Dockerfile => "dockerfile",
    };
    rsx! {
        SegmentedControl {
            items: vec![
                SegmentedItem::new("folder", "Folder"),
                SegmentedItem::new("dockerfile", "Write a Dockerfile"),
            ],
            selected: selected.to_string(),
            aria_label: "Build context".to_string(),
            on_select: move |id: String| {
                source.set(if id == "dockerfile" { Source::Dockerfile } else { Source::Folder });
            },
        }
    }
}

/// One build argument: name, masked value, remove.
#[component]
fn ArgField(row: ArgRow, args: Signal<Vec<ArgRow>>) -> Element {
    let mut args = args;
    let id = row.id;
    rsx! {
        div { class: "bi-arg",
            TextField {
                initial_value: row.name.clone(),
                placeholder: "NAME".to_string(),
                aria_label: "Argument name".to_string(),
                oninput: move |e: FormEvent| {
                    if let Some(r) = args.write().iter_mut().find(|r| r.id == id) {
                        r.name = e.value();
                    }
                },
            }
            TextField {
                initial_value: row.value.clone(),
                placeholder: "value".to_string(),
                aria_label: "Argument value".to_string(),
                input_type: "password".to_string(),
                oninput: move |e: FormEvent| {
                    if let Some(r) = args.write().iter_mut().find(|r| r.id == id) {
                        r.value = e.value();
                    }
                },
            }
            IconButton {
                icon: "x",
                label: "Remove argument".to_string(),
                onclick: move |_| args.write().retain(|r| r.id != id),
            }
        }
    }
}

/// The script that opens the editor in the page and passes its messages
/// back, ending on "dispose". It loads the bundle the way the bridge does and
/// taps `berth.emit` for this editor's messages only.
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
}}
berth.editor.dispose(id);
if (berth.emit === tap) berth.emit = previous;
"#
    )
}

/// The Dockerfile editor: CodeMirror in the page, its text mirrored into
/// `text` as the person types.
#[component]
fn DockerfileEditor(text: Signal<String>) -> Element {
    let state = use_context::<AppState>();
    let mut text = text;
    let seeded = try_consume_context::<SeedSource>().is_some();
    let eval = use_hook(move || {
        if seeded {
            return None;
        }
        let theme: Theme = state.settings.peek().theme.into();
        let options = EditorOptions {
            lang: Lang::Dockerfile,
            value: text.peek().clone(),
            read_only: false,
            theme,
        };
        let mut eval = document::eval(&editor_script(&options));
        // The text may hold secrets: it is never logged.
        spawn(async move {
            while let Ok(raw) = eval.recv::<String>().await {
                if let Some(Event::Editor(EditorEvent::Change { text: typed, .. })) =
                    parse_event(&raw)
                {
                    text.set(typed);
                }
            }
        });
        Some(eval)
    });
    use_drop(move || {
        if let Some(eval) = eval {
            let _ = eval.send("dispose");
        }
    });
    rsx! {
        div {
            class: "bi-editor",
            "aria-label": "Dockerfile",
            onkeydown: move |e| {
                use dioxus::html::ModifiersInteraction;
                let meta = e.data.modifiers().contains(dioxus::html::Modifiers::META);
                if editor_keeps_key(&e.data.key().to_string(), meta) {
                    e.stop_propagation();
                }
            },
            div { id: host_id(EDITOR_ID), class: "berth-editor" }
        }
    }
}

// ---------------------------------------------------------------------
// Running, done, failed
// ---------------------------------------------------------------------

/// The live log: the newest lines, the failing step marked.
#[component]
fn BuildLog(lines: Vec<BuildLine>, hidden: usize, failing: Option<usize>) -> Element {
    rsx! {
        div { class: "bi-log", role: "log", "aria-label": "Build output",
            div { class: "bi-log-lines",
                if hidden > 0 {
                    div { class: "bi-line-hidden", "{hidden} earlier lines not shown" }
                }
                if lines.is_empty() {
                    div { class: "bi-log-empty", "Waiting for the first line\u{2026}" }
                }
                for (i, line) in lines.iter().enumerate() {
                    {
                        let class = line_class(line.kind, failing == Some(i));
                        rsx! { div { key: "{hidden + i}", class, "{line.text}" } }
                    }
                }
            }
        }
    }
}

fn line_class(kind: BuildLineKind, failing_step: bool) -> &'static str {
    match kind {
        BuildLineKind::Failed => "bi-line bi-line-failed",
        BuildLineKind::Step if failing_step => "bi-line bi-line-step bi-line-failed",
        BuildLineKind::Step => "bi-line bi-line-step",
        BuildLineKind::Log => "bi-line",
    }
}

fn step_text(step: Option<(u32, u32)>) -> String {
    match step {
        Some((index, total)) => format!("Step {index} of {total}"),
        None => "Starting".to_string(),
    }
}

#[component]
fn RunningPage(view: BuildView) -> Element {
    let mut state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let key = view.key.clone();
    let name = view.tags.first().cloned().unwrap_or_default();
    rsx! {
        h2 { class: "bi-title", "Building {name}" }
        p { class: "bi-lede", "From {view.context}" }
        ProgressBar {
            fraction: view.fraction,
            label: Some(step_text(view.step)),
            aria_label: Some("Build progress".to_string()),
        }
        BuildLog { lines: view.lines.clone(), hidden: view.hidden, failing: view.failing }
        div { class: "bi-actions",
            p { class: "bi-actions-note", "Closing this keeps the build running." }
            Button {
                label: "Close".to_string(),
                variant: ButtonVariant::Ghost,
                size: ControlSize::Md,
                onclick: move |_| close(&mut state),
            }
            Button {
                label: "Cancel build".to_string(),
                variant: ButtonVariant::Secondary,
                size: ControlSize::Md,
                onclick: move |_| store.cancel_build(&key),
            }
        }
    }
}

#[component]
fn FinishedPage(view: BuildView, started: Signal<Option<BuildKey>>) -> Element {
    let mut state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let mut started = started;
    let mut refused = use_signal(|| None::<String>);
    let done = view.phase == BuildPhase::Done;
    let first = view.first_tag();
    let size = built_size(
        &store
            .by_engine
            .read()
            .get(&view.engine)
            .cloned()
            .unwrap_or_default(),
        &view.engine,
        &first,
    );
    let push = store
        .pushes
        .read()
        .iter()
        .find(|p| p.key() == (view.engine.clone(), first.clone()))
        .cloned();
    let engine_name = state
        .engines
        .read()
        .iter()
        .find(|e| e.id == view.engine)
        .map_or_else(|| view.engine.0.clone(), |e| e.name.clone());
    let also = view
        .tags
        .iter()
        .skip(1)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let reason = match &view.phase {
        BuildPhase::Failed(reason) => Some(view.failure.clone().unwrap_or_else(|| reason.clone())),
        _ => None,
    };

    let key = view.key.clone();
    let again = move |_| {
        store.dismiss_build(&key);
        started.set(None);
    };
    let key = view.key.clone();
    let finish = move |_| {
        store.dismiss_build(&key);
        close(&mut state);
    };
    let push_engine = view.engine.clone();
    let push_ref = first.clone();
    let push_next = move |_| {
        let Some(services) = services.as_ref() else {
            return;
        };
        match store.push(services, &push_engine, &push_ref) {
            Ok(()) => refused.set(None),
            Err(reason) => refused.set(Some(reason)),
        }
    };
    let open_problems = move |_| {
        let mut bottom = state.bottom.write();
        bottom.open = true;
        bottom.tab = BottomTab::Problems;
        drop(bottom);
        close(&mut state);
    };

    rsx! {
        if done {
            div { class: "bi-done-head",
                span { class: "bi-done-icon", Icon { name: "circle-check", size: IconSize::Md } }
                h2 { class: "bi-title", "Built {first}" }
            }
            dl { class: "bi-facts",
                div { class: "bi-fact",
                    dt { class: "bi-fact-label", "Size" }
                    dd { class: "bi-fact-value",
                        if let Some(size) = size {
                            "{size}"
                        } else {
                            "Reading the size\u{2026}"
                        }
                    }
                }
                div { class: "bi-fact",
                    dt { class: "bi-fact-label", "Engine" }
                    dd { class: "bi-fact-value", "{engine_name}" }
                }
                if view.tags.len() > 1 {
                    div { class: "bi-fact",
                        dt { class: "bi-fact-label", "Also" }
                        dd { class: "bi-fact-value", "{also}" }
                    }
                }
            }
        } else if view.phase == BuildPhase::Cancelled {
            h2 { class: "bi-title", "Cancelled the build of {first}" }
            p { class: "bi-lede", "Nothing was tagged." }
        } else {
            h2 { class: "bi-title", "Could not build {first}" }
            if let Some(reason) = reason {
                InlineMessage { message: reason, tone: InlineTone::Danger }
            }
        }
        BuildLog { lines: view.lines.clone(), hidden: view.hidden, failing: view.failing }
        if let Some(push) = push {
            PushCard { key: "{push.reference}", push }
        }
        if let Some(reason) = refused.read().clone() {
            div { class: "bi-form",
                InlineMessage { message: reason, tone: InlineTone::Danger }
            }
        }
        div { class: "bi-actions",
            if done {
                Button {
                    label: "Close".to_string(),
                    variant: ButtonVariant::Ghost,
                    size: ControlSize::Md,
                    onclick: finish,
                }
                Button {
                    label: "Build another".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: again,
                }
                Button {
                    label: "Push next".to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    icon: "arrow-up",
                    onclick: push_next,
                }
            } else {
                if matches!(view.phase, BuildPhase::Failed(_)) {
                    Button {
                        label: "Open Problems".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Md,
                        icon: "alert-triangle",
                        onclick: open_problems,
                    }
                }
                Button {
                    label: "Close".to_string(),
                    variant: ButtonVariant::Ghost,
                    size: ControlSize::Md,
                    onclick: finish,
                }
                Button {
                    label: "Edit and rebuild".to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    onclick: again,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// Push card
// ---------------------------------------------------------------------

/// One push: the title, the registry's latest line, Cancel while it runs
/// and Dismiss after. Used in the done page and under an image's header.
#[component]
pub fn PushCard(push: PushState) -> Element {
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let key = push.key();
    let running = push.is_running();
    let title = match &push.phase {
        PushPhase::Running => format!("Pushing {}", push.reference),
        PushPhase::Done => format!("Pushed {}", push.reference),
        PushPhase::Failed(_) => format!("Could not push {}", push.reference),
        PushPhase::Cancelled => format!("Cancelled the push of {}", push.reference),
    };
    let note = push
        .digest
        .clone()
        .filter(|_| push.phase == PushPhase::Done)
        .map_or_else(|| push.note.clone(), |d| format!("Digest {d}"));
    rsx! {
        div { class: "bi-push", role: "group", "aria-label": "{title}",
            if running || push.phase == PushPhase::Done {
                ProgressBar {
                    fraction: (push.phase == PushPhase::Done).then_some(1.0),
                    label: Some(title.clone()),
                    aria_label: Some("Push progress".to_string()),
                }
            } else {
                div { class: "bi-push-title", "{title}" }
            }
            if let PushPhase::Failed(message) = &push.phase {
                InlineMessage { message: message.clone(), tone: InlineTone::Danger }
            }
            if !note.is_empty() {
                p { class: "bi-push-note", "{note}" }
            }
            div { class: "bi-push-buttons",
                if running {
                    Button {
                        label: "Cancel push".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Sm,
                        onclick: move |_| store.cancel_push(&key),
                    }
                } else {
                    Button {
                        label: "Dismiss".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        onclick: move |_| store.dismiss_push(&key),
                    }
                }
            }
        }
    }
}

/// Is an export of `image` running? (The inspector shows Cancel then.)
#[must_use]
pub fn exporting(state: &AppState, image: &str) -> bool {
    state.pending.read().contains_key(&PendingKey {
        subject: berth_core::Subject::Image(image.to_string()),
        action: EXPORT_ACTION,
    })
}

#[cfg(test)]
#[path = "build_image_tests.rs"]
mod tests;
