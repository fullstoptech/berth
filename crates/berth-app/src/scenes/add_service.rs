//! Scenes of the Add service sheet (P5-T9). The design has no frame for it; these are dev scenes over
//! the mock world, whose `shop-api` project already has a `redis` service and whose `pg-scratch`
//! container holds port 5432.

use super::*;

use crate::sheets::add_service::DevAddService;
use crate::state::{AddServiceSeed, Overlay};

/// The password the scenes give the sheet, so a test can prove the sheet never draws it.
pub(crate) const SCENE_PASSWORD: &str = "Scene0Password0Never0Shown";

fn shop_api() -> berth_core::ProjectId {
    crate::services::mock_world_at(chrono::Utc::now())
        .projects
        .into_iter()
        .find(|p| p.name == "shop-api")
        .map_or_else(|| berth_core::ProjectId("shop-api".to_string()), |p| p.id)
}

fn open(state: &mut AppState, seed: AddServiceSeed, dev: DevAddService) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    provide_context(dev);
    state.overlays.write().push(Overlay::AddService(seed));
}

fn dev() -> DevAddService {
    DevAddService {
        password: Some(SCENE_PASSWORD.to_string()),
        ..DevAddService::default()
    }
}

fn seed(template: &str, project: Option<berth_core::ProjectId>) -> AddServiceSeed {
    AddServiceSeed {
        project,
        template: Some(template.to_string()),
        image: None,
    }
}

/// Opened from Templates: no project yet.
fn pick_project(state: &mut AppState) {
    open(state, seed("redis", None), dev());
}

/// Postgres into shop-api: 5432 is taken by pg-scratch, so the port field starts at 5433.
fn postgres(state: &mut AppState) {
    open(state, seed("postgres", Some(shop_api())), dev());
}

/// The user typed 5432 over the suggestion.
fn port_taken(state: &mut AppState) {
    let mut d = dev();
    d.values.insert("PORT".to_string(), "5432".to_string());
    open(state, seed("postgres", Some(shop_api())), d);
}

/// shop-api has a `redis` service: the name typed is taken.
fn name_exists(state: &mut AppState) {
    let mut d = dev();
    d.name = Some("redis".to_string());
    open(state, seed("redis", Some(shop_api())), d);
}

/// From Find images, "Add to project…" on an image.
fn from_image(state: &mut AppState) {
    open(
        state,
        AddServiceSeed {
            project: None,
            template: None,
            image: Some("nginx:1.27-alpine".to_string()),
        },
        dev(),
    );
}

/// RabbitMQ has two ports, a version and a password: the longest form.
fn rabbitmq(state: &mut AppState) {
    open(state, seed("rabbitmq", Some(shop_api())), dev());
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Add service / from image / 1440", from_image),
    Scene::dev("Add service / name exists / 1440", name_exists),
    Scene::dev("Add service / pick project / 1440", pick_project),
    Scene::dev("Add service / port taken / 1440", port_taken),
    Scene::dev("Add service / postgres / 1440", postgres),
    Scene::dev("Add service / rabbitmq / 1440", rabbitmq),
];
