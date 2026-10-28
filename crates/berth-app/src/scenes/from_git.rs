//! Scenes of Project from Git (P8-T5, no frame): the From Git sheet in its four states and the
//! Git panel of a project with two new commits. The data is made up; nothing runs git.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::screens::project_detail::{ProjectDetailSeed, Tab, Validity};
use crate::services::git_deploy::{
    parse_repo_url, CloneFailure, CloneState, FailureKind, GitDeployView, GitProject, GitStatus,
};
use crate::services::project_files::{Editor, MemoryFs, ProjectFsOverride};
use crate::sheets::from_git::SeedGit;
use crate::sheets::new_project::{SeedStage, Stage};
use crate::state::Overlay;
use berth_engine::git::IncomingCommit;

#[allow(clippy::expect_used)] // a static scene address, written below
fn repo() -> url::Url {
    parse_repo_url("git@github.com:acme/shop-api.git").expect("scene address parses")
}

fn open_sheet(state: &mut AppState, theme: berth_store::Theme) {
    super::projects::load_projects_world(state, theme);
    let home = berth_sys::paths::home();
    provide_context(SeedStage(Stage::Git));
    provide_context(SeedGit {
        url: "git@github.com:acme/shop-api.git".to_string(),
        branch: "main".to_string(),
        key: Some(home.join(".ssh/id_ed25519")),
        keys: vec![home.join(".ssh/id_ed25519"), home.join(".ssh/id_work")],
        parent: Some(home.join("Dev")),
    });
    state.overlays.write().push(Overlay::NewProject);
}

fn set_clone(state: CloneState) {
    let mut view = consume_context::<GitDeployView>();
    view.cloning.set(state);
}

/// "New project / from git / 1440": the form, filled in.
fn form(state: &mut AppState) {
    open_sheet(state, berth_store::Theme::Light);
}

/// Cloning: git is receiving objects.
fn cloning(state: &mut AppState) {
    open_sheet(state, berth_store::Theme::Light);
    set_clone(CloneState::Cloning {
        url: repo().to_string(),
        phase: "Receiving objects".to_string(),
        percent: Some(64),
    });
}

/// The server did not accept the key.
fn failed_auth(state: &mut AppState) {
    open_sheet(state, berth_store::Theme::Light);
    set_clone(CloneState::Failed(CloneFailure {
        kind: FailureKind::Auth,
        message: "git@github.com: Permission denied (publickey).".to_string(),
        fix: "The server did not accept your key. Run ssh-add for it so your agent offers it, or choose the key in the list.".to_string(),
    }));
}

/// The address finds no repository.
fn failed_url(state: &mut AppState) {
    open_sheet(state, berth_store::Theme::Dark);
    set_clone(CloneState::Failed(CloneFailure {
        kind: FailureKind::Url,
        message: "fatal: repository 'https://github.com/acme/shop-apy.git/' not found".to_string(),
        fix: "Check the address and the branch name. For a private repository, make sure your key or login can see it.".to_string(),
    }));
}

/// The clone worked and Compose will not load the file.
fn invalid_compose(state: &mut AppState) {
    open_sheet(state, berth_store::Theme::Light);
    set_clone(CloneState::InvalidCompose {
        folder: berth_sys::paths::home().join("Dev/shop-api"),
        message: "compose.yaml: services.api.ports must be a list".to_string(),
    });
}

/// shop-api on its Services tab, cloned from Git, with two commits waiting upstream.
fn project_git(state: &mut AppState) {
    seed_git_project(state, true);
}

/// The Projects list with shop-api cloned from Git: its row shows the commit and "2 new commits".
fn projects_git(state: &mut AppState) {
    seed_git_project(state, false);
}

fn seed_git_project(state: &mut AppState, detail: bool) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    let found = state
        .projects
        .peek()
        .iter()
        .find(|p| p.name == "shop-api")
        .map(|p| (p.id.clone(), p.folder.clone(), p.engine.clone()));
    let Some((id, Some(folder), engine)) = found else {
        return;
    };
    if detail {
        state.selection.set(Selection::Project(id));
        state.section.set(Section::Project);
    }
    provide_context(ProjectFsOverride(Arc::new(MemoryFs::new())));
    provide_context(ProjectDetailSeed {
        tab: Tab::Services,
        validity: Some(Validity::Valid),
        editor: Some(Editor {
            program: Some(PathBuf::from("/usr/local/bin/code")),
            label: "Visual Studio Code (detected)".to_string(),
        }),
        activity: None,
    });
    let mut view = consume_context::<GitDeployView>();
    let mut status = GitStatus::new(GitProject {
        folder: folder.clone(),
        engine,
        repo: repo(),
        branch: "main".to_string(),
        commit: Some("d544a3d".to_string()),
        poll: Some(Duration::from_secs(15 * 60)),
        auto_redeploy: false,
    });
    status.incoming = vec![
        IncomingCommit {
            hash: "a41c9f0".to_string(),
            subject: "api: add the health endpoint".to_string(),
        },
        IncomingCommit {
            hash: "7be20d1".to_string(),
            subject: "compose: pin redis to 7.4".to_string(),
        },
    ];
    view.projects.write().insert(folder, status);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("New project / from git / 1440", form),
    Scene::dev("New project / from git / clone failed / 1440", failed_auth),
    Scene::dev("New project / from git / cloning / 1440", cloning),
    Scene::dev("New project / from git / invalid compose / 1440", invalid_compose),
    Scene::dev("New project / from git / wrong address / 1440", failed_url),
    Scene::dev("Project / git / 1440", project_git),
    Scene::dev("Projects / git / 1440", projects_git),
];
