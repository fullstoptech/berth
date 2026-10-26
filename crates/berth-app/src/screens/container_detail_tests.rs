//! P3-T3 tests: the inspector body in each state (running, exited, restart
//! loop, paused, removed while open), the tabs and their placeholders, and
//! the Env tab and Inspect tree showing no env value that was not revealed.
//!
//! The values below are made-up stand-ins. The fixture password lives in
//! `fixtures/` and `scripts/` only (CLAUDE.md); where a test needs the
//! fixture's real shape it reads it from the fixture file at run time.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use berth_core::{
    Container, ContainerState, EnvVar, Fix, FixEffect, FixSource, Problem, ProblemId, ProblemKind,
    Severity,
};
use berth_engine::EngineClient;

use super::model::{
    env_rows, header_buttons, header_menu, mount_line, restart_label, short_id, state_note,
    tab_capacity, tab_strip, visible_tabs, DetailTab, EnvShown, Reveals, REVEAL_FOR,
};
use super::*;
use crate::services::actions::StateView;
use crate::services::backing::Backing;
use crate::services::detail::{mask_inspect, provide_detail_store, DetailEntry, Loaded};
use crate::state::{use_app_state, PendingKey, PendingState};

const SECRET_A: &str = "ex-secret-alpha-93f1";
const SECRET_B: &str = "ex-secret-bravo-77c2";

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn container(name: &str) -> Container {
    world()
        .containers
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} in the mock world"))
}

/// A detail as the mock engine gives it, with the log lines the service
/// would have read and the env this test wants.
fn loaded(c: &Container, env: &[(&str, &str)], last_lines: &[&str]) -> Loaded {
    let engine = berth_engine::MockEngine::bundled(&c.engine);
    let mut detail =
        futures::executor::block_on(engine.container(&c.id)).expect("the mock knows it");
    detail.container = c.clone();
    detail.env = env
        .iter()
        .map(|(key, value)| EnvVar {
            key: (*key).to_string(),
            value: (*value).to_string(),
        })
        .collect();
    let raw_env: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    detail.inspect = serde_json::json!({
        "Config": { "Env": raw_env, "Image": c.image.reference },
        "Name": format!("/{}", c.name),
        "State": { "Status": "running" },
    });
    let inspect = mask_inspect(detail.inspect.clone());
    Loaded {
        detail,
        inspect,
        last_lines: last_lines.iter().map(|l| (*l).to_string()).collect(),
    }
}

#[derive(Clone, PartialEq)]
struct Panel {
    containers: Vec<Container>,
    id: berth_core::ContainerId,
    entry: Option<DetailEntry>,
    problems: Vec<Problem>,
    pending: BTreeMap<PendingKey, PendingState>,
    tab: DetailTab,
    pane_px: u32,
    hide_stream_tabs: bool,
}

impl Panel {
    fn of(c: &Container) -> Self {
        Self {
            containers: world().containers_for(&c.engine),
            id: c.id.clone(),
            entry: Some(DetailEntry::Ready(Box::new(loaded(c, &[], &[])))),
            problems: Vec::new(),
            pending: BTreeMap::new(),
            tab: DetailTab::Overview,
            pane_px: 360,
            hide_stream_tabs: false,
        }
    }

    fn with(mut self, c: &Container) -> Self {
        for slot in &mut self.containers {
            if slot.id == c.id {
                *slot = c.clone();
            }
        }
        self
    }
}

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn PanelRoot(panel: Panel) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    let store = provide_detail_store();
    let seeded = panel.clone();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
        state.containers.set(seeded.containers.clone());
        state.engines.set(world().engines);
        state.problems.set(seeded.problems.clone());
        state.pending.set(seeded.pending.clone());
        if let Some(entry) = &seeded.entry {
            let mut entries = store.entries;
            entries.write().insert(seeded.id.clone(), entry.clone());
        }
    });
    rsx! {
        ContainerDetailPanel {
            id: panel.id.clone(),
            initial_tab: panel.tab,
            pane_px: panel.pane_px,
            hide_stream_tabs: panel.hide_stream_tabs,
        }
    }
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

fn render(panel: Panel) -> String {
    let mut dom = VirtualDom::new_with_props(PanelRoot, PanelRootProps { panel });
    let _ = dom.rebuild_to_vec();
    strip_styles(&dioxus_ssr::render(&dom))
}

fn count(html: &str, needle: &str) -> usize {
    html.matches(needle).count()
}

fn pending_of(
    c: &Container,
    action: &'static str,
    label: &str,
) -> BTreeMap<PendingKey, PendingState> {
    BTreeMap::from([(
        PendingKey {
            subject: berth_core::Subject::Container(c.id.clone()),
            action,
        },
        PendingState::Running {
            label: label.to_string(),
        },
    )])
}

fn problem_for(c: &Container, fixes: Vec<Fix>) -> Problem {
    let kind = ProblemKind::Unhealthy;
    let subject = berth_core::Subject::Container(c.id.clone());
    Problem {
        id: ProblemId::new(&kind, &subject),
        kind,
        severity: Severity::Error,
        subject,
        title: "worker keeps restarting".to_string(),
        explanation: "It cannot reach the database.".to_string(),
        evidence: vec!["Error: connect ECONNREFUSED".to_string()],
        fixes,
        first_seen: chrono::Utc::now() - chrono::Duration::seconds(130),
    }
}

fn stop_fix(c: &Container) -> Fix {
    Fix {
        label: "Stop the restart loop".to_string(),
        source: FixSource::Rule,
        effect: FixEffect::RunAction {
            action: berth_core::EngineAction::Container {
                engine: c.engine.clone(),
                id: c.id.clone(),
                action: ContainerAction::Stop { timeout_s: None },
            },
            effect: berth_core::EffectSummary {
                verb: "Stop".to_string(),
                noun_one: "container".to_string(),
                noun_many: "containers".to_string(),
                count: 1,
                bytes: None,
                lines: Vec::new(),
                typed_name: None,
            },
        },
    }
}

// ---------------------------------------------------------------------
// States
// ---------------------------------------------------------------------

#[test]
fn inspector_running_container_shows_its_buttons_note_and_overview_rows() {
    let api = container("shop-api-api-1");
    let html = render(Panel::of(&api));
    for needle in [
        "shop-api-api-1",
        ">Stop<",
        ">Restart<",
        ">Pause<",
        ">Kill<",
        ">Delete…<",
        "Image",
        "Command",
        "Ports",
        "Mounts",
        "Networks",
        "Restart policy",
        "role=\"tabpanel\"",
    ] {
        assert!(html.contains(needle), "missing {needle}: {html}");
    }
    assert!(html.contains("Up "), "the uptime note: {html}");
    assert!(!html.contains("Exit code"), "a running container has none");
    assert!(!html.contains("Last "), "no log tail while running");
}

#[test]
fn inspector_exited_container_shows_exit_code_and_its_last_lines() {
    let db = container("shop-api-db-1");
    assert_eq!(db.state, ContainerState::Exited);
    let mut panel = Panel::of(&db);
    panel.entry = Some(DetailEntry::Ready(Box::new(loaded(
        &db,
        &[],
        &["PostgreSQL init", "FATAL: port is already allocated"],
    ))));
    let html = render(panel);
    assert!(html.contains("Exit code"), "{html}");
    assert!(
        html.contains("kv-danger"),
        "a non-zero code is danger: {html}"
    );
    assert!(html.contains("Last 2 lines"), "{html}");
    assert!(html.contains("FATAL: port is already allocated"), "{html}");
    // Start replaces Stop; Pause and Kill say why they cannot run.
    assert!(html.contains(">Start<"), "{html}");
    assert!(
        html.contains("data-tooltip=\"Only a running container can be paused\""),
        "{html}"
    );
    assert!(html.contains("data-tooltip=\"Not running\""), "{html}");
}

#[test]
fn inspector_exited_container_without_output_says_so() {
    let db = container("shop-api-db-1");
    let html = render(Panel::of(&db));
    assert!(html.contains("No output."), "{html}");
}

#[test]
fn inspector_restart_loop_shows_the_count_and_offers_to_stop_it() {
    let worker = container("shop-api-worker-1");
    assert_eq!(worker.state, ContainerState::Restarting);
    let html = render(Panel::of(&worker));
    assert!(html.contains("restarted 14 times"), "{html}");
    assert!(html.contains("Restarting (1)"), "Docker's wording: {html}");
    assert!(html.contains("keeps restarting"), "{html}");
    assert_eq!(count(&html, "Stop the restart loop"), 1, "{html}");
    assert!(html.contains("restarted 14 times."), "{html}");
}

#[test]
fn inspector_restart_loop_with_a_problem_that_stops_it_shows_one_card_not_two() {
    let worker = container("shop-api-worker-1");
    let mut panel = Panel::of(&worker);
    panel.problems = vec![problem_for(&worker, vec![stop_fix(&worker)])];
    let html = render(panel);
    assert_eq!(count(&html, "Stop the restart loop"), 1, "{html}");
    assert!(
        !html.contains("shop-api-worker-1 keeps restarting"),
        "the built-in card: {html}"
    );
    assert!(html.contains("worker keeps restarting"), "{html}");
    assert!(html.contains("ECONNREFUSED"), "{html}");
    assert!(!html.contains("Go to "), "no dependency named: {html}");
}

#[test]
fn inspector_problem_card_offers_the_dependency_it_names() {
    let worker = container("shop-api-worker-1");
    let mut problem = problem_for(&worker, vec![stop_fix(&worker)]);
    problem.kind = ProblemKind::DependencyFailed {
        service: "db".to_string(),
    };
    let mut panel = Panel::of(&worker);
    panel.problems = vec![problem];
    let html = render(panel);
    assert!(html.contains("Go to shop-api-db-1"), "{html}");
}

#[test]
fn inspector_a_harness_fix_is_disabled_until_a_harness_exists() {
    let worker = container("shop-api-worker-1");
    let ai = Fix {
        label: "Fix with Claude Code".to_string(),
        source: FixSource::Harness(berth_core::HarnessId("claude".to_string())),
        effect: FixEffect::Explain,
    };
    let mut panel = Panel::of(&worker);
    panel.problems = vec![problem_for(&worker, vec![stop_fix(&worker), ai])];
    let html = render(panel);
    assert!(html.contains("Fix with Claude Code"), "{html}");
    assert!(
        html.contains("data-tooltip=\"Needs a harness: install Claude Code or muse\""),
        "{html}"
    );
}

#[test]
fn inspector_paused_container_offers_resume_and_says_it_is_paused() {
    let mut paused = container("shop-api-api-1");
    paused.state = ContainerState::Paused;
    let html = render(Panel::of(&paused).with(&paused));
    assert!(html.contains(">Resume<"), "{html}");
    assert!(!html.contains(">Pause<"), "{html}");
    assert!(html.contains("Paused. Its processes are frozen"), "{html}");
    assert!(html.contains("Paused, up"), "{html}");
}

#[test]
fn inspector_pending_action_shows_its_label_and_disables_the_buttons() {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.pending = pending_of(&api, "container.stop", "Stopping…");
    let html = render(panel);
    assert!(html.contains("Stopping…"), "{html}");
    assert!(html.contains("aria-busy=\"true\""), "{html}");
    assert!(
        html.contains("data-tooltip=\"Stopping… is still in progress\""),
        "{html}"
    );
}

#[test]
fn inspector_a_failed_action_shows_the_engines_message_and_a_dismiss() {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.pending = BTreeMap::from([(
        PendingKey {
            subject: berth_core::Subject::Container(api.id.clone()),
            action: "container.stop",
        },
        PendingState::Failed {
            message: "permission denied".to_string(),
        },
    )]);
    let html = render(panel);
    assert!(html.contains("permission denied"), "{html}");
    assert!(html.contains("Dismiss"), "{html}");
}

#[test]
fn inspector_removed_while_open_keeps_the_name_and_says_it_was_removed() {
    let worker = container("shop-api-worker-1");
    let mut dom = VirtualDom::new_with_props(
        PanelRoot,
        PanelRootProps {
            panel: Panel::of(&worker),
        },
    );
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("the panel captured the state");
    let before = strip_styles(&dioxus_ssr::render(&dom));
    assert!(before.contains("Stop the restart loop"), "{before}");

    dom.in_runtime(|| {
        let mut state = state;
        state.containers.write().retain(|c| c.id != worker.id);
    });
    dom.render_immediate(&mut dioxus::core::NoOpMutations);
    let after = strip_styles(&dioxus_ssr::render(&dom));
    assert!(after.contains("shop-api-worker-1"), "{after}");
    assert!(after.contains("This container was removed."), "{after}");
    assert!(after.contains("Close"), "{after}");
    assert!(!after.contains("Stop the restart loop"), "{after}");
}

#[test]
fn inspector_loading_and_failed_details_say_so_instead_of_an_empty_body() {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.entry = None;
    let html = render(panel);
    assert!(html.contains("Loading details…"), "{html}");
    let mut failed = Panel::of(&api);
    failed.entry = Some(DetailEntry::Failed {
        message: "engine said no".to_string(),
        gone: false,
    });
    let html = render(failed);
    assert!(
        html.contains("Could not read the details: engine said no"),
        "{html}"
    );
    let mut gone = Panel::of(&api);
    gone.entry = Some(DetailEntry::Failed {
        message: "x".to_string(),
        gone: true,
    });
    assert!(render(gone).contains("no longer exists"));
    // The header stays usable while the body loads.
    assert!(html.contains(">Stop<"), "{html}");
}

// ---------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------

#[test]
fn inspector_tab_strip_shows_five_tabs_and_keeps_a_selected_overflow_tab_visible() {
    let (items, overflow) = tab_strip(DetailTab::Overview, 360.0, 1.0, &[]);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Overview", "Logs", "Terminal", "Stats", "Inspect"]);
    let menu: Vec<&str> = overflow.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(menu, ["Env", "Mounts", "Files", "Limits"]);

    let (items, overflow) = tab_strip(DetailTab::Env, 360.0, 1.0, &[]);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Overview", "Logs", "Terminal", "Stats", "Env"]);
    assert!(overflow.iter().any(|e| e.label == "Inspect"));
    assert!(!overflow.iter().any(|e| e.label == "Env"));
    assert_eq!(
        visible_tabs(DetailTab::Limits, 360.0, 1.0, &[])[4],
        DetailTab::Limits
    );
}

#[test]
fn inspector_tab_strip_shows_fewer_tabs_as_the_text_grows() {
    use crate::text_scale::STEPS;
    for (scale, want) in [(0.85, 5), (1.0, 5), (1.1, 4), (1.2, 4), (1.3, 4), (1.5, 3)] {
        assert_eq!(tab_capacity(360.0, scale), want, "{scale}");
    }
    // The strip never grows with the text, and every tab stays reachable.
    let mut last = usize::MAX;
    for scale in STEPS {
        let capacity = tab_capacity(360.0, scale);
        assert!(capacity <= last, "{scale}");
        last = capacity;
        let (items, overflow) = tab_strip(DetailTab::Overview, 360.0, scale, &[]);
        assert_eq!(items.len() + overflow.len(), DetailTab::ALL.len());
    }
    // A selected tab under ⋯ takes the last place, however few fit.
    let (items, overflow) = tab_strip(DetailTab::Env, 360.0, 1.5, &[]);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Overview", "Logs", "Env"]);
    assert!(overflow.iter().any(|e| e.label == "Terminal"));
}

#[test]
fn inspector_tab_overflow_follows_the_pane_width_not_a_fixed_360() {
    // A wider pane shows more tabs, a narrower one fewer, at every text size.
    assert_eq!(tab_capacity(320.0, 1.0), 4);
    assert_eq!(tab_capacity(360.0, 1.0), 5);
    assert_eq!(tab_capacity(488.0, 1.0), 7);
    assert_eq!(tab_capacity(560.0, 1.0), 8);
    // The full-width bottom dock has room for all nine: no ⋯ at all.
    assert_eq!(tab_capacity(1220.0, 1.0), DetailTab::ALL.len());
    let (items, overflow) = tab_strip(DetailTab::Overview, 1220.0, 1.0, &[]);
    assert_eq!(items.len(), 9);
    assert!(overflow.is_empty());
    // Every width and text size keeps every tab reachable, and never shows
    // more tabs than fit.
    for width in [320.0, 360.0, 420.0, 488.0, 560.0, 804.0] {
        let mut last = usize::MAX;
        for scale in crate::text_scale::STEPS {
            let (items, overflow) = tab_strip(DetailTab::Overview, width, scale, &[]);
            assert_eq!(items.len() + overflow.len(), DetailTab::ALL.len());
            assert!(items.len() <= last, "{width} px at {scale}");
            last = items.len();
        }
    }
    // The rule is the label (grows with the text) plus fixed padding: the
    // strip a 322 px pane (1024 split) gets at 100 % is four tabs.
    assert_eq!(tab_capacity(322.0, 1.0), 4);
}

#[test]
fn inspector_split_mode_hides_the_logs_and_terminal_tabs() {
    let hidden = &DetailTab::STREAMS;
    let (items, overflow) = tab_strip(DetailTab::Overview, 488.0, 1.0, hidden);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        !labels.contains(&"Logs") && !labels.contains(&"Terminal"),
        "{labels:?}"
    );
    assert_eq!(labels[..3], ["Overview", "Stats", "Inspect"]);
    // Hidden means gone from ⋯ too: the panel beside shows them.
    assert!(!overflow
        .iter()
        .any(|e| e.label == "Logs" || e.label == "Terminal"));
    assert_eq!(items.len() + overflow.len(), 7);
    // Outside split mode both tabs are there.
    let (items, _) = tab_strip(DetailTab::Overview, 488.0, 1.0, &[]);
    assert!(items.iter().any(|i| i.label == "Logs"));
    assert!(items.iter().any(|i| i.label == "Terminal"));
    // A selected Logs tab is not forced back into a strip that hides it.
    let tabs = visible_tabs(DetailTab::Logs, 360.0, 1.0, hidden);
    assert!(!tabs.contains(&DetailTab::Logs));
    // The panel marks hidden tabs only with the flag set.
    let api = container("shop-api-api-1");
    let mut split = Panel::of(&api);
    split.hide_stream_tabs = true;
    split.pane_px = 488;
    let html = render(split);
    assert!(!html.contains(">Terminal<"), "{html}");
    assert!(html.contains(">Stats<"), "{html}");
    let mut alone = Panel::of(&api);
    alone.pane_px = 488;
    assert!(render(alone).contains(">Terminal<"));
}

#[test]
fn inspector_keeps_one_12px_left_edge_and_sets_delete_apart() {
    use super::super::css_tracks as css;
    let rule = |header: &str| css::block(CONTAINER_DETAIL_CSS, header);
    // Header, tabs, key/value list and the failure card all inset 12 px.
    assert!(rule("\n.ih {").contains("padding: var(--space-16) var(--space-12) var(--space-12);"));
    assert!(rule("\n.cd-tabs {").contains("padding: 0;"));
    assert!(rule("\n.cd-tabs .tab {").contains("padding: 0 var(--space-12);"));
    assert!(rule("\n.kv-list {").contains("padding: 0 var(--space-12);"));
    assert!(rule("\n.cd-failure {").contains("padding: var(--space-4) var(--space-12) 0;"));
    assert!(rule("\n.cd .pi {").contains("margin: var(--space-16) var(--space-12) 0;"));
    // Delete takes a line of its own below the four other buttons.
    assert!(rule("\n.ih-btn-danger {").contains("flex: 0 0 100%;"));
    assert!(rule("\n.ih-buttons {").contains("flex-wrap: wrap;"));
    // At 150 % the four buttons only fit one line if their padding gives a
    // little (6 px at 100 %, 4 px at 150 %).
    assert!(CONTAINER_DETAIL_CSS.contains(
        ".ih-btn .btn { padding: 0 calc(var(--space-6) - (var(--text-scale, 1) - 1) * var(--space-4));"
    ));
}

#[test]
fn inspector_limits_tab_mounts_the_limits_panel() {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.tab = DetailTab::Limits;
    let html = render(panel);
    assert!(!html.contains("Built in"), "{html}");
    assert!(html.contains("Apply limits"), "{html}");
}

#[test]
fn inspector_logs_tab_mounts_the_log_panel() {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.tab = DetailTab::Logs;
    let html = render(panel);
    assert!(!html.contains("Built in P3-T2"), "{html}");
    assert!(html.contains("log-panel"), "{html}");
}

#[test]
fn inspector_terminal_tab_mounts_the_terminal_launcher() {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.tab = DetailTab::Terminal;
    let html = render(panel);
    assert!(!html.contains("Built in P3-T6"), "{html}");
    assert!(html.contains("term-launch"), "{html}");
}

#[test]
fn inspector_files_tab_mounts_the_files_panel() {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.tab = DetailTab::Files;
    let html = render(panel);
    assert!(!html.contains("Built in P3-T8"), "{html}");
    assert!(html.contains("class=\"fb\""), "{html}");
}

#[test]
fn inspector_stats_tab_mounts_the_stats_panel() {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.tab = DetailTab::Stats;
    let html = render(panel);
    assert!(html.contains("class=\"stats-panel\""), "{html}");
    assert!(!html.contains("Built in P3-T4"), "{html}");
}

#[test]
fn inspector_header_menu_works_for_the_edit_items_and_defers_save_as_image() {
    let mut loose = container("shop-api-worker-1");
    loose.labels.clear();
    let menu = header_menu(&loose, Backing::Live);
    let by_id = |id: &str| menu.iter().find(|e| e.id == id).expect(id).clone();
    for id in ["edit", "duplicate", "recreate", "rename", "copy-id"] {
        assert!(by_id(id).disabled_reason.is_none(), "{id} works");
    }
    assert_eq!(by_id("edit").kbd_letter.as_deref(), Some("E"));
    let reason = by_id("save-image").disabled_reason.expect("disabled");
    assert!(reason.starts_with("Built in P"), "{reason}");
}

#[test]
fn inspector_header_menu_disables_what_a_mock_engine_cannot_create_and_says_why() {
    let mut loose = container("shop-api-worker-1");
    loose.labels.clear();
    let reason = "Creating containers need a real Docker engine. You're viewing mock data.";
    let menu = header_menu(&loose, Backing::Mock);
    for id in ["edit", "duplicate", "recreate"] {
        let entry = menu.iter().find(|e| e.id == id).expect(id);
        assert_eq!(entry.disabled_reason.as_deref(), Some(reason), "{id}");
    }
    // Rename is a plain engine call the mock answers.
    let rename = menu.iter().find(|e| e.id == "rename").expect("rename");
    assert!(rename.disabled_reason.is_none());
}

#[test]
fn inspector_header_menu_never_offers_recreate_for_a_compose_container() {
    let worker = container("shop-api-worker-1");
    assert!(worker.labels.contains_key("com.docker.compose.project"));
    let menu = header_menu(&worker, Backing::Live);
    let recreate = menu.iter().find(|e| e.id == "recreate").expect("recreate");
    assert_eq!(
        recreate.disabled_reason.as_deref(),
        Some("Managed by Compose")
    );
    // Edit still opens the sheet, which offers Apply limits.
    assert!(menu
        .iter()
        .find(|e| e.id == "edit")
        .expect("edit")
        .disabled_reason
        .is_none());
}

#[test]
fn inspector_mounts_tab_lists_each_mount_relative_to_the_compose_folder() {
    let worker = container("shop-api-worker-1");
    let mut panel = Panel::of(&worker);
    panel.tab = DetailTab::Mounts;
    let html = render(panel);
    assert!(html.contains("./worker → /app (bind)"), "{html}");
    let mut none = worker.clone();
    none.mounts.clear();
    let mut panel = Panel::of(&none).with(&none);
    panel.tab = DetailTab::Mounts;
    assert!(render(panel).contains("No mounts."));
}

// ---------------------------------------------------------------------
// Env: nothing until revealed
// ---------------------------------------------------------------------

#[component]
fn EnvHost(rows: Vec<model::EnvRow>) -> Element {
    rsx! { EnvList { rows, onreveal: |_| {}, onhide: |_| {} } }
}

#[component]
fn TreeHost(value: Arc<serde_json::Value>, open: std::collections::HashSet<String>) -> Element {
    rsx! { JsonTree { value, open, ontoggle: |_| {} } }
}

fn env_panel(tab: DetailTab) -> Panel {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.entry = Some(DetailEntry::Ready(Box::new(loaded(
        &api,
        &[("DB_PASSWORD", SECRET_A), ("API_TOKEN", SECRET_B)],
        &[],
    ))));
    panel.tab = tab;
    panel
}

#[test]
fn inspector_env_tab_lists_keys_masked_and_no_value_before_a_reveal() {
    let html = render(env_panel(DetailTab::Env));
    assert!(html.contains("DB_PASSWORD"), "{html}");
    assert!(html.contains("API_TOKEN"), "{html}");
    assert_eq!(count(&html, "••••"), 2, "{html}");
    assert!(!html.contains(SECRET_A), "value leaked: {html}");
    assert!(!html.contains(SECRET_B), "value leaked: {html}");
    assert!(html.contains("Show the value of DB_PASSWORD"), "{html}");
    assert!(html.contains("30 seconds"), "{html}");
}

#[test]
fn inspector_no_other_tab_shows_an_env_value() {
    for tab in [
        DetailTab::Overview,
        DetailTab::Inspect,
        DetailTab::Mounts,
        DetailTab::Env,
    ] {
        let html = render(env_panel(tab));
        assert!(!html.contains(SECRET_A), "{tab:?} leaked a value: {html}");
        assert!(!html.contains(SECRET_B), "{tab:?} leaked a value: {html}");
    }
}

#[test]
fn inspector_env_list_after_a_reveal_shows_only_that_one_value() {
    let env = vec![
        EnvVar {
            key: "DB_PASSWORD".to_string(),
            value: SECRET_A.to_string(),
        },
        EnvVar {
            key: "API_TOKEN".to_string(),
            value: SECRET_B.to_string(),
        },
    ];
    let now = Instant::now();
    let mut reveals = Reveals::default();
    reveals.reveal("DB_PASSWORD", now);
    let rows = env_rows(&env, &reveals, now);
    let mut dom = VirtualDom::new_with_props(EnvHost, EnvHostProps { rows });
    let _ = dom.rebuild_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains(SECRET_A), "{html}");
    assert!(!html.contains(SECRET_B), "{html}");
    assert!(html.contains("Hide the value of DB_PASSWORD"), "{html}");
    assert!(html.contains("Show the value of API_TOKEN"), "{html}");
    assert_eq!(count(&html, "••••"), 1, "{html}");
}

#[test]
fn inspector_a_revealed_value_masks_again_after_thirty_seconds() {
    let env = vec![EnvVar {
        key: "DB_PASSWORD".to_string(),
        value: SECRET_A.to_string(),
    }];
    let t0 = Instant::now();
    let mut reveals = Reveals::default();
    reveals.reveal("DB_PASSWORD", t0);
    let shown = |now| env_rows(&env, &reveals, now)[0].shown.clone();
    assert_eq!(
        shown(t0 + Duration::from_secs(29)),
        EnvShown::Revealed(SECRET_A.to_string())
    );
    assert_eq!(shown(t0 + REVEAL_FOR), EnvShown::Masked);
    assert_eq!(REVEAL_FOR, Duration::from_secs(30));
}

#[test]
fn inspector_reveals_are_per_key_and_expire_and_hide_independently() {
    let t0 = Instant::now();
    let mut reveals = Reveals::default();
    reveals.reveal("A", t0);
    reveals.reveal("B", t0 + Duration::from_secs(10));
    assert!(!reveals.is_revealed("C", t0));
    reveals.expire(t0 + Duration::from_secs(31));
    assert!(!reveals.is_revealed("A", t0 + Duration::from_secs(31)));
    assert!(reveals.is_revealed("B", t0 + Duration::from_secs(31)));
    reveals.hide("B");
    assert!(!reveals.is_revealed("B", t0 + Duration::from_secs(31)));
}

#[test]
fn inspector_an_env_row_that_is_masked_carries_no_value_at_all() {
    let env = vec![EnvVar {
        key: "K".to_string(),
        value: SECRET_A.to_string(),
    }];
    let rows = env_rows(&env, &Reveals::default(), Instant::now());
    assert_eq!(rows[0].shown, EnvShown::Masked);
    assert!(!format!("{rows:?}").contains(SECRET_A));
    // A revealed one does not print its value either.
    let mut reveals = Reveals::default();
    let now = Instant::now();
    reveals.reveal("K", now);
    let rows = env_rows(&env, &reveals, now);
    assert!(!format!("{rows:?}").contains(SECRET_A));
}

#[test]
fn inspector_env_without_variables_says_so() {
    let api = container("shop-api-api-1");
    let mut panel = Panel::of(&api);
    panel.tab = DetailTab::Env;
    let html = render(panel);
    assert!(html.contains("No environment variables."), "{html}");
}

// ---------------------------------------------------------------------
// Inspect: the masked tree
// ---------------------------------------------------------------------

/// The real shape of a Docker inspect (the fixture captured from
/// `berthfx-db-1`), with its real env: every value must be gone from the
/// tree while every key stays.
#[test]
fn inspector_inspect_tree_of_the_fixture_shows_no_env_value() {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("../../../../fixtures/docker/inspect-db.json"))
            .expect("fixture parses");
    let env: Vec<String> = raw["Config"]["Env"]
        .as_array()
        .expect("fixture has Config.Env")
        .iter()
        .filter_map(|e| e.as_str().map(ToString::to_string))
        .collect();
    assert!(env.iter().any(|e| e.starts_with("POSTGRES_PASSWORD=")));
    let values: Vec<&str> = env
        .iter()
        .filter_map(|e| e.split_once('=').map(|(_, v)| v))
        .filter(|v| v.len() > 3 && *v != "••••")
        .collect();
    assert!(!values.is_empty());

    let masked = mask_inspect(raw);
    let open: std::collections::HashSet<String> = ["", "/Config", "/Config/Env"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut dom = VirtualDom::new_with_props(
        TreeHost,
        TreeHostProps {
            value: Arc::new(masked),
            open,
        },
    );
    let _ = dom.rebuild_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("POSTGRES_PASSWORD=••••"), "{html}");
    for value in values {
        assert!(
            !html.contains(value),
            "an env value reached the tree: {value}"
        );
    }
}

#[test]
fn inspector_inspect_tab_starts_collapsed_and_has_a_search_field() {
    let html = render(env_panel(DetailTab::Inspect));
    assert!(html.contains("aria-label=\"Search the inspect\""), "{html}");
    assert!(html.contains("role=\"tree\""), "{html}");
    assert!(html.contains("Config"), "{html}");
    // Collapsed: the env array's entries are not drawn yet.
    assert!(!html.contains("DB_PASSWORD=••••"), "{html}");
}

// ---------------------------------------------------------------------
// The view model
// ---------------------------------------------------------------------

#[test]
fn inspector_state_note_reads_like_dockers_status() {
    let now = chrono::Utc::now();
    let mut c = container("shop-api-worker-1");
    c.started = Some(now - chrono::Duration::seconds(3));
    assert_eq!(
        state_note(&c, now).as_deref(),
        Some("Restarting (1) 3 seconds ago · restarted 14 times")
    );
    c.restart_count = 1;
    assert!(state_note(&c, now)
        .expect("a note")
        .ends_with("restarted 1 time"));
    c.restart_count = 0;
    c.state = ContainerState::Running;
    c.started = Some(now - chrono::Duration::hours(2));
    assert_eq!(state_note(&c, now).as_deref(), Some("Up 2 h"));
    c.state = ContainerState::Exited;
    c.started = None;
    assert_eq!(state_note(&c, now), None);
}

#[test]
fn inspector_short_id_is_twelve_characters() {
    assert_eq!(short_id("7b2e91c04f3a1d2c994f"), "7b2e91c04f3a");
    assert_eq!(short_id("abc"), "abc");
}

#[test]
fn inspector_restart_policy_and_mount_lines_read_like_docker() {
    use berth_core::{Mount, MountKind, RestartPolicy};
    assert_eq!(
        restart_label(&RestartPolicy::UnlessStopped),
        "unless-stopped"
    );
    assert_eq!(
        restart_label(&RestartPolicy::OnFailure { max_retries: 3 }),
        "on-failure:3"
    );
    assert_eq!(
        restart_label(&RestartPolicy::OnFailure { max_retries: 0 }),
        "on-failure"
    );
    let home = std::path::Path::new("/Users/someone");
    let bind = |source: &str, ro: bool| Mount {
        kind: MountKind::Bind,
        source: source.to_string(),
        destination: "/app".to_string(),
        read_only: ro,
        volume_name: None,
    };
    assert_eq!(
        mount_line(&bind("/srv/shop/worker", false), Some("/srv/shop"), home),
        "./worker → /app (bind)"
    );
    assert_eq!(
        mount_line(&bind("/Users/someone/code", true), None, home),
        "~/code → /app (bind, read-only)"
    );
    let volume = Mount {
        kind: MountKind::Volume,
        source: "/var/lib/docker/volumes/pg/_data".to_string(),
        destination: "/data".to_string(),
        read_only: false,
        volume_name: Some("pg".to_string()),
    };
    assert_eq!(mount_line(&volume, None, home), "pg → /data (volume)");
}

#[test]
fn inspector_header_buttons_follow_the_state_and_carry_the_reasons() {
    let world = world();
    let pending = BTreeMap::new();
    let view = StateView {
        engines: &world.engines,
        containers: &world.containers,
        projects: &world.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending: &pending,
        backing: crate::services::backing::Backing::default(),
    };
    let running = header_buttons(&container("shop-api-api-1"), &view);
    let ids: Vec<&str> = running.iter().map(|b| b.id).collect();
    assert_eq!(
        ids,
        ["start-stop", "restart", "pause-resume", "kill", "delete"]
    );
    assert_eq!(running[0].label, "Stop");
    assert!(running.iter().all(|b| b.disabled_reason.is_none()));

    let stopped = header_buttons(&container("shop-api-db-1"), &view);
    assert_eq!(stopped[0].label, "Start");
    assert_eq!(
        stopped[2].disabled_reason.as_deref(),
        Some("Only a running container can be paused")
    );
    assert_eq!(stopped[3].disabled_reason.as_deref(), Some("Not running"));
}

// ---------------------------------------------------------------------
// Clicks reach the actions service and the confirm dialogs
// ---------------------------------------------------------------------

struct Rig {
    dom: VirtualDom,
    state: AppState,
    service: ActionsService,
}

thread_local! {
    static RIG: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn RigHarness() -> Element {
    let state = use_app_state();
    use_hook(move || RIG.with(|slot| *slot.borrow_mut() = Some(state)));
    rsx! { div {} }
}

fn rig() -> Rig {
    let mut dom = VirtualDom::new(RigHarness);
    let _ = dom.rebuild_to_vec();
    let state = RIG
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the state");
    let world = world();
    let default = berth_core::EngineId("colima:default".to_string());
    let clients = crate::services::engines::client_map();
    clients.lock().expect("clients").insert(
        default.clone(),
        Arc::new(berth_engine::MockEngine::bundled(&default)) as Arc<dyn EngineClient>,
    );
    let (service, _finished) = ActionsService::new(
        clients,
        Arc::new(berth_store::Store::open_in_memory().expect("store")),
        Arc::new(berth_engine::MockCompose::bundled()),
        Arc::new(berth_engine::MockColima::bundled()),
        tokio_util::sync::CancellationToken::new(),
    );
    dom.in_runtime(|| {
        let mut state = state;
        state.engines.set(world.engines.clone());
        state.containers.set(world.containers_for(&default));
    });
    Rig {
        dom,
        state,
        service,
    }
}

impl Rig {
    fn container(&self, name: &str) -> Container {
        self.dom.in_runtime(|| {
            self.state
                .containers
                .peek()
                .iter()
                .find(|c| c.name == name)
                .cloned()
                .expect(name)
        })
    }

    fn pending_actions(&self) -> Vec<&'static str> {
        self.dom
            .in_runtime(|| self.state.pending.peek().keys().map(|k| k.action).collect())
    }

    fn overlays(&self) -> Vec<Overlay> {
        self.dom.in_runtime(|| self.state.overlays.peek().clone())
    }
}

#[tokio::test]
async fn inspector_stop_button_on_a_running_container_stops_it() {
    let rig = rig();
    let api = rig.container("shop-api-api-1");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        header_click(&mut state, Some(&rig.service), &api, "start-stop");
    });
    assert_eq!(rig.pending_actions(), ["container.stop"]);
}

#[tokio::test]
async fn inspector_start_button_on_an_exited_container_starts_it() {
    let rig = rig();
    let grafana = rig.container("analytics-grafana-1");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        header_click(&mut state, Some(&rig.service), &grafana, "start-stop");
    });
    assert_eq!(rig.pending_actions(), ["container.start"]);
}

#[tokio::test]
async fn inspector_stop_the_restart_loop_stops_that_container() {
    let rig = rig();
    let worker = rig.container("shop-api-worker-1");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        fix_click(
            &mut state,
            Some(&rig.service),
            None,
            None,
            &format!("stop-loop:{}", worker.id.0),
        );
    });
    assert_eq!(rig.pending_actions(), ["container.stop"]);
}

#[tokio::test]
async fn inspector_a_problems_run_action_fix_shows_its_effect_then_runs_through_the_service() {
    let rig = rig();
    let worker = rig.container("shop-api-worker-1");
    let problem = problem_for(&worker, vec![stop_fix(&worker)]);
    let fix_id = format!("{}#0", problem.id.0);
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state.problems.set(vec![problem]);
        fix_click(&mut state, Some(&rig.service), None, None, &fix_id);
    });
    // The click asks first, with the fix's own effect; nothing has run.
    assert!(rig.pending_actions().is_empty());
    let model = match rig.overlays().as_slice() {
        [Overlay::Confirm(model)] => model.clone(),
        other => panic!("one confirm dialog: {other:?}"),
    };
    assert_eq!(model.title, "Stop the restart loop?");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        confirm::run(&mut state, &rig.service, &model, false);
    });
    assert_eq!(rig.pending_actions(), ["container.stop"]);
}

#[tokio::test]
async fn inspector_go_to_selects_the_dependency() {
    let rig = rig();
    let db = rig.container("shop-api-db-1");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        fix_click(
            &mut state,
            Some(&rig.service),
            None,
            None,
            &format!("goto:{}", db.id.0),
        );
    });
    let selection = rig.dom.in_runtime(|| rig.state.selection.peek().clone());
    assert_eq!(selection, Selection::Containers(vec![db.id]));
}

#[tokio::test]
async fn inspector_kill_and_delete_ask_first_with_their_own_targets() {
    let rig = rig();
    let api = rig.container("shop-api-api-1");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        header_click(&mut state, Some(&rig.service), &api, "kill");
    });
    match rig.overlays().as_slice() {
        [Overlay::Confirm(model)] => {
            assert_eq!(model.title, "Kill shop-api-api-1?");
            assert_eq!(
                model.targets.len(),
                usize::try_from(model.effect.count).unwrap_or(0)
            );
        }
        other => panic!("one kill dialog, got {other:?}"),
    }
    assert!(
        rig.pending_actions().is_empty(),
        "nothing ran without a click"
    );

    let db = rig.container("shop-api-db-1");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state.overlays.write().clear();
        header_click(&mut state, Some(&rig.service), &db, "delete");
    });
    match rig.overlays().as_slice() {
        [Overlay::Confirm(model)] => assert_eq!(model.title, "Delete container shop-api-db-1?"),
        other => panic!("one delete dialog, got {other:?}"),
    }
    assert!(rig.pending_actions().is_empty());
}

#[tokio::test]
async fn inspector_pause_then_resume_pick_the_right_action() {
    let rig = rig();
    let api = rig.container("shop-api-api-1");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        header_click(&mut state, Some(&rig.service), &api, "pause-resume");
    });
    assert_eq!(rig.pending_actions(), ["container.pause"]);

    let rig = self::rig();
    let mut paused = rig.container("shop-api-api-1");
    paused.state = ContainerState::Paused;
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state.containers.write().iter_mut().for_each(|c| {
            if c.id == paused.id {
                c.state = ContainerState::Paused;
            }
        });
        header_click(&mut state, Some(&rig.service), &paused, "pause-resume");
    });
    assert_eq!(rig.pending_actions(), ["container.resume"]);
}
