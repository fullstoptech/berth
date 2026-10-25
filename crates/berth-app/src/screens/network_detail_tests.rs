//! P4-T5 tests: the network detail's rows and buttons, and a container's
//! Networks row in its Overview.

use super::*;

use crate::scenes;

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn default_networks(world: &berth_engine::MockWorld) -> Vec<Network> {
    world.networks_by_engine["colima:default"].clone()
}

fn shop(world: &berth_engine::MockWorld) -> Network {
    default_networks(world)
        .into_iter()
        .find(|n| n.name == "shop-api_default")
        .expect("shop-api_default")
}

fn markup(frame: &str) -> String {
    let mut out = scenes::render_scene_html(frame).expect("the frame has a scene");
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

#[test]
fn network_detail_rows_read_in_order_and_dash_what_is_missing() {
    let world = world();
    let rows = detail_rows(&shop(&world), Some("shop-api"), None);
    let pairs: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.label.as_str(), r.value.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("ID", "41581c8057a0"),
            ("Driver", "bridge"),
            ("Subnet", "172.19.0.0/16"),
            ("Gateway", "172.19.0.1"),
            ("Internal", "No"),
            ("Project", "shop-api"),
        ]
    );
    let host = default_networks(&world)
        .into_iter()
        .find(|n| n.name == "host")
        .expect("host");
    let rows = detail_rows(&host, None, Some("default"));
    let value = |label: &str| {
        rows.iter()
            .find(|r| r.label == label)
            .map(|r| r.value.clone())
    };
    assert_eq!(
        value("Subnet").as_deref(),
        Some(""),
        "empty shows as a dash"
    );
    assert_eq!(
        rows.iter()
            .find(|r| r.label == "Subnet")
            .map(|r| r.empty_text.clone())
            .as_deref(),
        Some("\u{2013}")
    );
    assert_eq!(value("Engine").as_deref(), Some("default"));
    let mut internal = host;
    internal.internal = true;
    assert!(detail_rows(&internal, None, None)
        .iter()
        .any(|r| r.label == "Internal" && r.value == "Yes"));
}

#[test]
fn network_detail_members_are_the_containers_on_it_by_name() {
    let world = world();
    let list = members(&shop(&world), &world.containers);
    assert_eq!(list.len(), 5);
    let names: Vec<&str> = list.iter().map(|m| m.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
    assert!(names.contains(&"shop-api-api-1"));
    let worker = list
        .iter()
        .find(|m| m.name == "shop-api-worker-1")
        .expect("worker");
    assert_eq!(worker.status, berth_core::StatusKind::Restarting);
}

#[test]
fn network_detail_delete_is_locked_disabled_or_allowed() {
    let world = world();
    let mut bridge = default_networks(&world)
        .into_iter()
        .find(|n| n.name == "bridge")
        .expect("bridge");
    assert_eq!(
        delete_state(&bridge, None),
        DeleteState::Locked("Built-in networks cannot be removed".to_string())
    );
    // Built-in wins over any other reason.
    assert!(matches!(
        delete_state(&bridge, Some("13 containers are connected".to_string())),
        DeleteState::Locked(_)
    ));
    bridge.built_in = false;
    assert_eq!(
        delete_state(&bridge, Some("13 containers are connected".to_string())),
        DeleteState::Disabled("13 containers are connected".to_string())
    );
    assert_eq!(delete_state(&bridge, None), DeleteState::Allowed);
}

#[test]
fn network_detail_container_rows_are_its_networks_by_name() {
    let world = world();
    let networks = default_networks(&world);
    let mut api = world
        .containers_for(&berth_core::EngineId("colima:default".to_string()))
        .into_iter()
        .find(|c| c.name == "shop-api-api-1")
        .expect("api");
    api.networks = vec![
        "shop-api_default".to_string(),
        "bridge".to_string(),
        "gone".to_string(),
    ];
    let rows = container_rows(&api, &networks);
    let summary: Vec<(&str, bool, bool)> = rows
        .iter()
        .map(|r| (r.name.as_str(), r.built_in, r.network.is_some()))
        .collect();
    assert_eq!(
        summary,
        [
            ("bridge", true, true),
            ("gone", false, false),
            ("shop-api_default", false, true)
        ]
    );
}

#[test]
fn network_detail_failures_belong_to_the_network_that_failed() {
    let world = world();
    let network = shop(&world);
    let mut pending = std::collections::BTreeMap::new();
    pending.insert(
        PendingKey {
            subject: Subject::Network(network.id.clone()),
            action: "network.disconnect",
        },
        PendingState::Failed {
            message: "container is paused".to_string(),
        },
    );
    pending.insert(
        PendingKey {
            subject: Subject::Network("someone-else".to_string()),
            action: "network.disconnect",
        },
        PendingState::Failed {
            message: "not here".to_string(),
        },
    );
    let found = failures_of(&pending, &network, &default_networks(&world));
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].1,
        "Could not disconnect network shop-api_default: container is paused"
    );
    assert_eq!(running_label(&pending, &network.id), None);
}

#[test]
fn network_detail_overview_row_has_disconnect_and_connect() {
    let html = markup("Containers / selected / 1440");
    assert!(html.contains("shop-api_default"), "{html}");
    assert!(html.contains(">Networks<"));
    assert_eq!(
        html.matches(">Disconnect<").count(),
        1,
        "one network, one button"
    );
    assert!(html.contains(">Connect to network<"));
    // The rows before and after it are still in the Overview.
    assert!(html.contains(">Restart policy<") && html.contains(">Mounts<"));
}
