//! P2-T3 shell tests: the chrome renders what the frame shows, from the
//! mock world, and the keyboard/overlay behaviour moves real state.

use crate::keys::{self, KeyContext, Modifiers};
use crate::scenes;
use crate::state::{use_app_state, AppState, BottomTab, EngineScope, Overlay, Section, Selection};
use crate::ui::sidebar::SETTINGS_ACTION;
use crate::ui::*;

const FRAME: &str = "Containers / selected / 1440";

fn engine_id(s: &str) -> berth_core::EngineId {
    berth_core::EngineId(s.to_string())
}

fn mock_world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn scene_html() -> String {
    scenes::render_scene_html(FRAME).expect("the frame has a scene")
}

// ---------- VirtualDom harness: real signals ----------

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { div {} }
}

fn harness() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    (dom, state)
}

/// A state setup as a prop: compares by address, which is enough for a
/// props check that never sees two different setups.
#[derive(Clone, Copy)]
struct Setup(fn(&mut AppState));

impl PartialEq for Setup {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.0, other.0)
    }
}

/// Render the Shell for a custom state.
#[component]
fn ShellRoot(setup: Setup) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    use_hook(move || (setup.0)(&mut state));
    rsx! { crate::app::Shell {} }
}

fn render_shell(setup: fn(&mut AppState)) -> String {
    let mut dom = VirtualDom::new_with_props(
        ShellRoot,
        ShellRootProps {
            setup: Setup(setup),
        },
    );
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

fn render(element: Element) -> String {
    dioxus_ssr::render_element(element)
}

/// A component that renders `element` with an `AppState` in context.
#[component]
fn WithState(setup: Setup, children: Element) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    use_hook(move || (setup.0)(&mut state));
    rsx! { {children} }
}

// ---------- the frame: sidebar ----------

#[test]
fn shell_sidebar_lists_the_seven_mock_projects_with_status_dots() {
    let world = mock_world();
    let model = sidebar_model(
        &world.engines,
        &EngineScope::One(engine_id("colima:default")),
        &world.projects,
        &world.containers,
    );
    let rows: Vec<(&str, &str, Option<usize>)> = model
        .projects
        .iter()
        .map(|p| (p.name.as_str(), p.dot.label, p.services))
        .collect();
    assert_eq!(
        rows,
        [
            ("shop-api", "Failing", Some(5)),
            ("analytics", "Partly running", Some(4)),
            ("berth-site", "Running", Some(2)),
            ("payments-mock", "Running", Some(1)),
            ("ml-notebooks", "Stopped", Some(2)),
            ("docs-site", "Stopped", Some(1)),
            ("docker-manger", "Not created", None),
        ]
    );

    // The same rows reach the DOM, each with its dot as a named image.
    let html = scene_html();
    for (name, dot, _) in rows {
        assert!(html.contains(name), "{name} missing from the sidebar");
        assert!(
            html.contains(&format!("aria-label=\"{dot}\"")),
            "dot {dot} missing"
        );
    }
    // The engine scope hides the other engine's project.
    assert!(!html.contains(">landing<"), "landing is on hetzner-1");
    // Failing is the danger tone, not-created a hollow ring.
    assert!(html.contains("status-dot tone-danger"));
    assert!(html.contains("status-dot tone-stopped status-dot-hollow"));
}

#[test]
fn shell_sidebar_counts_follow_the_engine_scope() {
    let world = mock_world();
    let one = sidebar_model(
        &world.engines,
        &EngineScope::One(engine_id("colima:default")),
        &world.projects,
        &world.containers,
    );
    assert_eq!(one.containers, 23);
    assert_eq!(one.images, 31);
    assert_eq!(one.volumes, 12);
    assert_eq!(one.engines, 3);
    // The Projects badge is the Projects screen's count: every engine's
    // projects, whatever the scope (frames: 8).
    assert_eq!(one.projects_total, world.projects.len());
    assert_eq!(one.projects_total, 8);
    let all = sidebar_model(
        &world.engines,
        &EngineScope::All,
        &world.projects,
        &world.containers,
    );
    assert_eq!(all.containers, 43);
    assert_eq!(
        all.projects.len(),
        8,
        "all engines adds hetzner-1's landing"
    );
}

fn empty_projects(state: &mut AppState) {
    let world = mock_world();
    state.engines.set(world.engines.clone());
    state.containers.set(world.containers);
    state.engine_scope.set(EngineScope::All);
    state.projects.set(Vec::new());
}

#[test]
fn shell_sidebar_without_projects_has_no_group_label_or_badge() {
    let html = render_shell(empty_projects);
    assert!(
        !html.contains("class=\"sidebar-group-label\""),
        "no PROJECTS label over an empty list: {html}"
    );
    // The Projects item carries no badge either (frame "Projects / empty");
    // Containers still does.
    let projects = html
        .find("class=\"sidebar-label\">Projects<")
        .expect("Projects item");
    let containers = html
        .find("class=\"sidebar-label\">Containers<")
        .expect("Containers item");
    assert!(
        !html[projects..containers].contains("sidebar-count"),
        "{html}"
    );
    // With projects the label is back.
    assert!(scene_html().contains("class=\"sidebar-group-label\""));
}

#[test]
fn shell_sidebar_hides_the_unassigned_bucket() {
    let mut world = mock_world();
    let mut loose = world.projects[0].clone();
    loose.name = "Unassigned".to_string();
    loose.source = berth_core::ProjectSource::Loose;
    world.projects.push(loose);
    let model = sidebar_model(
        &world.engines,
        &EngineScope::All,
        &world.projects,
        &world.containers,
    );
    assert!(model.projects.iter().all(|p| p.name != "Unassigned"));
}

#[test]
fn shell_sidebar_marks_the_current_section_and_orders_them_like_the_keys() {
    let html = scene_html();
    let order = [
        "Projects",
        "Containers",
        "Logs",
        "Images",
        "Volumes",
        "Networks",
        "Templates",
        "Engines",
        "Registries",
        "Activity",
    ];
    let mut at = 0;
    for label in order {
        let found = html[at..]
            .find(&format!("class=\"sidebar-label\">{label}<"))
            .unwrap_or_else(|| panic!("{label} out of order or missing"));
        at += found + 1;
    }
    // Exactly one item is the current page, and it is Containers.
    assert_eq!(html.matches("aria-current=\"page\"").count(), 1, "{html}");
    let current = html.find("aria-current=\"page\"").expect("selected");
    let item_start = html[..current].rfind("<button").expect("button");
    let item = &html[item_start..item_start + 400];
    assert!(item.contains("sidebar-item-selected"), "{item}");
    assert!(item.contains("Containers"), "{item}");
}

#[test]
fn shell_sidebar_has_a_settings_item_after_plugins() {
    let html = scene_html();
    let plugins = html
        .find("class=\"sidebar-label\">Plugins<")
        .expect("Plugins item");
    let settings = html
        .find("class=\"sidebar-label\">Settings<")
        .expect("Settings item missing from the sidebar");
    assert!(settings > plugins, "Settings must follow Plugins");
    // Same markup as its neighbours: a button with the gear icon.
    let item_start = html[..settings].rfind("<button").expect("button");
    let item = &html[item_start..settings];
    assert!(item.contains("sidebar-item"), "{item}");
    assert!(item.contains("/icons.svg#settings"), "{item}");
    // Not the open page in the Containers frame.
    assert!(!item.contains("aria-current"), "{item}");
}

#[test]
fn shell_sidebar_settings_item_runs_the_settings_action_and_shows_selected() {
    let (dom, mut state) = harness();
    dom.in_runtime(|| {
        assert_ne!(*state.section.peek(), Section::Settings);
        keys::dispatch(&mut state, SETTINGS_ACTION);
        assert_eq!(*state.section.peek(), Section::Settings);
    });
    let html = render_shell(|state| state.section.set(Section::Settings));
    let at = html
        .find("class=\"sidebar-label\">Settings<")
        .expect("Settings item");
    let start = html[..at].rfind("<button").expect("button");
    let item = &html[start..at];
    assert!(item.contains("sidebar-item-selected"), "{item}");
    assert!(item.contains("aria-current=\"page\""), "{item}");
}

// ---------- the frame: status bar ----------

#[test]
fn shell_status_bar_text_is_exactly_the_frames() {
    let html = scene_html();
    assert!(
        html.contains(
            "<span class=\"statusbar-summary\" role=\"status\">colima/default \u{b7} Docker 28.4.0 \u{b7} 4 CPU \u{b7} 8 GB \u{b7} 15 running</span>"
        ),
        "{html}"
    );
    let world = mock_world();
    assert_eq!(
        engine_summary(
            &world.engines,
            &EngineScope::One(engine_id("colima:default"))
        ),
        "colima/default \u{b7} Docker 28.4.0 \u{b7} 4 CPU \u{b7} 8 GB \u{b7} 15 running"
    );
}

#[test]
fn shell_status_bar_names_engines_that_are_not_running() {
    let world = mock_world();
    assert_eq!(
        engine_summary(&world.engines, &EngineScope::One(engine_id("colima:work"))),
        "colima/work \u{b7} Stopped"
    );
    assert_eq!(
        engine_summary(
            &world.engines,
            &EngineScope::One(engine_id("ssh:hetzner-1"))
        ),
        "ssh/hetzner-1 \u{b7} Unreachable"
    );
    assert_eq!(
        engine_summary(&world.engines, &EngineScope::All),
        "All engines \u{b7} 3 engines \u{b7} 15 running"
    );
    assert_eq!(
        engine_summary(&[], &EngineScope::All),
        "Looking for engines\u{2026}"
    );
}

#[test]
fn shell_status_bar_shows_colimas_whole_gib_as_gb() {
    let mut world = mock_world();
    let engine = world
        .engines
        .iter_mut()
        .find(|e| e.id == engine_id("colima:default"))
        .expect("default engine");
    let vm = engine.resources.as_mut().expect("resources");
    vm.memory = bytesize::ByteSize::gib(8); // as Colima reports 8 GiB
    let text = engine_summary(
        &world.engines,
        &EngineScope::One(engine_id("colima:default")),
    );
    assert!(text.contains("\u{b7} 8 GB \u{b7}"), "{text}");
}

#[test]
fn shell_problems_label_counts_and_pluralises() {
    assert_eq!(problems_label(0), "No problems");
    assert_eq!(problems_label(1), "1 problem");
    assert_eq!(problems_label(3), "3 problems");
    // The frame's title bar and status bar both say "3 problems".
    let html = scene_html();
    assert_eq!(html.matches("<span>3 problems</span>").count(), 2, "{html}");
}

// ---------- the frame: title bar ----------

#[test]
fn shell_title_bar_has_switcher_palette_trigger_and_problems() {
    let html = scene_html();
    let bar = &html[html.find("<header class=\"titlebar\"").expect("title bar")..];
    let bar = &bar[..bar.find("</header>").expect("end")];
    assert!(
        bar.contains("class=\"engine-switcher-name\">default<"),
        "{bar}"
    );
    assert!(
        bar.contains("aria-label=\"Running\""),
        "switcher dot: {bar}"
    );
    assert!(bar.contains("Search or run a command\u{2026}"));
    let mod_name = if crate::actions::mod_label() == "\u{2318}" {
        "Command \u{2318}"
    } else {
        "Control \u{2303}"
    };
    assert!(
        bar.contains(&format!("aria-label=\"{mod_name}\"")),
        "Kbd mod"
    );
    assert!(bar.contains(">K<"));
    assert!(bar.contains("3 problems"));
}

#[test]
fn shell_engine_switcher_names_the_scope_and_lists_every_engine_when_open() {
    let closed = render(rsx! {
        WithState { setup: Setup(|s: &mut AppState| { s.engines.set(mock_world().engines); s.engine_scope.set(EngineScope::All); }),
            EngineSwitcher {}
        }
    });
    assert!(closed.contains("All engines"), "{closed}");
    assert!(
        !closed.contains("role=\"menu\""),
        "closed menu renders nothing"
    );

    let open = render(rsx! {
        WithState { setup: Setup(|s: &mut AppState| { s.engines.set(mock_world().engines); s.engine_scope.set(EngineScope::All); }),
            EngineSwitcher { initially_open: true }
        }
    });
    for name in ["default", "work", "hetzner-1"] {
        assert!(
            open.contains(&format!(">{name}<")),
            "{name} in menu: {open}"
        );
    }
    assert!(open.contains("aria-label=\"Unreachable\""));
    assert!(open.contains("aria-label=\"Stopped\""));
    assert_eq!(
        open.matches("aria-checked=\"true\"").count(),
        1,
        "All engines is checked"
    );
}

#[test]
fn shell_engine_switcher_says_when_the_engine_is_stopped() {
    let html = render(rsx! {
        WithState { setup: Setup(|s: &mut AppState| { s.engines.set(mock_world().engines); s.engine_scope.set(EngineScope::One(engine_id("colima:work"))); }),
            EngineSwitcher {}
        }
    });
    assert!(html.contains("class=\"engine-state\">Stopped<"), "{html}");
    // Frame "engine stopped": the dot comes before the word.
    let dot = html.find("aria-label=\"Stopped\"").expect("dot");
    let word = html.find("class=\"engine-state\"").expect("word");
    assert!(dot < word, "dot before word: {html}");
}

// ---------- connection states in the chrome ----------

fn default_engine_with(
    edit: impl FnOnce(&mut berth_core::Engine),
) -> (Vec<berth_core::Engine>, EngineScope) {
    let mut engines = mock_world().engines;
    let engine = engines
        .iter_mut()
        .find(|e| e.id == engine_id("colima:default"))
        .expect("default engine");
    edit(engine);
    (engines, EngineScope::One(engine_id("colima:default")))
}

#[test]
fn shell_connection_follows_the_scoped_engine() {
    let world = mock_world();
    let default = EngineScope::One(engine_id("colima:default"));
    assert_eq!(connection(&world.engines, &default), Connection::Ready);
    assert_eq!(
        connection(&world.engines, &EngineScope::All),
        Connection::Ready
    );
    assert_eq!(connection(&[], &default), Connection::Connecting);
    let (engines, scope) = default_engine_with(|e| e.docker_version = None);
    assert_eq!(connection(&engines, &scope), Connection::Connecting);
    let (engines, scope) = default_engine_with(|e| e.state = berth_core::EngineState::Stopped);
    assert_eq!(connection(&engines, &scope), Connection::EngineStopped);
    // A stopped engine outside the scope does not change the chrome.
    assert_eq!(
        connection(
            &world.engines,
            &EngineScope::One(engine_id("colima:default"))
        ),
        Connection::Ready
    );
    let (engines, scope) = default_engine_with(|e| {
        e.state = berth_core::EngineState::Stopped;
        e.docker_version = None;
    });
    assert_eq!(connection(&engines, &scope), Connection::EngineStopped);
    assert_eq!(
        engine_summary(&engines, &scope),
        "colima/default \u{b7} Stopped"
    );
    let (engines, scope) = default_engine_with(|e| e.docker_version = None);
    assert_eq!(engine_summary(&engines, &scope), "Connecting\u{2026}");
}

fn titlebar_of(html: &str) -> &str {
    let bar = &html[html.find("<header class=\"titlebar\"").expect("title bar")..];
    &bar[..bar.find("</header>").expect("end")]
}

fn statusbar_of(html: &str) -> &str {
    &html[html
        .find("<footer class=\"statusbar\"")
        .expect("status bar")..]
}

#[test]
fn shell_loading_scene_checks_and_connects_and_keeps_the_last_known_count() {
    let html = scenes::render_scene_html("Containers / loading / 1440").expect("scene");
    let bar = titlebar_of(&html);
    assert!(bar.contains("Checking\u{2026}"), "{bar}");
    assert!(bar.contains("btn-spinner"), "a spinner, not a check: {bar}");
    assert!(!bar.contains("No problems"), "{bar}");
    assert!(
        !bar.contains("engine-state"),
        "a running engine says no word: {bar}"
    );
    let status = statusbar_of(&html);
    assert!(status.contains("Connecting\u{2026}"), "{status}");
    assert!(status.contains("Checking\u{2026}"), "{status}");
    // The sidebar keeps the 23 the engine last reported.
    let containers = html
        .find("class=\"sidebar-label\">Containers<")
        .expect("Containers item");
    let after = &html[containers..];
    assert!(
        after[..after.find("</button>").expect("end")].contains(">23<"),
        "{after}"
    );
}

#[test]
fn shell_engine_stopped_scene_warns_and_greys_the_project_dots() {
    let html = scenes::render_scene_html("Containers / engine stopped / 1440").expect("scene");
    let bar = titlebar_of(&html);
    assert!(bar.contains("Engine stopped"), "{bar}");
    assert!(bar.contains("/icons.svg#alert-triangle"), "{bar}");
    assert!(!bar.contains("No problems"), "{bar}");
    assert!(titlebar_of(&html).contains("class=\"engine-state\">Stopped<"));
    let status = statusbar_of(&html);
    assert!(status.contains("colima/default \u{b7} Stopped"), "{status}");
    assert!(status.contains("Engine stopped"), "{status}");
    // The sidebar's project dots no longer claim a live state.
    let sidebar = &html[html.find("<nav class=\"sidebar\"").expect("sidebar")..];
    let sidebar = &sidebar[..sidebar.find("</nav>").expect("end")];
    for live in ["tone-danger", "tone-warning", "tone-success"] {
        assert!(!sidebar.contains(live), "{live}: {sidebar}");
    }
    assert!(sidebar.contains("status-dot tone-stopped status-dot-hollow"));
}

#[test]
fn shell_sidebar_model_greys_dots_of_a_stopped_engine_only() {
    let world = mock_world();
    let (engines, scope) = default_engine_with(|e| e.state = berth_core::EngineState::Stopped);
    let model = sidebar_model(&engines, &scope, &world.projects, &world.containers);
    let labels: Vec<&str> = model.projects.iter().map(|p| p.dot.label).collect();
    assert_eq!(
        labels,
        [
            "Stopped",
            "Stopped",
            "Stopped",
            "Stopped",
            "Stopped",
            "Stopped",
            "Not created"
        ]
    );
    // A running engine keeps the live dots.
    let live = sidebar_model(&world.engines, &scope, &world.projects, &world.containers);
    assert_eq!(live.projects[0].dot.label, "Failing");
}

#[test]
fn shell_sidebar_count_keeps_the_last_known_total_until_the_engine_answers() {
    let world = mock_world();
    let (engines, scope) = default_engine_with(|e| e.docker_version = None);
    let model = sidebar_model(&engines, &scope, &world.projects, &[]);
    assert_eq!(model.containers, 23);
    // An engine that answered with nothing running shows its real zero.
    let model = sidebar_model(&world.engines, &scope, &world.projects, &[]);
    assert_eq!(model.containers, 0);
}

#[test]
fn shell_scope_includes_only_the_chosen_engine() {
    assert!(scope_includes(&EngineScope::All, &engine_id("x")));
    assert!(scope_includes(
        &EngineScope::One(engine_id("a")),
        &engine_id("a")
    ));
    assert!(!scope_includes(
        &EngineScope::One(engine_id("a")),
        &engine_id("b")
    ));
}

// ---------- inspector and bottom panel ----------

#[test]
fn shell_inspector_shows_for_a_matching_selection_only() {
    let c = Selection::Containers(vec![berth_core::ContainerId("x".to_string())]);
    assert!(inspector_visible(Section::Containers, &c));
    assert!(!inspector_visible(Section::Containers, &Selection::None));
    assert!(!inspector_visible(
        Section::Containers,
        &Selection::Containers(vec![])
    ));
    assert!(
        !inspector_visible(Section::Images, &c),
        "a container is not an image"
    );
    assert!(inspector_visible(
        Section::Images,
        &Selection::Image("sha".to_string())
    ));
    assert!(!inspector_visible(
        Section::Project,
        &Selection::Project(berth_core::ProjectId("p".to_string()))
    ));

    let html = scene_html();
    assert!(html.contains("<aside class=\"inspector\""), "{html}");
    let none = render_shell(|s| s.section.set(Section::Containers));
    assert!(
        !none.contains("class=\"inspector"),
        "no selection, no inspector"
    );
}

#[test]
fn shell_inspector_frames_carry_their_class_width_and_handle() {
    let docked = render(rsx! { Inspector { width_px: 420 } });
    assert!(docked.contains("class=\"inspector\""), "{docked}");
    assert!(
        docked.contains("style=\"width: 420px; min-width: 420px; flex: none;\""),
        "{docked}"
    );
    let overlay = render(rsx! { Inspector { frame: InspectorFrame::Overlay } });
    assert!(
        overlay.contains("class=\"inspector inspector-overlay\""),
        "{overlay}"
    );
    let bottom = render(rsx! { Inspector { frame: InspectorFrame::Bottom } });
    assert!(
        bottom.contains("class=\"inspector inspector-bottom\""),
        "{bottom}"
    );
    assert!(
        !bottom.contains("style="),
        "no width: the region gives it all: {bottom}"
    );
    // The body scrolls inside the frame so the handle stays on the edge.
    assert!(docked.contains("class=\"inspector-body\""));
    // The CSS: the overlay floats over the main area, the handle is an 8 px
    // col-resize strip, the body scrolls.
    let css = include_str!("../../assets/css/shell.css");
    let rule = |header: &str| crate::screens::css_tracks::block(css, header);
    assert!(rule("\n.inspector-overlay {").contains("position: absolute;"));
    assert!(rule("\n.split-handle {").contains("width: 8px;"));
    assert!(rule("\n.split-handle {").contains("cursor: col-resize;"));
    assert!(rule("\n.inspector-body {").contains("overflow-y: auto;"));
}

#[test]
fn shell_right_column_has_a_resize_separator_with_the_aria_values() {
    let html = scene_html();
    let at = html.find("split-handle-width").expect("a width handle");
    let handle = &html[at - 40..at + 400];
    for needle in [
        "role=\"separator\"",
        "tabindex=\"0\"",
        "aria-orientation=\"vertical\"",
        "aria-valuenow=\"360\"",
        "aria-valuemin=\"320\"",
        "aria-valuemax=\"560\"",
    ] {
        assert!(handle.contains(needle), "{needle} in {handle}");
    }
    assert!(html.contains("width: 360px; min-width: 360px;"), "{html}");
}

#[test]
fn shell_window_under_1200_overlays_the_inspector_and_hides_it_until_space() {
    // The frame names its window width: 1440 docks, 1024 does not.
    let wide = scenes::render_scene_html("Containers / selected / 1440").expect("scene");
    assert!(wide.contains("<aside class=\"inspector\""), "{wide}");
    let narrow = scenes::render_scene_html("Containers / selected / 1024").expect("scene");
    assert!(!narrow.contains("<aside"), "{narrow}");
    // Space opens it as an overlay at the stored width.
    let open = scenes::render_scene_html("Containers / inspector overlay / 1024").expect("scene");
    assert!(
        open.contains("class=\"inspector inspector-overlay\""),
        "{open}"
    );
    assert!(open.contains("width: 360px; min-width: 360px;"), "{open}");
}

fn select_first_container(s: &mut AppState) {
    let world = mock_world();
    s.engines.set(world.engines.clone());
    s.containers.set(world.containers.clone());
    s.projects.set(world.projects.clone());
    s.section.set(Section::Containers);
    s.selection
        .set(Selection::Containers(vec![world.containers[0].id.clone()]));
}

fn dock_bottom_inspector_alone(s: &mut AppState) {
    select_first_container(s);
    s.settings.write().inspector_dock = berth_store::InspectorDock::Bottom;
}

fn dock_bottom_split(s: &mut AppState) {
    dock_bottom_inspector_alone(s);
    let mut bottom = s.bottom.write();
    bottom.open = true;
    bottom.tab = BottomTab::Problems;
}

#[test]
fn shell_bottom_dock_alone_fills_the_region_with_the_inspector_at_the_shared_height() {
    let html = render_shell(dock_bottom_inspector_alone);
    assert!(html.contains("data-mode=\"inspector\""), "{html}");
    assert!(
        html.contains("class=\"inspector inspector-bottom\""),
        "{html}"
    );
    assert!(
        !html.contains("class=\"inspector\""),
        "not also in a right column"
    );
    assert!(!html.contains("bottom-main"), "the panel is closed");
    assert!(
        !html.contains("split-handle"),
        "no divider without a second pane"
    );
    // One height for the whole region: the panel's own, 320 px by default.
    assert!(html.contains("height: 320px;"), "{html}");
    // The inspector alone keeps its Logs and Terminal tabs.
    assert_eq!(html.matches("<span>Terminal</span>").count(), 1, "{html}");
}

#[test]
fn shell_bottom_dock_split_puts_the_panel_left_and_the_inspector_right_without_duplicate_tabs() {
    let html = render_shell(dock_bottom_split);
    assert!(html.contains("data-mode=\"split\""), "{html}");
    let panel = html.find("bottom-main").expect("the panel pane");
    let inspector = html
        .find("inspector inspector-bottom")
        .expect("the inspector pane");
    assert!(
        panel < inspector,
        "panel on the left, inspector on the right"
    );
    // 40 % of the 1220 px region.
    // `flex: none` keeps `.inspector-bottom`'s `flex: 1` from splitting the
    // region 50:50 whatever the width says.
    assert!(
        html.contains("width: 488px; min-width: 488px; flex: none;"),
        "{html}"
    );
    let at = html.find("split-handle-split").expect("a divider");
    let handle = &html[at - 40..at + 400];
    for needle in [
        "role=\"separator\"",
        "tabindex=\"0\"",
        "aria-orientation=\"vertical\"",
        "aria-valuenow=\"488\"",
        "aria-valuemin=\"320\"",
        "aria-valuemax=\"900\"",
    ] {
        assert!(handle.contains(needle), "{needle} in {handle}");
    }
    // The panel's Logs and Terminal tabs are the only ones.
    assert_eq!(html.matches("<span>Terminal</span>").count(), 1, "{html}");
    assert_eq!(html.matches("<span>Logs</span>").count(), 1, "{html}");
    assert!(html.contains("<span>Stats</span>"), "{html}");
}

#[test]
fn shell_right_dock_with_the_panel_open_keeps_both_sets_of_tabs() {
    let html = render_shell(|s| {
        select_first_container(s);
        let mut bottom = s.bottom.write();
        bottom.open = true;
        bottom.tab = BottomTab::Problems;
    });
    assert!(html.contains("data-mode=\"panel\""), "{html}");
    assert!(html.contains("<aside class=\"inspector\""), "{html}");
    assert_eq!(html.matches("<span>Terminal</span>").count(), 2, "{html}");
}

#[test]
fn shell_bottom_dock_without_a_selection_shows_the_panel_alone() {
    let html = render_shell(|s| {
        s.section.set(Section::Containers);
        s.settings.write().inspector_dock = berth_store::InspectorDock::Bottom;
        s.bottom.write().open = true;
    });
    assert!(html.contains("data-mode=\"panel\""), "{html}");
    assert!(!html.contains("<aside"), "{html}");
    // And with the panel closed too there is no region at all.
    let none = render_shell(|s| {
        s.section.set(Section::Containers);
        s.settings.write().inspector_dock = berth_store::InspectorDock::Bottom;
    });
    assert!(!none.contains("bottom-panel"), "{none}");
}

#[test]
fn shell_bottom_panel_renders_only_when_open_with_its_four_tabs() {
    let closed = render_shell(|s| s.section.set(Section::Containers));
    assert!(
        !closed.contains("class=\"bottom-panel\""),
        "closed by default"
    );

    let open = render_shell(|s| {
        let mut bottom = s.bottom.write();
        bottom.open = true;
        bottom.height_px = 300;
        bottom.tab = BottomTab::Problems;
    });
    assert!(open.contains("class=\"bottom-panel\""), "{open}");
    assert!(open.contains("height: 300px;"));
    for tab in ["Logs", "Terminal", "Problems", "Assistant"] {
        assert!(open.contains(&format!("<span>{tab}</span>")), "{tab} tab");
    }
    assert!(open.contains("aria-label=\"Resize panel\""));
    assert!(open.contains("aria-label=\"Close panel\""));
    let selected = open.find("aria-selected=\"true\"").expect("a selected tab");
    assert!(
        open[selected..selected + 200].contains("Problems"),
        "Problems selected"
    );
}

#[test]
fn shell_bottom_panel_tabs_have_the_inspectors_padding_and_underline() {
    // The same strip as the inspector's: underline tabs, the selected one
    // carries the accent underline on its own bottom border.
    let html = render_shell(|s| {
        s.bottom.write().open = true;
        s.bottom.write().tab = BottomTab::Terminal;
    });
    let strip = html
        .find("aria-label=\"Panel\"")
        .expect("the panel's tab strip");
    let head = &html[strip - 80..strip + 40];
    assert!(head.contains("tabs tabs-underline"), "{head}");
    assert!(!head.contains("tabs-panel"), "{head}");
    let selected = html.find("aria-selected=\"true\"").expect("a selected tab");
    assert!(
        html[selected - 80..selected].contains("tab tab-selected"),
        "{html}"
    );
    // The stylesheet gives them the inspector's padding and gap.
    use crate::screens::css_tracks::{block, declaration};
    let shell = include_str!("../../assets/css/shell.css");
    let detail = include_str!("../../assets/css/container_detail.css");
    let padding = |css: &str, header: &str| declaration(block(css, header), "padding");
    assert_eq!(
        padding(shell, "\n.bottom-head .tab {"),
        padding(detail, "\n.cd-tabs .tab {")
    );
    let gap = |css: &str, header: &str| declaration(block(css, header), "gap");
    assert_eq!(
        gap(shell, "\n.bottom-head .tabs-underline {"),
        gap(detail, "\n.cd-tabs .tabs-underline {")
    );
    // Nothing of the panel variant's top line is left on them.
    assert!(
        block(shell, "\n.bottom-head .tab {").contains("border-top: none;"),
        "the panel button rule's top line must not show on tabs"
    );
}

#[test]
fn shell_bottom_panel_height_is_clamped_between_160_and_60_percent() {
    assert_eq!(clamp_panel_height(50.0, 900.0), 160);
    assert_eq!(clamp_panel_height(280.0, 900.0), 280);
    assert_eq!(clamp_panel_height(5000.0, 900.0), 540);
    // A window shorter than 160 / 0.6 keeps the minimum instead of inverting the range.
    assert_eq!(clamp_panel_height(300.0, 200.0), 160);
}

// ---------- keyboard and overlay stack ----------

fn press(state: &mut AppState, key: &str, mods: Modifiers) {
    let ctx = keys::context_from_state(state);
    if let Some(action) = keys::map_key(key, mods, ctx) {
        keys::dispatch(state, action);
    }
}

#[test]
fn shell_cmd_digits_switch_sections_in_sidebar_order() {
    let (dom, mut state) = harness();
    let expected = [
        ("1", Section::Projects),
        ("2", Section::Containers),
        ("3", Section::Logs),
        ("4", Section::Images),
        ("5", Section::Volumes),
        ("6", Section::Networks),
        ("7", Section::Templates),
        ("8", Section::Engines),
        ("9", Section::Activity),
    ];
    dom.in_runtime(|| {
        for (key, section) in expected {
            press(&mut state, key, Modifiers::META);
            assert_eq!(*state.section.peek(), section, "Cmd+{key}");
        }
    });
}

#[test]
fn shell_ctrl_backtick_toggles_the_bottom_panel() {
    let (dom, mut state) = harness();
    dom.in_runtime(|| {
        assert!(!state.bottom.peek().open);
        press(&mut state, "`", Modifiers::CTRL);
        assert!(state.bottom.peek().open);
        press(&mut state, "`", Modifiers::CTRL);
        assert!(!state.bottom.peek().open);
    });
}

#[test]
fn shell_escape_closes_only_the_top_overlay() {
    let (dom, mut state) = harness();
    dom.in_runtime(|| {
        press(&mut state, "k", Modifiers::META); // palette
        state.overlays.write().push(Overlay::NewProject);
        assert_eq!(state.overlays.peek().len(), 2);
        press(&mut state, "Escape", Modifiers::NONE);
        assert_eq!(*state.overlays.peek(), vec![Overlay::Palette]);
        press(&mut state, "Escape", Modifiers::NONE);
        assert!(state.overlays.peek().is_empty());
        // With nothing open Esc is not Berth's key.
        assert_eq!(
            keys::map_key("Escape", Modifiers::NONE, KeyContext::default()),
            None
        );
    });
}

#[test]
fn shell_overlay_stack_renders_every_entry_with_a_dialog() {
    let html = render_shell(|s| {
        let mut overlays = s.overlays.write();
        overlays.push(Overlay::Palette);
        overlays.push(Overlay::NewProject);
    });
    assert_eq!(html.matches("class=\"overlay-scrim\"").count(), 2, "{html}");
    assert!(html.contains("aria-label=\"Command palette\""));
    assert!(html.contains("aria-label=\"New project\""));
    assert!(html.contains("z-index: 50;") && html.contains("z-index: 51;"));
    let none = render_shell(|_| {});
    assert!(!none.contains("overlay-scrim\""));
}

#[test]
fn shell_root_is_focusable_so_shortcuts_reach_the_handler() {
    let html = scene_html();
    assert!(html.contains("class=\"shell\" tabindex=\"-1\""), "{html}");
}

// ---------- palette trigger ----------

#[test]
fn shell_palette_trigger_is_a_button_with_the_cmd_k_hint() {
    let html = render(rsx! {
        WithState { setup: Setup(|_: &mut AppState| {}), PaletteTrigger {} }
    });
    assert!(html.contains("<button class=\"palette-trigger\""), "{html}");
    assert!(html.contains("Search or run a command\u{2026}"));
    let symbol = if crate::actions::mod_label() == "\u{2318}" {
        "cmd"
    } else {
        "ctrl"
    };
    assert!(html.contains(&format!("data-symbol=\"{symbol}\"")));
    assert!(html.contains(">K<"));
}

#[test]
fn shell_sidebar_projects_badge_does_not_count_the_unassigned_row() {
    let world = mock_world();
    let mut projects = world.projects.clone();
    projects.extend(scenes::loose_projects(&world));
    assert!(projects.len() > world.projects.len());
    let model = sidebar_model(
        &world.engines,
        &EngineScope::All,
        &projects,
        &world.containers,
    );
    assert_eq!(model.projects_total, 8);
    // Neither does its list.
    assert_eq!(model.projects.len(), 8);
}
