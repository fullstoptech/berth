use super::*;
use std::collections::BTreeMap;

use berth_core::{Container, ContainerId, EngineId, Subject};

use crate::services::actions::StateView;

/// The bundled mock world, owned, so tests can borrow a `StateView` of it.
struct World {
    engines: Vec<berth_core::Engine>,
    containers: Vec<Container>,
    projects: Vec<berth_core::Project>,
    images: Vec<berth_core::Image>,
    volumes: Vec<berth_core::Volume>,
    networks: Vec<berth_core::Network>,
    pending: BTreeMap<crate::state::PendingKey, crate::state::PendingState>,
}

fn mock() -> World {
    let w = berth_engine::MockWorld::bundled();
    World {
        engines: w.engines.clone(),
        containers: w.containers.clone(),
        projects: w.projects.clone(),
        images: w.images_by_engine.values().flatten().cloned().collect(),
        volumes: w.volumes_by_engine.values().flatten().cloned().collect(),
        networks: w.networks_by_engine.values().flatten().cloned().collect(),
        pending: BTreeMap::new(),
    }
}

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn scope_default() -> EngineScope {
    EngineScope::One(default_engine())
}

fn index(world: &World, selection: &Selection, scope: &EngineScope) -> Index {
    Index::build(
        &StateView {
            engines: &world.engines,
            containers: &world.containers,
            projects: &world.projects,
            images: &world.images,
            volumes: &world.volumes,
            networks: &world.networks,
            pending: &world.pending,
        },
        selection,
        scope,
    )
}

fn id_of(world: &World, name: &str) -> ContainerId {
    world
        .containers
        .iter()
        .find(|c| c.name == name)
        .map(|c| c.id.clone())
        .unwrap_or_else(|| panic!("mock container {name}"))
}

fn titles(results: &Results) -> Vec<String> {
    results.flat().iter().map(|i| i.title.clone()).collect()
}

fn group_titles(results: &Results) -> Vec<&'static str> {
    results.groups.iter().map(|g| g.group.title()).collect()
}

fn selecting_worker(world: &World) -> Selection {
    Selection::Containers(vec![id_of(world, "shop-api-worker-1")])
}

// ---------------------------------------------------------------------
// The done-when queries
// ---------------------------------------------------------------------

#[test]
fn search_rest_ranks_restart_shop_api_worker_first_in_the_mock_world() {
    // No selection: the restarting container is the one that most needs a
    // restart, so it wins the bare verb.
    let world = mock();
    let results = index(&world, &Selection::None, &scope_default()).search("rest", &[]);
    let first = results.flat().into_iter().next().expect("a result");
    assert_eq!(
        first.title,
        "Restart shop-api-worker-1",
        "{:?}",
        titles(&results)
    );
    assert_eq!(first.group, Group::Containers);
}

#[test]
fn search_rest_with_the_worker_selected_matches_the_palette_frame() {
    let world = mock();
    let results = index(&world, &selecting_worker(&world), &scope_default()).search("rest", &[]);
    assert_eq!(group_titles(&results), ["Containers", "Projects", "Engine"]);
    let items: Vec<(&str, &str, Option<&str>)> = results
        .flat()
        .iter()
        .map(|i| (i.title.as_str(), i.meta.as_str(), i.shortcut))
        .collect();
    assert_eq!(
        items,
        [
            (
                "Restart shop-api-worker-1",
                "Container · Restarting",
                Some("⌘R")
            ),
            ("Restart project shop-api", "5 containers", None),
            (
                "Restart engine default…",
                "Asks to confirm · stops 15 containers",
                None
            ),
        ]
    );
    assert!(results.flat()[2].confirm);
    assert!(results.docker_command.is_none());
}

#[test]
fn search_port_query_finds_pg_scratch_and_shop_api_db() {
    let world = mock();
    let results = index(&world, &Selection::None, &scope_default()).search(":5432", &[]);
    let names = titles(&results);
    assert!(names.contains(&"pg-scratch".to_string()), "{names:?}");
    assert!(names.contains(&"shop-api-db-1".to_string()), "{names:?}");
    // Nothing that does not publish 5432.
    assert!(!names.iter().any(|n| n.contains("grafana")), "{names:?}");
    assert_eq!(results.groups.len(), 1, "{names:?}");
    assert_eq!(results.groups[0].group, Group::Containers);
    let pg = results
        .flat()
        .into_iter()
        .find(|i| i.title == "pg-scratch")
        .expect("pg");
    assert!(pg.meta.contains(":5432"), "{}", pg.meta);
}

#[test]
fn search_exact_port_outranks_a_prefix_of_another_port() {
    let mut world = mock();
    let port = |host: u16| berth_core::PortMapping {
        host_ip: None,
        host_port: Some(host),
        container_port: host,
        proto: berth_core::Proto::Tcp,
    };
    // 8000 only starts with 80; the mock's other containers publish neither.
    for c in &mut world.containers {
        c.ports.clear();
    }
    world.containers[0].ports = vec![port(8000)];
    world.containers[1].ports = vec![port(80)];
    let idx = index(&world, &Selection::None, &scope_default());
    let names = titles(&idx.search(":80", &[]));
    assert_eq!(
        names,
        [
            world.containers[1].name.clone(),
            world.containers[0].name.clone()
        ]
    );
    // A bare number of two digits or more is a port query as well.
    assert_eq!(
        titles(&idx.search("8000", &[])),
        [world.containers[0].name.clone()]
    );
}

#[test]
fn search_abbreviations_with_a_small_gap_still_match() {
    let world = mock();
    let idx = index(&world, &Selection::None, &scope_default());
    // "shp" skips one letter of "shop-api"; "pgs" one of "pg-scratch".
    assert!(titles(&idx.search("shp", &[])).contains(&"shop-api".to_string()));
    assert_eq!(titles(&idx.search("pgs", &[]))[0], "pg-scratch");
}

#[test]
fn search_restart_with_a_name_acts_on_that_container() {
    let world = mock();
    let results =
        index(&world, &Selection::None, &scope_default()).search("restart berth-site-web", &[]);
    let first = results.flat().into_iter().next().expect("a result");
    assert_eq!(first.title, "Restart berth-site-web-1");
    assert_eq!(
        first.target,
        Target::Run {
            action: "container.restart",
            subject: Some(Subject::Container(id_of(&world, "berth-site-web-1"))),
        }
    );
    // A name given on its own selects, so the shortcut is not claimed.
    assert_eq!(first.shortcut, None);
    // The verb-less "restart" subject-order also works.
    let swapped =
        index(&world, &Selection::None, &scope_default()).search("berth-site-web restart", &[]);
    assert_eq!(swapped.flat()[0].title, "Restart berth-site-web-1");
}

// ---------------------------------------------------------------------
// Matching rules
// ---------------------------------------------------------------------

#[test]
fn search_matches_names_images_folders_and_projects() {
    let world = mock();
    let idx = index(&world, &Selection::None, &scope_default());
    // Image reference.
    let by_image = titles(&idx.search("postgres", &[]));
    assert!(by_image.contains(&"pg-scratch".to_string()), "{by_image:?}");
    // Project folder (the folder name is not in any container's name).
    let by_folder = titles(&idx.search("clients/acme", &[]));
    assert!(
        by_folder.contains(&"shop-api-api-1".to_string()),
        "{by_folder:?}"
    );
    assert!(by_folder.contains(&"shop-api".to_string()), "{by_folder:?}");
    // A name outranks the same text found only in a secondary field.
    let ranked = idx.search("shop-api", &[]);
    let flat = ranked.flat();
    let project_pos = flat
        .iter()
        .position(|i| i.title == "shop-api")
        .expect("project");
    assert!(project_pos < 7, "{:?}", titles(&ranked));
}

#[test]
fn search_scattered_letters_do_not_match() {
    // "rest" is a subsequence of the distroless image reference, but
    // spread over it; it must not produce an image or container result.
    let world = mock();
    let results = index(&world, &Selection::None, &EngineScope::All).search("rest", &[]);
    let names = titles(&results);
    assert!(!names.iter().any(|n| n.contains("distroless")), "{names:?}");
    assert!(
        !names.contains(&"metrics-exporter-1".to_string()),
        "{names:?}"
    );
}

#[test]
fn search_scope_one_hides_other_engines_and_all_reaches_them() {
    let world = mock();
    let one = index(&world, &Selection::None, &scope_default()).search("backup", &[]);
    assert!(one.is_empty(), "{:?}", titles(&one));
    let all = index(&world, &Selection::None, &EngineScope::All).search("backup", &[]);
    let item = all
        .flat()
        .into_iter()
        .find(|i| i.title == "backup-1")
        .expect("remote");
    // Other engines are named so results cannot be confused.
    assert!(item.meta.contains("hetzner-1"), "{}", item.meta);
}

#[test]
fn search_no_match_offers_the_docker_command_row() {
    let world = mock();
    let results = index(&world, &Selection::None, &scope_default()).search("zzzqqq", &[]);
    assert!(results.is_empty());
    assert_eq!(results.docker_command.as_deref(), Some("zzzqqq"));
    // With a match there is no fallback row.
    let hit = index(&world, &Selection::None, &scope_default()).search("pg-scratch", &[]);
    assert!(hit.docker_command.is_none());
}

#[test]
fn search_verbs_follow_container_state() {
    let world = mock();
    let idx = index(&world, &Selection::None, &scope_default());
    // Only the paused container can resume.
    let resume = idx.search("resume", &[]);
    assert_eq!(resume.flat()[0].title, "Resume api-worker-7f9c4d8b6-xk2lp");
    // Nothing is offered to pause an exited container by name.
    let pause = idx.search("pause eager_hopper", &[]);
    assert!(
        !titles(&pause).iter().any(|t| t == "Pause eager_hopper"),
        "{:?}",
        titles(&pause)
    );
    // Start by name finds the exited one but not a running one.
    let start = titles(&idx.search("start eager", &[]));
    assert!(
        start.contains(&"Start eager_hopper".to_string()),
        "{start:?}"
    );
    let start_running = titles(&idx.search("start pg-scratch", &[]));
    assert!(
        !start_running.contains(&"Start pg-scratch".to_string()),
        "{start_running:?}"
    );
}

#[test]
fn search_containers_of_a_stopped_engine_get_no_verbs() {
    let world = mock();
    let idx = index(&world, &Selection::None, &EngineScope::All);
    let results = idx.search("start crm-api", &[]);
    assert!(
        !titles(&results)
            .iter()
            .any(|t| t.starts_with("Start crm-api")),
        "{:?}",
        titles(&results)
    );
    // But the record still opens.
    assert!(titles(&idx.search("crm-api", &[])).contains(&"crm-api-1".to_string()));
}

#[test]
fn search_kill_and_delete_ask_to_confirm() {
    let world = mock();
    let idx = index(&world, &Selection::None, &scope_default());
    for (query, title) in [
        ("kill pg-scratch", "Kill pg-scratch…"),
        ("delete eager_hopper", "Delete eager_hopper…"),
    ] {
        let first = idx.search(query, &[]).flat()[0].clone();
        assert_eq!(first.title, title);
        assert!(first.confirm && first.danger, "{first:?}");
        assert!(first.meta.contains("Asks to confirm"), "{}", first.meta);
    }
    // Restart does not ask.
    assert!(!idx.search("restart pg-scratch", &[]).flat()[0].confirm);
}

#[test]
fn search_project_verbs_name_the_project() {
    let world = mock();
    let idx = index(&world, &Selection::None, &scope_default());
    let results = idx.search("stop project analytics", &[]);
    assert_eq!(results.flat()[0].title, "Stop project analytics");
    assert_eq!(results.groups[0].group, Group::Projects);
    // An all-running project cannot be started again.
    let start = idx.search("start project berth-site", &[]);
    assert!(
        !titles(&start).contains(&"Start project berth-site".to_string()),
        "{:?}",
        titles(&start)
    );
}

#[test]
fn search_global_actions_carry_their_shortcut() {
    let world = mock();
    let idx = index(&world, &Selection::None, &scope_default());
    let results = idx.search("settings", &[]);
    let first = results.flat()[0].clone();
    assert_eq!(first.title, "Settings");
    assert_eq!(first.group, Group::Actions);
    assert_eq!(first.shortcut, Some("⌘,"));
    assert_eq!(
        first.target,
        Target::Run {
            action: "global.settings",
            subject: None
        }
    );
    // Sections read "Go to X".
    assert_eq!(idx.search("go images", &[]).flat()[0].title, "Go to Images");
}

#[test]
fn search_every_palette_target_names_a_real_action_def() {
    for (id, _, _) in GLOBALS {
        assert!(action_by_id(id).is_some(), "{id} is not an ActionDef");
    }
    for verb in CONTAINER_VERBS
        .iter()
        .chain(PROJECT_VERBS)
        .chain(ENGINE_VERBS)
    {
        assert!(
            action_by_id(verb.id).is_some(),
            "{} is not an ActionDef",
            verb.id
        );
    }
}

#[test]
fn search_group_cap_keeps_the_list_short() {
    let world = mock();
    let results = index(&world, &Selection::None, &EngineScope::All).search("a", &[]);
    for group in &results.groups {
        assert!(group.items.len() <= GROUP_CAP, "{:?}", group.group);
    }
}

// ---------------------------------------------------------------------
// Empty query and recents
// ---------------------------------------------------------------------

#[test]
fn palette_empty_query_lists_recent_then_actions() {
    let world = mock();
    let idx = index(&world, &Selection::None, &scope_default());
    let worker = Subject::Container(id_of(&world, "shop-api-worker-1"));
    let recents = vec![
        RecentKey {
            action: "container.restart".to_string(),
            subject: Some(worker.clone()),
        },
        RecentKey {
            action: "open".to_string(),
            subject: Some(Subject::Volume("shop-api_pgdata".to_string())),
        },
        // A container that no longer exists is dropped.
        RecentKey {
            action: "container.restart".to_string(),
            subject: Some(Subject::Container(ContainerId("gone".to_string()))),
        },
    ];
    let results = idx.search("  ", &recents);
    assert_eq!(group_titles(&results), ["Recent", "Actions"]);
    assert_eq!(
        titles(&results)[..2],
        ["Restart shop-api-worker-1", "shop-api_pgdata"]
    );
    assert_eq!(results.groups[0].items.len(), 2);
    assert_eq!(results.groups[1].items[0].title, "Open folder…");
    assert!(results.docker_command.is_none());
}

#[test]
fn palette_remember_moves_to_front_once_and_caps() {
    let key = |n: usize| RecentKey {
        action: "open".to_string(),
        subject: Some(Subject::Volume(format!("v{n}"))),
    };
    let mut list = Vec::new();
    for n in 0..30 {
        remember(&mut list, key(n));
    }
    remember(&mut list, key(25));
    assert_eq!(list[0], key(25));
    assert_eq!(list.iter().filter(|k| **k == key(25)).count(), 1);
    assert_eq!(list.len(), RECENT_KEEP);
}

#[test]
fn palette_recents_from_activity_keep_berth_successes_only() {
    let entry = |origin, action: &str, outcome| berth_core::ActivityEntry {
        id: uuid::Uuid::nil(),
        ts: chrono::DateTime::from_timestamp(1, 0).unwrap_or_default(),
        engine: default_engine(),
        actor: berth_core::Actor::Local,
        origin,
        action: action.to_string(),
        subject: Subject::Container(ContainerId("c1".to_string())),
        subject_name: "c1".to_string(),
        project: None,
        outcome,
    };
    use berth_core::{ActivityOrigin as O, Outcome};
    let entries = vec![
        entry(O::Berth, "container.restart", Outcome::Ok),
        entry(O::Engine, "container.die", Outcome::Ok),
        entry(
            O::Berth,
            "container.stop",
            Outcome::Failed {
                reason: "no".to_string(),
            },
        ),
        entry(O::Berth, "container.restart", Outcome::Ok),
        entry(O::Berth, "container.start", Outcome::Ok),
    ];
    let keys = recents_from_activity(&entries);
    let actions: Vec<&str> = keys.iter().map(|k| k.action.as_str()).collect();
    assert_eq!(actions, ["container.restart", "container.start"]);
}

// ---------------------------------------------------------------------
// Running an item
// ---------------------------------------------------------------------

mod run {
    use super::*;
    use dioxus::prelude::*;
    use std::sync::Arc;

    thread_local! {
        static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
    }

    #[component]
    fn Harness() -> Element {
        let state = crate::state::use_app_state();
        use_hook(move || {
            SLOT.with(|slot| *slot.borrow_mut() = Some(state));
        });
        rsx! { div {} }
    }

    struct Rig {
        dom: VirtualDom,
        state: AppState,
        service: ActionsService,
        finished: tokio::sync::mpsc::UnboundedReceiver<crate::services::actions::Finished>,
    }

    fn rig() -> Rig {
        let mut dom = VirtualDom::new(Harness);
        let _ = dom.rebuild_to_vec();
        let mut state = SLOT
            .with(|slot| slot.borrow_mut().take())
            .expect("harness captured AppState");
        let world = berth_engine::MockWorld::bundled();
        let clients = crate::services::engines::client_map();
        clients.lock().expect("clients").insert(
            default_engine(),
            Arc::new(berth_engine::MockEngine::bundled(&default_engine()))
                as Arc<dyn berth_engine::EngineClient>,
        );
        let (service, finished) = ActionsService::new(
            clients,
            Arc::new(berth_store::Store::open_in_memory().expect("store")),
            Arc::new(berth_engine::MockCompose::bundled()),
            Arc::new(berth_engine::MockColima::bundled()),
            tokio_util::sync::CancellationToken::new(),
        );
        dom.in_runtime(|| {
            state.engines.set(world.engines.clone());
            state
                .containers
                .set(world.containers_for(&default_engine()));
            state.projects.set(world.projects.clone());
            state.engine_scope.set(scope_default());
        });
        Rig {
            dom,
            state,
            service,
            finished,
        }
    }

    impl Rig {
        fn first_item(&self, query: &str) -> SearchItem {
            self.dom.in_runtime(|| {
                let index = crate::services::actions::with_view(&self.state, |view| {
                    Index::build(view, &Selection::None, &scope_default())
                });
                index
                    .search(query, &[])
                    .flat()
                    .first()
                    .map(|i| (*i).clone())
                    .unwrap_or_else(|| panic!("no result for {query}"))
            })
        }

        fn run(&self, item: &SearchItem, actions: bool) -> RunOutcome {
            let mut state = self.state;
            self.dom
                .in_runtime(|| run(&mut state, actions.then_some(&self.service), item))
        }

        fn pending_actions(&self) -> Vec<&'static str> {
            self.dom
                .in_runtime(|| self.state.pending.peek().keys().map(|k| k.action).collect())
        }
    }

    #[tokio::test]
    async fn search_run_restart_goes_through_the_actions_service() {
        let mut rig = rig();
        let item = rig.first_item("restart pg-scratch");
        assert_eq!(item.title, "Restart pg-scratch");
        assert_eq!(rig.run(&item, true), RunOutcome::Done);
        // Optimistic state is set before the engine answers (D34).
        assert_eq!(rig.pending_actions(), ["container.restart"]);
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), rig.finished.recv())
            .await
            .expect("the action finishes")
            .expect("channel open");
        assert_eq!(done.result, Ok(()));
        let mut state = rig.state;
        rig.dom
            .in_runtime(|| crate::services::actions::apply_finished(&mut state, done));
        assert!(rig.pending_actions().is_empty());
    }

    #[tokio::test]
    async fn search_run_engine_restart_asks_before_acting() {
        let rig = rig();
        let item = rig.first_item("restart engine default");
        assert!(item.confirm, "{item:?}");
        assert_eq!(rig.run(&item, true), RunOutcome::Confirm);
        // Nothing runs; a confirm dialog is on the stack instead.
        assert!(rig.pending_actions().is_empty());
        let overlays = rig.dom.in_runtime(|| rig.state.overlays.peek().clone());
        match overlays.as_slice() {
            [Overlay::Confirm(model)] => {
                assert_eq!(model.title, "Restart engine default?");
                assert_eq!(model.effect.verb, "Restart");
                assert!(
                    matches!(
                        &model.action,
                        berth_core::EngineAction::Colima { profile, op: ColimaOp::Restart }
                            if profile == "default"
                    ),
                    "{:?}",
                    model.action
                );
                assert!(model.effect.lines[0].text.contains("15 running containers"));
            }
            other => panic!("expected one confirm overlay, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_run_refusal_carries_the_reason_and_a_toast() {
        let rig = rig();
        let pg = id_of(
            &World {
                containers: berth_engine::MockWorld::bundled().containers,
                ..mock()
            },
            "pg-scratch",
        );
        // pg-scratch is running, so "Start" is refused by the actions
        // service's own check, not by the palette.
        let item = SearchItem {
            group: Group::Containers,
            icon: "play",
            title: "Start pg-scratch".to_string(),
            meta: String::new(),
            shortcut: None,
            danger: false,
            confirm: false,
            target: Target::Run {
                action: "container.start",
                subject: Some(Subject::Container(pg)),
            },
            score: 0,
        };
        assert_eq!(
            rig.run(&item, true),
            RunOutcome::Refused("Already running".to_string())
        );
        assert!(rig.pending_actions().is_empty());
        let toasts = rig.dom.in_runtime(|| rig.state.toasts.peek().clone());
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].text, "Already running");
    }

    #[tokio::test]
    async fn search_run_open_selects_the_record_and_shows_its_section() {
        let rig = rig();
        let item = rig.first_item("pg-scratch");
        assert!(matches!(item.target, Target::Open(_)), "{item:?}");
        assert_eq!(rig.run(&item, false), RunOutcome::Done);
        let (section, selection) = rig.dom.in_runtime(|| {
            (
                *rig.state.section.peek(),
                rig.state.selection.peek().clone(),
            )
        });
        assert_eq!(section, Section::Containers);
        let world = mock();
        assert_eq!(
            selection,
            Selection::Containers(vec![id_of(&world, "pg-scratch")])
        );
    }

    #[tokio::test]
    async fn search_run_global_action_dispatches_after_the_palette_closed() {
        let rig = rig();
        let item = rig.first_item("go images");
        assert_eq!(rig.run(&item, false), RunOutcome::Done);
        assert_eq!(
            rig.dom.in_runtime(|| *rig.state.section.peek()),
            Section::Images
        );
        // A sheet-opening action leaves exactly that sheet on the stack.
        let create = rig.first_item("create volume");
        assert_eq!(rig.run(&create, false), RunOutcome::Done);
        assert_eq!(
            rig.dom.in_runtime(|| rig.state.overlays.peek().clone()),
            vec![Overlay::CreateVolume]
        );
    }

    #[tokio::test]
    async fn search_run_without_the_actions_service_refuses_instead_of_pretending() {
        let rig = rig();
        let item = rig.first_item("restart pg-scratch");
        assert!(matches!(
            rig.run(&item, false),
            RunOutcome::Refused(reason) if reason == "Actions are not running"
        ));
        assert!(rig.pending_actions().is_empty());
    }
}

// ---------------------------------------------------------------------
// Speed
// ---------------------------------------------------------------------

#[test]
fn search_a_thousand_containers_answers_within_the_ui_budget() {
    let mut world = mock();
    let template = world.containers[0].clone();
    world.containers = (0..1000)
        .map(|n| {
            let mut c = template.clone();
            c.id = ContainerId(format!("id{n:04}"));
            c.name = format!("svc-{n}-web-1");
            c.compose_project = Some(format!("proj-{}", n % 40));
            c
        })
        .collect();
    let idx = index(&world, &Selection::None, &scope_default());
    let _warm = idx.search("rest", &[]);
    let runs = 20u32;
    let started = std::time::Instant::now();
    for query in ["rest", "restart svc-77", "web", "svc-9"]
        .iter()
        .cycle()
        .take(runs as usize)
    {
        let _ = idx.search(query, &[]);
    }
    let per_query = started.elapsed() / runs;
    println!("search over 1000 containers: {per_query:?} per query (debug build)");
    // A debug build runs this about ten times slower than release; the 4 ms
    // UI budget (CLAUDE.md) is checked in release by the printed number.
    assert!(
        per_query < std::time::Duration::from_millis(40),
        "{per_query:?}"
    );
}
