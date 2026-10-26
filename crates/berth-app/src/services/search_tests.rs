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
            backing: crate::services::backing::Backing::default(),
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
    // Every engine is searched, so hetzner-1's `backup-1` (image
    // restic/restic) answers "rest" too. The palette scene shows the
    // default engine's records only, and those are the frame's rows.
    let (elsewhere, here): (Vec<&SearchItem>, Vec<&SearchItem>) = results
        .flat()
        .into_iter()
        .partition(|i| i.meta.contains("hetzner-1"));
    assert_eq!(
        elsewhere
            .iter()
            .map(|i| i.title.as_str())
            .collect::<Vec<_>>(),
        ["backup-1", "restic/restic:latest"]
    );
    let items: Vec<(&str, &str, Option<&str>)> = here
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
    assert!(here[2].confirm);
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

/// `name` cloned onto `engine` under a new id, listed before the original
/// so that a tie would put it first.
fn clone_to_engine(world: &mut World, name: &str, engine: &str) -> ContainerId {
    let mut copy = world
        .containers
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("mock container {name}"))
        .clone();
    copy.id = ContainerId(format!("{engine}-{}", copy.id.0));
    copy.engine = EngineId(engine.to_string());
    let id = copy.id.clone();
    world.containers.insert(0, copy);
    id
}

fn is_tagged(item: &SearchItem, engine: &str) -> bool {
    item.meta.ends_with(&format!(" · {engine}"))
}

#[test]
fn search_with_default_selected_still_finds_other_engines_and_tags_them() {
    let world = mock();
    let one = index(&world, &Selection::None, &scope_default()).search("backup", &[]);
    let item = one
        .flat()
        .into_iter()
        .find(|i| i.title == "backup-1")
        .unwrap_or_else(|| panic!("remote container hidden: {:?}", titles(&one)));
    assert!(is_tagged(item, "hetzner-1"), "{}", item.meta);
    // Under "All engines" it is tagged as before.
    let all = index(&world, &Selection::None, &EngineScope::All).search("backup", &[]);
    let item = all
        .flat()
        .into_iter()
        .find(|i| i.title == "backup-1")
        .expect("remote");
    assert!(is_tagged(item, "hetzner-1"), "{}", item.meta);
}

#[test]
fn search_selected_engine_ranks_above_an_equal_match_on_another_engine() {
    let mut world = mock();
    let on_work = clone_to_engine(&mut world, "shop-api-db-1", "colima:work");
    let results = index(&world, &Selection::None, &scope_default()).search("shop-api-db", &[]);
    let items: Vec<&SearchItem> = results
        .flat()
        .into_iter()
        .filter(|i| i.title == "shop-api-db-1")
        .collect();
    assert_eq!(items.len(), 2, "{:?}", titles(&results));
    // Default first and untagged; work second, tagged with the engine name.
    assert!(!items[0].meta.contains("work"), "{}", items[0].meta);
    assert_eq!(items[1].target, Target::Open(Subject::Container(on_work)));
    assert!(is_tagged(items[1], "work"), "{}", items[1].meta);
    assert_eq!(items[0].score - items[1].score, HOME_BONUS);
}

#[test]
fn search_all_engines_scope_prefers_no_engine_and_tags_every_result() {
    let mut world = mock();
    let on_work = clone_to_engine(&mut world, "shop-api-db-1", "colima:work");
    let results = index(&world, &Selection::None, &EngineScope::All).search("shop-api-db", &[]);
    let items: Vec<&SearchItem> = results
        .flat()
        .into_iter()
        .filter(|i| i.title == "shop-api-db-1")
        .collect();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].score, items[1].score);
    // The clone was listed first, so with no preference it stays first.
    assert_eq!(
        items[0].target,
        Target::Open(Subject::Container(on_work.clone()))
    );
    assert!(is_tagged(items[0], "work"), "{}", items[0].meta);
    assert!(is_tagged(items[1], "default"), "{}", items[1].meta);
}

#[test]
fn search_bare_verb_acts_on_the_selected_engine_first() {
    // Two restarting workers, the other engine's listed first: the bare
    // verb must still pick the selected engine's.
    let mut world = mock();
    world
        .engines
        .iter_mut()
        .find(|e| e.id.0 == "colima:work")
        .expect("work")
        .state = berth_core::EngineState::Running;
    let default_worker = id_of(&world, "shop-api-worker-1");
    clone_to_engine(&mut world, "shop-api-worker-1", "colima:work");
    let results = index(&world, &Selection::None, &scope_default()).search("rest", &[]);
    let first = results.flat().into_iter().next().expect("a result");
    assert_eq!(first.title, "Restart shop-api-worker-1");
    assert_eq!(
        first.target,
        Target::Run {
            action: "container.restart",
            subject: Some(Subject::Container(default_worker)),
        }
    );
    assert!(!first.meta.contains("work"), "{}", first.meta);
}

#[test]
fn search_port_query_lists_other_engines_after_the_selected_one() {
    let mut world = mock();
    let on_work = clone_to_engine(&mut world, "pg-scratch", "colima:work");
    let results = index(&world, &Selection::None, &scope_default()).search(":5432", &[]);
    let flat = results.flat();
    let titles = titles(&results);
    assert!(titles.contains(&"shop-api-db-1".to_string()), "{titles:?}");
    let work_pos = flat
        .iter()
        .position(|i| i.target == Target::Open(Subject::Container(on_work.clone())))
        .expect("work's pg-scratch");
    let default_pos = flat
        .iter()
        .position(|i| i.title == "pg-scratch" && !is_tagged(i, "work"))
        .expect("default's pg-scratch");
    assert!(default_pos < work_pos, "{titles:?}");
    assert!(is_tagged(flat[work_pos], "work"));
}

#[test]
fn search_images_follow_the_engines_of_their_users() {
    let world = mock();
    // restic/restic is used by hetzner-1's backup-1 only.
    let one = index(&world, &Selection::None, &scope_default()).search("restic", &[]);
    let image = one
        .flat()
        .into_iter()
        .find(|i| i.title == "restic/restic:latest")
        .expect("image searchable from another engine");
    assert!(is_tagged(image, "hetzner-1"), "{}", image.meta);
    // An image the selected engine uses carries no tag.
    let postgres = index(&world, &Selection::None, &scope_default()).search("postgres", &[]);
    let image = postgres
        .flat()
        .into_iter()
        .find(|i| i.group == Group::Images)
        .expect("postgres image");
    assert!(!image.meta.contains("hetzner"), "{}", image.meta);
    // Under "All engines" images keep today's rule: no tag.
    let all = index(&world, &Selection::None, &EngineScope::All).search("restic", &[]);
    let image = all
        .flat()
        .into_iter()
        .find(|i| i.title == "restic/restic:latest")
        .expect("image");
    assert!(!image.meta.contains("hetzner"), "{}", image.meta);
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
fn search_palette_lists_the_text_size_actions_with_their_shortcuts() {
    let world = mock();
    let idx = index(&world, &Selection::None, &scope_default());
    for (query, title, id, shortcut) in [
        ("bigger text", "Bigger text", "view.text_bigger", "⌘="),
        ("smaller text", "Smaller text", "view.text_smaller", "⌘-"),
        ("actual text", "Actual text size", "view.text_reset", "⌘0"),
    ] {
        let first = idx.search(query, &[]).flat()[0].clone();
        assert_eq!(first.title, title, "{query}");
        assert_eq!(first.shortcut, Some(shortcut), "{query}");
        assert_eq!(
            first.target,
            Target::Run {
                action: id,
                subject: None
            }
        );
    }
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
fn a11y_keyboard_only_verbs_find_their_subjects() {
    let world = mock();
    let idx = index(&world, &Selection::None, &scope_default());
    for (query, action, subject_is) in [
        ("terminal api", "container.terminal", "container"),
        ("edit and recreate api", "container.edit", "container"),
        ("open folder of shop", "project.open_folder", "project"),
        ("open in editor shop", "project.open_in_editor", "project"),
        ("delete project shop", "project.delete", "project"),
        (
            "edit resources of default",
            "engine.edit_resources",
            "engine",
        ),
        ("clean up default", "engine.clean_up", "engine"),
    ] {
        let results = idx.search(query, &[]);
        let found = results.flat().into_iter().find(|item| {
            matches!(
                &item.target,
                Target::Run { action: a, subject: Some(s) }
                    if *a == action
                        && matches!(
                            (subject_is, s),
                            ("container", Subject::Container(_))
                                | ("project", Subject::Project(_))
                                | ("engine", Subject::Engine(_))
                        )
            )
        });
        assert!(found.is_some(), "{query:?} finds no {action} item");
    }
}

/// Every `ActionDef` the palette does not list, with the other way in. A new
/// action must either join `GLOBALS` or a verb table, or be named here: the
/// test below fails on both a missing entry and a stale one (P6-T4).
const NOT_IN_PALETTE: &[(&str, &str)] = &[
    ("global.palette", "the palette itself, ⌘K"),
    ("global.logs_section", "⌘⇧L; Go to Logs is listed"),
    (
        "global.focus_filter",
        "⌘F has no handler yet; Tab reaches each list's filter",
    ),
    (
        "global.select_next",
        "J moves the list selection; keyboard only",
    ),
    (
        "global.select_prev",
        "K moves the list selection; keyboard only",
    ),
    ("overlay.close", "Esc"),
    ("palette.next", "the palette's own arrow keys"),
    ("palette.prev", "the palette's own arrow keys"),
    ("palette.run", "the palette's own Return"),
    ("folders.toggle_hidden", "⌘⇧. inside the Open folder sheet"),
    (
        "container.start_stop",
        "⌘.; the palette lists Start and Stop by state",
    ),
    (
        "project.start_stop",
        "⌘.; the palette lists Start and Stop by state",
    ),
    (
        "engine.start_stop",
        "the palette lists Start and Stop by state",
    ),
    (
        "container.duplicate",
        "container ⋯ menu; no palette route yet (open question)",
    ),
    (
        "container.recreate_latest",
        "container ⋯ menu; no palette route yet (open question)",
    ),
    (
        "container.rename",
        "container ⋯ menu; no palette route yet (open question)",
    ),
    (
        "container.save_as_image",
        "container ⋯ menu; no palette route yet (open question)",
    ),
    ("bulk.start", "the bulk bar over a multi-selection"),
    ("bulk.stop", "the bulk bar over a multi-selection"),
    ("bulk.restart", "the bulk bar over a multi-selection"),
    ("bulk.pause", "the bulk bar over a multi-selection"),
    ("bulk.delete", "the bulk bar over a multi-selection"),
    (
        "project.logs",
        "the Logs button of a project row; no handler yet (open question)",
    ),
    (
        "image.run",
        "image row buttons; no palette route yet (open question)",
    ),
    ("image.pull", "the Pull image form on Images"),
    (
        "image.pull_latest",
        "image row buttons; no palette route yet (open question)",
    ),
    (
        "image.tag",
        "image detail; no palette route yet (open question)",
    ),
    (
        "image.push",
        "image detail; no palette route yet (open question)",
    ),
    (
        "image.delete",
        "image row buttons and ⌫; no palette route yet (open question)",
    ),
    (
        "volume.browse",
        "volume row buttons; no palette route yet (open question)",
    ),
    (
        "volume.backup",
        "volume detail; no palette route yet (open question)",
    ),
    (
        "volume.delete",
        "volume row buttons and ⌫; no palette route yet (open question)",
    ),
    (
        "network.connect_container",
        "network detail; no palette route yet (open question)",
    ),
    (
        "network.disconnect",
        "network detail; no palette route yet (open question)",
    ),
    (
        "network.delete",
        "network row buttons and ⌫; no palette route yet (open question)",
    ),
    ("logs.next_error", "F8 inside the log view"),
    ("logs.prev_error", "⇧F8 inside the log view"),
    ("logs.follow", "the log toolbar"),
    ("logs.explain", "the log toolbar"),
    ("logs.ask", "the log toolbar"),
    ("logs.copy", "the log toolbar"),
    ("logs.save", "the log toolbar"),
    ("assistant.stop_turn", "⌘. inside the Assistant tab (D77)"),
    (
        "assistant.approve_tool",
        "the approval card in the Assistant tab",
    ),
    (
        "assistant.deny_tool",
        "the approval card in the Assistant tab",
    ),
];

#[test]
fn a11y_every_action_is_in_the_palette_or_named_with_its_other_way_in() {
    let listed: Vec<ActionId> = GLOBALS
        .iter()
        .map(|(id, _, _)| *id)
        .chain(CONTAINER_VERBS.iter().map(|v| v.id))
        .chain(PROJECT_VERBS.iter().map(|v| v.id))
        .chain(ENGINE_VERBS.iter().map(|v| v.id))
        .collect();
    let mut missing = Vec::new();
    for action in crate::actions::actions() {
        let exempt = NOT_IN_PALETTE.iter().any(|(id, _)| *id == action.id);
        if !listed.contains(&action.id) && !exempt {
            missing.push(action.id);
        }
    }
    assert!(
        missing.is_empty(),
        "not in the palette and not named in NOT_IN_PALETTE: {missing:?}"
    );
    for (id, way) in NOT_IN_PALETTE {
        assert!(action_by_id(id).is_some(), "{id} is not an ActionDef");
        assert!(!listed.contains(id), "{id} is in the palette now: drop it");
        assert!(!way.is_empty(), "{id} needs its other way in");
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

/// A world of `n` containers over 40 projects.
fn crowded(n: usize) -> World {
    let mut world = mock();
    let template = world.containers[0].clone();
    world.containers = (0..n)
        .map(|i| {
            let mut c = template.clone();
            c.id = ContainerId(format!("id{i:04}"));
            c.name = format!("svc-{i}-web-1");
            c.compose_project = Some(format!("proj-{}", i % 40));
            c
        })
        .collect();
    world
}

/// How many haystacks one query scored.
fn scored_by(idx: &Index, query: &str) -> usize {
    let before = with_matching(|m| m.scored);
    let _ = idx.search(query, &[]);
    with_matching(|m| m.scored) - before
}

const SPEED_QUERIES: [&str; 4] = ["rest", "restart svc-77", "web", "svc-9"];

/// The 4 ms UI budget (CLAUDE.md) is held by doing a bounded amount of work
/// per query; the wall-clock check is `perf_search_*` (scripts/perf.sh).
/// Each entry is scored a bounded number of times per atom (its name for the
/// verbs, then its name and secondary fields for the record), so the work
/// grows with the number of entries and never faster.
#[test]
fn search_a_thousand_containers_scores_each_entry_a_bounded_number_of_times() {
    let world = crowded(1000);
    let idx = index(&world, &Selection::None, &scope_default());
    assert!(
        idx.entries.len() >= 1000,
        "{} entries indexed",
        idx.entries.len()
    );
    let per_atom_bound: usize = idx.entries.iter().map(|e| 2 + e.secondary.len()).sum();
    // Globals and verb labels are scored per atom too, whatever the world.
    let fixed_bound = 4 * (GLOBALS.len() + 3 * 40);
    for query in SPEED_QUERIES {
        let atoms = parse_atoms(query).len();
        let scored = scored_by(&idx, query);
        assert!(
            scored >= idx.entries.len(),
            "{query:?}: {scored} haystacks scored for {} entries",
            idx.entries.len()
        );
        assert!(
            scored <= atoms * (per_atom_bound + fixed_bound),
            "{query:?}: {scored} haystacks scored, over {atoms} x ({per_atom_bound} + {fixed_bound})"
        );
    }
}

#[test]
fn search_work_grows_in_step_with_the_number_of_containers() {
    let small = index(&crowded(500), &Selection::None, &scope_default());
    let large = index(&crowded(1000), &Selection::None, &scope_default());
    for query in SPEED_QUERIES {
        let extra = scored_by(&large, query) - scored_by(&small, query);
        let atoms = parse_atoms(query).len();
        // 500 more entries, each scored at most (2 + secondary) times per
        // atom: a quadratic search would blow far past this.
        let secondary = large.entries.iter().map(|e| e.secondary.len()).max();
        let bound = 500 * atoms * (2 + secondary.unwrap_or(0));
        assert!(
            extra <= bound,
            "{query:?}: {extra} more haystacks scored for 500 more containers, bound {bound}"
        );
        assert!(
            extra >= 500,
            "{query:?}: only {extra} more haystacks scored"
        );
    }
}

#[test]
#[ignore = "timing; run by scripts/perf.sh"]
fn perf_search_a_thousand_containers_answers_within_the_ui_budget() {
    let world = crowded(1000);
    let idx = index(&world, &Selection::None, &scope_default());
    let _warm = idx.search("rest", &[]);
    let runs = 20u32;
    let started = std::time::Instant::now();
    for query in SPEED_QUERIES.iter().cycle().take(runs as usize) {
        let _ = idx.search(query, &[]);
    }
    let per_query = started.elapsed() / runs;
    println!("search over 1000 containers: {per_query:?} per query");
    // The 4 ms UI budget (CLAUDE.md), in release; a debug build is about ten
    // times slower.
    let budget = if cfg!(debug_assertions) { 40 } else { 4 };
    assert!(
        per_query < std::time::Duration::from_millis(budget),
        "{per_query:?}"
    );
}
