//! The Templates screen (P5-T9, brief 7.3; no frame, so it is built from the Networks header and
//! table). A plain list grouped by category: name, version, image, ports and a one-line description,
//! with "Add to project…" on each row. The built-in templates (D84) come first; the user's own
//! (saved from a project's service, P8-T4) follow under "Custom", with an empty state until there
//! are some, and a Delete button on each row that asks first.
//!
//! The view model is pure ([`groups`]); the component draws it and opens `Overlay::AddService`.

use dioxus::prelude::*;

use berth_core::{Template, TemplateKind};

use crate::services::plans::blocking;
use crate::services::templates::{categories, default_ports, default_version, delete_effect};
use crate::state::{AddServiceSeed, AppState, Overlay, Toast, ToastTone};
use crate::ui::{
    Badge, BadgeKind, Button, ButtonVariant, ConfirmDialog, ControlSize, EmptyState, IconButton,
    IconButtonTone, SearchField,
};

/// The screen's CSS.
pub const TEMPLATES_CSS: &str = include_str!("../../assets/css/templates.css");

/// The title of the group that holds the user's own templates.
pub const CUSTOM_GROUP: &str = "Custom";

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRow {
    pub id: String,
    pub name: String,
    /// "17" for Postgres; `None` for a template with no version choice.
    pub version: Option<String>,
    pub image: String,
    /// "5672, 15672"; empty when the template publishes nothing.
    pub ports: String,
    pub description: String,
    /// Saved by the user: the row offers Delete.
    pub custom: bool,
}

/// A group of rows under a heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub title: String,
    pub rows: Vec<TemplateRow>,
}

fn row_of(t: &Template) -> TemplateRow {
    TemplateRow {
        id: t.id.clone(),
        name: t.name.clone(),
        version: default_version(t),
        image: t.image.clone(),
        ports: default_ports(t)
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        description: t.description.clone(),
        custom: t.kind == TemplateKind::Custom,
    }
}

fn matches(t: &Template, terms: &[String]) -> bool {
    let haystack =
        format!("{} {} {} {}", t.name, t.image, t.description, t.category).to_lowercase();
    terms.iter().all(|term| haystack.contains(term))
}

/// The groups to draw: one per category of the built-in templates, in the order the templates list
/// them, then Custom. A filter keeps the templates whose name, image, description or category
/// contain every word and drops the groups it empties; with no filter the Custom group stays even
/// when it is empty, so its empty state can show.
#[must_use]
pub fn groups(templates: &[Template], filter: &str) -> Vec<Group> {
    let terms: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    let filtering = !terms.is_empty();
    let built_in: Vec<&Template> = templates
        .iter()
        .filter(|t| t.kind == TemplateKind::BuiltIn)
        .collect();
    let custom: Vec<&Template> = templates
        .iter()
        .filter(|t| t.kind == TemplateKind::Custom)
        .collect();
    let built_in_owned: Vec<Template> = built_in.iter().map(|t| (*t).clone()).collect();
    let mut out: Vec<Group> = categories(&built_in_owned)
        .into_iter()
        .map(|category| Group {
            title: category.clone(),
            rows: built_in
                .iter()
                .filter(|t| t.category == category && matches(t, &terms))
                .map(|t| row_of(t))
                .collect(),
        })
        .filter(|g| !g.rows.is_empty())
        .collect();
    let custom_rows: Vec<TemplateRow> = custom
        .iter()
        .filter(|t| matches(t, &terms))
        .map(|t| row_of(t))
        .collect();
    if !filtering || !custom_rows.is_empty() {
        out.push(Group {
            title: CUSTOM_GROUP.to_string(),
            rows: custom_rows,
        });
    }
    out
}

/// Scenes and tests start the screen with templates they chose instead of reading the store.
#[derive(Debug, Clone, PartialEq)]
pub struct SeedTemplates(pub Vec<Template>);

/// Scenes and tests open the Delete confirm of one custom template (by id) without a click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevConfirmDelete(pub String);

/// Open the Add service sheet on one template.
pub fn add_to_project(state: &mut AppState, template_id: &str) {
    state
        .overlays
        .write()
        .push(Overlay::AddService(AddServiceSeed {
            project: None,
            template: Some(template_id.to_string()),
            image: None,
        }));
}

#[component]
fn TemplateRowView(row: TemplateRow, ondelete: EventHandler<String>) -> Element {
    let mut state = use_context::<AppState>();
    let mut confirming = use_signal({
        let open = try_consume_context::<DevConfirmDelete>().is_some_and(|d| d.0 == row.id);
        move || open
    });
    let id = row.id.clone();
    let delete_id = row.id.clone();
    let delete_name = row.name.clone();
    rsx! {
        div { class: "tp-row", role: "row", "aria-label": "{row.name}",
            div { class: "tp-cell tp-name", role: "cell",
                span { class: "tp-name-text", title: "{row.name}", "{row.name}" }
            }
            div { class: "tp-cell tp-version tp-muted", role: "cell",
                if let Some(version) = &row.version {
                    "{version}"
                } else {
                    "\u{2013}"
                }
            }
            div { class: "tp-cell tp-image tp-mono", role: "cell", title: "{row.image}", "{row.image}" }
            div { class: "tp-cell tp-ports tp-mono", role: "cell",
                if row.ports.is_empty() {
                    span { class: "tp-muted", "\u{2013}" }
                } else {
                    "{row.ports}"
                }
            }
            div { class: "tp-cell tp-desc tp-muted", role: "cell", title: "{row.description}", "{row.description}" }
            div { class: "tp-cell tp-actions", role: "cell",
                Button {
                    label: "Add to project\u{2026}".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: move |_| add_to_project(&mut state, &id),
                }
                if row.custom {
                    IconButton {
                        icon: "trash-2",
                        label: format!("Delete {}", row.name),
                        tone: IconButtonTone::Danger,
                        size: ControlSize::Sm,
                        tooltip: Some("Delete".to_string()),
                        onclick: move |_| confirming.set(true),
                    }
                }
            }
        }
        if *confirming.read() {
            // One summary draws the title's noun, the lines and the button ("Delete template").
            div {
                class: "overlay-scrim",
                style: "z-index: 60;",
                onclick: move |_| confirming.set(false),
                onkeydown: move |e: KeyboardEvent| {
                    if e.key() == Key::Escape {
                        confirming.set(false);
                    }
                },
                ConfirmDialog {
                    title: format!("Delete template {delete_name}?"),
                    effect: delete_effect(&delete_name),
                    oncheck: move |_| {},
                    oncancel: move |()| confirming.set(false),
                    onconfirm: move |()| {
                        confirming.set(false);
                        ondelete.call(delete_id.clone());
                    },
                }
            }
        }
    }
}

#[component]
pub fn TemplatesScreen() -> Element {
    let seed = try_consume_context::<SeedTemplates>();
    let services = try_consume_context::<crate::services::Services>();
    let seeded = seed.is_some();
    let store = services.as_ref().map(|s| s.store.clone());
    let mut toasts = use_context::<AppState>().toasts;
    let mut templates = use_signal({
        let seed = seed.clone();
        move || seed.map_or_else(berth_rules::templates::built_in, |s| s.0)
    });
    let mut filter = use_signal(String::new);
    let mut epoch = use_signal(|| 0_u32);

    let load_store = store.clone();
    // The user's templates come from the store, read once off the UI thread; the built-in ones are
    // there from the first paint.
    use_effect(move || {
        if seeded {
            return;
        }
        let store = load_store.clone();
        spawn(async move {
            if let Some(all) =
                blocking(move || crate::services::templates::all(store.as_deref())).await
            {
                templates.set(all);
            }
        });
    });

    // Delete is confirmed in the row; the store is written first, then the list drops the row.
    let ondelete = EventHandler::new(move |id: String| {
        let store = store.clone();
        let name = templates
            .peek()
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        spawn(async move {
            let stored = match store {
                Some(store) => {
                    let id = id.clone();
                    blocking(move || crate::services::templates::delete(&store, &id))
                        .await
                        .unwrap_or_else(|| Err("Deleting the template did not finish.".to_string()))
                }
                None => Ok(()),
            };
            let (text, tone) = match stored {
                Ok(()) => {
                    templates.write().retain(|t| t.id != id);
                    (
                        crate::services::confirm::result_label(&delete_effect(&name), 1),
                        ToastTone::Success,
                    )
                }
                Err(reason) => (reason, ToastTone::Danger),
            };
            toasts.write().push(Toast {
                id: uuid::Uuid::now_v7(),
                text,
                tone,
                action: None,
            });
        });
    });

    let all = templates.read().clone();
    let total = all.len();
    let shown = groups(&all, &filter.read());
    let matched: usize = shown.iter().map(|g| g.rows.len()).sum();
    let filtering = !filter.read().trim().is_empty();

    rsx! {
        style { "{TEMPLATES_CSS}" }
        section { class: "templates-screen", "aria-label": "Templates",
            div { class: "tp-header",
                h1 { class: "tp-title", "Templates" }
                Badge { kind: BadgeKind::Count, text: "{total}" }
                span { class: "tp-spacer" }
                div { class: "tp-filter",
                    for key in [*epoch.read()] {
                        SearchField {
                            key: "{key}",
                            initial_value: filter.peek().clone(),
                            placeholder: "Filter templates".to_string(),
                            aria_label: "Filter templates".to_string(),
                            matched: if filtering { Some(matched) } else { None },
                            total: if filtering { Some(total) } else { None },
                            oninput: move |e: FormEvent| filter.set(e.value()),
                            onclear: move |_| {
                                filter.set(String::new());
                                let next = *epoch.peek() + 1;
                                epoch.set(next);
                            },
                        }
                    }
                }
            }
            if matched == 0 && filtering {
                div { class: "tp-empty",
                    EmptyState {
                        title: "No template matches".to_string(),
                        body: "Nothing in the list matches the filter.".to_string(),
                        icon: "file-text",
                        action_label: Some("Clear filter".to_string()),
                        onaction: move |()| {
                            filter.set(String::new());
                            let next = *epoch.peek() + 1;
                            epoch.set(next);
                        },
                    }
                }
            } else {
                div { class: "tp-list", role: "table", "aria-label": "Templates",
                    div { class: "tp-head", role: "row",
                        span { role: "columnheader", "Name" }
                        span { class: "tp-version", role: "columnheader", "Version" }
                        span { class: "tp-image", role: "columnheader", "Image" }
                        span { class: "tp-ports", role: "columnheader", "Ports" }
                        span { class: "tp-desc", role: "columnheader", "Description" }
                        span {}
                    }
                    for group in shown {
                        section { key: "{group.title}", class: "tp-group", "aria-label": "{group.title}",
                            h2 { class: "tp-group-title",
                                "{group.title}"
                                span { class: "tp-group-count", "{group.rows.len()}" }
                            }
                            if group.rows.is_empty() {
                                // Only the Custom group is ever empty.
                                div { class: "tp-custom-empty",
                                    div { class: "tp-custom-empty-title", "No custom templates yet" }
                                    div { class: "tp-muted", "Save any service as a template from its menu." }
                                }
                            }
                            for row in group.rows {
                                TemplateRowView { key: "{row.id}", row, ondelete }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;
