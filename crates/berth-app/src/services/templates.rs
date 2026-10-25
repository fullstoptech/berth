//! The templates the Templates screen lists and the Add service sheet offers (P5-T9): the built-in
//! ones of D84 and the user's own from the store. Custom templates are saved by P8-T4; until then
//! the store holds none.
//!
//! Reading the store is I/O, so callers run [`all`] off the UI thread.

use berth_core::{Template, TemplateInputKind};

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
