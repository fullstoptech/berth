#![allow(clippy::expect_used)]
//! P9-T5 live-ish check, no Docker and no real Coolify: the real
//! `berth-coolify` binary, started through `PluginsService` from a directory
//! laid out like the app bundle (the binary and its manifest next to the app
//! binary), answering a wiremock Coolify that serves the checked-in
//! fixtures. It loads the Resources and Deployments views, reads Deploy's
//! confirm, follows the failed deployment's log to its failing step and asks
//! for project badges.
//!
//! The same run is the recorder of the views the scenes draw: the answers are
//! compared with `fixtures/plugin_views/coolify/*.json`, and
//! `BERTH_RECORD_PLUGIN_VIEWS=1` rewrites those files from the real plugin.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use berth_app::screens::plugin_view_model::{confirm_view, failing_seq, log_rows};
use berth_app::services::plugin_views::{
    action_params, badge_requests, follow_log, load_badges, load_view, palette_commands,
    sidebar_groups, CommandKind, LogState, ViewState,
};
use berth_app::services::plugins::{plugin_dirs_from, ConfigUpdate, PluginsDeps, PluginsService};
use berth_plugin_host::{Backoff, PluginState};
use berth_plugin_protocol::{Cell, View};
use berth_store::{InMemorySecretStore, Store};
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{bearer_token, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "view-test-token-not-real";
const MANIFEST: &str = include_str!("../../../plugins/coolify/berth-plugin.json");
const FAILED: &str = "dep-uuid-00ff";

const RESOURCES: &str = include_str!("../../../fixtures/coolify/resources.json");
const SERVERS: &str = include_str!("../../../fixtures/coolify/servers.json");
const APPLICATION: &str = include_str!("../../../fixtures/coolify/application.json");
/// `GET /deployments`: what is running now, as the real API answers it.
const RUNNING: &str = include_str!("../../../fixtures/coolify/scene-running-deployments.json");
/// `GET /deployments/applications/{uuid}`: each application's own history
/// (the scene's twelve deployments, split by application).
const HISTORY: [(&str, &str); 4] = [
    (
        "app-uuid-0001",
        include_str!("../../../fixtures/coolify/scene-app-deployments-app-uuid-0001.json"),
    ),
    (
        "app-uuid-0012",
        include_str!("../../../fixtures/coolify/scene-app-deployments-app-uuid-0012.json"),
    ),
    (
        "app-uuid-0013",
        include_str!("../../../fixtures/coolify/scene-app-deployments-app-uuid-0013.json"),
    ),
    (
        "app-uuid-0014",
        include_str!("../../../fixtures/coolify/scene-app-deployments-app-uuid-0014.json"),
    ),
];
const APPLICATIONS: &str = include_str!("../../../fixtures/coolify/scene-applications.json");
const FAILED_DEPLOYMENT: &str =
    include_str!("../../../fixtures/coolify/scene-deployment-landing-failed.json");

fn recorded_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/plugin_views/coolify")
}

/// The newest modification time under `dir` (source files only).
fn newest_source(dir: &Path) -> Option<std::time::SystemTime> {
    let mut newest = None;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                newest = newest.max(Some(modified));
            }
        }
    }
    newest
}

/// `target/<profile>/berth-coolify`, built first when nothing built it yet or
/// when the plugin's sources are newer than it (`cargo nextest run
/// --workspace` builds it for berth-coolify's own tests; `-p berth-app` alone
/// does not, and a stale binary would test the old plugin).
fn plugin_binary() -> PathBuf {
    let profile_dir = std::env::current_exe()
        .expect("test binary path")
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("target profile dir");
    let binary = profile_dir.join("berth-coolify");
    let sources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/coolify/src");
    let built = binary.metadata().and_then(|m| m.modified()).ok();
    if built.is_none() || newest_source(&sources) > built {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let status = std::process::Command::new(cargo)
            .args(["build", "--quiet", "-p", "berth-coolify"])
            .status()
            .expect("cargo runs");
        assert!(status.success(), "cargo build -p berth-coolify failed");
    }
    assert!(binary.exists(), "{} was not built", binary.display());
    binary
}

fn json_response(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_string(body.to_owned())
}

async fn get(server: &MockServer, route: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/{route}")))
        .and(bearer_token(TOKEN))
        .respond_with(json_response(body))
        .mount(server)
        .await;
}

/// A Coolify that answers like the real API: `/deployments` lists only the
/// deployments running now, the history is per application. The scene's
/// twelve deployments, the four applications and landing's failed log.
async fn coolify() -> MockServer {
    coolify_with_history(&HISTORY).await
}

async fn coolify_with_history(history: &[(&str, &str)]) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/version"))
        .respond_with(ResponseTemplate::new(200).set_body_string("v4.5.0"))
        .mount(&server)
        .await;
    get(&server, "resources", RESOURCES).await;
    get(&server, "servers", SERVERS).await;
    get(&server, "deployments", RUNNING).await;
    get(&server, "applications", APPLICATIONS).await;
    get(&server, "applications/app-uuid-0001", APPLICATION).await;
    for (uuid, body) in history {
        get(&server, &format!("deployments/applications/{uuid}"), body).await;
    }
    get(&server, &format!("deployments/{FAILED}"), FAILED_DEPLOYMENT).await;
    server
}

/// The plugin installed like the bundle: `Contents/MacOS/{berth,
/// berth-coolify, berth-plugin.json}`, found through `plugin_dirs_from` with
/// the app binary's directory, enabled and configured through the service.
struct Rig {
    _root: tempfile::TempDir,
    _server: MockServer,
    service: PluginsService,
    app_dir: PathBuf,
}

impl Rig {
    async fn start() -> Self {
        Self::start_on(coolify().await).await
    }

    async fn start_on(server: MockServer) -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let app_dir = root.path().join("Berth.app/Contents/MacOS");
        std::fs::create_dir_all(&app_dir).expect("bundle dir");
        std::fs::write(app_dir.join("berth"), "").expect("stand-in app binary");
        std::fs::copy(plugin_binary(), app_dir.join("berth-coolify")).expect("copy plugin");
        std::fs::write(app_dir.join("berth-plugin.json"), MANIFEST).expect("manifest");

        let service = PluginsService::new(PluginsDeps {
            store: Arc::new(Store::open_in_memory().expect("store")),
            secrets: Arc::new(InMemorySecretStore::default()),
            dirs: plugin_dirs_from(None, &root.path().join("data"), Some(&app_dir)),
            path: Some(std::env::var_os("PATH").unwrap_or_default()),
            cancel: CancellationToken::new(),
            backoff: Backoff {
                delays: vec![Duration::from_millis(50)],
            },
        });
        service.scan().await;
        service.enable("coolify").await.expect("enable");
        let mut update = ConfigUpdate::default();
        update.values.insert("base_url".into(), json!(server.uri()));
        update.secrets.insert(
            "coolify/token".into(),
            SecretString::from(TOKEN.to_string()),
        );
        service
            .configure("coolify", update)
            .await
            .expect("configure");
        Self {
            _root: root,
            _server: server,
            service,
            app_dir,
        }
    }

    async fn view(&self, view: &str) -> View {
        match load_view(&self.service, "coolify", view).await {
            ViewState::Ready(view) => view,
            other => panic!("{view}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn coolify_views_the_bundled_plugin_is_found_next_to_the_app_binary() {
    let rig = Rig::start().await;
    let entry = rig.service.entry("coolify").expect("coolify is discovered");
    assert_eq!(entry.plugin.dir, rig.app_dir, "found beside the app binary");
    assert_eq!(entry.plugin.state, PluginState::Enabled);
    // The sidebar section and the palette commands come from the manifest.
    let groups = sidebar_groups(&rig.service.entries());
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].title, "Coolify");
    let titles: Vec<&str> = groups[0].items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["Resources", "Servers", "Deployments"]);
    let commands = palette_commands(&rig.service.entries());
    assert!(commands.iter().any(|c| c.title == "Coolify: Deployments"
        && c.kind
            == CommandKind::Open {
                view: "deployments".to_string()
            }));
    rig.service.shutdown().await;
}

#[tokio::test]
async fn coolify_views_resources_and_deployments_load_through_the_service() {
    let rig = Rig::start().await;
    let View::Table { rows, .. } = rig.view("resources").await else {
        panic!("resources is a table");
    };
    assert_eq!(rows.len(), 3);
    let View::Table { columns, rows, .. } = rig.view("deployments").await else {
        panic!("deployments is a table");
    };
    // Only one of the twelve is running; the other eleven are history read
    // from each application, newest first.
    assert_eq!(rows.len(), 12);
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids[..3], ["dep-uuid-0001", FAILED, "dep-uuid-0101"]);
    assert_eq!(
        ids.iter().filter(|id| **id == "dep-uuid-0001").count(),
        1,
        "a running deployment that is also in its application's history is one row"
    );
    assert_eq!(columns[0].label, "App");
    let failed = rows
        .iter()
        .find(|r| r.id == FAILED)
        .expect("the failed deployment is listed");
    assert_eq!(
        failed.open.as_deref(),
        Some("deployment/dep-uuid-00ff/logs")
    );
    assert!(
        matches!(&failed.cells["status"], Cell::Badge(b) if b.text == "Failed"),
        "{:?}",
        failed.cells["status"]
    );
    // The failed row's buttons: Redeploy\u{2026} with the frame's confirm, and Logs.
    let labels: Vec<&str> = failed.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, ["Redeploy\u{2026}", "Logs"]);
    let confirm = confirm_view(
        failed.actions[0]
            .confirm
            .as_ref()
            .expect("Redeploy confirms"),
    );
    assert_eq!(confirm.title, "Redeploy landing?");
    assert_eq!(confirm.button, "Redeploy anyway");
    let shown: BTreeMap<&str, &str> = confirm
        .rows
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(shown["App"], "landing");
    assert_eq!(shown["Server"], "hetzner-1");
    assert_eq!(shown["Branch"], "main");
    assert_eq!(shown["Commit"], "c2a7e94 \u{b7} fix caddy upstream");
    assert_eq!(
        confirm.warnings,
        ["This commit failed (Caddyfile:14: unknown directive 'reverse_proxy_x'). Redeploying it will likely fail again."]
    );
    assert_eq!(
        failed.cells["repository"],
        Cell::Text("example/landing".to_string())
    );
    // A finished deployment offers the same buttons without the warning.
    let finished = rows
        .iter()
        .find(|r| r.id == "dep-uuid-0105")
        .expect("a finished row");
    let finished_confirm = confirm_view(
        finished.actions[0]
            .confirm
            .as_ref()
            .expect("Redeploy confirms"),
    );
    assert_eq!(finished_confirm.button, "Redeploy");
    assert!(finished_confirm.warnings.is_empty(), "{finished_confirm:?}");
    rig.service.shutdown().await;
}

#[tokio::test]
async fn coolify_views_one_applications_failing_history_does_not_blank_the_deployments() {
    // berth-site's history is not answered (404); the other three applications and the
    // running list still show.
    let rig = Rig::start_on(
        coolify_with_history(&HISTORY[..3]).await, // no mock for app-uuid-0014: a 404
    )
    .await;
    let View::Table { rows, .. } = rig.view("deployments").await else {
        panic!("deployments is a table");
    };
    assert_eq!(rows.len(), 9, "twelve less berth-site's three");
    assert!(rows.iter().all(|r| r.id != "dep-uuid-0102"));
    assert!(rows.iter().any(|r| r.id == FAILED));
    rig.service.shutdown().await;
}

#[tokio::test]
async fn coolify_views_an_applications_deployments_tab_lists_its_history_with_actions() {
    // The Resources inspector's Deployments tab is the per-application view.
    let rig = Rig::start().await;
    let View::Table { rows, .. } = rig.view("deployments/app-uuid-0001").await else {
        panic!("an application's deployments is a table");
    };
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, [FAILED, "dep-uuid-0105", "dep-uuid-0109"]);
    let failed = &rows[0];
    let labels: Vec<&str> = failed.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, ["Redeploy\u{2026}", "Logs"]);
    assert_eq!(
        failed.open.as_deref(),
        Some("deployment/dep-uuid-00ff/logs")
    );
    rig.service.shutdown().await;
}

#[tokio::test]
async fn coolify_views_deploy_confirm_names_app_server_and_branch() {
    let rig = Rig::start().await;
    let View::Detail { actions, .. } = rig.view("application/app-uuid-0001").await else {
        panic!("an application is a detail");
    };
    let deploy = actions.iter().find(|a| a.id == "deploy").expect("Deploy");
    let confirm = deploy.confirm.as_ref().expect("Deploy confirms");
    let shown = confirm_view(confirm);
    let rows: BTreeMap<&str, &str> = shown
        .rows
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(rows["App"], "landing");
    assert_eq!(rows["Server"], "hetzner-1");
    assert_eq!(rows["Branch"], "main");
    assert_eq!(shown.button, "Deploy");
    // Ticking Force rebuild is what reaches the plugin.
    let params = action_params(deploy, &BTreeMap::from([("force".to_string(), true)]));
    assert_eq!(params["force"], true);
    assert_eq!(params["uuid"], "app-uuid-0001");
    rig.service.shutdown().await;
}

#[tokio::test]
async fn coolify_views_failed_deployment_log_ends_at_the_failing_step() {
    let rig = Rig::start().await;
    let View::LogStream { stream, .. } = rig.view("deployment/dep-uuid-00ff/logs").await else {
        panic!("a deployment's log is a log stream");
    };
    let follow = follow_log(&rig.service, "coolify", &stream);
    let mut state = follow.state.clone();
    let log: LogState = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if state.borrow().final_status.is_some() {
                return state.borrow().clone();
            }
            state.changed().await.expect("the follow is alive");
        }
    })
    .await
    .expect("the stream ends");
    assert_eq!(log.final_status.as_deref(), Some("failed"));
    assert_eq!(log.lines.len(), 10);
    let rows = log_rows(&log);
    let seq = failing_seq(&rows, &log).expect("a failed log has a failing step");
    let failing = rows.iter().find(|r| r.row.seq == seq).expect("row");
    assert!(
        failing.row.text.contains("Caddyfile:14: unknown directive"),
        "{}",
        failing.row.text
    );
    follow.close();
    rig.service.shutdown().await;
}

#[tokio::test]
async fn coolify_views_a_project_with_the_apps_remote_gets_its_badge() {
    let rig = Rig::start().await;
    let project = berth_core::Project {
        id: berth_core::ProjectId("landing-folder".into()),
        name: "landing".into(),
        folder: Some(PathBuf::from("/Users/dev/landing")),
        compose_files: Vec::new(),
        engine: berth_core::EngineId("colima:default".into()),
        source: berth_core::ProjectSource::Compose,
        services: Vec::new(),
        git: Some(berth_core::GitInfo {
            branch: Some("main".into()),
            dirty: false,
            remote: Some("git@github.com:example/landing.git".into()),
            ahead: 0,
            behind: 0,
        }),
        status: berth_core::ProjectStatus::AllRunning,
        pinned: false,
    };
    let requests = badge_requests(&[project]);
    assert_eq!(requests.len(), 1);
    let badges = load_badges(&rig.service, &requests).await;
    let badge = badges.get("landing-folder").expect("badge for the project");
    assert_eq!(badge.text, "Deployed on Coolify: running");
    rig.service.shutdown().await;
}

/// The views the scenes draw are what the real plugin answers. With
/// `BERTH_RECORD_PLUGIN_VIEWS=1` the files are rewritten instead.
#[tokio::test]
async fn coolify_views_recorded_scene_views_match_the_real_plugin() {
    let rig = Rig::start().await;
    let record = std::env::var_os("BERTH_RECORD_PLUGIN_VIEWS").is_some();
    let dir = recorded_dir();
    let check = |name: &str, value: Value| {
        let file = dir.join(format!("{name}.json"));
        let text = serde_json::to_string_pretty(&value).expect("json") + "\n";
        if record {
            std::fs::create_dir_all(&dir).expect("recorded dir");
            std::fs::write(&file, text).expect("write recording");
        } else {
            let saved = std::fs::read_to_string(&file).unwrap_or_else(|e| {
                panic!(
                    "{}: {e} (BERTH_RECORD_PLUGIN_VIEWS=1 writes it)",
                    file.display()
                )
            });
            assert_eq!(saved, text, "{name} drifted from the plugin's answer");
        }
    };
    for (name, view) in [
        ("resources", "resources"),
        ("deployments", "deployments"),
        ("application", "application/app-uuid-0001"),
        ("deployment-log-view", "deployment/dep-uuid-00ff/logs"),
        ("status", "status"),
    ] {
        let value = serde_json::to_value(rig.view(view).await).expect("view json");
        check(name, value);
    }
    // The lines the plugin streams for the failed deployment.
    let follow = follow_log(&rig.service, "coolify", "deployment/dep-uuid-00ff/logs");
    let mut state = follow.state.clone();
    let log = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if state.borrow().final_status.is_some() {
                return state.borrow().clone();
            }
            state.changed().await.expect("the follow is alive");
        }
    })
    .await
    .expect("the stream ends");
    let lines: Vec<&str> = log.lines.iter().map(|(_, l)| l.as_str()).collect();
    check(
        "deployment-log-lines",
        json!({ "lines": lines, "final_status": log.final_status }),
    );
    follow.close();
    rig.service.shutdown().await;
}
