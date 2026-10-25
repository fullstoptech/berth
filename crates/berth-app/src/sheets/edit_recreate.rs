//! The edit and recreate sheet (P3-T7, brief §6.7, PLAN §7): the form of a
//! container's settings from its detail, the diff `spec_diff` makes of
//! them, the loss warning, and one primary button: Apply limits when only
//! limits or the restart policy changed, Recreate container otherwise.
//! A failed recreate keeps the old container (the engine restores it) and
//! the sheet says so. For a Compose service the sheet also offers to write
//! the change into compose.yaml, which waits for P5-T3.
//!
//! The model (`model`) is pure; this file draws signals and sends clicks
//! to the actions service. Env values never enter the markup: an existing
//! variable shows `••••` as a placeholder, and the diff shows `KEY=••••`.

pub mod model;

use std::collections::BTreeMap;

use dioxus::prelude::*;

use berth_core::{ContainerId, EffectKind};

use crate::services::actions::{dismiss, ActionsService, ContainerTarget, EditOp};
use crate::services::confirm;
use crate::services::detail::{DetailEntry, DetailStore, Loaded};
use crate::services::Services;
use crate::state::{AppState, Overlay, PendingKey, PendingState};
use crate::ui::confirm_dialog::row_label;
use crate::ui::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonTone, InlineMessage, InlineTone,
    Select, SelectOption, TextField,
};
use model::{
    diff_rows, failure_text, not_carried, offer, spec_from_detail, takes_from_image, Draft, Field,
    FieldError, Offer, RestartKind,
};

/// The sheet's CSS, rendered with it (tokens only).
pub const EDIT_RECREATE_CSS: &str = include_str!("../../assets/css/edit_recreate.css");

/// Why "Write to compose.yaml instead" cannot be used yet.
pub const COMPOSE_WRITE_REASON: &str = "Built in P5-T3";

/// Dev scenes put edits into the form before it is drawn.
#[derive(Clone, Copy)]
pub struct DevEditDraft(pub fn(&mut Draft));

// ---------------------------------------------------------------------
// Pending state of the sheet's actions
// ---------------------------------------------------------------------

/// The ids of the pending actions the sheet starts.
const OWN_ACTIONS: [&str; 3] = [
    "container.recreate",
    "container.apply_limits",
    "container.recreate_latest",
];

#[derive(Debug, Clone, PartialEq)]
pub enum EditStatus {
    Idle,
    Running(String),
    Failed { key: PendingKey, message: String },
}

/// What the edit actions of this container are doing: a running one wins
/// over a failed one.
#[must_use]
pub fn edit_status(pending: &BTreeMap<PendingKey, PendingState>, id: &ContainerId) -> EditStatus {
    let own = pending.iter().filter(|(key, _)| {
        key.subject == berth_core::Subject::Container(id.clone())
            && OWN_ACTIONS.contains(&key.action)
    });
    let mut failed = None;
    for (key, value) in own {
        match value {
            PendingState::Running { label } => return EditStatus::Running(label.clone()),
            PendingState::Failed { message } => {
                failed = Some(EditStatus::Failed {
                    key: key.clone(),
                    message: message.clone(),
                });
            }
        }
    }
    failed.unwrap_or(EditStatus::Idle)
}

fn close_sheet(state: &mut AppState, id: &ContainerId) {
    state
        .overlays
        .write()
        .retain(|o| *o != Overlay::EditRecreate(id.clone()));
}

fn ready(entry: Option<&DetailEntry>) -> Option<&Loaded> {
    match entry {
        Some(DetailEntry::Ready(loaded)) => Some(loaded),
        _ => None,
    }
}

fn error_of(errors: &[FieldError], field: Field) -> Option<String> {
    errors
        .iter()
        .find(|e| e.field == field)
        .map(|e| e.message.clone())
}

fn edit(mut draft: Signal<Option<Draft>>, change: impl FnOnce(&mut Draft)) {
    if let Some(d) = draft.write().as_mut() {
        change(d);
    }
}

// ---------------------------------------------------------------------
// The sheet
// ---------------------------------------------------------------------

#[component]
pub fn EditRecreateSheet(id: ContainerId) -> Element {
    let mut state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<DetailStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let actions = try_consume_context::<ActionsService>();
    let dev = try_consume_context::<DevEditDraft>();

    let row_id = id.clone();
    let container = use_memo(move || {
        state
            .containers
            .read()
            .iter()
            .find(|c| c.id == row_id)
            .cloned()
            .map(|mut c| {
                c.stats = None;
                c
            })
    });

    use_effect(move || {
        if let (Some(c), Some(services)) = (container(), services.clone()) {
            store.ensure(&services, &c);
        }
    });

    let start_id = id.clone();
    let mut draft = use_signal(move || {
        ready(store.entries.peek().get(&start_id)).map(|loaded| {
            let mut draft = Draft::from_spec(&spec_from_detail(&loaded.detail));
            if let Some(dev) = dev {
                (dev.0)(&mut draft);
            }
            draft
        })
    });
    let late_id = id.clone();
    use_effect(move || {
        if draft.peek().is_some() {
            return;
        }
        let entries = store.entries.read();
        if let Some(loaded) = ready(entries.get(&late_id)) {
            let mut fresh = Draft::from_spec(&spec_from_detail(&loaded.detail));
            if let Some(dev) = dev {
                (dev.0)(&mut fresh);
            }
            draft.set(Some(fresh));
        }
    });

    // The sheet closes when its action succeeds: it was running and is
    // neither running nor failed any more.
    let mut was_running = use_signal(|| false);
    let watch_id = id.clone();
    use_effect(move || {
        let status = edit_status(&state.pending.read(), &watch_id);
        let before = *was_running.peek();
        match status {
            EditStatus::Running(_) => {
                if !before {
                    was_running.set(true);
                }
            }
            EditStatus::Idle if before => {
                was_running.set(false);
                close_sheet(&mut state, &watch_id);
            }
            _ => {
                if before {
                    was_running.set(false);
                }
            }
        }
    });

    let mut refused = use_signal(|| None::<String>);
    let status = edit_status(&state.pending.read(), &id);

    let entry = store.entries.read().get(&id).cloned();
    let title_name = container
        .read()
        .as_ref()
        .map_or_else(|| "container".to_string(), |c| c.name.clone());

    let body = match (ready(entry.as_ref()), draft.read().clone(), container()) {
        (Some(loaded), Some(current), Some(c)) => {
            let base = spec_from_detail(&loaded.detail);
            let built = current.to_spec(&base);
            let errors = built.as_ref().err().cloned().unwrap_or_default();
            let (offered, rows, effect) = match &built {
                Ok(new) => {
                    let offered = offer(&base, new);
                    let effect = (offered == Offer::Recreate).then(|| {
                        confirm::recreate_effect(
                            &c,
                            &state.volumes.peek(),
                            false,
                            &not_carried(&loaded.detail),
                            takes_from_image(&loaded.detail),
                        )
                    });
                    (offered, diff_rows(&base, new), effect)
                }
                Err(_) => (Offer::Nothing, Vec::new(), None),
            };
            let running = matches!(status, EditStatus::Running(_));
            let (label, reason) = primary_button(offered, built.is_err(), &status);
            let compose = c.compose_project.clone();
            let image_changed = current.image.trim() != base.image.reference;
            let target = ContainerTarget::from(&c);
            let click_actions = actions.clone();
            let failure = match &status {
                EditStatus::Failed { key, message } => Some((key.clone(), failure_text(message))),
                _ => None,
            };
            rsx! {
                div { class: "er-body",
                    div { class: "er-form",
                        FormFields { draft, errors: errors.clone(), env_values_note: true }
                        if image_changed {
                            InlineMessage {
                                message: "The command and entrypoint of the old image are kept. Clear them if the new image has its own.".to_string(),
                                tone: InlineTone::Warning,
                            }
                        }
                    }
                    div { class: "er-side",
                        section { class: "er-card",
                            h3 { class: "er-card-title", "Changes" }
                            if rows.is_empty() {
                                p { class: "er-muted", "Nothing differs from the container yet." }
                            } else {
                                dl { class: "er-diff",
                                    for row in rows {
                                        div { key: "{row.field}", class: "er-diff-row",
                                            dt { class: "er-diff-field", "{row.field}" }
                                            dd { class: "er-diff-values",
                                                span { class: "er-before", "{row.before}" }
                                                span { class: "er-after", "{row.after}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if offered == Offer::ApplyLimits {
                            p { class: "er-muted",
                                "Limits and the restart policy apply to the running container. Nothing is recreated."
                            }
                        }
                        if let Some(effect) = effect {
                            section { class: "er-card er-loss", "aria-label": "What a recreate loses",
                                h3 { class: "er-card-title", "Before it runs" }
                                dl { class: "er-loss-rows",
                                    for (index, line) in effect.lines.iter().enumerate() {
                                        div { key: "{index}", class: "er-loss-row",
                                            dt { class: "er-loss-kind", "{row_label(line.kind)}" }
                                            dd {
                                                class: if line.kind == EffectKind::Goes { "er-loss-text er-loss-goes" } else { "er-loss-text" },
                                                "{line.text}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(project) = compose {
                            section { class: "er-card",
                                p { class: "er-muted",
                                    "This container belongs to the Compose project {project}. A change made here is lost the next time Compose recreates it."
                                }
                                Button {
                                    label: "Write to compose.yaml instead".to_string(),
                                    variant: ButtonVariant::Secondary,
                                    size: ControlSize::Sm,
                                    disabled: true,
                                    disabled_reason: COMPOSE_WRITE_REASON.to_string(),
                                }
                            }
                        }
                    }
                }
                if let Some((key, message)) = failure {
                    div { class: "er-failure",
                        InlineMessage { message, tone: InlineTone::Danger }
                        button {
                            class: "er-dismiss",
                            r#type: "button",
                            "aria-label": "Dismiss message",
                            onclick: move |_| dismiss(&mut state, &key),
                            "Dismiss"
                        }
                    }
                }
                if let Some(message) = refused.read().clone() {
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
                        onclick: move |_| close_sheet(&mut state, &target.id.clone()),
                    }
                    Button {
                        label,
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Md,
                        disabled: reason.is_some(),
                        disabled_reason: reason,
                        loading: running,
                        onclick: {
                            let target = ContainerTarget::from(&c);
                            move |_| {
                                refused.set(None);
                                let Some(service) = click_actions.as_ref() else { return };
                                let Some(current) = draft.peek().clone() else { return };
                                let Ok(new) = current.to_spec(&base) else { return };
                                let op = match offer(&base, &new) {
                                    Offer::Nothing => return,
                                    Offer::ApplyLimits => EditOp::apply_limits(&base, &new),
                                    Offer::Recreate => EditOp::Recreate { spec: Box::new(new) },
                                };
                                if let Err(reason) = service.edit(&mut state, &target, op) {
                                    refused.set(Some(reason));
                                }
                            }
                        },
                    }
                }
            }
        }
        (None, _, _) => match entry {
            Some(DetailEntry::Failed { message, .. }) => rsx! {
                div { class: "er-body er-single",
                    InlineMessage {
                        message: format!("Could not read the container: {message}"),
                        tone: InlineTone::Danger,
                    }
                }
            },
            _ => rsx! { p { class: "er-muted er-loading", "Loading the container's settings…" } },
        },
        _ => rsx! { p { class: "er-muted er-loading", "This container is gone." } },
    };

    let close_id = id.clone();
    rsx! {
        style { "{EDIT_RECREATE_CSS}" }
        div {
            class: "er-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Edit and recreate",
            onclick: move |e| e.stop_propagation(),
            header { class: "er-header",
                h2 { class: "er-title", "Edit {title_name}" }
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

/// The primary button's text and, when it cannot be pressed, why.
#[must_use]
pub fn primary_button(
    offered: Offer,
    has_errors: bool,
    status: &EditStatus,
) -> (String, Option<String>) {
    if let EditStatus::Running(label) = status {
        return (label.clone(), None);
    }
    let label = match offered {
        Offer::ApplyLimits => "Apply limits",
        Offer::Nothing | Offer::Recreate => "Recreate container",
    };
    let reason = if has_errors {
        Some("Fix the fields marked in red first".to_string())
    } else if offered == Offer::Nothing {
        Some("Nothing has changed".to_string())
    } else {
        None
    };
    (label.to_string(), reason)
}

// ---------------------------------------------------------------------
// The form
// ---------------------------------------------------------------------

#[component]
fn FormFields(
    draft: Signal<Option<Draft>>,
    errors: Vec<FieldError>,
    #[props(default)] env_values_note: bool,
) -> Element {
    let Some(current) = draft.read().clone() else {
        return rsx! {};
    };
    rsx! {
        section { class: "er-group",
            h3 { class: "er-group-title", "Image and command" }
            TextField {
                label: "Image".to_string(),
                initial_value: current.image.clone(),
                aria_label: "Image".to_string(),
                error: error_of(&errors, Field::Image),
                oninput: move |e: FormEvent| edit(draft, |d| d.image = e.value()),
            }
            TextField {
                label: "Command".to_string(),
                initial_value: current.command.clone(),
                placeholder: "The image's own command".to_string(),
                aria_label: "Command".to_string(),
                error: error_of(&errors, Field::Command),
                oninput: move |e: FormEvent| edit(draft, |d| d.command = e.value()),
            }
        }
        section { class: "er-group",
            h3 { class: "er-group-title", "Environment" }
            if env_values_note {
                p { class: "er-muted", "Values stay hidden. Type a value only to replace it." }
            }
            for row in current.env.clone() {
                {
                    let row_id = row.id;
                    let key_error = error_of(&errors, Field::EnvKey(row_id));
                    rsx! {
                        div { key: "{row_id}", class: "er-row er-env-row",
                            if row.existing {
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
                                placeholder: if row.existing { "•••• unchanged".to_string() } else { "value".to_string() },
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
                    onclick: move |_| edit(draft, |d| {
                        d.add_env();
                    }),
                }
            }
        }
        section { class: "er-group",
            h3 { class: "er-group-title", "Ports" }
            p { class: "er-muted", "host:container, 127.0.0.1:8080:80 or 80/udp" }
            for (row_id , text) in current.ports.clone() {
                div { key: "{row_id}", class: "er-row",
                    TextField {
                        initial_value: text,
                        placeholder: "56080:80".to_string(),
                        aria_label: "Port".to_string(),
                        error: error_of(&errors, Field::Port(row_id)),
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
                    onclick: move |_| edit(draft, |d| {
                        d.add_port();
                    }),
                }
            }
        }
        section { class: "er-group",
            h3 { class: "er-group-title", "Mounts" }
            p { class: "er-muted", "/host/path:/in/container[:ro] or volume:/in/container" }
            for (row_id , text) in current.mounts.clone() {
                div { key: "{row_id}", class: "er-row",
                    TextField {
                        initial_value: text,
                        placeholder: "pgdata:/var/lib/postgresql/data".to_string(),
                        aria_label: "Mount".to_string(),
                        error: error_of(&errors, Field::Mount(row_id)),
                        oninput: move |e: FormEvent| edit(draft, |d| d.set_mount(row_id, &e.value())),
                    }
                    IconButton {
                        icon: "trash-2",
                        label: "Remove mount".to_string(),
                        tone: IconButtonTone::Danger,
                        size: ControlSize::Sm,
                        onclick: move |_| edit(draft, |d| d.remove_mount(row_id)),
                    }
                }
            }
            div { class: "er-add",
                Button {
                    label: "Add mount".to_string(),
                    variant: ButtonVariant::Ghost,
                    size: ControlSize::Sm,
                    icon: "plus",
                    onclick: move |_| edit(draft, |d| {
                        d.add_mount();
                    }),
                }
            }
        }
        section { class: "er-group",
            h3 { class: "er-group-title", "Networks" }
            TextField {
                initial_value: current.networks.clone(),
                placeholder: "bridge, shop_default".to_string(),
                aria_label: "Networks".to_string(),
                help: "Separate names with commas".to_string(),
                error: error_of(&errors, Field::Networks),
                oninput: move |e: FormEvent| edit(draft, |d| d.networks = e.value()),
            }
        }
        section { class: "er-group",
            h3 { class: "er-group-title", "Restart policy and limits" }
            LimitsFields { draft, errors }
        }
    }
}

/// Restart policy, CPU and memory: the fields of the Limits tab and of the
/// sheet's last group. Uncontrolled text; the draft follows `oninput`.
#[component]
pub fn LimitsFields(draft: Signal<Option<Draft>>, errors: Vec<FieldError>) -> Element {
    let Some(current) = draft.read().clone() else {
        return rsx! {};
    };
    let options: Vec<SelectOption> = RestartKind::ALL
        .iter()
        .map(|kind| SelectOption::new(kind.id(), kind.label()))
        .collect();
    rsx! {
        div { class: "er-limits",
            Select {
                label: "Restart policy".to_string(),
                options,
                value: current.restart.id().to_string(),
                aria_label: "Restart policy".to_string(),
                onchange: move |e: FormEvent| {
                    if let Some(kind) = RestartKind::from_id(&e.value()) {
                        edit(draft, |d| d.restart = kind);
                    }
                },
            }
            if current.restart == RestartKind::OnFailure {
                TextField {
                    label: "Retries".to_string(),
                    initial_value: current.retries.clone(),
                    placeholder: "No limit".to_string(),
                    aria_label: "Retries".to_string(),
                    error: error_of(&errors, Field::Retries),
                    oninput: move |e: FormEvent| edit(draft, |d| d.retries = e.value()),
                }
            }
            TextField {
                label: "CPUs".to_string(),
                initial_value: current.cpus.clone(),
                placeholder: "No limit".to_string(),
                aria_label: "CPUs".to_string(),
                help: "For example 0.5 or 2".to_string(),
                error: error_of(&errors, Field::Cpus),
                oninput: move |e: FormEvent| edit(draft, |d| d.cpus = e.value()),
            }
            TextField {
                label: "Memory".to_string(),
                initial_value: current.memory.clone(),
                placeholder: "No limit".to_string(),
                aria_label: "Memory".to_string(),
                help: "For example 256m or 1.5g".to_string(),
                error: error_of(&errors, Field::Memory),
                oninput: move |e: FormEvent| edit(draft, |d| d.memory = e.value()),
            }
            TextField {
                label: "Memory plus swap".to_string(),
                initial_value: current.swap.clone(),
                placeholder: "No limit".to_string(),
                aria_label: "Memory plus swap".to_string(),
                error: error_of(&errors, Field::Swap),
                oninput: move |e: FormEvent| edit(draft, |d| d.swap = e.value()),
            }
        }
    }
}

// ---------------------------------------------------------------------
// ⋯ menu items and rename
// ---------------------------------------------------------------------

fn toast_danger(state: &mut AppState, text: String) {
    state.toasts.write().push(crate::state::Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone: crate::state::ToastTone::Danger,
        action: None,
    });
}

/// Open the sheet for a container, once.
pub fn open_sheet(state: &mut AppState, id: &ContainerId) {
    let open = Overlay::EditRecreate(id.clone());
    if !state.overlays.peek().contains(&open) {
        state.overlays.write().push(open);
    }
}

/// A click on one of the header's ⋯ items that this task owns. Returns
/// true when the item asks for the rename bar. Edit opens the sheet,
/// Duplicate runs at once (it only adds a stopped container), Recreate
/// with latest image asks first, in the delete dialog's frame.
pub fn menu_click(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    store: DetailStore,
    container: &berth_core::Container,
    item: &str,
) -> bool {
    match item {
        "edit" => open_sheet(state, &container.id),
        "duplicate" => {
            let Some(service) = actions else { return false };
            let target = ContainerTarget::from(container);
            if let Err(reason) = service.edit(state, &target, EditOp::Duplicate) {
                toast_danger(state, reason);
            }
        }
        "recreate" => {
            let (lost, from_image) = match ready(store.entries.peek().get(&container.id)) {
                Some(loaded) => (
                    not_carried(&loaded.detail),
                    takes_from_image(&loaded.detail),
                ),
                None => (Vec::new(), false),
            };
            if let Some(model) =
                confirm::recreate_latest_model(state, &container.id, &lost, from_image)
            {
                state.overlays.write().push(Overlay::Confirm(model));
            }
        }
        "rename" => return true,
        _ => {}
    }
    false
}

/// ⌘E: the sheet for the selected container, when it is the only one and
/// nothing else is open.
pub fn open_for_selection(state: &mut AppState) {
    if !state.overlays.peek().is_empty() {
        return;
    }
    let id = match &*state.selection.peek() {
        crate::state::Selection::Containers(ids) if ids.len() == 1 => ids[0].clone(),
        _ => return,
    };
    open_sheet(state, &id);
}

/// The inline rename bar under the inspector header: one name field and
/// Rename / Cancel. The name is checked against Docker's rule and the
/// other containers of the engine before the button enables.
#[component]
pub fn RenameBar(container: berth_core::Container, onclose: EventHandler<()>) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let mut typed = use_signal(|| container.name.clone());
    let taken: Vec<String> = state
        .containers
        .read()
        .iter()
        .filter(|c| c.engine == container.engine)
        .map(|c| c.name.clone())
        .collect();
    let taken_refs: Vec<&str> = taken.iter().map(String::as_str).collect();
    let problem = model::rename_problem(&container.name, &typed.read(), &taken_refs);
    let target = ContainerTarget::from(&container);
    rsx! {
        style { "{EDIT_RECREATE_CSS}" }
        div { class: "rn", role: "group", "aria-label": "Rename container",
            TextField {
                initial_value: container.name.clone(),
                aria_label: "New name".to_string(),
                error: problem.clone().filter(|_| typed.read().trim() != container.name),
                oninput: move |e: FormEvent| typed.set(e.value()),
            }
            div { class: "rn-actions",
                Button {
                    label: "Rename".to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Sm,
                    disabled: problem.is_some(),
                    disabled_reason: problem,
                    onclick: move |_| {
                        let name = typed.peek().trim().to_string();
                        if let Some(service) = actions.as_ref() {
                            let action = berth_core::ContainerAction::Rename { name };
                            if let Err(reason) = service.container(&mut state, &target, action) {
                                toast_danger(&mut state, reason);
                            }
                        }
                        onclose.call(());
                    },
                }
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: move |_| onclose.call(()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
