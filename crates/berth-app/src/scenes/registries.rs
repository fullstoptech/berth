//! Scenes of the Registries screen (P8-T2, no frame).

use super::*;

#[allow(clippy::expect_used)] // static scene URLs, written above
fn url(text: &str) -> url::Url {
    url::Url::parse(text).expect("scene url parses")
}

/// "Dev / registries": two stored rows, one logged in, one failed.
fn registries_list(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Dark);
    state.section.set(Section::Registries);
    state.registries.set(vec![
        berth_core::Registry {
            id: "docker.io".to_string(),
            name: "Docker Hub".to_string(),
            url: url("https://index.docker.io"),
            username: Some("emmanuel".to_string()),
            status: berth_core::RegistryStatus::LoggedIn,
        },
        berth_core::Registry {
            id: "127.0.0.1:56500".to_string(),
            name: "Fixture".to_string(),
            url: url("http://127.0.0.1:56500"),
            username: Some("berthfx".to_string()),
            status: berth_core::RegistryStatus::Error {
                reason: "Wrong username or password".to_string(),
            },
        },
    ]);
}

/// "Dev / registry login": the Add registry sheet over the list.
fn registry_login(state: &mut AppState) {
    registries_list(state);
    state
        .overlays
        .write()
        .push(crate::state::Overlay::AddRegistry(None));
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Dev / registries", registries_list),
    Scene::dev("Dev / registry login", registry_login),
];
