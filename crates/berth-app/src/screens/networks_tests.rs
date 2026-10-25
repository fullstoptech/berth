//! P4-T5 tests: the rows of the table (built-in networks have no Delete and
//! say why; a network in use has a disabled one), filtering, sorting and the
//! engine scope, and the three scenes.

use super::*;

use crate::scenes;
use crate::state::use_app_state;

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn every_network(world: &berth_engine::MockWorld) -> Vec<Network> {
    world
        .networks_by_engine
        .values()
        .flatten()
        .cloned()
        .collect()
}

fn strip_styles(html: &str) -> String {
    let mut out = html.to_string();
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

fn markup(frame: &str) -> String {
    strip_styles(&scenes::render_scene_html(frame).expect("the frame has a scene"))
}

fn count(html: &str, needle: &str) -> usize {
    html.matches(needle).count()
}

fn user_network(name: &str, subnet: Option<&str>) -> Network {
    Network {
        id: format!("id-{name}"),
        name: name.to_string(),
        driver: "bridge".to_string(),
        subnet: subnet.map(str::to_string),
        gateway: None,
        internal: false,
        containers: Vec::new(),
        project: None,
        built_in: false,
        engine: berth_core::EngineId("colima:default".to_string()),
    }
}

struct Fixture {
    world: berth_engine::MockWorld,
    networks: Vec<Network>,
    pending: BTreeMap<PendingKey, PendingState>,
}

impl Fixture {
    fn new() -> Self {
        let world = world();
        let networks = every_network(&world);
        Self {
            world,
            networks,
            pending: BTreeMap::new(),
        }
    }

    fn model(
        &self,
        scope: &EngineScope,
        filter: &str,
        sort: Option<(SortColumn, SortDirection)>,
    ) -> ListModel {
        build_rows(&Input {
            networks: &self.networks,
            containers: &self.world.containers,
            projects: &self.world.projects,
            engines: &self.world.engines,
            scope,
            pending: &self.pending,
            filter,
            sort,
        })
    }
}

fn one(engine: &str) -> EngineScope {
    EngineScope::One(berth_core::EngineId(engine.to_string()))
}

fn names(model: &ListModel) -> Vec<&str> {
    model.rows.iter().map(|r| r.name.as_str()).collect()
}

// --- Built-in networks and Delete ---

#[test]
fn networks_built_in_rows_have_no_delete_and_carry_the_reason() {
    let fx = Fixture::new();
    let model = fx.model(&one("colima:default"), "", None);
    for name in ["bridge", "host", "none"] {
        let row = model
            .rows
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("{name} listed"));
        assert!(row.built_in, "{name}");
        assert_eq!(
            row.delete,
            DeleteState::Locked("Built-in networks cannot be removed".to_string()),
            "{name} has no Delete, only the reason"
        );
    }
    // Even one with no container on it (host, none) is locked, not allowed.
    assert!(model
        .rows
        .iter()
        .filter(|r| r.built_in)
        .all(|r| matches!(r.delete, DeleteState::Locked(_))));
}

#[test]
fn networks_built_in_markup_has_no_delete_button_and_shows_built_in() {
    let html = markup("Networks / default / 1440");
    for name in ["bridge", "host", "none"] {
        assert!(
            !html.contains(&format!("aria-label=\"Delete network {name}\"")),
            "{name} must have no Delete button"
        );
    }
    assert_eq!(
        count(
            &html,
            "data-tooltip=\"Built-in networks cannot be removed\""
        ),
        3,
        "bridge, host and none show the reason"
    );
    assert_eq!(count(&html, ">Built-in<"), 3);
}

#[test]
fn networks_a_network_in_use_has_a_disabled_delete_with_the_count() {
    let html = markup("Networks / default / 1440");
    let at = html
        .find("aria-label=\"Delete network shop-api_default\"")
        .expect("the user network has a Delete button");
    let button = &html[html[..at].rfind("<button").expect("a button")..at + 200];
    assert!(button.contains("disabled"), "{button}");
    assert!(
        button.contains("data-tooltip=\"5 containers are connected\""),
        "{button}"
    );
}

#[test]
fn networks_an_unused_user_network_can_be_deleted_whichever_engines_are_in_view() {
    let mut fx = Fixture::new();
    fx.networks
        .push(user_network("berthfx-net", Some("172.31.250.0/24")));
    let model = fx.model(&one("colima:default"), "berthfx", None);
    assert_eq!(names(&model), ["berthfx-net"]);
    assert_eq!(model.rows[0].delete, DeleteState::Allowed);
    // Several engines running and every engine in view: the network knows
    // its own engine, so nothing asks the user to choose one.
    for engine in &mut fx.world.engines {
        engine.state = berth_core::EngineState::Running;
    }
    let all = fx.model(&EngineScope::All, "berthfx", None);
    assert_eq!(all.rows[0].delete, DeleteState::Allowed);
}

#[test]
fn networks_a_running_action_replaces_the_button_with_its_label() {
    let mut fx = Fixture::new();
    fx.networks.push(user_network("berthfx-net", None));
    fx.pending.insert(
        PendingKey {
            subject: Subject::Network("id-berthfx-net".to_string()),
            action: "network.delete",
        },
        PendingState::Running {
            label: "Deleting…".to_string(),
        },
    );
    let model = fx.model(&one("colima:default"), "berthfx", None);
    assert_eq!(model.rows[0].busy.as_deref(), Some("Deleting…"));
    assert_eq!(
        model.rows[0].delete,
        DeleteState::Disabled("Deleting… is still in progress".to_string())
    );
}

// --- Rows, scope, filter, sort ---

#[test]
fn networks_table_lists_the_default_engines_networks_your_own_first() {
    let fx = Fixture::new();
    let model = fx.model(&one("colima:default"), "", None);
    let list = names(&model);
    // Other engines' networks stay out; host and none exist once.
    assert!(!list.contains(&"landing_default"), "{list:?}");
    assert_eq!(list.iter().filter(|n| **n == "host").count(), 1, "{list:?}");
    assert_eq!(list.iter().filter(|n| **n == "none").count(), 1, "{list:?}");
    // Your networks sorted by name, then Docker's own.
    let first_built_in = model
        .rows
        .iter()
        .position(|r| r.built_in)
        .expect("built-ins");
    assert!(model.rows[..first_built_in].iter().all(|r| !r.built_in));
    assert!(model.rows[first_built_in..].iter().all(|r| r.built_in));
    let user: Vec<&str> = list[..first_built_in].to_vec();
    let mut sorted = user.clone();
    sorted.sort_by_key(|n| n.to_lowercase());
    assert_eq!(user, sorted);
    assert!(user.contains(&"shop-api_default"));
    assert_eq!(model.total, model.rows.len());
}

#[test]
fn networks_row_carries_driver_subnet_count_and_project() {
    let fx = Fixture::new();
    let model = fx.model(&one("colima:default"), "", None);
    let shop = model
        .rows
        .iter()
        .find(|r| r.name == "shop-api_default")
        .expect("shop-api_default");
    assert_eq!(shop.driver, "bridge");
    assert_eq!(shop.subnet.as_deref(), Some("172.19.0.0/16"));
    assert_eq!(shop.containers, 5);
    assert_eq!(shop.project.as_deref(), Some("shop-api"));
    let host = model.rows.iter().find(|r| r.name == "host").expect("host");
    assert_eq!(
        (host.subnet.clone(), host.containers, host.project.clone()),
        (None, 0, None)
    );
}

#[test]
fn networks_project_is_found_from_the_containers_when_the_engine_gives_none() {
    let mut fx = Fixture::new();
    for network in &mut fx.networks {
        network.project = None;
    }
    let model = fx.model(&one("colima:default"), "shop-api", None);
    assert_eq!(names(&model), ["shop-api_default"]);
    assert_eq!(model.rows[0].project.as_deref(), Some("shop-api"));
}

#[test]
fn networks_all_engines_tags_each_row_with_its_engine() {
    let mut fx = Fixture::new();
    for engine in &mut fx.world.engines {
        engine.state = berth_core::EngineState::Running;
    }
    let all = fx.model(&EngineScope::All, "landing", None);
    assert_eq!(names(&all), ["landing_default"]);
    assert_eq!(all.rows[0].engine_tag.as_deref(), Some("hetzner-1"));
    let default_only = fx.model(&one("colima:default"), "shop-api", None);
    assert_eq!(
        default_only.rows[0].engine_tag, None,
        "one engine in view needs no tag"
    );
}

#[test]
fn networks_filter_matches_name_driver_subnet_and_project_and_counts() {
    let fx = Fixture::new();
    let scope = one("colima:default");
    let total = fx.model(&scope, "", None).total;
    for (text, expected) in [
        ("shop", "shop-api_default"),
        ("172.19", "shop-api_default"),
        ("ML-NOTEBOOKS", "ml-notebooks_default"),
    ] {
        let model = fx.model(&scope, text, None);
        assert!(
            names(&model).contains(&expected),
            "{text}: {:?}",
            names(&model)
        );
        assert_eq!(model.total, total, "total ignores the text filter");
        assert_eq!(model.matched, model.rows.len());
    }
    // Every term must match.
    let both = fx.model(&scope, "shop 172.20", None);
    assert!(both.rows.is_empty());
    let host = fx.model(&scope, "driver-that-does-not-exist", None);
    assert_eq!((host.matched, host.total > 0), (0, true));
    // The driver column is searchable.
    assert!(fx
        .model(&scope, "null", None)
        .rows
        .iter()
        .any(|r| r.name == "none"));
}

#[test]
fn networks_sort_by_subnet_is_numeric_and_puts_empty_last_either_way() {
    let mut fx = Fixture::new();
    fx.networks = vec![
        user_network("b-net", Some("172.19.0.0/16")),
        user_network("a-net", Some("172.9.0.0/16")),
        user_network("c-net", None),
        user_network("d-net", Some("fd00:1::/64")),
        user_network("e-net", Some("10.0.0.0/8")),
    ];
    let asc = fx.model(
        &EngineScope::All,
        "",
        Some((SortColumn::Subnet, SortDirection::Asc)),
    );
    assert_eq!(names(&asc), ["e-net", "a-net", "b-net", "d-net", "c-net"]);
    let desc = fx.model(
        &EngineScope::All,
        "",
        Some((SortColumn::Subnet, SortDirection::Desc)),
    );
    assert_eq!(names(&desc), ["d-net", "b-net", "a-net", "e-net", "c-net"]);
}

#[test]
fn networks_sort_by_containers_and_name_and_driver() {
    let mut fx = Fixture::new();
    let mut busy = user_network("busy", None);
    busy.containers = vec![
        berth_core::ContainerId("x".into()),
        berth_core::ContainerId("y".into()),
    ];
    let mut macv = user_network("macv", None);
    macv.driver = "macvlan".to_string();
    fx.networks = vec![user_network("zed", None), busy, macv];
    let by = |column, dir| {
        let model = fx.model(&EngineScope::All, "", Some((column, dir)));
        names(&model)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        by(SortColumn::Containers, SortDirection::Desc),
        ["busy", "macv", "zed"]
    );
    assert_eq!(
        by(SortColumn::Containers, SortDirection::Asc),
        ["macv", "zed", "busy"]
    );
    assert_eq!(
        by(SortColumn::Name, SortDirection::Desc),
        ["zed", "macv", "busy"]
    );
    assert_eq!(
        by(SortColumn::Driver, SortDirection::Desc),
        ["macv", "busy", "zed"]
    );
}

#[test]
fn networks_failures_name_the_action_and_the_network() {
    let networks = vec![user_network("berthfx-net", None)];
    let mut pending = BTreeMap::new();
    pending.insert(
        PendingKey {
            subject: Subject::Network("id-berthfx-net".to_string()),
            action: "network.delete",
        },
        PendingState::Failed {
            message: "network has active endpoints".to_string(),
        },
    );
    pending.insert(
        PendingKey {
            subject: Subject::Network("fresh".to_string()),
            action: "network.create",
        },
        PendingState::Failed {
            message: "Pool overlaps".to_string(),
        },
    );
    pending.insert(
        PendingKey {
            subject: Subject::Container(berth_core::ContainerId("c".into())),
            action: "container.stop",
        },
        PendingState::Failed {
            message: "not ours".to_string(),
        },
    );
    let texts: Vec<String> = failures(&pending, &networks)
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    assert_eq!(
        texts,
        [
            "Could not create network fresh: Pool overlaps",
            "Could not delete network berthfx-net: network has active endpoints"
        ]
    );
}

// --- Keys ---

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { div {} }
}

fn loaded_state() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let mut state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    let world = world();
    dom.in_runtime(|| {
        state.engines.set(world.engines.clone());
        state.containers.set(world.containers.clone());
        state.projects.set(world.projects.clone());
        state.networks.set(every_network(&world));
        state.engine_scope.set(one("colima:default"));
        state.section.set(Section::Networks);
    });
    (dom, state)
}

fn select(dom: &VirtualDom, mut state: AppState, name: &str) {
    dom.in_runtime(|| {
        let id = state
            .networks
            .peek()
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.id.clone())
            .unwrap_or_else(|| panic!("network {name}"));
        state.selection.set(Selection::Network(id));
    });
}

#[test]
fn networks_backspace_asks_before_deleting_a_user_network() {
    let (dom, mut state) = loaded_state();
    dom.in_runtime(|| {
        state
            .networks
            .write()
            .push(user_network("berthfx-net", None))
    });
    select(&dom, state, "berthfx-net");
    dom.in_runtime(|| delete_selected(&mut state));
    let overlays = dom.in_runtime(|| state.overlays.peek().clone());
    let [Overlay::Confirm(model)] = overlays.as_slice() else {
        panic!("one confirm dialog expected, got {overlays:?}");
    };
    assert_eq!(model.title, "Delete network berthfx-net?");
    assert_eq!(
        model.targets,
        vec![Subject::Network("id-berthfx-net".to_string())]
    );
}

#[test]
fn networks_backspace_does_nothing_on_a_built_in_or_a_network_in_use() {
    let (dom, mut state) = loaded_state();
    for name in ["bridge", "shop-api_default"] {
        select(&dom, state, name);
        dom.in_runtime(|| delete_selected(&mut state));
        assert!(
            dom.in_runtime(|| state.overlays.peek().is_empty()),
            "{name} opened a dialog"
        );
    }
}

#[test]
fn networks_backspace_is_ignored_elsewhere_and_over_another_overlay() {
    let (dom, mut state) = loaded_state();
    dom.in_runtime(|| {
        state
            .networks
            .write()
            .push(user_network("berthfx-net", None))
    });
    select(&dom, state, "berthfx-net");
    dom.in_runtime(|| state.section.set(Section::Containers));
    dom.in_runtime(|| delete_selected(&mut state));
    assert!(dom.in_runtime(|| state.overlays.peek().is_empty()));
    dom.in_runtime(|| {
        state.section.set(Section::Networks);
        state.overlays.write().push(Overlay::Palette);
        delete_selected(&mut state);
    });
    assert_eq!(dom.in_runtime(|| state.overlays.peek().len()), 1);
}

// --- Scenes ---

#[test]
fn networks_scene_default_shows_the_table() {
    let html = markup("Networks / default / 1440");
    assert!(html.contains("aria-label=\"Networks\""), "{html}");
    for head in ["Name", "Driver", "Subnet", "Containers", "Project"] {
        assert!(html.contains(&format!(">{head}<")), "{head} column");
    }
    assert!(html.contains("Filter networks"));
    assert!(html.contains(">Create network<"));
    assert!(html.contains("172.19.0.0/16"));
    assert!(html.contains("shop-api_default"));
    // Nothing is selected, so no detail.
    assert!(!html.contains("Connect container"));
    // The tiers' hooks are on the cells.
    assert!(html.contains("nw-driver") && html.contains("nw-project"));
}

#[test]
fn networks_scene_selected_shows_the_detail_with_connect_and_disconnect() {
    let html = markup("Networks / selected / 1440");
    assert!(html.contains("nw-selected"), "the row is marked");
    assert!(html.contains("Containers (5)"));
    assert!(html.contains(">Connect container<"));
    assert_eq!(count(&html, ">Disconnect<"), 5, "one per container on it");
    assert!(html.contains("172.19.0.1"), "the gateway row: {html}");
    // In use: Delete is there and disabled with the reason.
    let at = html
        .find(">Delete network<")
        .expect("Delete network button");
    let button = &html[html[..at].rfind("<button").expect("a button")..at];
    assert!(button.contains("disabled") && button.contains("5 containers are connected"));
}

#[test]
fn networks_scene_create_shows_the_invalid_subnet_inline() {
    let html = markup("Networks / create / 1440");
    assert!(html.contains("aria-label=\"Create network\""), "{html}");
    assert!(
        html.contains("Host bits are set. Did you mean 172.31.250.0/24?"),
        "the inline error: {html}"
    );
    let at = html.rfind(">Create network<").expect("the primary button");
    let button = &html[html[..at].rfind("<button").expect("a button")..at];
    assert!(button.contains("disabled"), "{button}");
    assert!(
        button.contains("Host bits are set"),
        "the reason is the tooltip: {button}"
    );
}
