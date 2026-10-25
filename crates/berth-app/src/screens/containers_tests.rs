//! P2-T7 tests: the five frame scenes render what the frames show from the
//! mock world, the view model groups, filters and sorts, the row buttons
//! carry the tooltips and disabled reasons of `actions.rs`, and J/K/Space
//! move real state.

use super::*;

use crate::scenes;
use crate::state::use_app_state;

fn engine_id(s: &str) -> berth_core::EngineId {
    berth_core::EngineId(s.to_string())
}

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn default_containers(world: &berth_engine::MockWorld) -> Vec<Container> {
    world
        .containers
        .iter()
        .filter(|c| c.engine == engine_id("colima:default"))
        .cloned()
        .collect()
}

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).expect("the frame has a scene")
}

/// Drop every `<style>` block so class names in assertions mean elements,
/// not rules of the screen's own stylesheet.
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
    strip_styles(&scene(frame))
}

fn count(html: &str, needle: &str) -> usize {
    html.matches(needle).count()
}

#[allow(clippy::too_many_arguments)]
fn input<'a>(
    world: &'a berth_engine::MockWorld,
    containers: &'a [Container],
    filters: &'a Filters,
    group_by: GroupBy,
    sort: Sort,
    collapsed: &'a BTreeMap<String, bool>,
    pending: &'a BTreeMap<PendingKey, PendingState>,
    scope: &'a EngineScope,
) -> ListInput<'a> {
    ListInput {
        containers,
        projects: &world.projects,
        engines: &world.engines,
        scope,
        filters,
        group_by,
        sort,
        collapsed,
        pending,
    }
}

fn model(filters: &Filters, group_by: GroupBy, sort: Sort) -> ListModel {
    let world = world();
    let scope = EngineScope::One(engine_id("colima:default"));
    let collapsed = BTreeMap::new();
    let pending = BTreeMap::new();
    build_list(&input(
        &world,
        &world.containers,
        filters,
        group_by,
        sort,
        &collapsed,
        &pending,
        &scope,
    ))
}

fn names(model: &ListModel, containers: &[Container]) -> Vec<String> {
    model
        .selection_order()
        .iter()
        .filter_map(|id| containers.iter().find(|c| c.id == *id))
        .map(|c| c.name.clone())
        .collect()
}

fn groups(model: &ListModel) -> Vec<&GroupInfo> {
    model
        .rows
        .iter()
        .filter_map(|r| match r {
            ListRow::Group(g) => Some(g),
            _ => None,
        })
        .collect()
}

// ---------- the five scenes ----------

#[test]
fn containers_scene_selected_1440_renders_the_grouped_table_with_the_selected_row() {
    let world = world();
    let html = markup("Containers / selected / 1440");
    let in_scope = default_containers(&world);
    // Header: title, count of the engine scope, group-by, filter field.
    assert!(html.contains("Containers"), "{html}");
    assert!(
        html.contains(&format!(">{}<", in_scope.len())),
        "count badge"
    );
    assert!(html.contains("Group by: Project"));
    assert!(html.contains("aria-label=\"Filter containers\""));
    // The engine chip with its ×.
    assert!(html.contains("Engine: default"));
    assert!(html.contains("Remove filter Engine: default"));
    // Groups in project order, Unassigned last; stopped groups collapsed.
    let order: Vec<usize> = [
        "shop-api",
        "analytics",
        "berth-site",
        "payments-mock",
        "ml-notebooks",
        "docs-site",
        "Unassigned",
    ]
    .iter()
    .map(|name| {
        html.find(&format!("Toggle group {name}"))
            .unwrap_or_else(|| panic!("group {name} missing"))
    })
    .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert!(html.contains("3 running · 1 restarting · 1 exited"));
    assert!(html.contains("Start 1 stopped"));
    assert!(html.contains("Stop all"));
    // ml-notebooks and docs-site hold only exited containers: collapsed.
    assert!(!html.contains("ml-notebooks-jupyter-1"));
    assert!(!html.contains("docs-site-web-1"));
    assert!(html.contains("shop-api-worker-1"));
    // One selected row, its buttons present, tooltips from the key table.
    assert_eq!(count(&html, "ct-row ct-selected"), 1, "{html}");
    assert!(html.contains("data-container=\"shop-api-worker-1\""));
    assert!(html.contains("data-tooltip=\"Restart · ⌘R\""), "{html}");
    assert!(html.contains("data-tooltip=\"Stop · ⌘.\""));
    assert!(html.contains("data-tooltip=\"Logs · ⌘L\""));
    // Ports are links to localhost; stats show a bar, a dash where absent.
    assert!(html.contains("href=\"http://localhost:3000\""));
    assert!(html.contains("3.2%"));
    assert!(html.contains("184 MB"));
    assert!(html.contains("ct-dash"));
    // Docked inspector: no Uptime column.
    assert!(!html.contains("class=\"ct-cell ct-uptime"), "{html}");
    assert!(
        html.contains("class=\"inspector"),
        "inspector column present"
    );
}

#[test]
fn containers_scene_selected_1440_light_is_the_same_table_in_the_light_theme() {
    let dark = markup("Containers / selected / 1440");
    let light = markup("Containers / selected / 1440 / light");
    assert_eq!(count(&dark, "ct-row"), count(&light, "ct-row"));
    assert!(light.contains("data-container=\"shop-api-worker-1\""));
    assert!(light.contains("Engine: default"));
    // The theme is a state of the scene, applied through settings.
    let mut seen = None;
    let mut dom = VirtualDom::new_with_props(
        ThemeProbe,
        ThemeProbeProps {
            frame: "Containers / selected / 1440 / light".to_string(),
        },
    );
    let _ = dom.rebuild_to_vec();
    THEME.with(|t| seen = t.borrow_mut().take());
    assert_eq!(seen, Some(berth_store::Theme::Light));
}

thread_local! {
    static THEME: std::cell::RefCell<Option<berth_store::Theme>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn ThemeProbe(frame: String) -> Element {
    let mut state = use_app_state();
    use_hook(move || {
        if let Some(found) = scenes::find(&frame) {
            (found.setup)(&mut state);
        }
        THEME.with(|t| *t.borrow_mut() = Some(state.settings.peek().theme));
    });
    rsx! { div {} }
}

#[test]
fn containers_scene_selected_1024_drops_uptime_and_engine_below_1200() {
    let html = scene("Containers / selected / 1024");
    // The same rows, the inspector toggle for the overlay, and a stylesheet
    // that removes Uptime and Engine below 1200 px (D78).
    assert!(html.contains("shop-api-worker-1"));
    assert!(html.contains(">Inspector<"), "overlay toggle button");
    let css = CONTAINERS_CSS;
    let media = css
        .split("@media (max-width: 1199px)")
        .nth(2)
        .expect("a narrow-width block for the table");
    // One rule hides both columns (cells and header cells share the classes).
    assert!(
        media.contains(".ct-table .ct-uptime,\n  .ct-table .ct-engine {\n    display: none;\n  }"),
        "{media}"
    );
}

#[test]
fn containers_scene_loading_shows_eight_skeleton_rows_and_the_uptime_column() {
    let html = markup("Containers / loading / 1440");
    assert_eq!(count(&html, "class=\"skeleton-row\""), 8, "{html}");
    assert!(html.contains("Loading…"));
    // Nothing selected, so the inspector is not docked and Uptime shows.
    assert!(html.contains(">Uptime<"), "{html}");
    assert!(!html.contains("ct-row"), "no data rows while loading");
    // The header count comes from the engine counts (rows are not loaded).
    let engine = world()
        .engines
        .into_iter()
        .find(|e| e.id == engine_id("colima:default"))
        .expect("default engine");
    assert!(html.contains(&format!(">{}<", engine.counts.containers_total)));
    assert!(html.contains("Engine: default"));
}

#[test]
fn containers_scene_engine_stopped_shows_the_banner_and_start_engine() {
    let html = markup("Containers / engine stopped / 1440");
    assert!(html.contains("Colima default is stopped. Containers on it are not running."));
    assert!(html.contains("banner tone-warning"));
    assert!(html.contains("Start the engine to see its containers"));
    assert!(html.contains("Start engine default"));
    let engine = world()
        .engines
        .into_iter()
        .find(|e| e.id == engine_id("colima:default"))
        .expect("default engine");
    assert!(html.contains(&format!(
        "Start it to see its {} containers, {} images and {} volumes.",
        engine.counts.containers_total, engine.counts.images, engine.counts.volumes
    )));
    assert!(
        html.contains("colima start default · 4 CPU · 8 GB (from "),
        "{html}"
    );
    assert!(html.contains("about 20 s"));
    assert!(!html.contains("ct-row"), "no rows for a stopped engine");
}

// ---------- the view model ----------

#[test]
fn containers_model_groups_by_project_with_counts_and_actions() {
    let world = world();
    let all = default_containers(&world);
    let m = model(&Filters::default(), GroupBy::Project, None);
    assert_eq!(m.total, all.len());
    assert_eq!(m.matched, all.len());
    let g = groups(&m);
    let names: Vec<&str> = g.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "shop-api",
            "analytics",
            "berth-site",
            "payments-mock",
            "ml-notebooks",
            "docs-site",
            "Unassigned"
        ]
    );
    assert_eq!(g[0].counts, "3 running · 1 restarting · 1 exited");
    assert_eq!(g[0].action_label().as_deref(), Some("Start 1 stopped"));
    assert_eq!(g[1].counts, "3 running · 1 exited");
    assert_eq!(g[2].counts, "2 running");
    assert_eq!(g[2].action_label().as_deref(), Some("Stop all"));
    assert_eq!(g[4].action_label().as_deref(), Some("Start 2 stopped"));
    assert_eq!(g[5].action_label().as_deref(), Some("Start 1 stopped"));
    // Groups without anything running start collapsed.
    assert!(g[0].expanded && g[1].expanded && g[2].expanded && g[3].expanded);
    assert!(!g[4].expanded && !g[5].expanded);
    // A collapsed group contributes no containers to the selection order.
    assert!(!names_in_order(&m).contains(&"ml-notebooks-jupyter-1".to_string()));
}

fn names_in_order(m: &ListModel) -> Vec<String> {
    names(m, &world().containers)
}

#[test]
fn containers_model_collapse_override_wins_over_the_default() {
    let world = world();
    let scope = EngineScope::One(engine_id("colima:default"));
    let filters = Filters::default();
    let pending = BTreeMap::new();
    let mut collapsed = BTreeMap::new();
    let key = groups(&model(&filters, GroupBy::Project, None))[4]
        .key
        .clone();
    collapsed.insert(key.clone(), false);
    let expanded = build_list(&input(
        &world,
        &world.containers,
        &filters,
        GroupBy::Project,
        None,
        &collapsed,
        &pending,
        &scope,
    ));
    assert!(names(&expanded, &world.containers).contains(&"ml-notebooks-jupyter-1".to_string()));
    collapsed.clear();
    collapsed.insert(groups(&expanded)[0].key.clone(), true);
    let shut = build_list(&input(
        &world,
        &world.containers,
        &filters,
        GroupBy::Project,
        None,
        &collapsed,
        &pending,
        &scope,
    ));
    assert!(!names(&shut, &world.containers).contains(&"shop-api-api-1".to_string()));
}

#[test]
fn containers_model_text_filter_narrows_rows_and_reports_matched_of_total() {
    let filters = Filters {
        text: "shop-api".to_string(),
        ..Filters::default()
    };
    let m = model(&filters, GroupBy::Project, None);
    let world = world();
    let found = names(&m, &world.containers);
    // Five shop-api services, plus the paused pod whose image is
    // shop-api-worker: the filter reads the image too.
    assert_eq!(m.matched, 6, "{found:?}");
    assert!(m.matched < m.total);
    assert!(found.contains(&"api-worker-7f9c4d8b6-xk2lp".to_string()));
    assert!(found
        .iter()
        .all(|n| n.starts_with("shop-api") || n.starts_with("api-worker")));
    // Several words must all match; a port query works too.
    let two = Filters {
        text: "shop db".to_string(),
        ..Filters::default()
    };
    assert_eq!(
        names(&model(&two, GroupBy::None, None), &world.containers),
        ["shop-api-db-1"]
    );
    let port = Filters {
        text: ":6379".to_string(),
        ..Filters::default()
    };
    assert_eq!(
        names(&model(&port, GroupBy::None, None), &world.containers),
        ["shop-api-redis-1"]
    );
    // A filter nothing matches leaves no rows at all, not even groups.
    let none = Filters {
        text: "zzz-nothing".to_string(),
        ..Filters::default()
    };
    let empty = model(&none, GroupBy::Project, None);
    assert_eq!(empty.matched, 0);
    assert!(empty.rows.is_empty());
    assert_eq!(empty.total, m.total);
}

#[test]
fn containers_model_state_and_project_chips_filter() {
    let world = world();
    let stopped = model(
        &Filters {
            state: Some(StateFilter::Stopped),
            ..Filters::default()
        },
        GroupBy::None,
        None,
    );
    let found = names(&stopped, &world.containers);
    assert!(found.contains(&"shop-api-db-1".to_string()));
    assert!(!found.contains(&"shop-api-api-1".to_string()));
    let unhealthy = model(
        &Filters {
            state: Some(StateFilter::Unhealthy),
            ..Filters::default()
        },
        GroupBy::None,
        None,
    );
    assert_eq!(names(&unhealthy, &world.containers), ["analytics-ingest-1"]);
    let project = model(
        &Filters {
            project: Some("berth-site".to_string()),
            ..Filters::default()
        },
        GroupBy::None,
        None,
    );
    assert_eq!(
        names(&project, &world.containers),
        ["berth-site-web-1", "berth-site-proxy-1"]
    );
}

#[test]
fn containers_model_sorts_within_groups_and_keeps_missing_values_last() {
    let world = world();
    let asc = model(
        &Filters::default(),
        GroupBy::None,
        Some((SortColumn::Name, SortDirection::Asc)),
    );
    let asc_names = names(&asc, &world.containers);
    let mut expected = asc_names.clone();
    expected.sort_by_key(|n| n.to_lowercase());
    assert_eq!(asc_names, expected);
    let desc = model(
        &Filters::default(),
        GroupBy::None,
        Some((SortColumn::Name, SortDirection::Desc)),
    );
    let mut reversed = asc_names.clone();
    reversed.reverse();
    assert_eq!(names(&desc, &world.containers), reversed);

    // CPU descending: busiest first, containers without stats at the end.
    let cpu = model(
        &Filters::default(),
        GroupBy::None,
        Some((SortColumn::Cpu, SortDirection::Desc)),
    );
    let order = names(&cpu, &world.containers);
    assert_eq!(order[0], "analytics-clickhouse-1", "12.4 % is the busiest");
    let stats_of = |name: &str| {
        world
            .containers
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.stats.as_ref().map(|s| s.cpu_pct))
    };
    let first_missing = order
        .iter()
        .position(|n| stats_of(n).is_none())
        .expect("some container has no stats");
    assert!(order[first_missing..].iter().all(|n| stats_of(n).is_none()));
    // Ascending keeps the missing ones last too.
    let cpu_asc = model(
        &Filters::default(),
        GroupBy::None,
        Some((SortColumn::Cpu, SortDirection::Asc)),
    );
    let order = names(&cpu_asc, &world.containers);
    assert!(stats_of(order.last().expect("rows")).is_none());

    // Sorting happens inside each group; groups keep their order.
    let grouped = model(
        &Filters::default(),
        GroupBy::Project,
        Some((SortColumn::Name, SortDirection::Desc)),
    );
    let first = groups(&grouped)[0];
    assert_eq!(first.name, "shop-api");
    let in_group: Vec<String> = grouped
        .rows
        .iter()
        .skip(1)
        .take_while(|r| matches!(r, ListRow::Container(_)))
        .filter_map(|r| match r {
            ListRow::Container(id) => world.containers.iter().find(|c| c.id == *id),
            _ => None,
        })
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(
        in_group,
        [
            "shop-api-worker-1",
            "shop-api-redis-1",
            "shop-api-mailpit-1",
            "shop-api-db-1",
            "shop-api-api-1"
        ]
    );
}

#[test]
fn containers_model_group_by_engine_and_none() {
    let world = world();
    let scope = EngineScope::All;
    let filters = Filters::default();
    let collapsed = BTreeMap::new();
    let pending = BTreeMap::new();
    let by_engine = build_list(&input(
        &world,
        &world.containers,
        &filters,
        GroupBy::Engine,
        None,
        &collapsed,
        &pending,
        &scope,
    ));
    let names: Vec<&str> = groups(&by_engine).iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["default", "work", "hetzner-1"]);
    assert_eq!(by_engine.total, world.containers.len());
    // None: no group rows at all.
    let flat = build_list(&input(
        &world,
        &world.containers,
        &filters,
        GroupBy::None,
        None,
        &collapsed,
        &pending,
        &scope,
    ));
    assert!(groups(&flat).is_empty());
    assert_eq!(flat.selection_order().len(), world.containers.len());
}

#[test]
fn containers_model_engine_scope_limits_rows_to_that_engine() {
    let world = world();
    let one = model(&Filters::default(), GroupBy::None, None);
    assert_eq!(one.total, default_containers(&world).len());
    assert!(one.total < world.containers.len());
}

#[test]
fn containers_model_failed_action_adds_a_message_row_under_its_container() {
    let world = world();
    let scope = EngineScope::One(engine_id("colima:default"));
    let worker = world
        .containers
        .iter()
        .find(|c| c.name == "shop-api-worker-1")
        .expect("worker");
    let mut pending = BTreeMap::new();
    pending.insert(
        PendingKey {
            subject: Subject::Container(worker.id.clone()),
            action: "container.restart",
        },
        PendingState::Failed {
            message: "Docker says no".to_string(),
        },
    );
    let m = build_list(&input(
        &world,
        &world.containers,
        &Filters::default(),
        GroupBy::None,
        None,
        &BTreeMap::new(),
        &pending,
        &scope,
    ));
    let at = m
        .rows
        .iter()
        .position(|r| *r == ListRow::Container(worker.id.clone()))
        .expect("worker row");
    assert_eq!(m.rows[at + 1], ListRow::Failure(worker.id.clone()));
    assert_eq!(
        m.rows
            .iter()
            .filter(|r| matches!(r, ListRow::Failure(_)))
            .count(),
        1
    );
    // The message row is not selectable.
    let selectable = m.selection_order();
    assert_eq!(selectable.iter().filter(|id| **id == worker.id).count(), 1);
}

#[test]
fn containers_list_status_loading_stopped_and_ready() {
    let world = world();
    let default = engine_id("colima:default");
    let scope = EngineScope::One(default.clone());
    let rows = default_containers(&world);
    assert_eq!(list_status(&[], &scope, &[], false), ListStatus::Loading);
    assert_eq!(
        list_status(&world.engines, &scope, &rows, false),
        ListStatus::Ready
    );
    // Running, version unknown, no rows yet: still loading.
    let mut engines = world.engines.clone();
    for e in &mut engines {
        if e.id == default {
            e.docker_version = None;
        }
    }
    assert_eq!(
        list_status(&engines, &scope, &[], false),
        ListStatus::Loading
    );
    // A stale snapshot is rows enough.
    assert_eq!(
        list_status(&engines, &scope, &rows, true),
        ListStatus::Ready
    );
    // An answered, empty engine is empty, not loading.
    assert_eq!(
        list_status(&world.engines, &scope, &[], false),
        ListStatus::Ready
    );
    // Stopped wins.
    let mut stopped = world.engines.clone();
    for e in &mut stopped {
        if e.id == default {
            e.state = berth_core::EngineState::Stopped;
        }
    }
    match list_status(&stopped, &scope, &rows, false) {
        ListStatus::EngineStopped(e) => assert_eq!(e.id, default),
        other => panic!("{other:?}"),
    }
    // Across all engines a stopped one only adds a banner.
    assert_eq!(
        list_status(&stopped, &EngineScope::All, &world.containers, false),
        ListStatus::Ready
    );
    let banners = engine_banners(&stopped, &EngineScope::All);
    assert!(banners
        .iter()
        .any(|(_, m)| m == "Colima default is stopped. Containers on it are not running."));
    assert!(banners
        .iter()
        .any(|(tone, m)| *tone == BannerTone::Danger && m.starts_with("hetzner-1 is unreachable")));
}

#[test]
fn containers_empty_copy_names_the_filter() {
    let stopped = empty_copy(&Filters {
        state: Some(StateFilter::Stopped),
        ..Filters::default()
    });
    assert_eq!(stopped.0, "No stopped containers");
    let text = empty_copy(&Filters {
        text: " zzz ".to_string(),
        ..Filters::default()
    });
    assert_eq!(text.0, "No containers match");
    assert!(text.1.contains("zzz"));
    assert_eq!(text.2, Some("Clear filter"));
    assert_eq!(empty_copy(&Filters::default()).0, "No containers yet");
}

#[test]
fn containers_chips_follow_scope_and_filters() {
    let world = world();
    let one = EngineScope::One(engine_id("colima:default"));
    let filters = Filters {
        state: Some(StateFilter::Running),
        project: Some("shop-api".to_string()),
        ..Filters::default()
    };
    let labels: Vec<String> = chips(&world.engines, &one, &filters)
        .iter()
        .map(Chip::label)
        .collect();
    assert_eq!(
        labels,
        ["Engine: default", "State: Running", "Project: shop-api"]
    );
    assert!(chips(&world.engines, &EngineScope::All, &Filters::default()).is_empty());
}

// ---------- selection, ports, rows ----------

fn ids(n: usize) -> Vec<ContainerId> {
    (0..n).map(|i| ContainerId(format!("c{i}"))).collect()
}

#[test]
fn containers_step_selection_moves_one_row_and_stops_at_the_ends() {
    let order = ids(3);
    let at = |i: usize| Selection::Containers(vec![order[i].clone()]);
    assert_eq!(step_selection(&order, &at(0), true), Some(order[1].clone()));
    assert_eq!(
        step_selection(&order, &at(1), false),
        Some(order[0].clone())
    );
    assert_eq!(step_selection(&order, &at(2), true), Some(order[2].clone()));
    assert_eq!(
        step_selection(&order, &at(0), false),
        Some(order[0].clone())
    );
    // No selection: J takes the first, K the last.
    assert_eq!(
        step_selection(&order, &Selection::None, true),
        Some(order[0].clone())
    );
    assert_eq!(
        step_selection(&order, &Selection::None, false),
        Some(order[2].clone())
    );
    // A selected container that is no longer listed counts as none.
    let gone = Selection::Containers(vec![ContainerId("gone".to_string())]);
    assert_eq!(step_selection(&order, &gone, true), Some(order[0].clone()));
    assert_eq!(step_selection(&[], &Selection::None, true), None);
}

#[test]
fn containers_port_links_build_the_open_command() {
    let world = world();
    let web = world
        .containers
        .iter()
        .find(|c| c.name == "berth-site-proxy-1")
        .expect("proxy");
    let port = web.ports.first().expect("a port");
    assert_eq!(port_url(port).as_deref(), Some("http://localhost:8080"));
    let (program, args) = open_command("http://localhost:56080");
    assert_eq!(
        program,
        if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        }
    );
    assert_eq!(args, ["http://localhost:56080"]);
    // Unpublished and UDP ports are not links.
    let hidden = PortMapping {
        host_ip: None,
        host_port: None,
        container_port: 80,
        proto: Proto::Tcp,
    };
    assert_eq!(port_url(&hidden), None);
    let udp = PortMapping {
        host_port: Some(53),
        proto: Proto::Udp,
        ..hidden
    };
    assert_eq!(port_url(&udp), None);
}

fn view_for<'a>(
    world: &'a berth_engine::MockWorld,
    pending: &'a BTreeMap<PendingKey, PendingState>,
) -> StateView<'a> {
    StateView {
        engines: &world.engines,
        containers: &world.containers,
        projects: &world.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending,
    }
}

fn container<'a>(world: &'a berth_engine::MockWorld, name: &str) -> &'a Container {
    world
        .containers
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} in the mock world"))
}

#[test]
fn containers_row_buttons_carry_tooltips_toggle_and_disabled_reasons() {
    let world = world();
    let pending = BTreeMap::new();
    let view = view_for(&world, &pending);
    let running = row_data(
        container(&world, "shop-api-api-1"),
        &view,
        "default",
        chrono::Utc::now(),
        false,
    );
    let tooltip = |data: &ContainerRowData, action: RowAction| {
        data.buttons
            .iter()
            .find(|b| b.action == action)
            .map(|b| (b.tooltip.clone(), b.disabled, b.icon))
            .expect("button")
    };
    assert_eq!(
        tooltip(&running, RowAction::StartStop),
        ("Stop · ⌘.".to_string(), false, "square")
    );
    assert_eq!(tooltip(&running, RowAction::Restart).0, "Restart · ⌘R");
    assert_eq!(tooltip(&running, RowAction::Logs).0, "Logs · ⌘L");
    // Terminal and Delete wait for their tasks and say so.
    let terminal = tooltip(&running, RowAction::Terminal);
    assert!(terminal.1);
    assert_eq!(terminal.0, "Terminal is not built yet");
    let delete = tooltip(&running, RowAction::Delete);
    assert!(delete.1);
    assert_eq!(delete.0, "Delete confirmation is not built yet");
    // An exited container offers Start, and Restart still works.
    let exited = row_data(
        container(&world, "shop-api-db-1"),
        &view,
        "default",
        chrono::Utc::now(),
        false,
    );
    assert_eq!(
        tooltip(&exited, RowAction::StartStop),
        ("Start · ⌘.".to_string(), false, "play")
    );
    assert_eq!(exited.status_text, "Exited (1)");
    assert!(
        exited.cpu.is_none() && exited.memory.is_none(),
        "dashes without stats"
    );
    assert!(exited.uptime.is_none());
    // Stats show number and bar.
    let cpu = running.cpu.as_ref().expect("cpu");
    assert_eq!(cpu.text, "3.2%");
    assert!(cpu.fraction > 0.0 && cpu.fraction <= 1.0);
    assert_eq!(running.memory.as_ref().expect("mem").text, "184 MB");
    assert_eq!(running.ports[0].label, ":3000→3000/tcp");
    assert_eq!(running.ports[0].host_port, Some(3000));
}

#[test]
fn containers_row_shows_the_pending_label_and_blocks_other_actions() {
    let world = world();
    let api = container(&world, "shop-api-api-1");
    let mut pending = BTreeMap::new();
    pending.insert(
        PendingKey {
            subject: Subject::Container(api.id.clone()),
            action: "container.stop",
        },
        PendingState::Running {
            label: "Stopping…".to_string(),
        },
    );
    let view = view_for(&world, &pending);
    let data = row_data(api, &view, "default", chrono::Utc::now(), false);
    assert_eq!(data.status_text, "Stopping…");
    let toggle = &data.buttons[0];
    assert!(toggle.loading, "the running action shows a spinner");
    let restart = data
        .buttons
        .iter()
        .find(|b| b.action == RowAction::Restart)
        .expect("restart");
    assert!(restart.disabled);
    assert!(
        restart.tooltip.contains("still in progress"),
        "{}",
        restart.tooltip
    );
}

#[test]
fn containers_row_menu_offers_pause_or_resume_and_copy_id() {
    let world = world();
    let pending = BTreeMap::new();
    let view = view_for(&world, &pending);
    let running = row_menu(container(&world, "shop-api-api-1"), &view);
    assert_eq!(running[0].id, "pause");
    assert!(running[0].disabled_reason.is_none());
    assert_eq!(running[1].id, "copy-id");
    let paused = row_menu(container(&world, "api-worker-7f9c4d8b6-xk2lp"), &view);
    assert_eq!(paused[0].id, "resume");
    let exited = row_menu(container(&world, "shop-api-db-1"), &view);
    assert_eq!(
        exited[0].disabled_reason.as_deref(),
        Some("Only a running container can be paused")
    );
}

#[test]
fn containers_tooltip_for_uses_the_action_table_shortcuts() {
    assert_eq!(tooltip_for("Restart", "container.restart"), "Restart · ⌘R");
    assert_eq!(tooltip_for("Delete", "container.delete"), "Delete · ⌫");
    assert_eq!(tooltip_for("More", "no.such.action"), "More");
}

#[test]
fn containers_cpu_fraction_clamps_and_lifts_small_loads() {
    assert_eq!(cpu_fraction(0.0), 0.0);
    assert_eq!(cpu_fraction(100.0), 1.0);
    assert_eq!(cpu_fraction(250.0), 1.0);
    assert!(cpu_fraction(3.2) > 0.1, "a light load is still visible");
    assert!(cpu_fraction(12.4) > cpu_fraction(3.2));
}

// ---------- J / K / Space on real state ----------

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, ContainersView)>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let view = provide_containers_view();
    use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some((state, view))));
    rsx! { div {} }
}

fn harness() -> (VirtualDom, AppState, ContainersView) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (state, view) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the state");
    (dom, state, view)
}

fn load_default_engine(dom: &VirtualDom, mut state: AppState) {
    let world = world();
    dom.in_scope(ScopeId::APP, || {
        state.engines.set(world.engines.clone());
        state
            .engine_scope
            .set(EngineScope::One(engine_id("colima:default")));
        state.containers.set(world.containers.clone());
        state.projects.set(world.projects.clone());
        state.section.set(Section::Containers);
    });
}

fn selected_names(dom: &VirtualDom, state: AppState) -> Vec<String> {
    dom.in_scope(ScopeId::APP, || match &*state.selection.peek() {
        Selection::Containers(ids) => ids
            .iter()
            .filter_map(|id| {
                state
                    .containers
                    .peek()
                    .iter()
                    .find(|c| c.id == *id)
                    .map(|c| c.name.clone())
            })
            .collect(),
        _ => Vec::new(),
    })
}

#[test]
fn containers_keys_j_and_k_walk_the_visible_rows_through_the_root_handler() {
    let (dom, state, view) = harness();
    load_default_engine(&dom, state);
    // AppState is a bundle of Copy signal handles: the closure and the
    // direct edits below drive the same signals.
    let key = |k: &str| {
        let mut handle = state;
        let action = crate::keys::map_key(
            k,
            crate::keys::Modifiers::NONE,
            crate::keys::context_from_state(&handle),
        )
        .expect("J and K map to an action");
        dom.in_scope(ScopeId::APP, || crate::keys::dispatch(&mut handle, action));
    };
    // From nothing, J takes the first row of the first group.
    key("j");
    assert_eq!(selected_names(&dom, state), ["shop-api-api-1"]);
    key("j");
    assert_eq!(selected_names(&dom, state), ["shop-api-worker-1"]);
    key("k");
    assert_eq!(selected_names(&dom, state), ["shop-api-api-1"]);
    // K at the top stays on the first row.
    key("k");
    assert_eq!(selected_names(&dom, state), ["shop-api-api-1"]);
    // Collapsed groups are skipped: from the last berth... walk to the end
    // of payments-mock and J lands on Unassigned's first row, not on the
    // collapsed ml-notebooks or docs-site.
    dom.in_scope(ScopeId::APP, || {
        let mut handle = state;
        let id = state
            .containers
            .peek()
            .iter()
            .find(|c| c.name == "payments-mock-stripe-1")
            .map(|c| c.id.clone())
            .expect("stripe");
        select_only(&mut handle, &id);
    });
    key("j");
    let landed = selected_names(&dom, state);
    assert_eq!(
        landed,
        ["pg-scratch"],
        "collapsed groups are not stepped into"
    );
    // A filter changes what J can reach.
    dom.in_scope(ScopeId::APP, || {
        let mut handle = state;
        let mut filters = view.filters;
        filters.write().text = "berth-site".to_string();
        let web = state
            .containers
            .peek()
            .iter()
            .find(|c| c.name == "berth-site-web-1")
            .map(|c| c.id.clone())
            .expect("web");
        select_only(&mut handle, &web);
    });
    key("j");
    assert_eq!(selected_names(&dom, state), ["berth-site-proxy-1"]);
    key("j");
    assert_eq!(
        selected_names(&dom, state),
        ["berth-site-proxy-1"],
        "the filtered list ends here"
    );
}

#[test]
fn containers_keys_j_k_do_nothing_outside_the_containers_section() {
    let (dom, mut state, _view) = harness();
    load_default_engine(&dom, state);
    dom.in_scope(ScopeId::APP, || {
        state.section.set(Section::Images);
        crate::keys::dispatch(&mut state, "global.select_next");
    });
    assert!(selected_names(&dom, state).is_empty());
}

#[test]
fn containers_keys_space_toggles_the_inspector_flag_the_shell_reads() {
    let (dom, mut state, view) = harness();
    load_default_engine(&dom, state);
    let space = crate::keys::map_key(
        " ",
        crate::keys::Modifiers::NONE,
        crate::keys::context_from_state(&state),
    );
    assert_eq!(space, Some("global.toggle_inspector"));
    assert!(!dom.in_scope(ScopeId::APP, || *view.inspector_toggled.peek()));
    dom.in_scope(ScopeId::APP, || {
        crate::keys::dispatch(&mut state, "global.toggle_inspector")
    });
    assert!(dom.in_scope(ScopeId::APP, || *view.inspector_toggled.peek()));
    dom.in_scope(ScopeId::APP, || {
        crate::keys::dispatch(&mut state, "global.toggle_inspector")
    });
    assert!(!dom.in_scope(ScopeId::APP, || *view.inspector_toggled.peek()));
}

#[test]
fn containers_selection_helpers_select_alone_and_toggle_membership() {
    let (dom, mut state, _view) = harness();
    load_default_engine(&dom, state);
    let world = world();
    let a = container(&world, "shop-api-api-1").id.clone();
    let b = container(&world, "shop-api-redis-1").id.clone();
    dom.in_scope(ScopeId::APP, || {
        select_only(&mut state, &a);
        toggle_selected(&mut state, &b);
    });
    assert_eq!(
        dom.in_scope(ScopeId::APP, || state.selection.peek().clone()),
        Selection::Containers(vec![a.clone(), b.clone()])
    );
    dom.in_scope(ScopeId::APP, || {
        toggle_selected(&mut state, &a);
        toggle_selected(&mut state, &b);
    });
    assert_eq!(
        dom.in_scope(ScopeId::APP, || state.selection.peek().clone()),
        Selection::None
    );
}

// ---------- the rendered screen reacts to state ----------

#[component]
fn ScreenRoot(setup: SetupFn) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    let mut view = provide_containers_view();
    use_hook(move || (setup.0)(&mut state, &mut view));
    rsx! { ContainersScreen {} }
}

#[derive(Clone, Copy)]
struct SetupFn(fn(&mut AppState, &mut ContainersView));

impl PartialEq for SetupFn {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.0, other.0)
    }
}

fn render_screen(setup: fn(&mut AppState, &mut ContainersView)) -> String {
    let mut dom = VirtualDom::new_with_props(
        ScreenRoot,
        ScreenRootProps {
            setup: SetupFn(setup),
        },
    );
    let _ = dom.rebuild_to_vec();
    strip_styles(&dioxus_ssr::render(&dom))
}

fn mock_state(state: &mut AppState) {
    let world = world();
    state.engines.set(world.engines.clone());
    state
        .engine_scope
        .set(EngineScope::One(engine_id("colima:default")));
    state.containers.set(world.containers.clone());
    state.projects.set(world.projects.clone());
    state.section.set(Section::Containers);
}

#[test]
fn containers_screen_filter_shows_matched_of_total_and_the_matching_rows_only() {
    let html = render_screen(|state, view| {
        mock_state(state);
        view.filters.set(Filters {
            text: "berthfx".to_string(),
            ..Filters::default()
        });
    });
    // Nothing in the mock world is called berthfx: the empty state names it.
    assert!(html.contains("No containers match"), "{html}");
    assert!(html.contains("berthfx"));
    assert!(html.contains("Clear filter"));
    assert_eq!(count(&html, "class=\"ct-row"), 0);

    let html = render_screen(|state, view| {
        mock_state(state);
        view.filters.set(Filters {
            text: "analytics".to_string(),
            ..Filters::default()
        });
    });
    assert_eq!(count(&html, "class=\"ct-row"), 4, "{html}");
    assert!(
        html.contains("4 of "),
        "the field says how many matched: {html}"
    );
    assert!(!html.contains("shop-api-api-1"));
}

#[test]
fn containers_screen_no_group_has_no_group_rows_and_all_engines_adds_the_engine_column() {
    let html = render_screen(|state, view| {
        mock_state(state);
        state.engine_scope.set(EngineScope::All);
        view.group_by.set(GroupBy::None);
    });
    assert_eq!(count(&html, "group-header-row"), 0, "{html}");
    assert!(html.contains("ct-has-engine"));
    assert!(html.contains("class=\"ct-cell ct-engine\""));
    assert!(html.contains("hetzner-1"));
    // Across engines the unreachable one gets a banner.
    assert!(html.contains("hetzner-1 is unreachable"), "{html}");
    // No engine chip when every engine is shown.
    assert!(!html.contains("Engine: "));
}

#[test]
fn containers_screen_sort_arrow_marks_the_sorted_column() {
    let html = render_screen(|state, view| {
        mock_state(state);
        view.sort.set(Some((SortColumn::Cpu, SortDirection::Desc)));
    });
    assert!(html.contains("aria-sort=\"descending\""), "{html}");
    assert!(html.contains("/icons.svg#arrow-down"));
}

#[test]
fn containers_screen_failed_action_shows_its_message_under_the_row() {
    let html = render_screen(|state, view| {
        let _ = view;
        mock_state(state);
        let worker = state
            .containers
            .peek()
            .iter()
            .find(|c| c.name == "shop-api-worker-1")
            .map(|c| c.id.clone())
            .expect("worker");
        state.pending.write().insert(
            PendingKey {
                subject: Subject::Container(worker),
                action: "container.restart",
            },
            PendingState::Failed {
                message: "The engine refused to restart it".to_string(),
            },
        );
    });
    assert!(html.contains("The engine refused to restart it"), "{html}");
    assert!(html.contains("ct-failure"));
    // The pending state is not "running": no spinner, status text unchanged.
    assert!(html.contains("Restarting"));
}

#[test]
fn containers_screen_running_action_shows_its_label_in_the_status_cell() {
    let html = render_screen(|state, view| {
        let _ = view;
        mock_state(state);
        let api = state
            .containers
            .peek()
            .iter()
            .find(|c| c.name == "shop-api-api-1")
            .map(|c| c.id.clone())
            .expect("api");
        state.pending.write().insert(
            PendingKey {
                subject: Subject::Container(api),
                action: "container.stop",
            },
            PendingState::Running {
                label: "Stopping…".to_string(),
            },
        );
    });
    assert!(html.contains("Stopping…"), "{html}");
    assert!(html.contains("aria-busy=\"true\""));
}

#[test]
fn containers_screen_uptime_shows_without_a_docked_inspector_and_hides_with_one() {
    let without = render_screen(|state, _| mock_state(state));
    assert!(without.contains(">Uptime<"), "{without}");
    assert!(without.contains("class=\"ct-cell ct-uptime"));
    // A running container shows its uptime text, a stopped one a dash.
    let with = render_screen(|state, _| {
        mock_state(state);
        let id = state.containers.peek()[0].id.clone();
        state.selection.set(Selection::Containers(vec![id]));
    });
    assert!(!with.contains(">Uptime<"), "{with}");
    assert!(!with.contains("class=\"ct-cell ct-uptime"));
    // Space gives the column back.
    let toggled = render_screen(|state, view| {
        mock_state(state);
        let id = state.containers.peek()[0].id.clone();
        state.selection.set(Selection::Containers(vec![id]));
        view.inspector_toggled.set(true);
    });
    assert!(toggled.contains(">Uptime<"));
}

#[test]
fn containers_group_by_values_round_trip() {
    for g in GroupBy::ALL {
        assert_eq!(GroupBy::parse(g.value()), g);
    }
    assert_eq!(GroupBy::parse("garbage"), GroupBy::Project);
    assert_eq!(GroupBy::Engine.label(), "Group by: Engine");
}
