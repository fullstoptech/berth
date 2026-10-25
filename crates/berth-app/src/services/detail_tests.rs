//! P3-T3 tests for the detail service: what `load` reads, what it keeps of
//! the log, that the inspect it hands the Inspect tab has no env value,
//! and that a request goes out once per container state.

use std::sync::Arc;

use super::*;
use crate::state::{use_app_state, AppState};

fn default_engine() -> berth_core::EngineId {
    berth_core::EngineId("colima:default".to_string())
}

fn world_with_db_log(lines: usize) -> berth_engine::MockWorld {
    let mut json: serde_json::Value =
        serde_json::from_str(berth_engine::mock::BUNDLED_JSON).expect("bundled json");
    let log: Vec<serde_json::Value> = (1..=lines)
        .map(|n| serde_json::json!({ "stream": "Stdout", "text": format!("line {n}") }))
        .collect();
    json["logs"]["shop-api-db-1"] = serde_json::Value::Array(log);
    berth_engine::MockWorld::from_json(&json.to_string()).expect("world")
}

fn engine_for(world: berth_engine::MockWorld) -> berth_engine::MockEngine {
    berth_engine::MockEngine::from_world(world, &default_engine()).expect("engine")
}

fn container(world: &berth_engine::MockWorld, name: &str) -> Container {
    world
        .containers
        .iter()
        .find(|c| c.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} in the mock world"))
}

#[tokio::test]
async fn inspector_detail_load_keeps_only_the_last_twenty_lines_of_an_exited_container() {
    let world = world_with_db_log(30);
    let db = container(&world, "shop-api-db-1");
    let loaded = load(&engine_for(world), &db, CancellationToken::new())
        .await
        .expect("loads");
    assert_eq!(loaded.last_lines.len(), 20, "{:?}", loaded.last_lines);
    assert_eq!(
        loaded.last_lines.first().map(String::as_str),
        Some("line 11")
    );
    assert_eq!(
        loaded.last_lines.last().map(String::as_str),
        Some("line 30")
    );
}

#[tokio::test]
async fn inspector_detail_load_reads_no_log_for_a_running_container() {
    let world = world_with_db_log(5);
    let api = container(&world, "shop-api-api-1");
    let loaded = load(&engine_for(world), &api, CancellationToken::new())
        .await
        .expect("loads");
    assert!(loaded.last_lines.is_empty(), "{:?}", loaded.last_lines);
}

#[tokio::test]
async fn inspector_detail_load_reports_a_container_that_is_gone() {
    let world = world_with_db_log(1);
    let mut ghost = container(&world, "shop-api-db-1");
    ghost.id = berth_core::ContainerId("not-a-container".to_string());
    let err = load(&engine_for(world), &ghost, CancellationToken::new())
        .await
        .expect_err("not found");
    assert!(err.1, "gone: {err:?}");
}

#[test]
fn inspector_detail_mask_inspect_replaces_every_env_value_and_nothing_else() {
    let raw = serde_json::json!({
        "Config": {
            "Env": ["API_TOKEN=hunter2-example", "EMPTY=", "NOEQUALS"],
            "Image": "postgres:17-alpine"
        },
        "Name": "/db"
    });
    let masked = mask_inspect(raw);
    assert_eq!(
        masked["Config"]["Env"],
        serde_json::json!(["API_TOKEN=••••", "EMPTY=••••", "NOEQUALS=••••"])
    );
    assert_eq!(masked["Config"]["Image"], "postgres:17-alpine");
    assert_eq!(masked["Name"], "/db");
    assert!(!masked.to_string().contains("hunter2-example"));
}

#[test]
fn inspector_detail_mask_inspect_is_idempotent_and_leaves_other_shapes_alone() {
    let once = mask_inspect(serde_json::json!({ "Config": { "Env": ["A=1"] } }));
    assert_eq!(mask_inspect(once.clone()), once);
    let no_env = serde_json::json!({ "Config": {} });
    assert_eq!(mask_inspect(no_env.clone()), no_env);
}

#[tokio::test]
async fn inspector_detail_debug_output_has_no_env_values() {
    let world = world_with_db_log(1);
    let db = container(&world, "shop-api-db-1");
    let mut loaded = load(&engine_for(world), &db, CancellationToken::new())
        .await
        .expect("loads");
    loaded.detail.env = vec![berth_core::EnvVar {
        key: "TOKEN".to_string(),
        value: "never-printed-value".to_string(),
    }];
    let printed = format!("{loaded:?}");
    assert!(!printed.contains("never-printed-value"), "{printed}");
    let entry = format!("{:?}", DetailEntry::Ready(Box::new(loaded)));
    assert!(!entry.contains("never-printed-value"), "{entry}");
}

// ---------------------------------------------------------------------
// The store: one request per container state
// ---------------------------------------------------------------------

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, DetailStore)>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    let store = DetailStore::new();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some((state, store)));
    });
    rsx! { div {} }
}

fn services_with_mock(engine: Arc<berth_engine::MockEngine>) -> Services {
    let clients = crate::services::engines::client_map();
    if let Ok(mut map) = clients.lock() {
        map.insert(
            default_engine(),
            engine as Arc<dyn berth_engine::EngineClient>,
        );
    }
    Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients,
        cancel: CancellationToken::new(),
    }
}

async fn settle(dom: &mut VirtualDom, store: DetailStore, id: &ContainerId) {
    let waited = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let ready = dom.in_runtime(|| store.entries.peek().contains_key(id));
            if ready {
                return;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work())
                .await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    assert!(waited.is_ok(), "the detail did not arrive");
}

#[tokio::test]
async fn inspector_detail_store_loads_a_container_once_per_state() {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (_state, store) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the store");
    let world = world_with_db_log(3);
    let db = container(&world, "shop-api-db-1");
    let services = services_with_mock(Arc::new(engine_for(world)));

    dom.in_scope(ScopeId::APP, || store.ensure(&services, &db));
    settle(&mut dom, store, &db.id).await;
    let entry = dom.in_runtime(|| store.entries.peek().get(&db.id).cloned());
    let Some(DetailEntry::Ready(loaded)) = entry else {
        panic!("a loaded detail, got {entry:?}");
    };
    assert_eq!(loaded.last_lines, ["line 1", "line 2", "line 3"]);

    // Same state again: no new request, so a hand-edited entry stays.
    dom.in_scope(ScopeId::APP, || {
        let mut entries = store.entries;
        entries.write().insert(
            db.id.clone(),
            DetailEntry::Failed {
                message: "marker".to_string(),
                gone: false,
            },
        );
        store.ensure(&services, &db);
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    dom.render_immediate(&mut dioxus::core::NoOpMutations);
    let again = dom.in_runtime(|| store.entries.peek().get(&db.id).cloned());
    assert!(
        matches!(again, Some(DetailEntry::Failed { .. })),
        "{again:?}"
    );

    // A new state (it restarted) asks again.
    let mut restarted = db.clone();
    restarted.restart_count += 1;
    dom.in_scope(ScopeId::APP, || store.ensure(&services, &restarted));
    let waited = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let ready = dom.in_runtime(|| {
                matches!(
                    store.entries.peek().get(&db.id),
                    Some(DetailEntry::Ready(_))
                )
            });
            if ready {
                return;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work())
                .await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    assert!(waited.is_ok(), "the second request did not arrive");
}

#[test]
fn inspector_detail_store_without_a_connection_says_so() {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (_state, store) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the store");
    let world = world_with_db_log(1);
    let db = container(&world, "shop-api-db-1");
    let services = Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients: crate::services::engines::client_map(),
        cancel: CancellationToken::new(),
    };
    dom.in_scope(ScopeId::APP, || store.ensure(&services, &db));
    let entry = dom.in_runtime(|| store.entries.peek().get(&db.id).cloned());
    match entry {
        Some(DetailEntry::Failed { message, gone }) => {
            assert!(message.contains("not connected"), "{message}");
            assert!(!gone);
        }
        other => panic!("a failed entry, got {other:?}"),
    }
}
