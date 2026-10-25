//! AD31: rows keep their place and their y offset while the screen is open.

use dioxus::prelude::*;

use super::FrozenOrder;
use crate::screens::projects::{
    list_projects, ordered_projects, ProjectFilter, ProjectsScreen, PROJECTS_CSS,
};
use crate::state::{use_app_state, AppState};

// --- The pure order ---

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn names(projects: &[&berth_core::Project]) -> Vec<String> {
    projects.iter().map(|p| p.name.clone()).collect()
}

fn set_status(projects: &mut [berth_core::Project], name: &str, status: berth_core::ProjectStatus) {
    projects
        .iter_mut()
        .find(|p| p.name == name)
        .expect("project")
        .status = status;
}

fn fresh_project(template: &berth_core::Project, name: &str) -> berth_core::Project {
    let mut fresh = template.clone();
    fresh.name = name.to_string();
    fresh.id = berth_core::ProjectId(format!("colima:default/{name}"));
    fresh.status = berth_core::ProjectStatus::Failing;
    fresh
}

#[test]
fn order_first_sort_puts_failing_first_and_a_status_change_moves_nothing() {
    let w = world();
    let mut frozen = FrozenOrder::default();
    let mut projects = w.projects.clone();
    let before = names(&frozen.arrange(&projects, &w.engines, true));
    assert_eq!(before.first().map(String::as_str), Some("shop-api"));
    // A stopped project fails, the failing one recovers.
    set_status(
        &mut projects,
        "ml-notebooks",
        berth_core::ProjectStatus::Failing,
    );
    set_status(
        &mut projects,
        "shop-api",
        berth_core::ProjectStatus::AllRunning,
    );
    assert_eq!(names(&frozen.arrange(&projects, &w.engines, true)), before);
    // The live sort would have moved them.
    assert_ne!(names(&ordered_projects(&projects, &w.engines)), before);
}

#[test]
fn order_a_new_project_goes_last_and_a_removed_one_leaves_a_gap_that_closes() {
    let w = world();
    let mut frozen = FrozenOrder::default();
    let mut projects = w.projects.clone();
    let before = names(&frozen.arrange(&projects, &w.engines, true));
    // A new failing project would sort to the top if the order were live.
    projects.push(fresh_project(&projects[0], "brand-new"));
    let grown = names(&frozen.arrange(&projects, &w.engines, true));
    assert_eq!(grown[..before.len()], before[..]);
    assert_eq!(grown.last().map(String::as_str), Some("brand-new"));
    // Removing one project closes only its own gap.
    projects.retain(|p| p.name != "berth-site");
    let shrunk = names(&frozen.arrange(&projects, &w.engines, true));
    let mut expected = grown.clone();
    expected.retain(|n| n != "berth-site");
    assert_eq!(shrunk, expected);
}

#[test]
fn order_an_engine_slice_rewritten_at_the_end_moves_nothing() {
    let w = world();
    let mut frozen = FrozenOrder::default();
    let mut projects = w.projects.clone();
    let before = names(&frozen.arrange(&projects, &w.engines, true));
    // What replace_engine_slice does: one engine's rows go to the end, and
    // their statuses are whatever the engine says now.
    projects.rotate_left(3);
    set_status(
        &mut projects,
        "docs-site",
        berth_core::ProjectStatus::Failing,
    );
    assert_eq!(names(&frozen.arrange(&projects, &w.engines, true)), before);
}

#[test]
fn order_waits_for_live_data_then_sorts_once() {
    let w = world();
    let mut frozen = FrozenOrder::default();
    let mut projects = w.projects.clone();
    // The cached snapshot is not settled: the live sort answers, nothing is kept.
    let cached = names(&frozen.arrange(&projects, &w.engines, false));
    set_status(
        &mut projects,
        "ml-notebooks",
        berth_core::ProjectStatus::Failing,
    );
    let live = names(&frozen.arrange(&projects, &w.engines, false));
    assert_ne!(cached, live);
    // Live data arrives: this sort is the one that stays.
    let settled = names(&frozen.arrange(&projects, &w.engines, true));
    assert_eq!(settled, live);
    set_status(
        &mut projects,
        "ml-notebooks",
        berth_core::ProjectStatus::Stopped,
    );
    assert_eq!(names(&frozen.arrange(&projects, &w.engines, true)), settled);
}

#[test]
fn order_filters_hide_rows_of_the_frozen_order_without_sorting_them() {
    let w = world();
    let mut frozen = FrozenOrder::default();
    let mut projects = w.projects.clone();
    let before = names(&frozen.arrange(&projects, &w.engines, true));
    set_status(
        &mut projects,
        "docs-site",
        berth_core::ProjectStatus::Failing,
    );
    set_status(
        &mut projects,
        "berth-site",
        berth_core::ProjectStatus::Failing,
    );
    let ordered = frozen.arrange(&projects, &w.engines, true);
    let failing = list_projects(&ordered, &w.engines, &[], "", ProjectFilter::Failing).visible;
    let expected: Vec<String> = before
        .iter()
        .filter(|n| matches!(n.as_str(), "shop-api" | "berth-site" | "docs-site"))
        .cloned()
        .collect();
    assert_eq!(names(&failing), expected);
    let by_text = list_projects(&ordered, &w.engines, &[], "site", ProjectFilter::All).visible;
    let expected: Vec<String> = before
        .iter()
        .filter(|n| n.contains("site"))
        .cloned()
        .collect();
    assert_eq!(names(&by_text), expected);
}

#[test]
fn order_a_slot_is_decided_once_per_project() {
    let mut frozen = FrozenOrder::default();
    let with = berth_core::ProjectId("a".to_string());
    let without = berth_core::ProjectId("b".to_string());
    assert!(frozen.slot(&with, true));
    assert!(!frozen.slot(&without, false));
    // The problem on b appears, the problem on a clears.
    assert!(!frozen.slot(&without, true));
    assert!(frozen.slot(&with, false));
}

// --- The screen: y offsets of the drawn rows ---

/// Heights in the row model: a row is `--size-row-lg` (36 px at the
/// default density and text size), its problem slot `--size-row` (32 px).
const ROW_PX: u32 = 36;
const SLOT_PX: u32 = 32;

thread_local! {
    static LIVE: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

fn error_on(container: &str, title: &str) -> berth_core::Problem {
    let id = world()
        .containers
        .iter()
        .find(|c| c.name == container)
        .expect("container")
        .id
        .clone();
    let kind = berth_core::ProblemKind::PortAllocated { port: 5432 };
    let subject = berth_core::Subject::Container(id);
    berth_core::Problem {
        id: berth_core::ProblemId::new(&kind, &subject),
        kind,
        severity: berth_core::Severity::Error,
        subject,
        title: title.to_string(),
        explanation: String::new(),
        evidence: Vec::new(),
        fixes: Vec::new(),
        first_seen: chrono::Utc::now(),
    }
}

#[component]
fn LiveHarness() -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    use_hook(move || {
        let w = world();
        state.engines.set(w.engines.clone());
        state.containers.set(w.containers.clone());
        state.projects.set(w.projects.clone());
        state.problems.set(vec![error_on(
            "shop-api-db-1",
            "Port 5432 is already in use",
        )]);
        LIVE.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { ProjectsScreen {} }
}

struct Live {
    dom: VirtualDom,
    state: AppState,
}

impl Live {
    fn open() -> Self {
        let mut dom = VirtualDom::new(LiveHarness);
        let _ = dom.rebuild_to_vec();
        let state = LIVE
            .with(|slot| slot.borrow_mut().take())
            .expect("the harness stored the state");
        Self { dom, state }
    }

    fn edit(&mut self, change: impl FnOnce(&mut AppState)) {
        let mut state = self.state;
        self.dom.in_runtime(|| change(&mut state));
        self.dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }

    fn html(&self) -> String {
        dioxus_ssr::render(&self.dom).replace(PROJECTS_CSS, "")
    }

    /// Each drawn row: name, y offset, and whether its problem line shows.
    fn rows(&self) -> Vec<(String, u32, bool)> {
        rows_of(&self.html())
    }
}

fn rows_of(html: &str) -> Vec<(String, u32, bool)> {
    let mut y = 0;
    let mut out = Vec::new();
    for chunk in html.split("class=\"project-row-wrap\"").skip(1) {
        let name = chunk
            .split("project-name-text\">")
            .nth(1)
            .and_then(|rest| rest.split('<').next())
            .expect("row name")
            .to_string();
        out.push((name, y, chunk.contains("project-problem-line")));
        y += ROW_PX
            + if chunk.contains("project-problem-slot") {
                SLOT_PX
            } else {
                0
            };
    }
    out
}

fn places(rows: &[(String, u32, bool)]) -> Vec<(String, u32)> {
    rows.iter().map(|(n, y, _)| (n.clone(), *y)).collect()
}

#[test]
fn screen_opens_failing_first_with_the_problem_line_under_it() {
    let live = Live::open();
    let rows = live.rows();
    assert_eq!(rows.len(), 8, "{rows:?}");
    assert_eq!(rows[0].0, "shop-api");
    assert!(rows[0].2, "the failing row shows its problem line");
    assert_eq!(rows[1], ("analytics".to_string(), ROW_PX + SLOT_PX, false));
}

#[test]
fn screen_rows_keep_index_and_y_through_status_changes_new_problems_and_slice_refreshes() {
    let mut live = Live::open();
    let before = places(&live.rows());

    // A status change: a stopped project fails, the failing one recovers.
    live.edit(|state| {
        let mut projects = state.projects.peek().clone();
        set_status(
            &mut projects,
            "ml-notebooks",
            berth_core::ProjectStatus::Failing,
        );
        set_status(
            &mut projects,
            "shop-api",
            berth_core::ProjectStatus::AllRunning,
        );
        state.projects.set(projects);
    });
    assert_eq!(places(&live.rows()), before, "status change");

    // A problem appears on a row that had none: the tag shows, nothing moves.
    live.edit(|state| {
        let mut problems = state.problems.peek().clone();
        problems.push(error_on("analytics-ingest-1", "ingest is down"));
        state.problems.set(problems);
    });
    let rows = live.rows();
    assert_eq!(places(&rows), before, "new problem");
    let analytics = rows.iter().find(|r| r.0 == "analytics").expect("row");
    assert!(!analytics.2, "no slot was reserved, so no inline line");
    assert!(live.html().contains("1 problem</span>"), "the tag shows it");
    assert!(
        live.html().contains("ingest is down"),
        "Needs attention lists it"
    );

    // The first problem clears: its slot stays, so the rows below hold.
    live.edit(|state| {
        state
            .problems
            .set(vec![error_on("analytics-ingest-1", "ingest is down")]);
    });
    let rows = live.rows();
    assert_eq!(places(&rows), before, "problem cleared");
    assert!(!rows[0].2, "the line is gone, the slot is not");

    // An engine slice refresh: its rows move to the end of the list.
    live.edit(|state| {
        let mut projects = state.projects.peek().clone();
        projects.rotate_left(3);
        state.projects.set(projects);
    });
    assert_eq!(places(&live.rows()), before, "slice refresh");
}

#[test]
fn screen_a_new_project_is_appended_and_a_removed_one_closes_its_gap() {
    let mut live = Live::open();
    let before = places(&live.rows());
    live.edit(|state| {
        let mut projects = state.projects.peek().clone();
        projects.push(fresh_project(&projects[0].clone(), "brand-new"));
        state.projects.set(projects);
    });
    let grown = places(&live.rows());
    assert_eq!(grown[..before.len()], before[..]);
    assert_eq!(grown.last().map(|r| r.0.as_str()), Some("brand-new"));

    live.edit(|state| {
        let mut projects = state.projects.peek().clone();
        projects.retain(|p| p.name != "analytics");
        state.projects.set(projects);
    });
    let shrunk = live.rows();
    assert!(shrunk.iter().all(|r| r.0 != "analytics"));
    // The row above the gap holds; the rows below move up by one row.
    assert_eq!(shrunk[0].1, 0);
    assert_eq!(shrunk[1].0, "berth-site");
    assert_eq!(shrunk[1].1, ROW_PX + SLOT_PX);
}

thread_local! {
    static REMOUNT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn RemountHarness() -> Element {
    let state = REMOUNT.with(|slot| slot.borrow().expect("state"));
    use_context_provider(|| state);
    rsx! { ProjectsScreen {} }
}

#[test]
fn screen_remounting_sorts_again_with_failing_first() {
    let mut live = Live::open();
    live.edit(|state| {
        let mut projects = state.projects.peek().clone();
        set_status(
            &mut projects,
            "ml-notebooks",
            berth_core::ProjectStatus::Failing,
        );
        set_status(
            &mut projects,
            "shop-api",
            berth_core::ProjectStatus::AllRunning,
        );
        state.projects.set(projects);
    });
    assert_eq!(
        live.rows()[0].0,
        "shop-api",
        "an open screen keeps its order"
    );
    // Leaving the screen drops it; coming back mounts a new one over the same state.
    REMOUNT.with(|slot| *slot.borrow_mut() = Some(live.state));
    let mut again = VirtualDom::new(RemountHarness);
    let _ = again.rebuild_to_vec();
    let html = dioxus_ssr::render(&again).replace(PROJECTS_CSS, "");
    let rows = rows_of(&html);
    assert_eq!(rows[0].0, "ml-notebooks", "{rows:?}");
}

#[test]
fn screen_slot_height_is_the_line_height_in_css() {
    let slot = crate::screens::css_tracks::block(PROJECTS_CSS, ".project-problem-slot {");
    let line = crate::screens::css_tracks::block(PROJECTS_CSS, ".project-problem-line {");
    assert!(slot.contains("\n  height: var(--size-row);"), "{slot}");
    assert!(line.contains("\n  height: var(--size-row);"), "{line}");
    assert!(
        PROJECTS_CSS.contains("\n.project-row {\n  height: var(--size-row-lg);"),
        "the row is --size-row-lg tall"
    );
}
