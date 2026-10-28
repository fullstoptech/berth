//! P5-T9 tests for the Templates screen: the groups, the filter, the custom empty state, and what
//! "Add to project…" opens.

use dioxus::prelude::ReadableExt;

use berth_core::{Template, TemplateKind};

use super::{add_to_project, groups, CUSTOM_GROUP};
use crate::scenes;
use crate::state::{AddServiceSeed, Overlay};

fn all() -> Vec<Template> {
    berth_rules::templates::built_in()
}

fn custom(id: &str, name: &str) -> Template {
    let mut t = all()
        .into_iter()
        .find(|t| t.id == "postgres")
        .expect("postgres");
    t.id = id.to_string();
    t.name = name.to_string();
    t.kind = TemplateKind::Custom;
    t.image = "pgvector/pgvector:pg17".to_string();
    t
}

fn titles(g: &[super::Group]) -> Vec<&str> {
    g.iter().map(|g| g.title.as_str()).collect()
}

#[test]
fn templates_are_grouped_by_category_with_custom_last_even_when_empty() {
    let g = groups(&all(), "");
    assert_eq!(
        titles(&g),
        [
            "Database",
            "Cache",
            "Queue",
            "Search",
            "Storage",
            "Mail",
            "Tools",
            CUSTOM_GROUP
        ]
    );
    let db = &g[0];
    let names: Vec<&str> = db.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Postgres", "MySQL", "MariaDB", "MongoDB"]);
    assert!(g.last().expect("custom").rows.is_empty());
    let total: usize = g.iter().map(|g| g.rows.len()).sum();
    assert_eq!(total, all().len());
}

#[test]
fn templates_rows_show_version_image_and_ports() {
    let g = groups(&all(), "");
    let row = |id: &str| {
        g.iter()
            .flat_map(|g| g.rows.iter())
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("{id}"))
            .clone()
    };
    let pg = row("postgres");
    assert_eq!(pg.version.as_deref(), Some("17"));
    assert_eq!(pg.image, "postgres:17-alpine");
    assert_eq!(pg.ports, "5432");
    assert_eq!(row("rabbitmq").ports, "5672, 15672");
    assert_eq!(row("mailpit").ports, "1025, 8025");
    assert_eq!(row("adminer").version, None);
}

#[test]
fn templates_custom_ones_follow_the_built_in_groups() {
    let mut list = all();
    list.push(custom("custom-1", "Postgres with pgvector"));
    let g = groups(&list, "");
    let last = g.last().expect("custom group");
    assert_eq!(last.title, CUSTOM_GROUP);
    assert_eq!(last.rows.len(), 1);
    assert_eq!(last.rows[0].name, "Postgres with pgvector");
    assert_eq!(last.rows[0].image, "pgvector/pgvector:pg17");
    // A custom template filed under Database does not join the built-in Database group.
    assert_eq!(g[0].rows.len(), 4);
}

#[test]
fn templates_filter_matches_every_word_and_drops_empty_groups() {
    let g = groups(&all(), "redis cache");
    assert_eq!(
        titles(&g),
        ["Cache"],
        "no Custom group while filtering an empty one"
    );
    // Valkey is a cache and says "Redis-compatible", so every word matches it too.
    let ids: Vec<&str> = g[0].rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["redis", "valkey"]);
    let g = groups(&all(), "redis rabbit");
    assert!(g.is_empty(), "every word has to match: {g:?}");

    let g = groups(&all(), "SEAWEED");
    assert_eq!(titles(&g), ["Storage"]);
    let g = groups(&all(), "nothing like this");
    assert!(g.is_empty());

    // A match among the custom templates keeps their group.
    let mut list = all();
    list.push(custom("custom-1", "Postgres with pgvector"));
    let g = groups(&list, "pgvector");
    assert_eq!(titles(&g), [CUSTOM_GROUP]);
}

#[test]
fn templates_add_to_project_opens_the_sheet_on_that_template_with_no_project() {
    let (dom, mut state) = scenes_state();
    dom.in_runtime(|| add_to_project(&mut state, "redis"));
    let overlays = dom.in_runtime(|| state.overlays.peek().clone());
    assert_eq!(
        overlays,
        [Overlay::AddService(AddServiceSeed {
            project: None,
            template: Some("redis".to_string()),
            image: None,
        })]
    );
}

fn scenes_state() -> (dioxus::prelude::VirtualDom, crate::state::AppState) {
    use dioxus::prelude::*;
    thread_local! {
        static SLOT: std::cell::RefCell<Option<crate::state::AppState>> = const { std::cell::RefCell::new(None) };
    }
    #[component]
    fn Harness() -> Element {
        let state = crate::state::use_app_state();
        use_hook(move || SLOT.with(|s| *s.borrow_mut() = Some(state)));
        rsx! { div {} }
    }
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT.with(|s| s.borrow_mut().take()).expect("state");
    (dom, state)
}

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).unwrap_or_else(|| panic!("{frame} renders"))
}

#[test]
fn templates_screen_lists_every_template_with_its_button() {
    let html = scene("Templates / custom empty / 1440");
    assert!(html.contains("aria-label=\"Templates\""), "{html}");
    for name in [
        "Postgres",
        "Redis",
        "S3 (SeaweedFS)",
        "Mailpit",
        "Adminer",
        "Meilisearch",
    ] {
        assert!(html.contains(&format!(">{name}<")), "{name}: {html}");
    }
    for group in [
        "Database", "Cache", "Queue", "Search", "Storage", "Mail", "Tools", "Custom",
    ] {
        assert!(
            html.contains(&format!("aria-label=\"{group}\"")),
            "{group}: {html}"
        );
    }
    assert_eq!(
        html.matches(">Add to project\u{2026}<").count(),
        all().len(),
        "one button per template"
    );
    assert!(html.contains("postgres:17-alpine"), "{html}");
    assert!(html.contains("5672, 15672"), "{html}");
}

#[test]
fn templates_screen_custom_group_is_empty_with_the_briefs_words() {
    let html = scene("Templates / custom empty / 1440");
    assert!(html.contains("No custom templates yet"), "{html}");
    assert!(
        html.contains("Save any service as a template from its menu."),
        "{html}"
    );

    let html = scene("Templates / list / 1440");
    assert!(!html.contains("No custom templates yet"), "{html}");
    assert!(html.contains(">Postgres with pgvector<"), "{html}");
    assert!(html.contains("pgvector/pgvector:pg17"), "{html}");
    assert_eq!(
        html.matches(">Add to project\u{2026}<").count(),
        all().len() + 1
    );
}

// ---------------------------------------------------------------------
// P8-T4: custom templates and Delete
// ---------------------------------------------------------------------

#[test]
fn custom_templates_only_the_users_own_rows_offer_delete() {
    let mut list = all();
    list.push(custom("custom-1", "Postgres with pgvector"));
    let g = groups(&list, "");
    let flags: Vec<(&str, bool)> = g
        .iter()
        .flat_map(|g| g.rows.iter())
        .map(|r| (r.id.as_str(), r.custom))
        .collect();
    assert_eq!(
        flags.iter().filter(|(_, custom)| *custom).count(),
        1,
        "{flags:?}"
    );
    assert!(flags.contains(&("custom-1", true)));
    assert!(flags.contains(&("postgres", false)));
}

#[test]
fn custom_templates_screen_has_a_delete_button_on_the_custom_row_only() {
    let html = scene("Templates / list / 1440");
    assert_eq!(
        html.matches("aria-label=\"Delete Postgres with pgvector\"")
            .count(),
        1,
        "{html}"
    );
    assert_eq!(
        html.matches("aria-label=\"Delete ").count(),
        1,
        "no built-in row can be deleted"
    );
    assert!(!scene("Templates / custom empty / 1440").contains("aria-label=\"Delete "));
    // Nothing is asked until the button is clicked.
    assert!(!html.contains("confirm-dialog\" role=\"dialog\""), "{html}");
}

#[test]
fn custom_templates_delete_asks_from_one_summary() {
    let html = scene("Templates / delete confirm / 1440");
    assert!(
        html.contains("aria-label=\"Delete template Postgres with pgvector?\""),
        "{html}"
    );
    // The button, the lines and the result all come from `delete_effect`.
    let effect = crate::services::templates::delete_effect("Postgres with pgvector");
    for line in &effect.lines {
        assert!(html.contains(&line.text), "{}: {html}", line.text);
    }
    assert!(html.contains(">Delete template<"), "{html}");
    assert!(html.contains(">Cancel<"), "{html}");
    assert!(html.contains("btn-danger"), "{html}");
}
