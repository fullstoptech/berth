use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_core::{ContainerId, EngineId, Network, Subject};
use berth_engine::EngineClient as _;

use super::*;
use crate::services::actions::{apply_finished, ActionsService, Finished};
use crate::state::{use_app_state, AppState, PendingKey, PendingState};

fn network(name: &str, subnet: Option<&str>) -> Network {
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
    }
}

fn draft(name: &str, subnet: &str, gateway: &str) -> Draft {
    Draft {
        name: name.to_string(),
        subnet: subnet.to_string(),
        gateway: gateway.to_string(),
        ..Draft::default()
    }
}

// --- Subnets ---

#[test]
fn networks_subnet_accepts_a_block_and_prints_it_back() {
    for text in [
        "172.31.250.0/24",
        "10.0.0.0/8",
        "192.168.1.128/25",
        "172.31.250.4/30",
        "fd00:1::/64",
        " 172.31.250.0/24 ",
    ] {
        let subnet = parse_subnet(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(subnet.to_string(), text.trim(), "{text}");
    }
}

#[test]
fn networks_subnet_names_what_is_wrong() {
    let cases = [
        ("", SubnetError::NotCidr),
        ("172.31.250.0", SubnetError::NotCidr),
        ("172.31.250/24", SubnetError::BadAddress),
        ("300.1.1.0/24", SubnetError::BadAddress),
        ("172.031.250.0/24", SubnetError::BadAddress),
        ("web/24", SubnetError::BadAddress),
        ("/24", SubnetError::BadAddress),
        ("172.31.250.0/", SubnetError::BadPrefix { max: 30 }),
        ("172.31.250.0/0", SubnetError::BadPrefix { max: 30 }),
        ("172.31.250.0/31", SubnetError::BadPrefix { max: 30 }),
        ("172.31.250.0/33", SubnetError::BadPrefix { max: 30 }),
        ("172.31.250.0/+24", SubnetError::BadPrefix { max: 30 }),
        ("172.31.250.0/2 4", SubnetError::BadPrefix { max: 30 }),
        ("172.31.250.0/-1", SubnetError::BadPrefix { max: 30 }),
        ("172.31.250.0/999", SubnetError::BadPrefix { max: 30 }),
        ("fd00::/127", SubnetError::BadPrefix { max: 126 }),
        (
            "172.31.250.5/24",
            SubnetError::HostBits {
                network: "172.31.250.0/24".to_string(),
            },
        ),
        (
            "10.1.0.0/8",
            SubnetError::HostBits {
                network: "10.0.0.0/8".to_string(),
            },
        ),
        (
            "fd00:1::5/64",
            SubnetError::HostBits {
                network: "fd00:1::/64".to_string(),
            },
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(parse_subnet(text), Err(expected), "{text:?}");
    }
}

#[test]
fn networks_subnet_error_text_says_what_to_do() {
    let host_bits = parse_subnet("172.31.250.5/24").expect_err("host bits");
    assert_eq!(
        host_bits.to_string(),
        "Host bits are set. Did you mean 172.31.250.0/24?"
    );
    let no_prefix = parse_subnet("172.31.250.0").expect_err("no prefix");
    assert!(no_prefix.to_string().contains("172.31.250.0/24"));
    let prefix = parse_subnet("172.31.250.0/31").expect_err("prefix");
    assert!(prefix.to_string().contains("1 to 30"));
}

#[test]
fn networks_subnets_overlap_when_one_holds_the_other() {
    let sub = |s: &str| parse_subnet(s).expect(s);
    assert!(sub("172.31.0.0/16").overlaps(sub("172.31.250.0/24")));
    assert!(sub("172.31.250.0/24").overlaps(sub("172.31.0.0/16")));
    assert!(sub("172.31.250.0/24").overlaps(sub("172.31.250.0/24")));
    assert!(!sub("172.31.250.0/24").overlaps(sub("172.31.251.0/24")));
    assert!(!sub("172.20.0.0/16").overlaps(sub("172.21.0.0/16")));
    assert!(!sub("10.0.0.0/8").overlaps(sub("fd00::/64")));
    assert!(sub("172.31.250.0/24").contains("172.31.250.77".parse().expect("ip")));
    assert!(!sub("172.31.250.0/24").contains("172.31.251.1".parse().expect("ip")));
    assert!(!sub("172.31.250.0/24").contains("fd00::1".parse().expect("ip")));
}

// --- Gateways ---

#[test]
fn networks_gateway_must_be_a_host_address_inside_the_subnet() {
    let subnet = parse_subnet("172.31.250.0/24").ok();
    assert_eq!(
        parse_gateway("172.31.250.1", subnet),
        Ok("172.31.250.1".parse().expect("ip"))
    );
    assert_eq!(
        parse_gateway("172.31.251.1", subnet),
        Err(GatewayError::Outside {
            subnet: "172.31.250.0/24".to_string()
        })
    );
    assert_eq!(
        parse_gateway("172.31.250.0", subnet),
        Err(GatewayError::Reserved)
    );
    assert_eq!(
        parse_gateway("172.31.250.255", subnet),
        Err(GatewayError::Reserved)
    );
    assert_eq!(
        parse_gateway("fd00::1", subnet),
        Err(GatewayError::WrongFamily)
    );
    assert_eq!(
        parse_gateway("gateway", subnet),
        Err(GatewayError::NotAddress)
    );
    assert_eq!(
        parse_gateway("172.31.250.1", None),
        Err(GatewayError::NeedsSubnet)
    );
    // A /30 has two hosts: .1 and .2 work, .3 is the broadcast address.
    let small = parse_subnet("172.31.250.4/30").ok();
    assert!(parse_gateway("172.31.250.5", small).is_ok());
    assert!(parse_gateway("172.31.250.6", small).is_ok());
    assert_eq!(
        parse_gateway("172.31.250.7", small),
        Err(GatewayError::Reserved)
    );
}

// --- The draft ---

#[test]
fn networks_draft_builds_the_spec_the_engine_gets() {
    let mut d = draft("  berthfx-net ", " 172.31.250.0/24 ", "172.31.250.1");
    d.internal = true;
    d.attach = vec![ContainerId("c1".to_string())];
    let spec = validate(&d, &[network("bridge", Some("172.17.0.0/16"))]).expect("valid");
    assert_eq!(spec.name, "berthfx-net");
    assert_eq!(spec.driver, "bridge");
    assert_eq!(spec.subnet.as_deref(), Some("172.31.250.0/24"));
    assert_eq!(spec.gateway.as_deref(), Some("172.31.250.1"));
    assert!(spec.internal);
    assert_eq!(spec.attach, vec![ContainerId("c1".to_string())]);
}

#[test]
fn networks_draft_with_only_a_name_leaves_subnet_and_gateway_to_docker() {
    let spec = validate(&draft("web", "", ""), &[]).expect("valid");
    assert_eq!((spec.subnet, spec.gateway), (None, None));
}

#[test]
fn networks_draft_shows_an_invalid_subnet_inline_and_blocks_create() {
    let errors = validate(&draft("berthfx-net", "172.31.250.5/24", ""), &[]).expect_err("invalid");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].field, Field::Subnet);
    assert!(!errors[0].blank, "a typed subnet is red, not quiet");
    assert_eq!(
        errors[0].message,
        "Host bits are set. Did you mean 172.31.250.0/24?"
    );
    assert_eq!(
        create_blocker(&errors).as_deref(),
        Some("Host bits are set. Did you mean 172.31.250.0/24?")
    );
}

#[test]
fn networks_draft_empty_name_blocks_quietly() {
    let errors = validate(&draft("", "", ""), &[]).expect_err("needs a name");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].field, Field::Name);
    assert!(errors[0].blank, "an untouched form draws no red text");
    assert_eq!(create_blocker(&errors).as_deref(), Some("Name the network"));
}

#[test]
fn networks_draft_checks_the_name() {
    let existing = [network("shop-api_default", None)];
    for bad in ["a", "-web", "web net", "web/net", "web:net", "été"] {
        let errors = validate(&draft(bad, "", ""), &existing).expect_err(bad);
        assert_eq!(errors[0].field, Field::Name, "{bad}");
        assert!(!errors[0].blank, "{bad}");
    }
    for good in ["ab", "a1", "web_net", "web.net", "web-net", "9lives"] {
        assert!(validate(&draft(good, "", ""), &existing).is_ok(), "{good}");
    }
    let taken = validate(&draft("shop-api_default", "", ""), &existing).expect_err("taken");
    assert_eq!(
        taken[0].message,
        "A network named shop-api_default already exists"
    );
}

#[test]
fn networks_draft_refuses_a_subnet_another_network_holds() {
    let existing = [network("shop-api_default", Some("172.19.0.0/16"))];
    let errors = validate(&draft("web", "172.19.4.0/24", ""), &existing).expect_err("overlap");
    assert_eq!(errors[0].field, Field::Subnet);
    assert_eq!(
        errors[0].message,
        "Overlaps 172.19.0.0/16 on network shop-api_default"
    );
    assert!(validate(&draft("web", "172.20.4.0/24", ""), &existing).is_ok());
}

#[test]
fn networks_draft_blames_the_subnet_not_the_gateway_when_the_subnet_is_wrong() {
    let errors = validate(&draft("web", "172.31.250.5/24", "172.31.250.1"), &[]).expect_err("bad");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].field, Field::Subnet);
    let errors = validate(&draft("web", "", "172.31.250.1"), &[]).expect_err("no subnet");
    assert_eq!(errors[0].field, Field::Gateway);
    assert!(errors[0].message.contains("subnet first"));
    let errors = validate(&draft("web", "172.31.250.0/24", "10.0.0.1"), &[]).expect_err("outside");
    assert_eq!(
        errors[0].message,
        "The gateway must be inside 172.31.250.0/24"
    );
}

// --- What the data model leaves open ---

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn mock_network(world: &berth_engine::MockWorld, name: &str) -> Network {
    world.networks_by_engine["colima:default"]
        .iter()
        .find(|n| n.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("mock network {name}"))
}

#[test]
fn networks_engine_comes_from_the_containers_on_it_then_the_scope() {
    let world = world();
    let all_containers = world.containers.clone();
    let shop = mock_network(&world, "shop-api_default");
    assert_eq!(
        engine_of(&shop, &all_containers, &world.engines, &EngineScope::All),
        Some(default_engine())
    );
    // An unused network has no container to tell: the selected engine wins.
    let unused = network("berthfx-net", None);
    let work = EngineId("colima:work".to_string());
    assert_eq!(
        engine_of(
            &unused,
            &all_containers,
            &world.engines,
            &EngineScope::One(work.clone())
        ),
        Some(work.clone())
    );
    // With every engine in view and several running it cannot be told.
    let mut two = world.engines.clone();
    for engine in &mut two {
        if engine.id == work {
            engine.state = berth_core::EngineState::Running;
        }
    }
    assert_eq!(
        engine_of(&unused, &all_containers, &two, &EngineScope::All),
        None
    );
    // One running engine is the answer (the bundled world runs only default).
    assert_eq!(
        engine_of(&unused, &all_containers, &world.engines, &EngineScope::All),
        Some(default_engine())
    );
}

#[test]
fn networks_project_is_the_one_every_container_on_it_belongs_to() {
    let world = world();
    let containers = world.containers_for(&default_engine());
    let mut shop = mock_network(&world, "shop-api_default");
    let by_label = project_of(&shop, &containers, &world.projects);
    assert_eq!(by_label.as_deref(), Some("shop-api"));
    // The live engine reports no project; the containers still say it.
    shop.project = None;
    assert_eq!(
        project_of(&shop, &containers, &world.projects).as_deref(),
        Some("shop-api")
    );
    // The default bridge carries containers of many projects: none.
    let mut bridge = mock_network(&world, "bridge");
    bridge.project = None;
    assert_eq!(project_of(&bridge, &containers, &world.projects), None);
    assert_eq!(
        project_of(&network("empty", None), &containers, &world.projects),
        None
    );
}

#[test]
fn networks_connectable_lists_containers_of_the_engine_not_yet_on_it() {
    let world = world();
    let containers = world.containers_for(&default_engine());
    let shop = mock_network(&world, "shop-api_default");
    let offered = connectable(&shop, &default_engine(), &containers);
    assert!(!offered.is_empty());
    assert!(offered.iter().all(|c| c.engine == default_engine()));
    assert!(offered
        .iter()
        .all(|c| !shop.containers.contains(&c.id) && !c.networks.contains(&shop.name)));
    // Running containers first.
    let first_not_running = offered.iter().position(|c| {
        !matches!(
            c.state,
            berth_core::ContainerState::Running | berth_core::ContainerState::Restarting
        )
    });
    if let Some(at) = first_not_running {
        assert!(offered[at..].iter().all(|c| !matches!(
            c.state,
            berth_core::ContainerState::Running | berth_core::ContainerState::Restarting
        )));
    }
}

#[test]
fn networks_joinable_skips_host_none_and_what_the_container_is_on() {
    let world = world();
    let containers = world.containers_for(&default_engine());
    let networks = world.networks_by_engine["colima:default"].clone();
    let api = containers
        .iter()
        .find(|c| c.name == "shop-api-api-1")
        .expect("api");
    let scope = EngineScope::One(default_engine());
    let names: Vec<&str> = joinable(api, &networks, &containers, &world.engines, &scope)
        .into_iter()
        .map(|n| n.name.as_str())
        .collect();
    assert!(
        !names.contains(&"host") && !names.contains(&"none"),
        "{names:?}"
    );
    assert!(
        !names.contains(&"shop-api_default"),
        "already on it: {names:?}"
    );
    assert!(names.contains(&"analytics_default"), "{names:?}");
    // User networks come before the built-in bridge.
    if let (Some(user), Some(bridge)) = (
        names.iter().position(|n| *n == "analytics_default"),
        names.iter().position(|n| *n == "bridge"),
    ) {
        assert!(user < bridge, "{names:?}");
    }
}

#[test]
fn networks_a_container_network_name_finds_the_network_on_its_own_engine() {
    let world = world();
    let containers = world.containers_for(&default_engine());
    let api = containers
        .iter()
        .find(|c| c.name == "shop-api-api-1")
        .expect("api");
    // `bridge` exists once per engine; the one the container is on wins.
    let mut every_bridge: Vec<Network> = Vec::new();
    for list in world.networks_by_engine.values() {
        every_bridge.extend(list.iter().filter(|n| n.name == "bridge").cloned());
    }
    assert!(every_bridge.len() > 1, "the mock has a bridge per engine");
    let mut on_bridge = api.clone();
    on_bridge.networks = vec!["bridge".to_string()];
    let mut mine = every_bridge[0].clone();
    mine.containers = vec![api.id.clone()];
    let mut rest = every_bridge[1..].to_vec();
    rest.insert(0, mine.clone());
    let found = network_named("bridge", &on_bridge, &rest).expect("found");
    assert_eq!(found.id, mine.id);
}

#[test]
fn networks_op_ids_are_the_action_ids_of_the_registry() {
    let spec = NetworkSpec {
        name: "web".to_string(),
        driver: "bridge".to_string(),
        subnet: None,
        gateway: None,
        internal: false,
        attach: Vec::new(),
    };
    let c = ContainerId("c".to_string());
    assert_eq!(NetOp::Create(spec).action_id(), "network.create");
    assert_eq!(
        NetOp::Remove { id: "i".into() }.action_id(),
        "network.delete"
    );
    assert_eq!(
        NetOp::Connect {
            network: "n".into(),
            container: c.clone()
        }
        .action_id(),
        "network.connect_container"
    );
    assert_eq!(
        NetOp::Disconnect {
            network: "n".into(),
            container: c
        }
        .action_id(),
        "network.disconnect"
    );
    for id in [
        "network.create",
        "network.delete",
        "network.connect_container",
        "network.disconnect",
    ] {
        assert!(
            crate::actions::action_by_id(id).is_some(),
            "{id} is registered"
        );
    }
}

// --- Through the actions service, against the mock engine ---

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

struct Rig {
    dom: VirtualDom,
    state: AppState,
    service: ActionsService,
    rx: tokio::sync::mpsc::UnboundedReceiver<Finished>,
    store: Arc<berth_store::Store>,
    engine: Arc<berth_engine::MockEngine>,
}

impl Rig {
    fn new() -> Self {
        let mut dom = VirtualDom::new(Harness);
        let _ = dom.rebuild_to_vec();
        let mut state = SLOT
            .with(|slot| slot.borrow_mut().take())
            .expect("harness captured AppState");
        let world = world();
        let engine = Arc::new(berth_engine::MockEngine::bundled(&default_engine()));
        let clients = crate::services::engines::client_map();
        clients.lock().expect("clients").insert(
            default_engine(),
            Arc::clone(&engine) as Arc<dyn berth_engine::EngineClient>,
        );
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let (service, rx) = ActionsService::new(
            clients,
            Arc::clone(&store),
            Arc::new(berth_engine::MockCompose::bundled()),
            Arc::new(berth_engine::MockColima::bundled()),
            CancellationToken::new(),
        );
        dom.in_runtime(|| {
            state.engines.set(world.engines.clone());
            state
                .containers
                .set(world.containers_for(&default_engine()));
            state.projects.set(world.projects.clone());
            state
                .networks
                .set(world.networks_by_engine["colima:default"].clone());
            state.engine_scope.set(EngineScope::One(default_engine()));
        });
        Self {
            dom,
            state,
            service,
            rx,
            store,
            engine,
        }
    }

    fn run(&mut self, id: &str, name: &str, op: NetOp) -> Result<(), String> {
        let service = self.service.clone();
        let mut state = self.state;
        self.dom
            .in_runtime(|| service.network(&mut state, &default_engine(), id, name, op))
    }

    async fn settle(&mut self) -> PendingKey {
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), self.rx.recv())
            .await
            .expect("a Finished within 5 s")
            .expect("channel open");
        let key = done.key.clone();
        let mut state = self.state;
        self.dom.in_runtime(|| apply_finished(&mut state, done));
        // What the engine's event would do: refresh the list.
        let fresh = self.engine.networks().await.expect("networks");
        self.dom.in_runtime(|| state.networks.set(fresh));
        key
    }

    fn pending(&self) -> Vec<(PendingKey, PendingState)> {
        self.dom.in_runtime(|| {
            self.state
                .pending
                .peek()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
    }

    fn toasts(&self) -> Vec<String> {
        self.dom.in_runtime(|| {
            self.state
                .toasts
                .peek()
                .iter()
                .map(|t| t.text.clone())
                .collect()
        })
    }

    fn network_id(&self, name: &str) -> String {
        self.dom.in_runtime(|| {
            self.state
                .networks
                .peek()
                .iter()
                .find(|n| n.name == name)
                .map(|n| n.id.clone())
                .unwrap_or_else(|| panic!("network {name} in the list"))
        })
    }

    fn container(&self, name: &str) -> berth_core::Container {
        self.dom.in_runtime(|| {
            self.state
                .containers
                .peek()
                .iter()
                .find(|c| c.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("mock container {name}"))
        })
    }

    async fn engine_networks_of(&self, container: &str) -> Vec<String> {
        let id = self.container(container).id;
        self.engine
            .containers()
            .await
            .expect("containers")
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.networks)
            .unwrap_or_default()
    }
}

fn spec(name: &str) -> NetworkSpec {
    NetworkSpec {
        name: name.to_string(),
        driver: "bridge".to_string(),
        subnet: Some("172.31.250.0/24".to_string()),
        gateway: None,
        internal: false,
        attach: Vec::new(),
    }
}

#[tokio::test]
async fn networks_create_connect_disconnect_and_delete_run_through_the_service() {
    let mut rig = Rig::new();

    // Create: pending at once, the engine has it afterwards, a toast says so.
    rig.run(
        "berthfx-net",
        "berthfx-net",
        NetOp::Create(spec("berthfx-net")),
    )
    .expect("accepted");
    assert_eq!(
        rig.pending(),
        vec![(
            PendingKey {
                subject: Subject::Network("berthfx-net".to_string()),
                action: "network.create"
            },
            PendingState::Running {
                label: "Creating…".to_string()
            }
        )]
    );
    rig.settle().await;
    assert!(rig.pending().is_empty());
    assert!(rig
        .toasts()
        .contains(&"Created network berthfx-net".to_string()));
    let listed = rig.engine.networks().await.expect("networks");
    let created = listed
        .iter()
        .find(|n| n.name == "berthfx-net")
        .expect("created");
    assert_eq!(created.subnet.as_deref(), Some("172.31.250.0/24"));

    // Connect the container; the engine lists the network on it.
    let id = rig.network_id("berthfx-net");
    let web = rig.container("shop-api-api-1").id;
    rig.run(
        &id,
        "berthfx-net",
        NetOp::Connect {
            network: "berthfx-net".to_string(),
            container: web.clone(),
        },
    )
    .expect("accepted");
    assert_eq!(
        rig.pending()[0].1,
        PendingState::Running {
            label: "Connecting…".to_string()
        }
    );
    rig.settle().await;
    assert!(rig
        .engine_networks_of("shop-api-api-1")
        .await
        .contains(&"berthfx-net".to_string()));
    assert!(rig
        .toasts()
        .contains(&"Connected to berthfx-net".to_string()));

    // Disconnect it again.
    rig.run(
        &id,
        "berthfx-net",
        NetOp::Disconnect {
            network: "berthfx-net".to_string(),
            container: web,
        },
    )
    .expect("accepted");
    rig.settle().await;
    assert!(!rig
        .engine_networks_of("shop-api-api-1")
        .await
        .contains(&"berthfx-net".to_string()));
    assert!(rig
        .toasts()
        .contains(&"Disconnected from berthfx-net".to_string()));

    // Delete through the confirm builder, the way the button goes.
    let model = {
        let state = rig.state;
        rig.dom
            .in_runtime(|| crate::services::confirm::network_delete_model(&state, &id))
            .expect("the dialog opens for an unused network")
    };
    assert_eq!(model.title, "Delete network berthfx-net?");
    assert_eq!(model.targets, vec![Subject::Network(id.clone())]);
    assert_eq!(
        crate::services::confirm::button_label(&model.effect),
        "Delete network"
    );
    assert_eq!(
        model.action,
        berth_core::EngineAction::RemoveNetwork {
            engine: default_engine(),
            id: id.clone()
        }
    );
    let launched = {
        let service = rig.service.clone();
        let mut state = rig.state;
        rig.dom
            .in_runtime(|| crate::services::confirm::run(&mut state, &service, &model, false))
    };
    assert!(launched.refused.is_empty(), "{:?}", launched.refused);
    assert_eq!(launched.keys.len(), 1);
    rig.settle().await;
    assert!(rig
        .engine
        .networks()
        .await
        .expect("networks")
        .iter()
        .all(|n| n.name != "berthfx-net"));
    assert_eq!(
        crate::services::confirm::result_label(&model.effect, 1),
        "Deleted 1 network"
    );
    // A delete says its result once: the dialog's, not the service's.
    assert!(!rig.toasts().iter().any(|t| t.contains("Deleted network")));
}

#[tokio::test]
async fn networks_every_action_writes_an_activity_entry() {
    let mut rig = Rig::new();
    rig.run(
        "berthfx-net",
        "berthfx-net",
        NetOp::Create(spec("berthfx-net")),
    )
    .expect("accepted");
    rig.settle().await;
    let entries = rig
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    let entry = entries
        .iter()
        .find(|e| e.action == "network.create")
        .expect("the create is recorded");
    assert_eq!(entry.subject, Subject::Network("berthfx-net".to_string()));
    assert_eq!(entry.subject_name, "berthfx-net");
    assert_eq!(entry.outcome, berth_core::Outcome::Ok);
}

#[tokio::test]
async fn networks_built_in_cannot_be_deleted_and_nothing_is_set_pending() {
    let mut rig = Rig::new();
    let bridge = rig.network_id("bridge");
    let refused = rig.run(&bridge, "bridge", NetOp::Remove { id: bridge.clone() });
    assert_eq!(
        refused,
        Err("Built-in networks cannot be removed".to_string())
    );
    assert!(rig.pending().is_empty());
    // No dialog is built for it either, and the engine still has it.
    let state = rig.state;
    assert!(rig
        .dom
        .in_runtime(|| crate::services::confirm::network_delete_model(&state, &bridge))
        .is_none());
    assert!(rig
        .engine
        .networks()
        .await
        .expect("networks")
        .iter()
        .any(|n| n.name == "bridge"));
}

#[tokio::test]
async fn networks_delete_is_refused_while_containers_are_on_it() {
    let mut rig = Rig::new();
    let shop = rig.network_id("shop-api_default");
    let refused = rig.run(
        &shop,
        "shop-api_default",
        NetOp::Remove { id: shop.clone() },
    );
    let reason = refused.expect_err("in use");
    assert!(reason.ends_with("connected"), "{reason}");
    assert!(rig.pending().is_empty());
}

#[tokio::test]
async fn networks_create_with_a_taken_name_is_refused_before_the_engine() {
    let mut rig = Rig::new();
    let refused = rig.run(
        "shop-api_default",
        "shop-api_default",
        NetOp::Create(spec("shop-api_default")),
    );
    assert_eq!(
        refused,
        Err("A network named shop-api_default already exists".to_string())
    );
    assert!(rig.pending().is_empty());
}

#[tokio::test]
async fn networks_an_engine_failure_shows_on_the_network_and_dismisses() {
    let mut rig = Rig::new();
    // The list does not know the name yet (a stale list), the engine does.
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state
            .networks
            .write()
            .retain(|n| n.name != "analytics_default");
    });
    rig.run(
        "analytics_default",
        "analytics_default",
        NetOp::Create(spec("analytics_default")),
    )
    .expect("accepted");
    let key = rig.settle().await;
    let pending = rig.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, key);
    assert!(
        matches!(&pending[0].1, PendingState::Failed { message } if message.contains("already exists")),
        "{pending:?}"
    );
    let mut state = rig.state;
    rig.dom
        .in_runtime(|| crate::services::actions::dismiss(&mut state, &key));
    assert!(rig.pending().is_empty());
}

#[tokio::test]
async fn networks_run_net_stops_when_cancelled() {
    let engine = berth_engine::MockEngine::bundled(&default_engine());
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = run_net(&engine, "web", NetOp::Create(spec("web")), &cancel).await;
    assert_eq!(result, Err("cancelled".to_string()));
    assert!(engine
        .networks()
        .await
        .expect("networks")
        .iter()
        .all(|n| n.name != "web"));
}
