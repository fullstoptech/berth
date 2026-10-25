//! Scenes of the Generate sheet (P5-T3). The design has no frame for it; these are dev scenes.

use super::*;

use berth_core::{
    Confidence, Detection, FoundFile, FoundKind, ServiceGuess, ServiceKind, StackGuess,
};

use crate::sheets::generate::SeedDetection;
use crate::state::Overlay;

fn found(kind: FoundKind, name: &str) -> FoundFile {
    FoundFile {
        kind,
        path: std::path::PathBuf::from(name),
    }
}

/// What the Next.js fixture detects: a Dockerfile, a `.env` with three keys, Postgres and Redis.
pub(crate) fn next_detection() -> Detection {
    Detection {
        folder: berth_sys::paths::home().join("Dev/side/new-app"),
        found: vec![
            found(
                FoundKind::Dockerfile {
                    from: Some("node:22-alpine".to_string()),
                },
                "Dockerfile",
            ),
            found(FoundKind::Env { keys: 3 }, ".env"),
        ],
        stack: vec![StackGuess::Node {
            pm: "pnpm".to_string(),
            framework: Some("Next.js".to_string()),
            framework_version: Some("15".to_string()),
            version: Some("22".to_string()),
        }],
        services_needed: vec![
            ServiceGuess {
                kind: ServiceKind::Postgres,
                reason: "from DATABASE_URL".to_string(),
            },
            ServiceGuess {
                kind: ServiceKind::Redis,
                reason: "from REDIS_URL".to_string(),
            },
        ],
        confidence: Confidence::High,
        git_branch: Some("main".to_string()),
    }
}

fn open(state: &mut AppState, detection: Detection) {
    let folder = detection.folder.clone();
    provide_context(SeedDetection(detection));
    state.overlays.write().push(Overlay::Generate(folder));
}

fn generate_sheet(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    open(state, next_detection());
}

fn generate_compose_exists(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    let mut d = next_detection();
    d.found.push(found(FoundKind::Compose, "compose.yaml"));
    open(state, d);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Generate / sheet / 1440", generate_sheet),
    Scene::dev("Generate / compose exists / 1440", generate_compose_exists),
];
