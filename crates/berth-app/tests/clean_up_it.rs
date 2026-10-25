#![allow(clippy::expect_used)]
//! P4-T7's live check, as a test: two stopped containers and one volume,
//! all labelled `dev.berth.fixture=1`, are cleaned up through the real
//! plan, button text, run and result text.
//!
//! The engine also holds the owner's own containers, images and volumes, so
//! this test never runs a plan it did not narrow: the plan is built from the
//! live lists as the sheet builds it, then cut to exactly the three
//! resources the test created, and [`narrow`] refuses (the test fails before
//! any removal) unless every remaining item is a `berthfx` resource that
//! carries the fixture label on the engine. An unnarrowed plan is never
//! passed to [`prune::run`].
//!
//! Run with `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only
//! clean_up_it` while Colima is up. `alpine:3` must already be on the
//! engine (the test never pulls).

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use berth_app::services::prune::{self, Lists};
use berth_core::{
    Container, ContainerAction, ContainerSpec, ImageRef, Limits, PruneItem, PruneKind, PrunePlan,
    RestartPolicy, Volume, VolumeSpec,
};
use berth_engine::EngineClient;

const NAMES: [&str; 2] = ["berthfx-clean-a", "berthfx-clean-b"];
const VOLUME: &str = "berthfx-clean-vol";
const FIXTURE_LABEL: &str = "dev.berth.fixture";

fn fixture_labels() -> BTreeMap<String, String> {
    [(FIXTURE_LABEL.to_string(), "1".to_string())]
        .into_iter()
        .collect()
}

fn labelled(labels: &BTreeMap<String, String>) -> bool {
    labels.get(FIXTURE_LABEL).map(String::as_str) == Some("1")
}

/// Is `item` a container or volume that is named `berthfx…` and carries the
/// fixture label on the engine? Everything else (images, networks, build
/// cache, resources the engine does not list) is not a fixture.
fn is_fixture(item: &PruneItem, containers: &[Container], volumes: &[Volume]) -> bool {
    match item.kind {
        PruneKind::Container => containers
            .iter()
            .any(|c| c.id.0 == item.id && c.name.starts_with("berthfx") && labelled(&c.labels)),
        PruneKind::Volume => volumes
            .iter()
            .any(|v| v.name == item.id && v.name.starts_with("berthfx") && labelled(&v.labels)),
        PruneKind::Image | PruneKind::Network | PruneKind::BuildCache => false,
    }
}

/// Cut `plan` to the items whose ids are in `allowed` and refuse unless
/// every one of them is a labelled fixture and none is missing.
fn narrow(
    plan: &PrunePlan,
    allowed: &[String],
    containers: &[Container],
    volumes: &[Volume],
) -> Result<PrunePlan, String> {
    let items: Vec<PruneItem> = plan
        .items
        .iter()
        .filter(|i| allowed.contains(&i.id))
        .cloned()
        .map(|mut i| {
            i.selected = true;
            i
        })
        .collect();
    if items.len() != allowed.len() {
        return Err(format!(
            "the plan holds {} of the {} fixtures",
            items.len(),
            allowed.len()
        ));
    }
    if let Some(stray) = items.iter().find(|i| !is_fixture(i, containers, volumes)) {
        return Err(format!(
            "refusing to run: {} ({:?}) is not a labelled berthfx fixture",
            stray.name, stray.kind
        ));
    }
    Ok(PrunePlan {
        engine: plan.engine.clone(),
        items,
    })
}

fn container(id: &str, name: &str, labels: BTreeMap<String, String>) -> Container {
    let world = berth_engine::MockWorld::bundled();
    let mut c = world.containers[0].clone();
    c.id = berth_core::ContainerId(id.to_string());
    c.name = name.to_string();
    c.labels = labels;
    c
}

fn item(kind: PruneKind, id: &str, name: &str) -> PruneItem {
    PruneItem {
        kind,
        id: id.to_string(),
        name: name.to_string(),
        size: None,
        selected: true,
    }
}

fn plan_of(items: Vec<PruneItem>) -> PrunePlan {
    PrunePlan {
        engine: berth_core::EngineId("colima:default".to_string()),
        items,
    }
}

/// Runs offline: the guard that keeps the live run to the fixtures.
#[test]
fn clean_up_it_guard_refuses_anything_without_the_fixture_label() {
    let ours = container("c1", "berthfx-clean-a", fixture_labels());
    // The owner's container has the same shape, but no label.
    let theirs = container("c2", "server-db-1", BTreeMap::new());
    // A berthfx name alone is not enough either.
    let unlabelled = container("c3", "berthfx-clean-b", BTreeMap::new());
    let containers = vec![ours, theirs, unlabelled];
    let plan = plan_of(vec![
        item(PruneKind::Container, "c1", "berthfx-clean-a"),
        item(PruneKind::Container, "c2", "server-db-1"),
        item(PruneKind::Container, "c3", "berthfx-clean-b"),
        item(PruneKind::Image, "sha256:abc", "alpine:3"),
        item(PruneKind::BuildCache, "build-cache", "Build cache"),
    ]);

    let only_ours = narrow(&plan, &["c1".to_string()], &containers, &[]).expect("fixture passes");
    assert_eq!(only_ours.items.len(), 1);

    for stray in ["c2", "c3", "sha256:abc", "build-cache"] {
        let allowed = vec!["c1".to_string(), stray.to_string()];
        let refused = narrow(&plan, &allowed, &containers, &[]).expect_err(stray);
        assert!(refused.contains("refusing to run"), "{stray}: {refused}");
    }
    // A fixture the plan does not hold is an error, not a smaller plan.
    let missing = narrow(
        &plan,
        &["c1".to_string(), "gone".to_string()],
        &containers,
        &[],
    )
    .expect_err("missing");
    assert!(missing.contains("1 of the 2"), "{missing}");
}

async fn connect() -> Arc<dyn EngineClient> {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima = berth_engine::ColimaCli::new(search);
    let engines = berth_engine::discovery::discover(&colima, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running");
    Arc::new(berth_engine::BollardEngine::connect(engine).expect("connect"))
}

/// Remove what this test may have left: the three names, and only when the
/// engine says they carry the fixture label.
async fn sweep(client: &dyn EngineClient) {
    if let Ok(containers) = client.containers().await {
        for c in containers
            .iter()
            .filter(|c| NAMES.contains(&c.name.as_str()) && labelled(&c.labels))
        {
            let _ = client
                .act(
                    &c.id,
                    ContainerAction::Remove {
                        force: true,
                        volumes: false,
                    },
                )
                .await;
        }
    }
    if let Ok(volumes) = client.volumes().await {
        if volumes
            .iter()
            .any(|v| v.name == VOLUME && labelled(&v.labels))
        {
            let _ = client.remove_volume(VOLUME).await;
        }
    }
}

fn spec(name: &str) -> ContainerSpec {
    ContainerSpec {
        name: name.to_string(),
        image: ImageRef {
            reference: "alpine:3".to_string(),
            id: None,
        },
        command: Some(vec!["true".to_string()]),
        entrypoint: None,
        env: Vec::new(),
        ports: Vec::new(),
        mounts: Vec::new(),
        networks: Vec::new(),
        labels: fixture_labels(),
        restart: RestartPolicy::No,
        limits: Limits::default(),
        platform: None,
        working_dir: None,
        user: None,
        healthcheck: None,
        cap_add: Vec::new(),
        cap_drop: Vec::new(),
        network_aliases: BTreeMap::new(),
    }
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn clean_up_it_removes_two_stopped_fixture_containers_and_a_fixture_volume() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let client = connect().await;
    let client = client.as_ref();
    sweep(client).await;

    // a. Two stopped containers and a volume, all labelled.
    let images = client.images().await.expect("images");
    assert!(
        images
            .iter()
            .any(|i| i.repo_tags.iter().any(|t| t == "alpine:3")),
        "alpine:3 is expected on this engine (the test never pulls)"
    );
    let mut ids = Vec::new();
    for name in NAMES {
        let id = client.create(&spec(name), false).await.expect("create");
        ids.push(id.0);
    }
    client
        .create_volume(&VolumeSpec {
            name: VOLUME.to_string(),
            driver: "local".to_string(),
            driver_opts: BTreeMap::new(),
            labels: fixture_labels(),
        })
        .await
        .expect("create volume");
    ids.push(VOLUME.to_string());

    // b. The plan as the sheet builds it, then narrowed to the three.
    let engine_id = client.id().clone();
    let containers = client.containers().await.expect("containers");
    let images = client.images().await.expect("images");
    let volumes = client.volumes().await.expect("volumes");
    let networks = client.networks().await.expect("networks");
    let usage = client.disk_usage().await.expect("disk usage");
    let entries = prune::candidates(
        &engine_id,
        &Lists {
            containers: &containers,
            images: &images,
            volumes: &volumes,
            networks: &networks,
            usage: Some(&usage),
        },
    );
    let mut checked = prune::default_checked();
    checked.insert(prune::Group::UnusedVolumes);
    let wide = prune::plan(&engine_id, &entries, &checked);
    let narrowed = match narrow(&wide, &ids, &containers, &volumes) {
        Ok(plan) => plan,
        Err(refusal) => {
            sweep(client).await;
            panic!("{refusal}");
        }
    };
    assert_eq!(narrowed.items.len(), 3);
    assert!(narrowed
        .items
        .iter()
        .all(|i| i.selected && is_fixture(i, &containers, &volumes)));
    drop(wide);

    // c. Run it: the button and the result are the same sentence.
    let effect = narrowed.effect();
    let button = effect.button_label();
    assert!(button.starts_with("Remove 3 items"), "{button}");
    let outcome = prune::run(client, &narrowed, CancellationToken::new(), |_| {}).await;
    assert!(outcome.clean(), "{outcome:?}");
    assert_eq!(
        button.replacen("Remove", "Removed", 1),
        outcome.result_label()
    );

    // All three are gone, and nothing but them.
    let containers_after = client.containers().await.expect("containers");
    for name in NAMES {
        assert!(
            !containers_after.iter().any(|c| c.name == name),
            "{name} survived"
        );
    }
    let volumes_after = client.volumes().await.expect("volumes");
    assert!(!volumes_after.iter().any(|v| v.name == VOLUME));
    for before in containers
        .iter()
        .filter(|c| !NAMES.contains(&c.name.as_str()))
    {
        assert!(
            containers_after.iter().any(|c| c.id == before.id),
            "{} was removed",
            before.name
        );
    }
    for before in volumes.iter().filter(|v| v.name != VOLUME) {
        assert!(
            volumes_after.iter().any(|v| v.name == before.name),
            "{} was removed",
            before.name
        );
    }
    sweep(client).await;
}
