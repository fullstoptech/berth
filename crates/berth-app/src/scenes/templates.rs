//! Scenes of the Templates screen (P5-T9). The design has no frame for it (brief 7.3 lists its
//! states: default, custom empty, dark); these are dev scenes.

use super::*;

use berth_core::{Template, TemplateKind};

use crate::screens::templates::{DevConfirmDelete, SeedTemplates};

fn templates_world(state: &mut AppState, theme: berth_store::Theme) {
    load_mock_world(state);
    state.settings.write().theme = theme;
    state.section.set(Section::Templates);
}

/// The templates a user saved from a service (what P8-T4 writes): Postgres with another image.
fn custom_templates() -> Vec<Template> {
    berth_rules::templates::built_in()
        .into_iter()
        .filter(|t| t.id == "postgres")
        .map(|mut t| {
            t.id = "custom-pg-pgvector".to_string();
            t.name = "Postgres with pgvector".to_string();
            t.kind = TemplateKind::Custom;
            t.image = "pgvector/pgvector:pg17".to_string();
            t.description =
                "Postgres 17 with the pgvector extension, saved from shop-api.".to_string();
            t
        })
        .collect()
}

fn list(state: &mut AppState) {
    templates_world(state, berth_store::Theme::Dark);
    let mut all = berth_rules::templates::built_in();
    all.extend(custom_templates());
    provide_context(SeedTemplates(all));
}

fn custom_empty(state: &mut AppState) {
    templates_world(state, berth_store::Theme::Dark);
    provide_context(SeedTemplates(berth_rules::templates::built_in()));
}

fn list_light(state: &mut AppState) {
    templates_world(state, berth_store::Theme::Light);
    let mut all = berth_rules::templates::built_in();
    all.extend(custom_templates());
    provide_context(SeedTemplates(all));
}

/// Delete on the custom row was clicked: the dialog asks, from one `EffectSummary`.
fn delete_confirm(state: &mut AppState) {
    list(state);
    provide_context(DevConfirmDelete("custom-pg-pgvector".to_string()));
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Templates / custom empty / 1440", custom_empty),
    Scene::dev("Templates / delete confirm / 1440", delete_confirm),
    Scene::dev("Templates / light / 1440", list_light),
    Scene::dev("Templates / list / 1440", list),
];
