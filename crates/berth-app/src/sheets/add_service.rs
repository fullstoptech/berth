//! The Add service sheet (P5-T9, brief 6.8): a template (or an image from Find images) becomes a
//! service of a project. The sheet asks for the project when it was opened without one, the service's
//! name, and the template's inputs (host ports with a free one suggested, a version, a data volume, a
//! generated password that stays masked). Nothing is written here: the button builds a plan, parks it
//! in the `PlanStore` and opens `Overlay::ReviewChanges`, whose button writes the files and starts the
//! new service.
//!
//! The name is editable so a project that has a service of that name (the compose file has `cache`,
//! the template says `redis`: or the other way round) never ends in `ServiceExists`: the field starts
//! with a name that is free, says so when the typed one is not, and takes the compose file's word when
//! the plan builder finds the name taken in a file the project list did not show.

use std::collections::BTreeMap;

use dioxus::prelude::*;

use uuid::Uuid;

use berth_core::{Project, ProjectId, Template};

use crate::services::plans::{blocking, PlanStore};
use crate::state::{AddServiceSeed, AppState, Overlay};
use crate::ui::{
    Button, ButtonVariant, ControlSize, InlineMessage, InlineTone, Select, SelectOption, TextField,
};
use model::{
    base_name, blocker, eligible_projects, engine_containers, engine_taken_ports,
    existing_services, image_template, input_rows, is_image, name_error, pending_plan, plan_inputs,
    unique_name, AddError, RowKind,
};

pub mod model;

/// The sheet's CSS: Generate's sheet frame and the few rules of this one (tokens only).
pub const ADD_SERVICE_CSS: &str = concat!(
    include_str!("../../assets/css/generate.css"),
    include_str!("../../assets/css/add_service.css")
);

/// What the password row shows. A constant: the password itself is never drawn (CLAUDE.md, secrets).
pub const PASSWORD_MASK: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

/// Scenes and tests start the sheet from a state a click would have to reach: a password they know (to
/// prove it is not drawn), a typed name, typed inputs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DevAddService {
    pub password: Option<String>,
    pub name: Option<String>,
    pub values: BTreeMap<String, String>,
}

/// Build the plan for the click on Review changes and park it for that sheet. Nothing is written.
///
/// # Errors
/// [`AddError::NameTaken`] when the compose file already has the service, else a sentence.
pub async fn submit(
    store: &PlanStore,
    project: Project,
    template: Template,
    name: String,
    inputs: BTreeMap<String, String>,
) -> Result<Uuid, AddError> {
    let pending = blocking(move || pending_plan(&project, &template, &name, &inputs))
        .await
        .ok_or_else(|| AddError::Failed("The plan did not finish.".to_string()))??;
    Ok(store.insert(pending))
}

/// The template the sheet is on: the seed's image, else the picked (or seeded) template.
fn current_template(
    seed: &AddServiceSeed,
    templates: &[Template],
    picked: &str,
) -> Result<Template, String> {
    if let Some(image) = &seed.image {
        return image_template(image);
    }
    templates
        .iter()
        .find(|t| t.id == picked)
        .or_else(|| templates.first())
        .cloned()
        .ok_or_else(|| "There are no templates.".to_string())
}

#[component]
pub fn AddServiceSheet(seed: AddServiceSeed) -> Element {
    let mut state = use_context::<AppState>();
    let store = try_consume_context::<PlanStore>();
    let services = try_consume_context::<crate::services::Services>();
    let dev = try_consume_context::<DevAddService>();
    let dev_set = dev.is_some();

    let mut templates = use_signal(berth_rules::templates::built_in);
    let mut picked = use_signal({
        let seed_template = seed.template.clone();
        move || seed_template.unwrap_or_else(|| "postgres".to_string())
    });
    let mut project_id = use_signal({
        let project = seed.project.clone();
        move || project
    });
    let mut edited_name = use_signal({
        let name = dev.as_ref().and_then(|d| d.name.clone());
        move || name
    });
    let mut values = use_signal({
        let values = dev.as_ref().map(|d| d.values.clone()).unwrap_or_default();
        move || values
    });
    let mut password = use_signal({
        let known = dev.as_ref().and_then(|d| d.password.clone());
        move || known.unwrap_or_else(berth_rules::templates::generate_password)
    });
    let mut rejected = use_signal(|| None::<String>);
    let mut notice = use_signal(|| None::<String>);
    let mut building = use_signal(|| false);

    // The user's own templates, read once off the UI thread; the built-in ones are there from the
    // first paint.
    use_effect(move || {
        if dev_set {
            return;
        }
        let store = services.as_ref().map(|s| s.store.clone());
        spawn(async move {
            if let Some(all) =
                blocking(move || crate::services::templates::all(store.as_deref())).await
            {
                templates.set(all);
            }
        });
    });

    let template = current_template(&seed, &templates.read(), &picked.read());
    let projects = eligible_projects(&state.projects.read());
    let project: Option<Project> = project_id
        .read()
        .as_ref()
        .and_then(|id| state.projects.read().iter().find(|p| p.id == *id).cloned());
    let containers = state.containers.read().clone();
    let (taken, on_engine) = match &project {
        Some(p) => (
            engine_taken_ports(p, &containers),
            engine_containers(p, &containers),
        ),
        None => (Vec::new(), Vec::new()),
    };
    let existing = project.as_ref().map(existing_services).unwrap_or_default();

    let shown_seed = seed.clone();
    let cancel_seed = seed.clone();

    let body = match template {
        Err(message) => rsx! {
            InlineMessage { message, tone: InlineTone::Danger }
        },
        Ok(t) => {
            let default_name = unique_name(&base_name(&t), &existing);
            let name = edited_name.read().clone().unwrap_or(default_name);
            let rejected_name = rejected.read().clone();
            let name_problem = name_error(&name, &existing, rejected_name.as_deref());
            let rows = input_rows(&t, &values.read(), &taken, &on_engine);
            let reason = blocker(project.as_ref(), name_problem.as_deref(), &rows).or_else(|| {
                state
                    .backing
                    .read()
                    .unsupported(crate::services::backing::Capability::WriteFiles)
            });
            let form_key = format!(
                "{}|{}|{}",
                t.id,
                project_id.read().as_ref().map_or("", |p| p.0.as_str()),
                rows.iter()
                    .map(|r| r.initial.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let name_for_field = name.clone();
            let image_note = if is_image(&t) {
                None
            } else {
                Some(t.image.clone())
            };

            let project_options: Vec<SelectOption> =
                std::iter::once(SelectOption::new("", "Choose a project"))
                    .chain(
                        projects
                            .iter()
                            .map(|p| SelectOption::new(p.id.0.clone(), p.name.clone())),
                    )
                    .collect();
            let template_options: Vec<SelectOption> = templates
                .read()
                .iter()
                .map(|t| {
                    SelectOption::new(t.id.clone(), format!("{} \u{b7} {}", t.name, t.category))
                })
                .collect();
            let template_value = t.id.clone();
            let template_image = t.image.clone();
            let template_description = t.description.clone();
            let from_image = is_image(&t);

            let click_project = project.clone();
            let click_template = t.clone();
            let click_name = name.clone();
            let click_taken = taken.clone();
            let click_store = store.clone();
            let on_review = move |_| {
                if *building.peek() {
                    return;
                }
                let (Some(project), Some(store)) = (click_project.clone(), click_store.clone())
                else {
                    notice.set(Some("Review changes is not connected.".to_string()));
                    return;
                };
                building.set(true);
                notice.set(None);
                let template = click_template.clone();
                let name = click_name.clone();
                let inputs = plan_inputs(&template, &values.peek(), &click_taken, &password.peek());
                spawn(async move {
                    let outcome = submit(&store, project, template, name, inputs).await;
                    building.set(false);
                    match outcome {
                        Ok(id) => state.overlays.write().push(Overlay::ReviewChanges(id)),
                        Err(AddError::NameTaken(taken_name)) => rejected.set(Some(taken_name)),
                        Err(AddError::Failed(message)) => notice.set(Some(message)),
                    }
                });
            };

            rsx! {
                p { class: "gen-lede",
                    "Berth adds the service to the project\u{2019}s compose file and its secrets to .env. You review every change before it is written."
                }
                section { class: "gen-card", "aria-label": "Service",
                    h3 { class: "gen-card-title", "Service" }
                    div { class: "gen-field",
                        if let Some(p) = shown_seed.project.as_ref().and_then(|id| state.projects.read().iter().find(|p| p.id == *id).cloned()) {
                            dl { class: "gen-facts",
                                div { class: "gen-fact",
                                    dt { "Project" }
                                    dd { "{p.name}" }
                                }
                            }
                        } else {
                            Select {
                                label: "Project".to_string(),
                                options: project_options,
                                value: project_id.read().as_ref().map(|p| p.0.clone()).unwrap_or_default(),
                                aria_label: "Project".to_string(),
                                onchange: move |e: FormEvent| {
                                    let value = e.value();
                                    project_id.set((!value.is_empty()).then_some(ProjectId(value)));
                                    edited_name.set(None);
                                    rejected.set(None);
                                    values.write().clear();
                                },
                            }
                        }
                    }
                    div { class: "gen-field",
                        if from_image {
                            dl { class: "gen-facts",
                                div { class: "gen-fact",
                                    dt { "Image" }
                                    dd { class: "as-mono", "{template_image}" }
                                }
                            }
                        } else {
                            Select {
                                label: "Template".to_string(),
                                options: template_options,
                                value: template_value,
                                aria_label: "Template".to_string(),
                                onchange: move |e: FormEvent| {
                                    picked.set(e.value());
                                    edited_name.set(None);
                                    rejected.set(None);
                                    values.write().clear();
                                },
                            }
                            p { class: "as-note", "{template_description}" }
                            if let Some(image) = image_note {
                                p { class: "as-note as-mono", "{image}" }
                            }
                        }
                    }
                    // Keyed by the project and the template, so a new service starts from its own name.
                    for _ in std::iter::once(()) {
                        div { key: "{form_key}-name", class: "gen-field",
                            TextField {
                                label: "Service name".to_string(),
                                initial_value: name_for_field.clone(),
                                aria_label: "Service name".to_string(),
                                error: name_problem.clone(),
                                help: "Other services in the project reach it by this name.".to_string(),
                                oninput: move |e: FormEvent| {
                                    edited_name.set(Some(e.value().trim().to_string()));
                                    rejected.set(None);
                                },
                            }
                        }
                    }
                }
                if !rows.is_empty() {
                    section { class: "gen-card", "aria-label": "Settings",
                        h3 { class: "gen-card-title", "Settings" }
                        // Keyed by the template, the project and the suggestions, so a new set of
                        // fields starts from its own values.
                        for _ in std::iter::once(()) {
                            div { key: "{form_key}", class: "gen-ports",
                                for row in rows.iter().cloned() {
                                    {
                                        let key = row.key.clone();
                                        let initial = dev
                                            .as_ref()
                                            .and_then(|d| d.values.get(&row.key).cloned())
                                            .unwrap_or_else(|| row.initial.clone());
                                        match row.kind.clone() {
                                            RowKind::Password => rsx! {
                                                div { key: "{row.key}", class: "gen-port as-password",
                                                    span { class: "field-label", "{row.label}" }
                                                    div { class: "as-password-row",
                                                        span {
                                                            class: "as-mask",
                                                            role: "img",
                                                            "aria-label": "Generated password, hidden",
                                                            "{PASSWORD_MASK}"
                                                        }
                                                        Button {
                                                            label: "New password".to_string(),
                                                            variant: ButtonVariant::Ghost,
                                                            size: ControlSize::Sm,
                                                            icon: "rotate-cw",
                                                            onclick: move |_| password.set(berth_rules::templates::generate_password()),
                                                        }
                                                    }
                                                    p { class: "as-note", "Generated for you. It goes in .env, not in compose.yaml." }
                                                }
                                            },
                                            RowKind::Version { choices } => rsx! {
                                                div { key: "{row.key}", class: "gen-port",
                                                    Select {
                                                        label: row.label.clone(),
                                                        options: choices
                                                            .iter()
                                                            .map(|c| SelectOption::new(c.clone(), c.clone()))
                                                            .collect::<Vec<_>>(),
                                                        value: row.current.clone(),
                                                        aria_label: row.label.clone(),
                                                        onchange: move |e: FormEvent| {
                                                            let text = e.value();
                                                            values.with_mut(|v| {
                                                                v.insert(key.clone(), text);
                                                            });
                                                        },
                                                    }
                                                }
                                            },
                                            RowKind::Port | RowKind::Volume => rsx! {
                                                div { key: "{row.key}", class: "gen-port",
                                                    TextField {
                                                        label: row.label.clone(),
                                                        initial_value: initial,
                                                        aria_label: row.label.clone(),
                                                        error: row.error.clone(),
                                                        help: row.warning.clone(),
                                                        oninput: move |e: FormEvent| {
                                                            let text = e.value();
                                                            values.with_mut(|v| {
                                                                v.insert(key.clone(), text);
                                                            });
                                                        },
                                                    }
                                                }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(text) = notice.read().clone() {
                    InlineMessage { message: text, tone: InlineTone::Danger }
                }
                div { class: "gen-footer",
                    Button {
                        label: "Cancel".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        onclick: move |_| {
                            state.overlays.write().retain(|o| *o != Overlay::AddService(cancel_seed.clone()));
                        },
                    }
                    Button {
                        label: "Review changes".to_string(),
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Md,
                        loading: *building.read(),
                        disabled: reason.is_some() || *building.read(),
                        disabled_reason: reason,
                        onclick: on_review,
                    }
                }
            }
        }
    };

    rsx! {
        style { "{ADD_SERVICE_CSS}" }
        div {
            class: "gen-sheet as-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Add service",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "gen-title", "Add service" }
            div { class: "gen-body", {body} }
        }
    }
}

#[cfg(test)]
mod tests;
