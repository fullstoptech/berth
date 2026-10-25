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
    assert!(
        html.contains("ct-inspector-toggle ct-inspector-toggle-on"),
        "shown, because 1024 px cannot dock the inspector: {html}"
    );
    let css = CONTAINERS_CSS;
    let media = css
        .split("@media (max-width: 1199px)")
        .nth(1)
        .expect("a narrow-width block for the table");
    // One rule hides both columns (cells and header cells share the classes).
    assert!(
        media.contains(".ct-table .ct-uptime,\n  .ct-table .ct-engine {\n    display: none;\n  }"),
        "{media}"
    );
}

use super::super::css_tracks as css;
use super::super::table_tiers::containers::{containers_state_at, Col};
use super::super::table_tiers::View;
use crate::text_scale::STEPS;

/// The body of the table's block for the viewports under 1200 px.
fn narrow_viewport_block() -> &'static str {
    css::block(CONTAINERS_CSS, "@media (max-width: 1199px) {\n  .ct-table,")
}

#[test]
fn containers_1024_columns_measure_what_the_frame_measures() {
    // "Containers / selected / 1024": main is 1024 - 220 wide, rows pad 16
    // on each side and columns sit 12 apart. The frame's Name, Image, Ports
    // and Status measure 120, 134, 112 and 138 px, which fills the row.
    let tracks = css::tracks(&css::declaration(narrow_viewport_block(), "--ct-cols"));
    assert_eq!(tracks.len(), 8, "{tracks:?}");
    let mins: Vec<f32> = tracks.iter().map(|t| css::min_px(t, 1.0)).collect();
    // Name, image, ports, status are the flexible tracks.
    assert_eq!(
        [mins[2], mins[3], mins[4], mins[7]],
        [120.0, 134.0, 112.0, 138.0]
    );
    let available = 1024.0 - 220.0 - 32.0 - 12.0 * 7.0;
    assert!(
        (mins.iter().sum::<f32>() - available).abs() < 0.5,
        "minimums {mins:?} fill {available} px exactly"
    );
    // CPU, Memory and Status hold numbers, bars and the row buttons: they
    // grow with the text size (the others truncate).
    for i in [5, 6, 7] {
        assert!(
            css::has_scale(&tracks[i]),
            "track {i} `{}` needs a scale term",
            tracks[i]
        );
    }
}

#[test]
fn containers_every_column_list_scales_its_data_tracks() {
    // Each list is: dot, dot, name, [image], ports, CPU, memory, [uptime],
    // [engine], status. CPU, memory, uptime, engine and status hold numbers,
    // bars or the row buttons, so they need a scale term; so does Ports once
    // Image is gone and it is the column that takes the room.
    let narrow = narrow_viewport_block();
    let mut lists = vec![
        (
            "--ct-cols-noimage",
            css::declaration(narrow, "--ct-cols-noimage"),
        ),
        ("--ct-cols", css::declaration(narrow, "--ct-cols")),
    ];
    for selector in [
        "\n.ct-table {",
        "\n.ct-table.ct-has-uptime {",
        "\n.ct-table.ct-has-engine {",
        "\n.ct-table.ct-has-uptime.ct-has-engine {",
    ] {
        let body = css::block(CONTAINERS_CSS, selector);
        for name in ["--ct-cols", "--ct-cols-noimage"] {
            lists.push((name, css::declaration(body, name)));
        }
    }
    for (name, list) in lists {
        let tracks = css::tracks(&list);
        let first_scaled = if name == "--ct-cols" { 5 } else { 3 };
        for (i, t) in tracks.iter().enumerate().skip(first_scaled) {
            assert!(
                css::has_scale(t),
                "track {i} `{t}` of {name} needs a scale term in `{list}`"
            );
        }
    }
}

/// The list in force for a table `width` px wide at `scale`, for a
/// viewport under (`narrow`) or over 1200 px with the given column classes,
/// from the generated tiers: whether Image is hidden and the tracks.
fn containers_tracks_at(
    width: f32,
    scale: f32,
    narrow: bool,
    uptime: bool,
    engine: bool,
) -> (bool, Vec<String>) {
    let view = if narrow { View::Narrow } else { View::Wide };
    let state = containers_state_at(view, uptime, engine, width, scale);
    (state.hidden.contains(&Col::Image), state.tracks)
}

/// The table's width next to the inspector at `stored` px in a `window` px
/// wide window at `scale` text, and whether the inspector took room from it
/// (D21, D24): docked, clamped to keep the main area at 640 px x scale, only
/// from 1200 px up and when 320 px of it fit; else it overlays and the table
/// has the whole main area.
fn table_width_with_inspector(window: f32, stored: u32, scale: f32) -> (f32, bool) {
    match crate::ui::dock::docked_width(f64::from(window), stored, scale) {
        Some(width) => (window - 220.0 - width as f32, true),
        None => (window - 220.0, false),
    }
}

#[test]
fn containers_table_fits_every_window_inspector_width_and_text_size() {
    // The D24 sweep: windows 1024 / 1220 / 1440, inspector widths 320 / 360 /
    // 480 / 560, every text size, with and without Engine (all engines). A
    // docked inspector takes Uptime away; an overlay or none leaves it.
    let mut docked_cases = 0;
    for window in [1024.0, 1220.0, 1440.0] {
        for stored in [320, 360, 480, 560] {
            for scale in STEPS {
                let (width, docked) = table_width_with_inspector(window, stored, scale);
                docked_cases += usize::from(docked);
                for engine in [false, true] {
                    assert_table_fits(width, window < 1200.0, !docked, engine, scale);
                }
            }
        }
    }
    // The sweep is not vacuous: plenty of the cases dock the inspector.
    assert!(docked_cases > 30, "{docked_cases} docked cases");
}

#[test]
fn containers_table_fits_the_main_area_floor_at_every_text_size() {
    // The floor itself (640 px x text size) is what the narrowest list must
    // fit: a docked inspector can leave exactly that much.
    for scale in STEPS {
        let floor = crate::ui::dock::main_min_px(scale) as f32;
        for engine in [false, true] {
            assert_table_fits(floor, false, false, engine, scale);
        }
    }
}

#[test]
fn containers_inspector_overlays_at_1440_and_150_percent_and_leaves_the_table_whole() {
    // Accepted in D24: 1440 - 220 - 960 leaves no room for 320 px.
    let (width, docked) = table_width_with_inspector(1440.0, 360, 1.5);
    assert!(!docked);
    assert_eq!(width, 1220.0);
    // At 130 % a 480 px inspector docks clamped to 388, not at 480.
    let (width, docked) = table_width_with_inspector(1440.0, 480, 1.3);
    assert!(docked);
    assert_eq!(width, 1440.0 - 220.0 - 388.0);
}

/// Every text size's (or just `scale`'s) column list fits `width` px.
fn assert_table_fits(width: f32, narrow: bool, uptime: bool, engine: bool, scale: f32) {
    let (_, tracks) = containers_tracks_at(width, scale, narrow, uptime, engine);
    // The page gutter is 16 px under 1200 px wide, 20 px from there up.
    let gutters = if narrow { 32.0 } else { 40.0 };
    let used = css::min_sum(&tracks, scale) + 12.0 * (tracks.len() as f32 - 1.0) + gutters;
    assert!(
        used <= width + 0.5,
        "{width} px at {scale} (narrow {narrow}, uptime {uptime}, engine {engine}): needs {used} ({tracks:?})"
    );
}

#[test]
fn containers_table_fits_at_1024_and_1440_for_every_text_size() {
    // (table width, narrow viewport, uptime, engine): 1024 has the inspector
    // as an overlay; at 1440 a selection docks it (860 px, no Uptime) and
    // without one Uptime shows (1220 px); Engine shows with all engines.
    let cases = [
        (1024.0 - 220.0, true, false, false),
        (1440.0 - 220.0 - 360.0, false, false, false),
        (1440.0 - 220.0 - 360.0, false, false, true),
        (1440.0 - 220.0, false, true, false),
        (1440.0 - 220.0, false, true, true),
    ];
    for (width, narrow, uptime, engine) in cases {
        for scale in STEPS {
            assert_table_fits(width, narrow, uptime, engine, scale);
        }
    }
}

#[test]
fn containers_tables_keep_image_at_100_percent_and_drop_it_where_it_no_longer_fits() {
    // The frames at 100 % show Image everywhere. A bigger text size drops it
    // from the tables that no longer have the room: the 1024 and the docked
    // inspector's, not the full-width 1220 px one.
    for (width, narrow, uptime, engine, drops_at_150) in [
        (1024.0 - 220.0, true, false, false, true),
        (1440.0 - 220.0 - 360.0, false, false, false, true),
        (1440.0 - 220.0 - 360.0, false, false, true, true),
        (1440.0 - 220.0, false, true, false, false),
        (1440.0 - 220.0, false, true, true, false),
    ] {
        let (dropped, _) = containers_tracks_at(width, 1.0, narrow, uptime, engine);
        assert!(!dropped, "{width} px at 100 % keeps Image");
        let (dropped, _) = containers_tracks_at(width, 1.5, narrow, uptime, engine);
        assert_eq!(dropped, drops_at_150, "{width} px at 150 %");
    }
    // The generated tiers are in the stylesheet, for every text size.
    for scale in STEPS {
        assert!(
            CONTAINERS_CSS.contains(&format!(":root[data-text-scale=\"{scale}\"]")) || scale == 1.0,
            "tiers for {scale}"
        );
    }
    assert!(!CONTAINERS_CSS.contains("em) {"), "no em thresholds left");
    // Cells and header cell both go with the track.
    assert!(CONTAINERS_CSS.contains(".ct-table .ct-image-head {\n      display: none;"));
    assert!(CONTAINERS_CSS.contains(".ct-table .ct-image,"));
    assert!(CONTAINERS_CSS.contains(".ct-table.ct-has-uptime.ct-has-engine .ct-image-head"));
    let html = markup("Containers / selected / 1440");
    assert!(
        html.contains("class=\"ct-image-head\""),
        "the header cell is wrapped"
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

#[test]
fn containers_group_menu_enables_only_the_actions_some_member_can_take() {
    let m = model(&Filters::default(), GroupBy::Project, None);
    let g = groups(&m);
    let find = |items: &[MenuEntry], id: &str| items.iter().find(|e| e.id == id).cloned();
    // shop-api: 1 stopped, 4 running or restarting.
    let items = g[0].menu_items();
    assert_eq!(items[0].label, "Start 1 stopped");
    assert!(items.iter().all(|e| e.disabled_reason.is_none()));
    // berth-site: everything runs, nothing to start.
    let items = g[2].menu_items();
    assert_eq!(
        find(&items, "start-stopped").and_then(|e| e.disabled_reason),
        Some("Nothing is stopped".to_string())
    );
    assert!(find(&items, "stop-all").is_some_and(|e| e.disabled_reason.is_none()));
    assert!(g[2].restart_reason().is_none());
    // ml-notebooks: nothing runs, so stop and restart say why.
    let items = g[4].menu_items();
    assert!(find(&items, "start-stopped").is_some_and(|e| e.disabled_reason.is_none()));
    for id in ["stop-all", "restart-running"] {
        assert_eq!(
            find(&items, id).and_then(|e| e.disabled_reason),
            Some("Nothing is running".to_string()),
            "{id}"
        );
    }
    assert_eq!(g[4].restart_reason(), Some("Nothing is running"));
}

/// The opening tag of the first element whose markup contains `needle`.
fn tag_around<'a>(html: &'a str, needle: &str) -> &'a str {
    let at = html
        .find(needle)
        .unwrap_or_else(|| panic!("{needle} missing"));
    let start = html[..at].rfind('<').unwrap_or(0);
    let end = html[at..].find('>').map_or(html.len(), |i| at + i);
    &html[start..end]
}

#[test]
fn containers_screen_every_group_header_has_restart_and_more_even_when_collapsed() {
    let html = render_screen(|state, _| mock_state(state));
    for name in [
        "shop-api",
        "analytics",
        "berth-site",
        "payments-mock",
        "ml-notebooks",
        "docs-site",
        "Unassigned",
    ] {
        let more = format!("aria-label=\"More actions for {name}\"");
        assert_eq!(count(&html, &more), 1, "{name}: {html}");
        let restart = format!("aria-label=\"Restart running containers in {name}\"");
        assert_eq!(count(&html, &restart), 1, "{name}");
    }
    // Collapsed groups show the restart button, disabled with its reason.
    let tag = tag_around(&html, "Restart running containers in ml-notebooks");
    assert!(tag.contains("disabled"), "{tag}");
    assert!(tag.contains("data-tooltip=\"Nothing is running\""), "{tag}");
    let tag = tag_around(&html, "Restart running containers in shop-api\"");
    assert!(!tag.contains("disabled"), "{tag}");
    // The menu stays closed until the ⋯ button opens it.
    assert!(!html.contains("group-menu"), "{html}");
}

#[test]
fn containers_screen_group_menu_lists_the_group_actions_when_open() {
    let html = render_screen(|state, view| {
        mock_state(state);
        let m = model(&Filters::default(), GroupBy::Project, None);
        view.group_menu.set(Some(groups(&m)[0].key.clone()));
        let _ = state;
    });
    assert_eq!(count(&html, "group-menu"), 1, "{html}");
    assert!(
        html.contains("aria-label=\"More actions for shop-api\" role=\"menu\"")
            || html.contains("role=\"menu\" aria-label=\"More actions for shop-api\""),
        "{html}"
    );
    for label in ["Start 1 stopped", "Stop all running", "Restart running"] {
        assert!(html.contains(label), "{label}: {html}");
    }
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
        ["berth-site-proxy-1", "berth-site-web-1"]
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
        backing: crate::services::backing::Backing::default(),
    }
}

fn container<'a>(world: &'a berth_engine::MockWorld, name: &str) -> &'a Container {
    world
        .containers
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} in the mock world"))
}

/// Under mock data the row's Terminal button is disabled and says why;
/// under a real engine it is not. The other buttons are untouched.
#[test]
fn containers_row_terminal_button_is_disabled_with_a_reason_under_mock_data_only() {
    let world = world();
    let pending = BTreeMap::new();
    let terminal_of = |backing: crate::services::backing::Backing| {
        let view = StateView {
            backing,
            ..view_for(&world, &pending)
        };
        let data = row_data(
            container(&world, "shop-api-api-1"),
            &view,
            "default",
            chrono::Utc::now(),
            false,
        );
        let button = |action: RowAction| {
            data.buttons
                .iter()
                .find(|b| b.action == action)
                .map(|b| (b.disabled, b.tooltip.clone()))
                .expect("button")
        };
        (button(RowAction::Terminal), button(RowAction::Restart))
    };
    let (live_terminal, _) = terminal_of(crate::services::backing::Backing::Live);
    assert!(!live_terminal.0, "{live_terminal:?}");
    let (mock_terminal, mock_restart) = terminal_of(crate::services::backing::Backing::Mock);
    assert_eq!(
        mock_terminal,
        (
            true,
            "Terminals need a real Docker engine. You're viewing mock data.".to_string()
        )
    );
    assert!(!mock_restart.0, "Restart still works on the mock engine");
}

#[test]
fn containers_mock_uptimes_vary_like_the_frames_whatever_the_date() {
    // Frame Uptime column: 2 h, 6 h, 35 min, 3 d; the paused worker 1 d;
    // the restarting worker and exited containers a dash.
    let expected = [
        ("shop-api-api-1", Some("2 h")),
        ("shop-api-mailpit-1", Some("2 h")),
        ("analytics-clickhouse-1", Some("6 h")),
        ("berth-site-web-1", Some("35 min")),
        ("payments-mock-stripe-1", Some("35 min")),
        ("pg-scratch", Some("3 d")),
        ("api-worker-7f9c4d8b6-xk2lp", Some("1 d")),
        ("shop-api-worker-1", None),
        ("shop-api-db-1", None),
    ];
    for now in [
        chrono::Utc::now(),
        crate::services::mock_reference_now(),
        crate::services::mock_reference_now() + chrono::Duration::days(400),
    ] {
        let world = crate::services::mock_world_at(now);
        let pending = BTreeMap::new();
        let view = view_for(&world, &pending);
        for (name, uptime) in expected {
            let data = row_data(container(&world, name), &view, "default", now, false);
            assert_eq!(data.uptime.as_deref(), uptime, "{name} at {now}");
        }
    }
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
    // Terminal opens a shell (P3-T6); Delete asks first (P2-T8).
    let terminal = tooltip(&running, RowAction::Terminal);
    assert!(!terminal.1);
    assert_eq!(terminal.0, "Terminal · ⌘T");
    let delete = tooltip(&running, RowAction::Delete);
    assert!(!delete.1);
    assert_eq!(delete.0, "Delete \u{00b7} \u{232b}");
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
fn containers_row_lists_ports_only_while_the_container_is_bound() {
    let world = world();
    let pending = BTreeMap::new();
    let view = view_for(&world, &pending);
    let ports_of = |c: &Container| {
        let data = row_data(c, &view, "default", chrono::Utc::now(), false);
        (data.ports.len(), data.extra_ports)
    };
    // The frames' exited rows (shop-api-db-1, analytics-grafana-1) show a
    // dash although the container is configured with a host port.
    for name in ["shop-api-db-1", "analytics-grafana-1"] {
        let c = container(&world, name);
        assert_eq!(c.state, ContainerState::Exited, "{name}");
        assert!(!c.ports.is_empty(), "{name} is configured with a port");
        assert_eq!(ports_of(c), (0, 0), "{name}");
    }
    // The same container shows its port in every bound state, and none
    // in the others.
    let mut probe = container(&world, "berth-site-proxy-1").clone();
    assert_eq!(ports_of(&probe), (1, 0));
    for (state, shown) in [
        (ContainerState::Running, true),
        (ContainerState::Paused, true),
        (ContainerState::Restarting, false),
        (ContainerState::Created, false),
        (ContainerState::Exited, false),
    ] {
        probe.state = state;
        assert_eq!(ports_of(&probe).0, usize::from(shown), "{state:?}");
    }
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
    // Rows inside a group run by name, not by the engine's listing.
    assert_eq!(selected_names(&dom, state), ["shop-api-db-1"]);
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
        ["api-worker-7f9c4d8b6-xk2lp"],
        "collapsed groups are not stepped into"
    );
    // A filter changes what J can reach.
    dom.in_scope(ScopeId::APP, || {
        let mut handle = state;
        let mut filters = view.filters;
        filters.write().text = "berth-site".to_string();
        let proxy = state
            .containers
            .peek()
            .iter()
            .find(|c| c.name == "berth-site-proxy-1")
            .map(|c| c.id.clone())
            .expect("proxy");
        select_only(&mut handle, &proxy);
    });
    key("j");
    assert_eq!(selected_names(&dom, state), ["berth-site-web-1"]);
    key("j");
    assert_eq!(
        selected_names(&dom, state),
        ["berth-site-web-1"],
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
    crate::ui::dock::use_dock_view(state);
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

// ---------- live stats feed the rows (P3-T4, D26) ----------

mod stats_wiring {
    use std::cell::RefCell;

    use bytesize::ByteSize;

    use super::*;
    use crate::services::stats::{StatsKey, StatsService};

    thread_local! {
        static SERVICE: RefCell<Option<StatsService>> = const { RefCell::new(None) };
    }

    fn provide_service(state: &mut AppState, _view: &mut ContainersView) {
        mock_state(state);
        let service = StatsService::new(
            crate::services::engines::client_map(),
            tokio_util::sync::CancellationToken::new(),
        );
        SERVICE.with(|slot| *slot.borrow_mut() = Some(service.clone()));
        provide_context(service);
    }

    fn service() -> StatsService {
        SERVICE
            .with(|slot| slot.borrow().clone())
            .expect("service provided")
    }

    fn api_key() -> StatsKey {
        let world = world();
        let api = container(&world, "shop-api-api-1");
        StatsKey::new(&api.engine, &api.id)
    }

    fn reading(cpu: f32) -> berth_core::LiveStats {
        berth_core::LiveStats {
            cpu_pct: cpu,
            mem_used: ByteSize::mb(99),
            mem_limit: ByteSize::mb(512),
            net_rx: ByteSize::b(1),
            net_tx: ByteSize::b(1),
            at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn stats_rows_register_only_the_running_containers_they_render() {
        let mut dom = VirtualDom::new_with_props(
            ScreenRoot,
            ScreenRootProps {
                setup: SetupFn(provide_service),
            },
        );
        let _ = dom.rebuild_to_vec();
        let opened = service().reconcile().opened;
        let world = world();
        let api = container(&world, "shop-api-api-1");
        let db = container(&world, "shop-api-db-1");
        assert!(
            opened.contains(&StatsKey::new(&api.engine, &api.id)),
            "{opened:?}"
        );
        assert!(
            !opened.iter().any(|k| k.id == db.id),
            "an exited container has no stats stream"
        );
        assert!(opened.len() <= crate::services::stats::MAX_SUBSCRIPTIONS);
        assert!(opened.iter().all(|k| {
            world
                .containers
                .iter()
                .find(|c| c.id == k.id)
                .is_some_and(|c| {
                    matches!(c.state, ContainerState::Running | ContainerState::Paused)
                })
        }));
        // The screen going away releases every row: nothing stays open.
        drop(dom);
        let closed = service().reconcile().closed;
        assert_eq!(closed.len(), opened.len());
        assert_eq!(service().subscription_count(), 0);
    }

    #[test]
    fn stats_a_fresh_sample_replaces_the_listed_value_in_the_row() {
        let before = render_screen(provide_service);
        let again = render_screen(|state, view| {
            provide_service(state, view);
            service().seed(&api_key(), [(chrono::Utc::now(), reading(77.7))]);
        });
        assert!(!before.contains("77.7%"));
        assert!(again.contains("77.7%"), "{again}");
        assert!(again.contains("99 MB"), "{again}");
    }

    #[test]
    fn stats_a_stale_sample_is_not_shown() {
        let html = render_screen(|state, view| {
            provide_service(state, view);
            let old = chrono::Utc::now() - chrono::Duration::seconds(60);
            service().seed(&api_key(), [(old, reading(77.7))]);
        });
        assert!(!html.contains("77.7%"), "{html}");
    }

    #[test]
    fn stats_without_a_service_the_row_keeps_the_listed_value() {
        // The frame scenes run no service: the mock's own values stay.
        let html = render_screen(|state, _view| mock_state(state));
        assert!(html.contains("3.2%"), "{html}");
    }
}
