//! The templates the Templates screen lists and the Add service sheet offers (P5-T9): the built-in
//! ones of D84 and the user's own from the store. Custom templates are saved from a project's service
//! ([`save_from_service`], P8-T4) and deleted from the Templates screen ([`delete_effect`], [`delete`]).
//!
//! Saving writes to Berth's own store, never to the project's files, and it reads the compose file
//! only: no `.env` value is read, and the template that comes out holds no secret (see
//! `berth_rules::templates::from_service`). Adding a service from a custom template goes through
//! Review changes like every other template, so nothing here needs a plan.
//!
//! Reading and writing the store is I/O, so callers run these off the UI thread.

use berth_core::{
    EffectKind, EffectLine, EffectSummary, Project, ProjectId, Template, TemplateInputKind,
};

/// Built-in templates first, in D84's order, then the custom ones by name. A store that cannot be read
/// leaves the built-in list alone (the screen still works) and says so in the log.
#[must_use]
pub fn all(store: Option<&berth_store::Store>) -> Vec<Template> {
    let mut templates = berth_rules::templates::built_in();
    if let Some(store) = store {
        match store.custom_templates() {
            Ok(mut custom) => {
                custom.sort_by_key(|t| t.name.to_lowercase());
                templates.extend(custom);
            }
            Err(err) => tracing::warn!(%err, "reading the custom templates failed"),
        }
    }
    templates
}

/// The categories of `templates` in the order they first appear.
#[must_use]
pub fn categories(templates: &[Template]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for t in templates {
        if !seen.contains(&t.category) {
            seen.push(t.category.clone());
        }
    }
    seen
}

/// The host ports a template publishes by default, in the order of its inputs ("5672, 15672").
#[must_use]
pub fn default_ports(t: &Template) -> Vec<u16> {
    t.inputs
        .iter()
        .filter_map(|i| match i.kind {
            TemplateInputKind::Port { default } => Some(default),
            _ => None,
        })
        .collect()
}

/// The version a template starts with: the first of its version choices, if it has any.
#[must_use]
pub fn default_version(t: &Template) -> Option<String> {
    t.inputs.iter().find_map(|i| match &i.kind {
        TemplateInputKind::Version { choices } => choices.first().cloned(),
        _ => None,
    })
}

/// The store key of the template saved from `service` of `project`: saving that service again replaces
/// the template instead of listing a second one.
#[must_use]
pub fn custom_id(project: &ProjectId, service: &str) -> String {
    let slug: String = format!("{}-{service}", project.0)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.') {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("custom-{slug}")
}

/// What saving did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub name: String,
    /// A template saved from this service before was replaced.
    pub replaced: bool,
}

impl Saved {
    /// The toast text.
    #[must_use]
    pub fn message(&self) -> String {
        if self.replaced {
            format!("Updated the {} template.", self.name)
        } else {
            format!("Saved {} as a template.", self.name)
        }
    }
}

/// The text of the first compose file of `project` that has `service`. Blocking.
fn compose_text_of(project: &Project, service: &str) -> Result<String, String> {
    let folder = project
        .folder
        .clone()
        .ok_or_else(|| "This project has no folder on this Mac.".to_string())?;
    let mut read_any = false;
    for file in &project.compose_files {
        let path = folder.join(file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        read_any = true;
        if berth_rules::compose_edit::service_names(&text)
            .is_ok_and(|names| names.iter().any(|n| n == service))
        {
            return Ok(text);
        }
    }
    if read_any {
        Err(format!(
            "{service} is not in this project's compose file any more."
        ))
    } else {
        Err("Berth cannot find this project's compose file on this Mac.".to_string())
    }
}

/// Save `service` of `project` as a custom template: its block from the compose file on disk, with
/// ports and volumes as inputs and secrets left out. Blocking.
///
/// # Errors
/// A sentence: no compose file, the service is built from a folder, the store cannot be written.
pub fn save_from_service(
    store: &berth_store::Store,
    project: &Project,
    service: &str,
) -> Result<Saved, String> {
    let text = compose_text_of(project, service)?;
    let id = custom_id(&project.id, service);
    let template =
        berth_rules::templates::from_service(&text, service, &id, service, &project.name).map_err(
            |e| match e {
                berth_rules::RulesError::Yaml { message, .. } => message,
                other => other.to_string(),
            },
        )?;
    let replaced = store
        .custom_templates()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|t| t.id == id);
    store.save_template(&template).map_err(|e| e.to_string())?;
    Ok(Saved {
        name: template.name,
        replaced,
    })
}

/// Save from the UI: the work runs off the UI thread and a toast says how it went.
pub fn save(
    state: &mut crate::state::AppState,
    services: &crate::services::Services,
    project: Project,
    service: String,
) {
    let store = services.store.clone();
    let mut state = *state;
    crate::services::actions::spawn_job(async move {
        let result =
            crate::services::plans::blocking(move || save_from_service(&store, &project, &service))
                .await
                .unwrap_or_else(|| Err("Saving the template did not finish.".to_string()));
        let (text, tone) = match result {
            Ok(saved) => (saved.message(), crate::state::ToastTone::Success),
            Err(reason) => (reason, crate::state::ToastTone::Danger),
        };
        dioxus::prelude::WritableExt::write(&mut state.toasts).push(crate::state::Toast {
            id: uuid::Uuid::now_v7(),
            text,
            tone,
            action: None,
        });
    });
}

/// What deleting a custom template does, for the one confirm the Templates screen shows: the button
/// ("Delete template"), the lines and the result ("Deleted 1 template") all come from this value (D33).
#[must_use]
pub fn delete_effect(name: &str) -> EffectSummary {
    EffectSummary {
        verb: "Delete".to_string(),
        noun_one: "template".to_string(),
        noun_many: "templates".to_string(),
        count: 1,
        bytes: None,
        lines: vec![
            EffectLine {
                kind: EffectKind::Goes,
                text: format!("The {name} template leaves the Templates list."),
            },
            EffectLine {
                kind: EffectKind::Stays,
                text: "Services already added from it stay in their projects.".to_string(),
            },
        ],
        typed_name: None,
    }
}

/// Delete the custom template `id`. Blocking.
///
/// # Errors
/// A sentence when the store cannot be written.
pub fn delete(store: &berth_store::Store, id: &str) -> Result<(), String> {
    store.delete_template(id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built_in(id: &str) -> Template {
        berth_rules::templates::built_in()
            .into_iter()
            .find(|t| t.id == id)
            .expect("built-in template")
    }

    #[test]
    fn templates_all_lists_built_ins_then_custom_ones_by_name() {
        let store = berth_store::Store::open_in_memory().expect("store");
        let mut zed = built_in("redis");
        zed.id = "custom-zed".to_string();
        zed.name = "Zed cache".to_string();
        zed.kind = berth_core::TemplateKind::Custom;
        let mut abe = zed.clone();
        abe.id = "custom-abe".to_string();
        abe.name = "abe cache".to_string();
        store.save_template(&zed).expect("save");
        store.save_template(&abe).expect("save");

        let listed = all(Some(&store));
        let built_ins = berth_rules::templates::built_in().len();
        assert_eq!(listed.len(), built_ins + 2);
        assert!(listed[..built_ins]
            .iter()
            .all(|t| t.kind == berth_core::TemplateKind::BuiltIn));
        let custom: Vec<&str> = listed[built_ins..]
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(custom, ["abe cache", "Zed cache"]);
    }

    #[test]
    fn templates_all_without_a_store_is_the_built_in_list() {
        assert_eq!(all(None), berth_rules::templates::built_in());
    }

    #[test]
    fn templates_categories_keep_the_order_of_first_appearance() {
        let cats = categories(&berth_rules::templates::built_in());
        assert_eq!(
            cats,
            ["Database", "Cache", "Queue", "Search", "Storage", "Mail", "Tools"]
        );
    }

    #[test]
    fn templates_defaults_read_the_inputs() {
        let rabbit = built_in("rabbitmq");
        assert_eq!(default_ports(&rabbit), [5672, 15672]);
        assert_eq!(default_version(&rabbit).as_deref(), Some("4"));
        assert_eq!(default_version(&built_in("adminer")), None);
    }
}

#[cfg(test)]
#[path = "templates_custom_tests.rs"]
mod custom_tests;
