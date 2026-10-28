//! Scenes of the Build image sheet and its push card (P8-T3). No frame draws
//! them; they follow the Images frames and the Create volume sheet.

use berth_core::{BuildContext, BuildEvent, BuildSpec, EngineId, PullProgress};

use super::*;
use crate::services::images::{BuildPhase, BuildState, ImageStore, PushState};
use crate::sheets::build_image::{SeedSource, Source};
use crate::state::Overlay;

const ENGINE: &str = "colima:default";
/// A tag the mock engine already lists, so the done page can read its size.
const TAG: &str = "shop-api-worker:latest";

fn build_base(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Images);
    state.overlays.write().push(Overlay::BuildImage);
}

fn build_form(state: &mut AppState) {
    build_base(state);
}

/// The Dockerfile source: the editor's host element in place of the folder.
fn build_dockerfile(state: &mut AppState) {
    build_base(state);
    provide_context(SeedSource(Source::Dockerfile));
}

fn spec() -> BuildSpec {
    BuildSpec {
        context: BuildContext::Folder("/Users/dev/shop-api".into()),
        dockerfile: None,
        tags: vec![TAG.to_string(), "shop-api-worker:2".to_string()],
        build_args: Vec::new(),
        platform: None,
    }
}

fn step(index: u32, text: &str) -> BuildEvent {
    BuildEvent::Step {
        index,
        total: 6,
        text: format!("Step {index}/6 : {text}"),
    }
}

fn log(text: &str) -> BuildEvent {
    BuildEvent::Log(text.to_string())
}

/// A build that has run its first steps.
fn started() -> BuildState {
    let mut build = BuildState::new(EngineId(ENGINE.to_string()), &spec());
    for event in [
        step(1, "FROM node:22-alpine"),
        log(" ---> 3f1a0c47d6b2"),
        step(2, "WORKDIR /app"),
        log(" ---> Using cache"),
        log(" ---> a2318d6c47ec"),
        step(3, "COPY package.json package-lock.json ./"),
        log(" ---> 1f8c0ab51b3e"),
        step(4, "RUN npm ci"),
        log(" ---> Running in 9c1d4be0f7a2"),
    ] {
        build.apply(&event);
    }
    build
}

fn put_build(build: BuildState) {
    let Some(store) = try_consume_context::<ImageStore>() else {
        return;
    };
    let mut builds = store.builds;
    builds.write().push(build);
}

fn build_running(state: &mut AppState) {
    build_base(state);
    put_build(started());
}

fn build_failed(state: &mut AppState) {
    build_base(state);
    let mut build = started();
    for event in [
        log("npm ERR! code ERESOLVE"),
        log("npm ERR! ERESOLVE could not resolve peer dependency react@18"),
        BuildEvent::Failed {
            text: "The command '/bin/sh -c npm ci' returned a non-zero code: 1".to_string(),
        },
    ] {
        build.apply(&event);
    }
    build.phase = BuildPhase::Failed("The command '/bin/sh -c npm ci' returned a non-zero code: 1".to_string());
    put_build(build);
}

fn finished() -> BuildState {
    let mut build = started();
    for event in [
        log("added 312 packages in 9s"),
        log(" ---> 5d2b9e1c7a40"),
        step(5, "COPY . ."),
        log(" ---> e90a1c47d6b2"),
        step(6, "CMD [\"node\", \"worker.js\"]"),
        log(" ---> 7c13f08ea9d5"),
        log("Successfully built 7c13f08ea9d5"),
        log("Successfully tagged shop-api-worker:latest"),
    ] {
        build.apply(&event);
    }
    build.phase = BuildPhase::Done;
    build
}

fn build_done(state: &mut AppState) {
    build_base(state);
    put_build(finished());
}

/// Done, with Push next pressed and the registry part way through.
fn build_pushing(state: &mut AppState) {
    build_done(state);
    let Some(store) = try_consume_context::<ImageStore>() else {
        return;
    };
    let mut push = PushState::new(EngineId(ENGINE.to_string()), TAG);
    push.apply(&PullProgress {
        layer: None,
        status: "The push refers to repository [ghcr.io/dev/shop-api-worker]".to_string(),
        current: None,
        total: None,
    });
    let mut pushes = store.pushes;
    pushes.write().push(push);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Images / build / 1440", build_form),
    Scene::dev("Images / build dockerfile / 1440", build_dockerfile),
    Scene::dev("Images / build running / 1440", build_running),
    Scene::dev("Images / build failed / 1440", build_failed),
    Scene::dev("Images / build done / 1440", build_done),
    Scene::dev("Images / build pushing / 1440", build_pushing),
];
