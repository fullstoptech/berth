#![allow(clippy::expect_used)]
//! The by-hand checks of P4-T1 as live tests:
//!
//! (a) Pull `alpine:3` streams per-layer progress (the image is untagged
//!     first when nothing uses it, so layers really arrive) and the image is
//!     listed afterwards,
//! (b) an image's detail has layers and a platform, and Tag adds a name
//!     that `docker image inspect` then shows,
//! (c) Run on `nginx:alpine` with the name `berthfx-run1`, port 56081 and
//!     the label `dev.berth.fixture=1` typed into the sheet's fields serves
//!     `curl 127.0.0.1:56081`, and Delete on that image shows its reason.
//!
//! Each goes the way the UI goes: the store over a real `BollardEngine`, the
//! Run sheet's draft and spec. Run with `BERTH_IT=1 cargo nextest run -p
//! berth-app --run-ignored only images_it` (live tests share the engine:
//! run them one at a time). Everything this file makes is named
//! `berthfx-run1` or `berthfx/…` and it removes it on every exit path;
//! the only image it untags is `alpine:3`, and only when no container
//! uses it (the pull puts it back).

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::screens::images::{build_rows, delete_reason};
use berth_app::services::engines::client_map;
use berth_app::services::images::{base_spec, ImageEntry, ImageStore, LayerPhase, PullPhase};
use berth_app::services::Services;
use berth_app::sheets::run::RunDraft;
use berth_app::state::{use_app_state, AppState, EngineScope};
use berth_core::EngineId;
use berth_engine::EngineClient;

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, ImageStore)>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    let store = ImageStore::new();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some((state, store)));
    });
    rsx! { div {} }
}

const RUN_NAME: &str = "berthfx-run1";
const RUN_PORT: &str = "56081";
const TAG_REPO: &str = "berthfx/alpine-tag";
const TAG: &str = "1";

fn docker(bin: &std::path::Path, host: &str, args: &[&str]) -> std::process::Output {
    std::process::Command::new(bin)
        .env("DOCKER_HOST", host)
        .args(args)
        .output()
        .expect("docker CLI runs")
}

fn text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

struct Rig {
    dom: VirtualDom,
    store: ImageStore,
    services: Services,
    client: Arc<dyn EngineClient>,
    engine: EngineId,
    state: AppState,
    bin: std::path::PathBuf,
    host: String,
}

impl Rig {
    fn cli(&self, args: &[&str]) -> String {
        text(&docker(&self.bin, &self.host, args))
    }

    /// Drive the dom until `done` says so (`secs` at most).
    async fn until(&mut self, what: &str, secs: u64, mut done: impl FnMut(&VirtualDom) -> bool) {
        let waited = tokio::time::timeout(std::time::Duration::from_secs(secs), async {
            loop {
                if done(&self.dom) {
                    return;
                }
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    self.dom.wait_for_work(),
                )
                .await;
                self.dom.render_immediate(&mut dioxus::core::NoOpMutations);
            }
        })
        .await;
        assert!(waited.is_ok(), "{what} did not happen in {secs} s");
    }
}

/// The engine's image list, asked again while the machine is busy (the
/// engine client's budget is 2 s a request, and a loaded Mac can miss it).
async fn images_of(client: &Arc<dyn EngineClient>) -> Vec<berth_core::Image> {
    let mut last = None;
    for _ in 0..6 {
        match client.images().await {
            Ok(images) => return images,
            Err(err) => {
                last = Some(err);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
    panic!("images: {last:?}");
}

fn clean(bin: &std::path::Path, host: &str) {
    // The run container: by name and only with the fixture label.
    let listed = docker(
        bin,
        host,
        &[
            "ps",
            "-a",
            "-q",
            "--filter",
            "label=dev.berth.fixture=1",
            "--filter",
            &format!("name={RUN_NAME}"),
        ],
    );
    for id in text(&listed).lines() {
        let _ = docker(bin, host, &["rm", "-f", id.trim()]);
    }
    // The extra name Tag made (untagging leaves the image itself).
    let _ = docker(bin, host, &["rmi", &format!("{TAG_REPO}:{TAG}")]);
    let _ = docker(bin, host, &["rmi", &format!("{DELETE_REPO}:1")]);
}

struct CleanOnDrop(std::path::PathBuf, String);

impl Drop for CleanOnDrop {
    fn drop(&mut self) {
        clean(&self.0, &self.1);
    }
}

async fn rig() -> Option<(Rig, CleanOnDrop)> {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return None;
    }
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (mut state, store) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the state");
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search.clone());
    let mut engines = Vec::new();
    for _ in 0..6 {
        engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
        if engines
            .iter()
            .any(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running")
        .clone();
    let host = engine.docker_host.clone().expect("docker host");
    let bin = search
        .find("docker")
        .unwrap_or_else(|| std::path::PathBuf::from("docker"));
    let guard = CleanOnDrop(bin.clone(), host.clone());
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), Arc::clone(&client));
    let services = Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients,
        cancel: CancellationToken::new(),
    };
    dom.in_runtime(|| {
        state.engines.set(engines.clone());
        state.engine_scope.set(EngineScope::One(engine.id.clone()));
    });
    clean(&bin, &host);
    Some((
        Rig {
            dom,
            store,
            services,
            client,
            engine: engine.id.clone(),
            state,
            bin,
            host,
        },
        guard,
    ))
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn images_it_pull_streams_layers_and_the_image_is_listed() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    // `alpine:3` is the fixtures' own image (berthfx-logger-1 runs it), so
    // the layered pull is of `alpine:3.19`: pulled here, untagged after,
    // unless it was there before.
    let probe = "alpine:3.19";
    let was_there = !rig.cli(&["image", "ls", "-q", probe]).is_empty();
    if !was_there {
        // Nothing to remove first: it is absent.
    } else {
        println!("images_it: {probe} was already on the engine; it stays");
    }
    let store = rig.store;
    let services = rig.services.clone();
    let engine = rig.engine.clone();
    if was_there {
        // Present and not ours to remove: pull `alpine:3` instead.
        let started = rig.dom.in_scope(ScopeId::APP, || {
            store.pull(&services, &engine, "alpine:3", None)
        });
        assert_eq!(started, Ok(()));
    } else {
        let started = rig.dom.in_scope(ScopeId::APP, || {
            store.pull(&services, &engine, probe, Some("linux/arm64"))
        });
        assert_eq!(started, Ok(()));
    }
    rig.until("the pull to end", 120, |dom| {
        dom.in_runtime(|| store.pulls.peek().iter().all(|p| !p.is_running()))
    })
    .await;
    let pull = rig.dom.in_runtime(|| store.pulls.peek()[0].clone());
    println!(
        "images_it: {} phase={:?} layers={:?} bytes={:?} note={:?}",
        pull.reference,
        pull.phase,
        pull.layers
            .iter()
            .map(|l| (l.id.as_str(), l.phase))
            .collect::<Vec<_>>(),
        pull.bytes(),
        pull.note
    );
    assert_eq!(pull.phase, PullPhase::Done);
    if !was_there {
        assert!(!pull.layers.is_empty(), "layers streamed: {pull:?}");
        assert!(
            pull.layers.iter().any(|l| l.phase == LayerPhase::Complete),
            "a layer pulled completely: {pull:?}"
        );
        assert!(pull.percent().is_some(), "a download size was reported");
        assert_eq!(pull.platform.as_deref(), Some("linux/arm64"));
    }
    // The refresh after the pull lists it.
    let wanted = pull.reference.clone();
    rig.until("the list to have the image", 20, |dom| {
        dom.in_runtime(|| {
            store
                .by_engine
                .peek()
                .get(&engine)
                .is_some_and(|images| images.iter().any(|i| i.repo_tags.contains(&wanted)))
        })
    })
    .await;
    // A pulled-here probe goes again: only the tag this test made.
    if !was_there {
        let _ = rig.cli(&["rmi", probe]);
        assert!(rig.cli(&["image", "ls", "-q", probe]).is_empty());
    }
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn images_it_detail_has_layers_and_tag_adds_a_name() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    if rig.cli(&["image", "ls", "-q", "alpine:3"]).is_empty() {
        let _ = rig.cli(&["pull", "alpine:3"]);
    }
    let image = images_of(&rig.client)
        .await
        .into_iter()
        .find(|i| i.repo_tags.iter().any(|t| t == "alpine:3"))
        .expect("alpine:3 is on the engine");
    let store = rig.store;
    let services = rig.services.clone();
    let engine = rig.engine.clone();
    let id = image.id.clone();
    rig.dom.in_scope(ScopeId::APP, || {
        store.ensure_detail(&services, &engine, &id)
    });
    let key = (engine.clone(), id.clone());
    rig.until("the detail", 20, |dom| {
        dom.in_runtime(|| store.details.peek().contains_key(&key))
    })
    .await;
    let entry = rig
        .dom
        .in_runtime(|| store.details.peek().get(&key).cloned());
    let Some(ImageEntry::Ready(detail)) = entry else {
        panic!("a loaded detail, got {entry:?}");
    };
    println!(
        "images_it: layers={} platform={}/{}",
        detail.layers.len(),
        detail.image.os,
        detail.image.arch.docker_name()
    );
    assert!(!detail.layers.is_empty());
    assert_eq!(detail.image.os, "linux");

    // Tag adds a name; docker sees it.
    let heard = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = heard.clone();
    rig.dom.in_scope(ScopeId::APP, || {
        store.tag(&services, &engine, &image, TAG_REPO, TAG, move |r| {
            *sink.borrow_mut() = Some(r);
        });
    });
    rig.until("the tag result", 20, |_| heard.borrow().is_some())
        .await;
    assert_eq!(*heard.borrow(), Some(Ok(())));
    let shown = rig.cli(&[
        "image",
        "inspect",
        "-f",
        "{{json .RepoTags}}",
        &format!("{TAG_REPO}:{TAG}"),
    ]);
    println!("images_it: RepoTags={shown}");
    assert!(shown.contains(&format!("{TAG_REPO}:{TAG}")), "{shown}");
    assert!(shown.contains("alpine:3"), "{shown}");
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn images_it_run_nginx_alpine_serves_curl_and_delete_says_why_not() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    if rig.cli(&["image", "ls", "-q", "nginx:alpine"]).is_empty() {
        let _ = rig.cli(&["pull", "nginx:alpine"]);
    }
    let engine = rig.engine.clone();
    let store = rig.store;
    let services = rig.services.clone();
    let image = images_of(&rig.client)
        .await
        .into_iter()
        .find(|i| i.repo_tags.iter().any(|t| t == "nginx:alpine"))
        .expect("nginx:alpine is on the engine");
    let id = image.id.clone();
    rig.dom.in_scope(ScopeId::APP, || {
        store.ensure_detail(&services, &engine, &id)
    });
    let key = (engine.clone(), id.clone());
    rig.until("the detail", 20, |dom| {
        dom.in_runtime(|| store.details.peek().contains_key(&key))
    })
    .await;
    let Some(ImageEntry::Ready(detail)) = rig
        .dom
        .in_runtime(|| store.details.peek().get(&key).cloned())
    else {
        panic!("a loaded detail");
    };

    // What the sheet starts from, then what the user types: the name, the
    // port and the fixture label.
    let containers = rig.client.containers().await.expect("containers");
    let taken: Vec<&str> = containers.iter().map(|c| c.name.as_str()).collect();
    let mut draft = RunDraft::prefill(&image, Some(&detail), &taken);
    println!(
        "images_it: prefill name={} ports={:?}",
        draft.name,
        draft
            .ports
            .iter()
            .map(|(_, p)| p.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        draft.ports.iter().any(|(_, p)| p == "80"),
        "{:?}",
        draft.ports
    );
    draft.name = RUN_NAME.to_string();
    let port_row = draft
        .ports
        .iter()
        .find(|(_, p)| p == "80")
        .map(|(id, _)| *id);
    draft.set_port(port_row.expect("port 80 row"), &format!("{RUN_PORT}:80"));
    draft.add_label();
    let label_row = draft.labels[0].0;
    draft.set_label(label_row, "dev.berth.fixture=1");
    let base = base_spec(&image, &draft.name);
    let spec = draft.to_spec(&base, &taken).expect("a valid sheet");

    let heard = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = heard.clone();
    rig.dom.in_scope(ScopeId::APP, || {
        store.run(&services, &engine, spec, move |r| {
            *sink.borrow_mut() = Some(r);
        });
    });
    rig.until("the run result", 60, |_| heard.borrow().is_some())
        .await;
    let created = heard.borrow().clone().expect("a result");
    println!("images_it: run -> {created:?}");
    assert!(created.is_ok(), "{created:?}");

    // curl 127.0.0.1:56081 answers with nginx's page.
    let mut body = String::new();
    for _ in 0..20 {
        let out = std::process::Command::new("curl")
            .args([
                "-s",
                "--max-time",
                "2",
                &format!("http://127.0.0.1:{RUN_PORT}"),
            ])
            .output()
            .expect("curl runs");
        body = String::from_utf8_lossy(&out.stdout).into_owned();
        if body.contains("nginx") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    println!(
        "images_it: curl -> {}",
        body.lines().next().unwrap_or_default()
    );
    assert!(body.contains("nginx"), "{body}");
    assert_eq!(
        rig.cli(&[
            "inspect",
            "-f",
            "{{index .Config.Labels \"dev.berth.fixture\"}}",
            RUN_NAME
        ]),
        "1"
    );

    // The image is in use now: its row says so, and Delete shows why.
    let fresh = images_of(&rig.client).await;
    let by_engine = std::collections::BTreeMap::from([(engine.clone(), fresh)]);
    let rows = {
        let state = rig.state;
        rig.dom.in_runtime(|| {
            build_rows(
                &state.engines.peek(),
                &by_engine,
                &EngineScope::One(engine.clone()),
            )
        })
    };
    let row = rows
        .iter()
        .find(|r| r.name == "nginx:alpine")
        .expect("the nginx:alpine row");
    let reason = delete_reason(row).expect("an image in use has a reason");
    println!("images_it: delete reason = {reason}");
    assert!(reason.starts_with("Used by "), "{reason}");
    assert!(reason.contains("container"), "{reason}");
}

const DELETE_REPO: &str = "berthfx/delete-me";

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn images_it_delete_removes_a_fixture_image_and_refuses_one_a_container_uses() {
    let Some((rig, _guard)) = rig().await else {
        return;
    };
    // A throwaway image of our own: `FROM alpine:3` with the fixture label.
    if rig.cli(&["image", "ls", "-q", "alpine:3"]).is_empty() {
        let _ = rig.cli(&["pull", "alpine:3"]);
    }
    let tag = format!("{DELETE_REPO}:1");
    let mut build = std::process::Command::new(&rig.bin)
        .env("DOCKER_HOST", &rig.host)
        .args([
            "build",
            "-q",
            "--label",
            "dev.berth.fixture=1",
            "-t",
            &tag,
            "-",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("docker build starts");
    {
        use std::io::Write as _;
        let mut stdin = build.stdin.take().expect("stdin");
        stdin
            .write_all(b"FROM alpine:3\nLABEL dev.berth.fixture=1\nRUN echo berthfx > /marker\n")
            .expect("dockerfile");
    }
    assert!(build.wait().expect("build ends").success());
    let images = images_of(&rig.client).await;
    let mut state = rig.state;
    rig.dom.in_runtime(|| state.images.set(images.clone()));
    let mine = images
        .iter()
        .find(|i| i.repo_tags.contains(&tag))
        .expect("the image just built")
        .clone();
    assert!(mine.used_by.is_empty());
    assert_eq!(
        rig.cli(&[
            "image",
            "inspect",
            "-f",
            "{{index .Config.Labels \"dev.berth.fixture\"}}",
            &tag
        ]),
        "1",
        "refusing to delete an image without the fixture label"
    );

    // The dialog's model, then what Confirm does with it.
    let services = rig.services.clone();
    let engine = rig.engine.clone();
    let model = berth_app::services::confirm::delete_image_model(&engine, &mine);
    assert_eq!(model.title, format!("Delete image {tag}?"));
    let (actions, mut finished) = berth_app::services::actions::ActionsService::new(
        services.clients.clone(),
        services.store.clone(),
        Arc::new(berth_engine::MockCompose::bundled()),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    );
    let mut state_copy = state;
    let launched = rig
        .dom
        .in_runtime(|| berth_app::services::confirm::run(&mut state_copy, &actions, &model, false));
    assert!(launched.refused.is_empty(), "{:?}", launched.refused);
    let done = tokio::time::timeout(std::time::Duration::from_secs(30), finished.recv())
        .await
        .expect("the delete finishes")
        .expect("a result");
    rig.dom
        .in_runtime(|| berth_app::services::actions::apply_finished(&mut state_copy, done));
    println!(
        "images_it: after delete, image ls = {:?}",
        rig.cli(&["image", "ls", "-q", &tag])
    );
    assert!(
        rig.cli(&["image", "ls", "-q", &tag]).is_empty(),
        "the image is gone"
    );

    // An image a container uses is refused before the engine is asked.
    let running = images
        .iter()
        .find(|i| !i.used_by.is_empty() && i.repo_tags.iter().any(|t| t == "nginx:alpine"))
        .expect("nginx:alpine is used by berthfx-web-1")
        .clone();
    let refused = rig.dom.in_runtime(|| {
        actions.remove(
            &mut state_copy,
            &engine,
            &berth_core::ResourceRef::Image(running.id.clone()),
            "nginx:alpine",
            false,
        )
    });
    println!("images_it: delete of nginx:alpine -> {refused:?}");
    assert!(
        refused.as_ref().is_err_and(|r| r.starts_with("Used by ")),
        "{refused:?}"
    );
    assert!(!rig.cli(&["image", "ls", "-q", "nginx:alpine"]).is_empty());
}
