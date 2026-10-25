//! Scenes of the Project detail screen (P5-T6). The mock projects have no
//! folder on this Mac, so each scene hands the Files tab a folder held in
//! memory (`ProjectFsOverride`); the values in its `.env` are made up.

use std::sync::Arc;

use super::*;
use crate::screens::project_detail::{ProjectDetailSeed, Tab, Validity};
use crate::services::project_files::{
    ComposeProblem, Editor, MemoryFs, ProjectFsOverride,
};

/// The Compose file of the frame: 20 lines.
pub(crate) const SHOP_API_COMPOSE: &str = "services:
  api:
    build: .
    ports: [\"3000:3000\"]
    env_file: .env
    depends_on: [db, redis]
  redis:
    image: redis:7.4-alpine
    ports: [\"6379:6379\"]
  mailpit:
    image: axllent/mailpit:v1.21
  db:
    image: postgres:17-alpine
    ports: [\"5432:5432\"]
  worker:
    build: ./worker
    depends_on: [db]
    environment:
      QUEUE_URL: redis://redis:6379
    restart: always
";

/// Six variables with made-up values: nothing here is a secret.
pub(crate) const SHOP_API_ENV: &str = "DATABASE_URL=postgres://shop:made-up-1@db:5432/shop
REDIS_URL=redis://redis:6379
SMTP_HOST=mailpit
SMTP_PORT=1025
JWT_SECRET=made-up-2
STRIPE_KEY=made-up-3
";

/// The `shop-api` folder of the frame, in memory.
pub(crate) fn shop_api_folder(compose: &str) -> MemoryFs {
    MemoryFs::new()
        .file("compose.yaml", compose)
        .file(".env", SHOP_API_ENV)
        .file(".env.example", "DATABASE_URL=\nREDIS_URL=\n")
        .file("Dockerfile", "FROM node:22-alpine\nWORKDIR /app\nCOPY . .\nCMD [\"node\", \"src/index.js\"]\n")
        .file("worker/Dockerfile", "FROM node:22-alpine\nCMD [\"node\", \"worker.js\"]\n")
        .file("src/index.js", "console.log('shop-api');\n")
        .file("package.json", "{\n  \"name\": \"shop-api\"\n}\n")
}

/// The editor the frame names, whatever this Mac has installed.
fn frame_editor() -> Editor {
    Editor {
        program: Some(std::path::PathBuf::from("/usr/local/bin/code")),
        label: "Visual Studio Code (detected)".to_string(),
    }
}

/// Select the mock project called `name` on its own screen.
fn open_project(state: &mut AppState, name: &str) {
    let id = state
        .projects
        .peek()
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.id.clone());
    if let Some(id) = id {
        state.selection.set(Selection::Project(id));
        state.section.set(Section::Project);
    }
}

fn seed(tab: Tab, validity: Validity, files: MemoryFs) {
    provide_context(ProjectFsOverride(Arc::new(files)));
    provide_context(ProjectDetailSeed {
        tab,
        validity: Some(validity),
        editor: Some(frame_editor()),
        activity: None,
    });
}

/// "Project / files / 1440": shop-api, Failing, its Files tab, dark.
fn project_files(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Dark);
    open_project(state, "shop-api");
    seed(Tab::Files, Validity::Valid, shop_api_folder(SHOP_API_COMPOSE));
}

/// The Compose file with a broken indent on line 4, and what Compose says of it.
fn project_invalid(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    open_project(state, "shop-api");
    let broken = SHOP_API_COMPOSE.replacen("    build: .\n", "   build: .\n", 1);
    let problem = ComposeProblem {
        line: Some(4),
        message: "mapping values are not allowed in this context".to_string(),
        file: None,
    };
    seed(Tab::Files, Validity::Invalid(problem), shop_api_folder(&broken));
}

/// A project that is partly running, on its Services tab.
fn project_services(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    open_project(state, "analytics");
    seed(Tab::Services, Validity::Valid, MemoryFs::new());
}

/// A known folder with nothing created yet.
fn project_not_created(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    open_project(state, "docker-manger");
    seed(Tab::Services, Validity::Unknown, MemoryFs::new());
}

/// The Activity tab of shop-api over the mock feed.
fn project_activity(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    open_project(state, "shop-api");
    provide_context(ProjectFsOverride(Arc::new(MemoryFs::new())));
    provide_context(ProjectDetailSeed {
        tab: Tab::Activity,
        validity: Some(Validity::Valid),
        editor: Some(frame_editor()),
        activity: Some(crate::screens::recent_activity::mock_entries(
            &berth_engine::MockWorld::bundled(),
            chrono::Utc::now(),
        )),
    });
}

/// The Delete project sheet over shop-api.
fn project_delete(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    open_project(state, "shop-api");
    seed(Tab::Services, Validity::Valid, MemoryFs::new());
    let id = match &*state.selection.peek() {
        Selection::Project(id) => Some(id.clone()),
        _ => None,
    };
    if let Some(id) = id {
        state
            .overlays
            .write()
            .push(crate::state::Overlay::DeleteProject(id));
    }
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Project / files / 1440", project_files),
    Scene::dev("Project / activity / 1440", project_activity),
    Scene::dev("Project / compose invalid / 1440", project_invalid),
    Scene::dev("Project / delete / 1440", project_delete),
    Scene::dev("Project / not created / 1440", project_not_created),
    Scene::dev("Project / services / 1440", project_services),
];
