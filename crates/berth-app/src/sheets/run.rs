//! The Run sheet (P4-T1, PLAN §6.11 `Overlay::Run`): start a container from
//! an image. Name, ports, environment, volumes and labels, prefilled from
//! the image (one port row per exposed port, one environment row per
//! variable the image sets, a name from the repository); Run creates the
//! container with `create` and starts it.
//!
//! The draft (`RunDraft`) is pure: it turns the typed text into a
//! `ContainerSpec` or says which field is wrong. The sheet reads signals and
//! the [`ImageStore`] and sends Run to it (D23). An image's own environment
//! values are never read: a row for an image variable only overrides it
//! when a value is typed, and the typed value never reaches a log.

use dioxus::prelude::*;

use berth_core::{ContainerSpec, EngineId, Image, ImageDetail};

use crate::services::images::{base_spec, suggest_name, ImageEntry, ImageStore};
use crate::services::Services;
use crate::sheets::edit_recreate::model::{parse_mount, parse_port, valid_name};
use crate::state::{AppState, Overlay, Toast, ToastTone};
use crate::ui::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonTone, InlineMessage, InlineTone,
    TextField,
};

/// The sheet shares the Edit-and-recreate sheet's frame and rows.
pub const RUN_CSS: &str = include_str!("../../assets/css/edit_recreate.css");

// ---------------------------------------------------------------------
// The draft (pure)
// ---------------------------------------------------------------------

/// Which field a message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunField {
    Name,
    Port(u32),
    EnvKey(u32),
    Mount(u32),
    Label(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunError {
    pub field: RunField,
    pub message: String,
}

fn error(field: RunField, message: impl Into<String>) -> RunError {
    RunError {
        field,
        message: message.into(),
    }
}

/// One environment row. A row for a variable the image sets (`from_image`)
/// has a fixed key and only counts when a value is typed.
#[derive(Clone, PartialEq, Eq)]
pub struct RunEnv {
    pub id: u32,
    pub key: String,
    pub value: String,
    pub from_image: bool,
}

// Not derived: a derived `Debug` would print a typed value.
impl std::fmt::Debug for RunEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunEnv")
            .field("id", &self.id)
            .field("key", &self.key)
            .field("typed_value", &!self.value.is_empty())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunDraft {
    pub name: String,
    pub ports: Vec<(u32, String)>,
    pub env: Vec<RunEnv>,
    pub mounts: Vec<(u32, String)>,
    pub labels: Vec<(u32, String)>,
    next_id: u32,
}

impl RunDraft {
    /// The sheet's starting point: a free name from the repository, one port
    /// row per exposed port (the container port alone: Docker picks the
    /// host port), one environment row per variable the image sets.
    #[must_use]
    pub fn prefill(image: &Image, detail: Option<&ImageDetail>, taken: &[&str]) -> Self {
        let repo = image.repo_tags.first().map_or_else(
            || "container".to_string(),
            |t| crate::screens::image_detail::split_tag(t).0,
        );
        let mut draft = Self {
            name: suggest_name(&repo, taken),
            ..Self::default()
        };
        if let Some(detail) = detail {
            for port in &detail.exposed_ports {
                let id = draft.take_id();
                draft.ports.push((id, port.to_string()));
            }
            for key in &detail.env_keys {
                let id = draft.take_id();
                draft.env.push(RunEnv {
                    id,
                    key: key.clone(),
                    value: String::new(),
                    from_image: true,
                });
            }
        }
        draft
    }

    fn take_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    pub fn add_port(&mut self) {
        let id = self.take_id();
        self.ports.push((id, String::new()));
    }

    pub fn add_env(&mut self) {
        let id = self.take_id();
        self.env.push(RunEnv {
            id,
            key: String::new(),
            value: String::new(),
            from_image: false,
        });
    }

    pub fn add_mount(&mut self) {
        let id = self.take_id();
        self.mounts.push((id, String::new()));
    }

    pub fn add_label(&mut self) {
        let id = self.take_id();
        self.labels.push((id, String::new()));
    }

    pub fn remove_port(&mut self, id: u32) {
        self.ports.retain(|(i, _)| *i != id);
    }

    pub fn remove_env(&mut self, id: u32) {
        self.env.retain(|e| e.id != id);
    }

    pub fn remove_mount(&mut self, id: u32) {
        self.mounts.retain(|(i, _)| *i != id);
    }

    pub fn remove_label(&mut self, id: u32) {
        self.labels.retain(|(i, _)| *i != id);
    }

    pub fn set_port(&mut self, id: u32, text: &str) {
        if let Some(row) = self.ports.iter_mut().find(|(i, _)| *i == id) {
            row.1 = text.to_string();
        }
    }

    pub fn set_mount(&mut self, id: u32, text: &str) {
        if let Some(row) = self.mounts.iter_mut().find(|(i, _)| *i == id) {
            row.1 = text.to_string();
        }
    }

    pub fn set_label(&mut self, id: u32, text: &str) {
        if let Some(row) = self.labels.iter_mut().find(|(i, _)| *i == id) {
            row.1 = text.to_string();
        }
    }

    pub fn set_env_key(&mut self, id: u32, key: &str) {
        if let Some(row) = self.env.iter_mut().find(|e| e.id == id && !e.from_image) {
            row.key = key.to_string();
        }
    }

    pub fn set_env_value(&mut self, id: u32, value: &str) {
        if let Some(row) = self.env.iter_mut().find(|e| e.id == id) {
            row.value = value.to_string();
        }
    }

    /// The spec the typed text asks for, laid over `base` (the bare spec of
    /// the image). `taken` are the container names the engine already has.
    /// A blank row is skipped; an image variable with no typed value stays
    /// the image's own.
    ///
    /// # Errors
    /// Every field that is wrong, each with its sentence.
    pub fn to_spec(
        &self,
        base: &ContainerSpec,
        taken: &[&str],
    ) -> Result<ContainerSpec, Vec<RunError>> {
        let mut errors = Vec::new();
        let mut spec = base.clone();

        let name = self.name.trim();
        if name.is_empty() {
            errors.push(error(RunField::Name, "Give the container a name"));
        } else if !valid_name(name) {
            errors.push(error(
                RunField::Name,
                "Use letters, digits, dashes, dots and underscores, starting with a letter or digit",
            ));
        } else if taken.contains(&name) {
            errors.push(error(
                RunField::Name,
                format!("{name} is already a container on this engine"),
            ));
        }
        spec.name = name.to_string();

        spec.ports.clear();
        for (id, text) in &self.ports {
            if text.trim().is_empty() {
                continue;
            }
            match parse_port(text) {
                Ok(port) => spec.ports.push(port),
                Err(message) => errors.push(error(RunField::Port(*id), message)),
            }
        }

        spec.env.clear();
        let mut seen: Vec<&str> = Vec::new();
        for row in &self.env {
            let key = row.key.trim();
            if row.from_image && row.value.is_empty() {
                continue;
            }
            if key.is_empty() {
                if !row.value.is_empty() {
                    errors.push(error(RunField::EnvKey(row.id), "Name the variable"));
                }
                continue;
            }
            if key.contains('=') || key.chars().any(char::is_whitespace) {
                errors.push(error(
                    RunField::EnvKey(row.id),
                    "A variable name has no spaces or =",
                ));
                continue;
            }
            if seen.contains(&key) {
                errors.push(error(
                    RunField::EnvKey(row.id),
                    format!("{key} is set twice"),
                ));
                continue;
            }
            seen.push(key);
            spec.env.push((key.to_string(), row.value.clone()));
        }

        spec.mounts.clear();
        for (id, text) in &self.mounts {
            if text.trim().is_empty() {
                continue;
            }
            match parse_mount(text) {
                Ok(mount) => spec.mounts.push(mount),
                Err(message) => errors.push(error(RunField::Mount(*id), message)),
            }
        }

        spec.labels.clear();
        for (id, text) in &self.labels {
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            match parse_label(text) {
                Ok((key, value)) => {
                    spec.labels.insert(key, value);
                }
                Err(message) => errors.push(error(RunField::Label(*id), message)),
            }
        }

        if errors.is_empty() {
            Ok(spec)
        } else {
            Err(errors)
        }
    }
}

/// `dev.berth.fixture=1`: a label key, then `=`, then any value.
///
/// # Errors
/// A sentence saying what is wrong with the text.
pub fn parse_label(text: &str) -> Result<(String, String), String> {
    let Some((key, value)) = text.split_once('=') else {
        return Err("Write key=value, for example dev.berth.fixture=1".to_string());
    };
    let key = key.trim();
    if key.is_empty() {
        return Err("The label needs a key before the =".to_string());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("A label key has no spaces".to_string());
    }
    Ok((key.to_string(), value.trim().to_string()))
}

fn error_of(errors: &[RunError], field: RunField) -> Option<String> {
    errors
        .iter()
        .find(|e| e.field == field)
        .map(|e| e.message.clone())
}

// ---------------------------------------------------------------------
// The sheet
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum RunStatus {
    Idle,
    Running,
    Failed(String),
}

fn close_sheet(state: &mut AppState, id: &str) {
    state
        .overlays
        .write()
        .retain(|o| *o != Overlay::Run(id.to_string()));
}

fn edit(mut draft: Signal<Option<RunDraft>>, change: impl FnOnce(&mut RunDraft)) {
    if let Some(d) = draft.write().as_mut() {
        change(d);
    }
}

/// Where the image is: the engine, and the image itself.
fn locate(
    store: &ImageStore,
    scope: &crate::state::EngineScope,
    id: &str,
) -> Option<(EngineId, Image)> {
    store
        .find(id, scope)
        .map(|image| (image.engine.clone(), image))
}

#[component]
pub fn RunSheet(id: String) -> Element {
    let mut state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let mut status = use_signal(|| RunStatus::Idle);

    let scope = state.engine_scope.read().clone();
    let located = locate(&store, &scope, &id);
    let owner = located.as_ref().map(|(engine, _)| engine.clone());

    let ensure_id = id.clone();
    let ensure_services = services.clone();
    use_effect(move || {
        let _ = store.by_engine.read();
        if let (Some(engine), Some(services)) = (owner.clone(), ensure_services.as_ref()) {
            store.ensure_detail(services, &engine, &ensure_id);
        }
    });

    // The draft starts once the image's detail has arrived (its exposed
    // ports and environment keys fill the form).
    let mut draft = use_signal(|| None::<RunDraft>);
    let detail = located.as_ref().and_then(|(engine, _)| {
        match store.details.read().get(&(engine.clone(), id.clone())) {
            Some(ImageEntry::Ready(detail)) => Some((**detail).clone()),
            _ => None,
        }
    });
    let failed = located.as_ref().and_then(|(engine, _)| {
        match store.details.read().get(&(engine.clone(), id.clone())) {
            Some(ImageEntry::Failed { message, .. }) => Some(message.clone()),
            _ => None,
        }
    });
    if draft.peek().is_none() {
        if let Some((engine, image)) = located.as_ref() {
            if detail.is_some() || failed.is_some() {
                let containers = state.containers.peek();
                let taken: Vec<&str> = containers
                    .iter()
                    .filter(|c| c.engine == *engine)
                    .map(|c| c.name.as_str())
                    .collect();
                draft.set(Some(RunDraft::prefill(image, detail.as_ref(), &taken)));
            }
        }
    }

    let close_id = id.clone();
    let title = located
        .as_ref()
        .and_then(|(_, i)| i.repo_tags.first().cloned())
        .unwrap_or_else(|| crate::screens::images::short_id(&id));

    let body = match (located.clone(), draft.read().clone()) {
        (Some((engine, image)), Some(current)) => {
            let taken_names: Vec<String> = state
                .containers
                .read()
                .iter()
                .filter(|c| c.engine == engine)
                .map(|c| c.name.clone())
                .collect();
            let taken: Vec<&str> = taken_names.iter().map(String::as_str).collect();
            let base = base_spec(&image, &current.name);
            let built = current.to_spec(&base, &taken);
            let errors = built.as_ref().err().cloned().unwrap_or_default();
            let running = *status.read() == RunStatus::Running;
            let failure = match &*status.read() {
                RunStatus::Failed(message) => Some(message.clone()),
                _ => None,
            };
            let reason = built
                .is_err()
                .then(|| "Fix the fields marked in red first".to_string());
            let run_id = id.clone();
            let run_engine = engine.clone();
            let run_services = services.clone();
            rsx! {
                div { class: "er-body er-single",
                    div { class: "er-form",
                        section { class: "er-group",
                            h3 { class: "er-group-title", "Container" }
                            TextField {
                                label: "Name".to_string(),
                                initial_value: current.name.clone(),
                                aria_label: "Container name".to_string(),
                                error: error_of(&errors, RunField::Name),
                                oninput: move |e: FormEvent| edit(draft, |d| d.name = e.value()),
                            }
                        }
                        section { class: "er-group",
                            h3 { class: "er-group-title", "Ports" }
                            p { class: "er-muted",
                                "host:container, 127.0.0.1:8080:80 or 80/udp. A container port alone gets a free host port."
                            }
                            for (row_id , text) in current.ports.clone() {
                                div { key: "{row_id}", class: "er-row",
                                    TextField {
                                        initial_value: text,
                                        placeholder: "56080:80".to_string(),
                                        aria_label: "Port".to_string(),
                                        error: error_of(&errors, RunField::Port(row_id)),
                                        oninput: move |e: FormEvent| edit(draft, |d| d.set_port(row_id, &e.value())),
                                    }
                                    IconButton {
                                        icon: "trash-2",
                                        label: "Remove port".to_string(),
                                        tone: IconButtonTone::Danger,
                                        size: ControlSize::Sm,
                                        onclick: move |_| edit(draft, |d| d.remove_port(row_id)),
                                    }
                                }
                            }
                            div { class: "er-add",
                                Button {
                                    label: "Add port".to_string(),
                                    variant: ButtonVariant::Ghost,
                                    size: ControlSize::Sm,
                                    icon: "plus",
                                    onclick: move |_| edit(draft, RunDraft::add_port),
                                }
                            }
                        }
                        section { class: "er-group",
                            h3 { class: "er-group-title", "Environment" }
                            p { class: "er-muted",
                                "The image's own variables are listed; type a value only to replace one."
                            }
                            for row in current.env.clone() {
                                {
                                    let row_id = row.id;
                                    let key_error = error_of(&errors, RunField::EnvKey(row_id));
                                    rsx! {
                                        div { key: "{row_id}", class: "er-row er-env-row",
                                            if row.from_image {
                                                span { class: "er-env-key", title: "{row.key}", "{row.key}" }
                                            } else {
                                                TextField {
                                                    initial_value: row.key.clone(),
                                                    placeholder: "NAME".to_string(),
                                                    aria_label: "Name of the new variable".to_string(),
                                                    error: key_error.clone(),
                                                    oninput: move |e: FormEvent| edit(draft, |d| d.set_env_key(row_id, &e.value())),
                                                }
                                            }
                                            TextField {
                                                input_type: "password".to_string(),
                                                placeholder: if row.from_image { "image default".to_string() } else { "value".to_string() },
                                                aria_label: format!("Value of {}", if row.key.is_empty() { "the new variable" } else { row.key.as_str() }),
                                                oninput: move |e: FormEvent| edit(draft, |d| d.set_env_value(row_id, &e.value())),
                                            }
                                            IconButton {
                                                icon: "trash-2",
                                                label: format!("Remove {}", if row.key.is_empty() { "the new variable" } else { row.key.as_str() }),
                                                tone: IconButtonTone::Danger,
                                                size: ControlSize::Sm,
                                                onclick: move |_| edit(draft, |d| d.remove_env(row_id)),
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "er-add",
                                Button {
                                    label: "Add variable".to_string(),
                                    variant: ButtonVariant::Ghost,
                                    size: ControlSize::Sm,
                                    icon: "plus",
                                    onclick: move |_| edit(draft, RunDraft::add_env),
                                }
                            }
                        }
                        section { class: "er-group",
                            h3 { class: "er-group-title", "Volumes" }
                            p { class: "er-muted", "/host/path:/in/container[:ro] or volume:/in/container" }
                            for (row_id , text) in current.mounts.clone() {
                                div { key: "{row_id}", class: "er-row",
                                    TextField {
                                        initial_value: text,
                                        placeholder: "pgdata:/var/lib/postgresql/data".to_string(),
                                        aria_label: "Volume".to_string(),
                                        error: error_of(&errors, RunField::Mount(row_id)),
                                        oninput: move |e: FormEvent| edit(draft, |d| d.set_mount(row_id, &e.value())),
                                    }
                                    IconButton {
                                        icon: "trash-2",
                                        label: "Remove volume".to_string(),
                                        tone: IconButtonTone::Danger,
                                        size: ControlSize::Sm,
                                        onclick: move |_| edit(draft, |d| d.remove_mount(row_id)),
                                    }
                                }
                            }
                            div { class: "er-add",
                                Button {
                                    label: "Add volume".to_string(),
                                    variant: ButtonVariant::Ghost,
                                    size: ControlSize::Sm,
                                    icon: "plus",
                                    onclick: move |_| edit(draft, RunDraft::add_mount),
                                }
                            }
                        }
                        section { class: "er-group",
                            h3 { class: "er-group-title", "Labels" }
                            p { class: "er-muted", "key=value" }
                            for (row_id , text) in current.labels.clone() {
                                div { key: "{row_id}", class: "er-row",
                                    TextField {
                                        initial_value: text,
                                        placeholder: "dev.berth.fixture=1".to_string(),
                                        aria_label: "Label".to_string(),
                                        error: error_of(&errors, RunField::Label(row_id)),
                                        oninput: move |e: FormEvent| edit(draft, |d| d.set_label(row_id, &e.value())),
                                    }
                                    IconButton {
                                        icon: "trash-2",
                                        label: "Remove label".to_string(),
                                        tone: IconButtonTone::Danger,
                                        size: ControlSize::Sm,
                                        onclick: move |_| edit(draft, |d| d.remove_label(row_id)),
                                    }
                                }
                            }
                            div { class: "er-add",
                                Button {
                                    label: "Add label".to_string(),
                                    variant: ButtonVariant::Ghost,
                                    size: ControlSize::Sm,
                                    icon: "plus",
                                    onclick: move |_| edit(draft, RunDraft::add_label),
                                }
                            }
                        }
                    }
                }
                if let Some(message) = failure {
                    div { class: "er-failure",
                        InlineMessage { message, tone: InlineTone::Danger }
                    }
                }
                div { class: "er-footer",
                    Button {
                        label: "Cancel".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        disabled: running,
                        onclick: {
                            let id = id.clone();
                            move |_| close_sheet(&mut state, &id)
                        },
                    }
                    Button {
                        label: if running { "Starting\u{2026}".to_string() } else { "Run".to_string() },
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Md,
                        disabled: reason.is_some(),
                        disabled_reason: reason,
                        loading: running,
                        onclick: move |_| {
                            let Some(services) = run_services.as_ref() else { return };
                            let Some(current) = draft.peek().clone() else { return };
                            let containers = state.containers.peek();
                            let taken: Vec<&str> = containers
                                .iter()
                                .filter(|c| c.engine == run_engine)
                                .map(|c| c.name.as_str())
                                .collect();
                            let base = base_spec(&image, &current.name);
                            let Ok(spec) = current.to_spec(&base, &taken) else { return };
                            drop(containers);
                            status.set(RunStatus::Running);
                            let name = spec.name.clone();
                            let sheet_id = run_id.clone();
                            store.run(services, &run_engine, spec, move |result| match result {
                                Ok(_) => {
                                    close_sheet(&mut state, &sheet_id);
                                    state.toasts.write().push(Toast {
                                        id: uuid::Uuid::now_v7(),
                                        text: format!("Started {name}"),
                                        tone: ToastTone::Success,
                                        action: None,
                                    });
                                }
                                Err(message) => status.set(RunStatus::Failed(message)),
                            });
                        },
                    }
                }
            }
        }
        (None, _) => rsx! {
            div { class: "er-body er-single",
                p { class: "er-muted er-loading", "This image is gone." }
            }
        },
        (Some(_), None) => rsx! {
            p { class: "er-muted er-loading", "Loading the image's ports and environment\u{2026}" }
        },
    };

    rsx! {
        style { "{RUN_CSS}" }
        div {
            class: "er-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Run image",
            onclick: move |e| e.stop_propagation(),
            header { class: "er-header",
                h2 { class: "er-title", "Run {title}" }
                IconButton {
                    icon: "x",
                    label: "Close".to_string(),
                    size: ControlSize::Sm,
                    onclick: move |_| close_sheet(&mut state, &close_id),
                }
            }
            {body}
        }
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
