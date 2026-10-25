use super::*;
use crate::state::use_app_state;

/// The screen carries its CSS in a `<style>` block, whose class names
/// would satisfy (or break) every `contains` below; look at markup only.
fn strip_style(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<style>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</style>") {
            Some(end) => rest = &rest[start + end + "</style>".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn scene(frame: &str) -> String {
    strip_style(&crate::scenes::render_scene_html(frame).expect("scene exists"))
}

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn project<'a>(w: &'a berth_engine::MockWorld, name: &str) -> &'a berth_core::Project {
    w.projects
        .iter()
        .find(|p| p.name == name)
        .expect("project in the mock world")
}

// --- The empty body: centred, or two columns with a first-run card ---

#[test]
fn projects_empty_body_is_centred_without_a_first_run_card() {
    let html = strip_style(&dioxus_ssr::render_element(rsx! { EmptyBody {} }));
    assert!(html.contains("class=\"projects-empty\""), "{html}");
    assert!(!html.contains("has-first-run"), "{html}");
    assert!(!html.contains("projects-first-run"), "{html}");
    assert!(html.contains("No projects yet"), "{html}");
    assert!(html.contains("Drop a folder here"), "{html}");
    // The real scene has no card today, so it is the centred case.
    let page = scene("Projects / empty / 1440");
    assert!(page.contains("class=\"projects-empty\""), "{page}");
    assert!(!page.contains("has-first-run"), "{page}");
}

#[test]
fn projects_empty_body_moves_left_when_a_first_run_card_is_given() {
    let html = strip_style(&dioxus_ssr::render_element(rsx! {
        EmptyBody { first_run: Some(rsx! { div { class: "placeholder-card", "First-run checks" } }) }
    }));
    assert!(
        html.contains("class=\"projects-empty has-first-run\""),
        "{html}"
    );
    // The block comes first (left), the card in its slot after it (right).
    let block = html.find("projects-empty-column").expect("block");
    let card = html.find("projects-first-run").expect("card slot");
    assert!(block < card, "{html}");
    assert!(html.contains("placeholder-card"), "{html}");
    assert!(html.contains("First-run checks"), "{html}");
    assert!(html.contains("No projects yet"), "{html}");
}

// --- The three scenes ---

fn assert_default_rows(html: &str) {
    // Header: title, count, segmented counts, disabled Open folder.
    assert!(html.contains(">Projects</h1>"), "{html}");
    assert!(html.contains("badge badge-count\">8<"), "{html}");
    for (label, count) in [("All", 8), ("Running", 3), ("Failing", 1), ("Stopped", 2)] {
        assert!(
            html.contains(&format!(
                "<span>{label}</span><span class=\"segment-count\">{count}</span>"
            )),
            "segment {label} {count}: {html}"
        );
    }
    assert!(html.contains("Filter projects"), "{html}");
    assert!(html.contains("Open folder"), "{html}");
    assert!(
        html.contains("data-tooltip=\"Folder browser not built yet\""),
        "{html}"
    );
    // One row per project, failing first, unreachable engine last.
    let names = [
        "shop-api",
        "analytics",
        "berth-site",
        "payments-mock",
        "ml-notebooks",
        "docs-site",
        "docker-manger",
        "landing",
    ];
    let mut at = 0;
    for name in names {
        let found = html[at..]
            .find(&format!("<span class=\"project-name-text\">{name}</span>"))
            .unwrap_or_else(|| panic!("row {name} in order after byte {at}"));
        at += found + 1;
    }
    assert_eq!(html.matches("class=\"project-row\"").count(), 6, "{html}");
    assert!(
        html.contains("project-row project-row-unreachable"),
        "{html}"
    );
    assert!(html.contains("project-row project-row-notsetup"), "{html}");
    // Labelled buttons, ports, totals, problem line.
    assert!(html.contains("Stop project"), "{html}");
    assert!(html.contains("Start project"), "{html}");
    assert!(html.contains(">Restart<"), "{html}");
    assert!(html.contains("~/Dev/…/shop-api"), "{html}");
    // The frame's Path column middle-truncates these as well; analytics
    // and docs-site are short enough to stay whole.
    for collapsed in [
        "~/Dev/…/berth-site",
        "~/Dev/…/payments-mock",
        "~/Dev/…/ml-notebooks",
        "~/Dev/…/docker-manger",
    ] {
        assert!(html.contains(collapsed), "{collapsed}: {html}");
    }
    assert!(html.contains(">~/Dev/analytics<"), "{html}");
    assert!(html.contains(">~/Dev/docs-site<"), "{html}");
    assert!(html.contains("feat/checkout-v2"), "{html}");
    assert!(
        html.contains(">:3000<") && html.contains(">:6379<"),
        "{html}"
    );
    assert!(html.contains(">+1<"), "{html}");
    assert!(html.contains("5 services"), "{html}");
    assert!(html.contains(" · db failing"), "{html}");
    assert!(html.contains(">3.7%<"), "{html}");
    assert!(html.contains("2 problems"), "{html}");
    assert!(
        html.contains("2 problems · Port 5432 is already in use"),
        "{html}"
    );
    assert!(html.contains("Engine unreachable"), "{html}");
    assert!(html.contains("Set up project…"), "{html}");
    assert!(html.contains("Shows a diff before writing"), "{html}");
    // Needs attention, errors first, newest first.
    assert!(html.contains("Needs attention · 3"), "{html}");
    let port = html.find("attention-title\">Port 5432").expect("port card");
    let worker = html
        .find("attention-title\">worker keeps restarting")
        .expect("worker card");
    let ingest = html
        .find("attention-title\">ingest health check is failing")
        .expect("ingest card");
    assert!(port < worker && worker < ingest, "{html}");
    assert!(html.contains("Open shop-api-db-1"), "{html}");
    assert!(html.contains(">2 min<"), "{html}");
}

#[test]
fn projects_scene_default_renders_rows_counts_and_needs_attention() {
    let html = scene("Projects / default / 1440");
    assert_default_rows(&html);
}

#[test]
fn projects_scene_default_dark_renders_the_same_screen() {
    let html = scene("Projects / default / 1440 / dark");
    assert_default_rows(&html);
}

#[test]
fn projects_scene_empty_shows_the_empty_state_and_count_zero() {
    let html = scene("Projects / empty / 1440");
    assert!(html.contains("badge badge-count\">0<"), "{html}");
    assert!(html.contains("No projects yet"), "{html}");
    assert!(
        html.contains("Open a folder and Berth finds its Compose file, or writes one for you."),
        "{html}"
    );
    assert!(html.contains("Drop a folder here"), "{html}");
    // The brief's copy (7.3) is the accessible name of the empty region.
    assert!(
        html.contains("aria-label=\"Open a folder to see its containers here\""),
        "{html}"
    );
    assert!(
        html.contains("data-tooltip=\"Folder browser not built yet\""),
        "{html}"
    );
    // No table, filter or segmented control without projects.
    assert!(!html.contains("project-row"), "{html}");
    assert!(!html.contains("Filter projects"), "{html}");
    assert!(!html.contains("Needs attention"), "{html}");
}

// --- Other states of brief 7.3 ---

type Setup = Box<dyn FnOnce(&mut AppState)>;

thread_local! {
    static SETUP: std::cell::RefCell<Option<Setup>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    use_hook(move || {
        if let Some(setup) = SETUP.with(|slot| slot.borrow_mut().take()) {
            setup(&mut state);
        }
    });
    rsx! { ProjectsScreen {} }
}

fn render(setup: impl FnOnce(&mut AppState) + 'static) -> String {
    SETUP.with(|slot| *slot.borrow_mut() = Some(Box::new(setup)));
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    strip_style(&dioxus_ssr::render(&dom))
}

#[test]
fn projects_state_before_any_engine_answers_is_loading_without_a_skeleton_yet() {
    let html = render(|_| {});
    assert!(html.contains("projects-loading"), "{html}");
    // SSR runs no effects, so the 300 ms timer has not fired: no skeleton,
    // no empty-state copy, no table.
    assert!(!html.contains("skeleton-row"), "{html}");
    assert!(!html.contains("No projects yet"), "{html}");
    assert!(!html.contains("project-row"), "{html}");
}

#[test]
fn projects_loading_body_draws_eight_skeleton_rows_once_shown() {
    let html = dioxus_ssr::render_element(rsx! { LoadingBody { shown: true } });
    assert_eq!(html.matches("skeleton-row\"").count(), 8, "{html}");
    let hidden = dioxus_ssr::render_element(rsx! { LoadingBody {} });
    assert!(!hidden.contains("skeleton-row"), "{hidden}");
}

#[test]
fn projects_skeleton_waits_three_hundred_milliseconds() {
    assert_eq!(SKELETON_DELAY_MS, 300);
    assert!(delay_script().contains("setTimeout(r, 300)"));
}

#[test]
fn projects_state_engine_stopped_offers_start_engine() {
    let html = render(|state| {
        let mut engines = world().engines.clone();
        for engine in &mut engines {
            if engine.id.0 == "colima:default" {
                engine.state = berth_core::EngineState::Stopped;
            }
        }
        engines.retain(|e| e.id.0 == "colima:default");
        state.engines.set(engines);
    });
    assert!(
        html.contains("Start the engine to see its projects"),
        "{html}"
    );
    assert!(html.contains("Colima default is stopped."), "{html}");
    assert!(html.contains("Start engine default"), "{html}");
    assert!(!html.contains("No projects yet"), "{html}");
}

#[test]
fn projects_state_stopped_engine_with_rows_shows_a_banner_above_the_table() {
    let html = render(|state| {
        let w = world();
        let mut engines = w.engines.clone();
        for engine in &mut engines {
            if engine.id.0 == "colima:default" {
                engine.state = berth_core::EngineState::Stopped;
            }
        }
        state.engines.set(engines);
        state.projects.set(w.projects.clone());
        state.containers.set(w.containers.clone());
    });
    assert!(
        html.contains("Colima default is stopped. Its projects are not running."),
        "{html}"
    );
    assert!(html.contains("Start engine"), "{html}");
    assert!(html.contains("project-row"), "{html}");
    let banner = html.find("projects-banners").expect("banner");
    let table = html.find("projects-table").expect("table");
    assert!(banner < table, "{html}");
}

#[test]
fn projects_state_unreachable_engine_row_says_so_in_place_of_ports() {
    let html = render(|state| {
        let w = world();
        state.engines.set(w.engines.clone());
        state.projects.set(w.projects.clone());
        state.containers.set(w.containers.clone());
    });
    assert!(html.contains("ports-unreachable"), "{html}");
    assert!(html.contains("Engine unreachable"), "{html}");
}

#[test]
fn projects_state_failed_action_shows_the_engine_message_under_the_row() {
    let html = render(|state| {
        let w = world();
        let id = project(&w, "berth-site").id.clone();
        state.engines.set(w.engines.clone());
        state.projects.set(w.projects.clone());
        state.containers.set(w.containers.clone());
        state.pending.write().insert(
            crate::state::PendingKey {
                subject: berth_core::Subject::Project(id),
                action: "project.stop",
            },
            crate::state::PendingState::Failed {
                message: "no configuration file provided: not found".to_string(),
            },
        );
    });
    assert!(html.contains("project-failure"), "{html}");
    assert!(
        html.contains("no configuration file provided: not found"),
        "{html}"
    );
    assert!(html.contains("Dismiss message for berth-site"), "{html}");
}

#[test]
fn projects_state_running_action_shows_its_pending_label_on_the_button() {
    let html = render(|state| {
        let w = world();
        let id = project(&w, "berth-site").id.clone();
        state.engines.set(w.engines.clone());
        state.projects.set(w.projects.clone());
        state.containers.set(w.containers.clone());
        state.pending.write().insert(
            crate::state::PendingKey {
                subject: berth_core::Subject::Project(id),
                action: "project.stop",
            },
            crate::state::PendingState::Running {
                label: "Stopping…".to_string(),
            },
        );
    });
    assert!(html.contains("Stopping…"), "{html}");
    assert!(html.contains("aria-busy=\"true\""), "{html}");
    assert!(html.contains("Stopping… is still in progress"), "{html}");
}

// --- Pure helpers ---

#[test]
fn projects_filter_counts_follow_the_status_of_each_project() {
    let w = world();
    let all: Vec<&berth_core::Project> = w.projects.iter().collect();
    assert_eq!(filter_counts(&all), [8, 3, 1, 2]);
    assert!(ProjectFilter::Running.matches(berth_core::ProjectStatus::AllRunning));
    assert!(
        !ProjectFilter::Running.matches(berth_core::ProjectStatus::Partial {
            running: 3,
            total: 4
        })
    );
    assert!(ProjectFilter::Failing.matches(berth_core::ProjectStatus::Failing));
    assert!(!ProjectFilter::Stopped.matches(berth_core::ProjectStatus::NotCreated));
    assert!(ProjectFilter::All.matches(berth_core::ProjectStatus::NotCreated));
    assert_eq!(ProjectFilter::from_id("failing"), ProjectFilter::Failing);
    assert_eq!(ProjectFilter::from_id("nonsense"), ProjectFilter::All);
}

#[test]
fn projects_query_matches_name_folder_branch_and_engine_case_insensitively() {
    let w = world();
    let shop = project(&w, "shop-api");
    assert!(matches_query(shop, "default", ""));
    assert!(matches_query(shop, "default", "  SHOP "));
    assert!(matches_query(shop, "default", "clients/acme"));
    assert!(matches_query(shop, "default", "checkout"));
    assert!(matches_query(shop, "default", "DEFAULT"));
    assert!(!matches_query(shop, "default", "zzz"));
    let landing = project(&w, "landing");
    assert!(matches_query(landing, "hetzner-1", "hetzner"));
    assert!(!matches_query(landing, "hetzner-1", "feat"));
}

#[test]
fn projects_sort_puts_failing_first_and_down_engines_last() {
    let w = world();
    let mut ordered: Vec<&berth_core::Project> = w.projects.iter().collect();
    ordered.reverse();
    ordered.sort_by_key(|p| sort_rank(p, &w.engines));
    let names: Vec<&str> = ordered.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names.first(), Some(&"shop-api"));
    assert_eq!(names.get(1), Some(&"analytics"));
    assert_eq!(names.last(), Some(&"landing"));
}

#[test]
fn projects_screen_state_covers_loading_empty_stopped_and_rows() {
    let w = world();
    let running = w
        .engines
        .iter()
        .find(|e| e.id.0 == "colima:default")
        .expect("default")
        .clone();
    let mut stopped = running.clone();
    stopped.state = berth_core::EngineState::Stopped;
    let mut starting = running.clone();
    starting.state = berth_core::EngineState::Starting;
    let mut down = running.clone();
    down.state = berth_core::EngineState::Unreachable {
        reason: "timed out".to_string(),
        last_seen: None,
    };
    assert_eq!(screen_state(&[], &[], false), ScreenState::Loading);
    assert_eq!(
        screen_state(std::slice::from_ref(&running), &[], false),
        ScreenState::Empty
    );
    assert_eq!(
        screen_state(std::slice::from_ref(&running), &[], true),
        ScreenState::Loading,
        "an engine that has not delivered data yet is still loading"
    );
    assert_eq!(
        screen_state(std::slice::from_ref(&starting), &[], false),
        ScreenState::Loading
    );
    assert_eq!(
        screen_state(std::slice::from_ref(&stopped), &[], false),
        ScreenState::EngineStopped(Box::new(stopped.clone()))
    );
    assert_eq!(
        screen_state(std::slice::from_ref(&down), &[], true),
        ScreenState::Unreachable
    );
    assert_eq!(
        screen_state(&[stopped], &w.projects, false),
        ScreenState::Rows,
        "known projects always show, even with their engine stopped"
    );
    assert_eq!(
        screen_state(&[], &w.projects, true),
        ScreenState::Rows,
        "a stale snapshot renders before any engine answers (D23)"
    );
}

#[test]
fn projects_attention_orders_errors_first_newest_first_and_names_the_subject() {
    let w = world();
    let now = chrono::DateTime::from_timestamp(1_800_000_000, 0).expect("ts");
    let make = |name: &str,
                severity: berth_core::Severity,
                age_s: i64,
                title: &str|
     -> berth_core::Problem {
        let id = w
            .containers
            .iter()
            .find(|c| c.name == name)
            .expect("container")
            .id
            .clone();
        let kind = berth_core::ProblemKind::Unhealthy;
        let subject = berth_core::Subject::Container(id);
        berth_core::Problem {
            id: berth_core::ProblemId::new(&kind, &subject),
            kind,
            severity,
            subject,
            title: title.to_string(),
            explanation: String::new(),
            evidence: Vec::new(),
            fixes: Vec::new(),
            first_seen: now - chrono::Duration::seconds(age_s),
        }
    };
    let problems = vec![
        make(
            "analytics-ingest-1",
            berth_core::Severity::Warning,
            60,
            "warn",
        ),
        make(
            "shop-api-worker-1",
            berth_core::Severity::Error,
            300,
            "old error",
        ),
        make(
            "shop-api-db-1",
            berth_core::Severity::Error,
            120,
            "new error",
        ),
        make("shop-api-api-1", berth_core::Severity::Info, 10, "info"),
    ];
    let items = attention_items(&problems, &w.containers, &w.projects, &w.engines, now);
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["new error", "old error", "warn"]);
    assert_eq!(items[0].subject_name, "shop-api-db-1");
    assert_eq!(items[0].age, "2 min");
    assert_eq!(items[2].age, "1 min");
}

// --- Narrow main area (D78): the table drops Engine and Branch ---

use super::super::css_tracks as css;
use crate::text_scale::STEPS;

use super::super::table_tiers::projects::{projects_lists, projects_tier_at, projects_tracks_at};
use super::super::table_tiers::View;

/// The track list of tier `n` (0 the full list, 1 without Engine and
/// Branch, 2 also without Ports, 3 also without Path).
fn tier_tracks(n: usize) -> Vec<String> {
    css::tracks(&projects_lists()[n])
}

#[test]
fn projects_table_hides_engine_and_branch_below_1200() {
    // The header's third and fourth cells and the row's two cells, hidden
    // by the generated tier (here at 130 %).
    let hidden = ":root[data-text-scale=\"1.3\"] .projects-head > :nth-child(3),\n    \
                  :root[data-text-scale=\"1.3\"] .projects-head > :nth-child(4),\n    \
                  :root[data-text-scale=\"1.3\"] .project-cell-engine,\n    \
                  :root[data-text-scale=\"1.3\"] .project-cell-branch,";
    assert!(PROJECTS_CSS.contains(hidden), "{PROJECTS_CSS}");
    // The nth-child rule is only right while those two are the third and
    // fourth header cells, and the row cells carry the classes hidden above.
    let html = scene("Projects / default / 1440");
    let head = html.split("class=\"projects-head\"").nth(1).expect("head");
    let labels: Vec<&str> = head
        .split("table-header-label\">")
        .skip(1)
        .take(4)
        .filter_map(|s| s.split('<').next())
        .collect();
    assert_eq!(labels, ["Project", "Path", "Engine", "Branch"]);
    assert!(html.contains("project-cell project-cell-engine"));
    assert!(html.contains("project-cell project-cell-branch"));
    // The screen is the container the block queries.
    assert!(PROJECTS_CSS.contains("container: projects / inline-size;"));
}

#[test]
fn projects_table_columns_at_1024_leave_room_for_the_buttons() {
    // Main at 1024 is 1024 - 220 wide; the table has 16 px margins and rows
    // pad 8 px each side. The last track must keep the labelled buttons
    // (Restart 65 + Start project 95 + two 24 px icons + 3 gaps of 4 = 216;
    // the gaps are 8 px from 1200 px wide).
    let tracks = tier_tracks(1);
    assert_eq!(tracks.len(), 7, "{tracks:?}");
    // Name, Ports, CPU, Memory and the buttons grow with the text size;
    // Path and Services share what is left.
    for i in [0, 3, 4, 5, 6] {
        assert!(
            css::has_scale(&tracks[i]),
            "track {i} `{}` needs a scale term",
            tracks[i]
        );
    }
    assert_eq!(tracks[1], "minmax(0, 104fr)");
    assert_eq!(tracks[2], "minmax(0, 112fr)");
    let fixed = css::min_sum_with(&tracks, 1.0, &actions_vars(MAIN_1024));
    let available = 1024.0 - 220.0 - 32.0 - 16.0;
    assert!(
        (available - fixed - (104.0 + 112.0)).abs() < 0.5,
        "at 100 % Path and Services get 104 and 112 px: fixed {fixed}"
    );
    let buttons = css::min_px_with(&tracks[6], 1.0, &actions_vars(MAIN_1024));
    assert!(
        (216.0..=232.0).contains(&buttons),
        "{buttons} px for the buttons"
    );
}

#[test]
fn projects_table_wide_tracks_grow_with_the_text() {
    let cols = css::declaration(
        css::block(PROJECTS_CSS, ".projects-screen {"),
        "--projects-cols",
    );
    let tracks = css::tracks(&cols);
    assert_eq!(tracks.len(), 9, "{cols}");
    for (i, t) in tracks[..8].iter().enumerate() {
        assert!(css::has_scale(t), "track {i} `{t}` needs a scale term");
    }
    assert_eq!(tracks[8], "1fr");
}

#[test]
fn projects_table_ports_track_closes_in_the_narrowest_tier() {
    let tracks = tier_tracks(2);
    assert_eq!(tracks.len(), 7, "{tracks:?}");
    assert_eq!(tracks[3], "0px", "the Ports track");
    for i in [0, 4, 5, 6] {
        assert!(
            css::has_scale(&tracks[i]),
            "track {i} `{}` needs a scale term",
            tracks[i]
        );
    }
}

/// Main-area widths of the two windows the tests sweep.
const MAIN_1024: f32 = 1024.0 - 220.0;
const MAIN_1440: f32 = 1440.0 - 220.0;

/// The buttons' track takes 44 px (scaled text apart) plus three gaps: 4 px
/// under 1200 px wide, 8 px from there up. `width` is a main width above.
fn actions_vars(width: f32) -> [(&'static str, f32); 1] {
    [(
        "--projects-actions-w",
        if width <= MAIN_1024 { 56.0 } else { 68.0 },
    )]
}

/// The viewport a main-area width stands for: the 1024 window is under 1200.
fn view_of(width: f32) -> View {
    if width <= MAIN_1024 {
        View::Narrow
    } else {
        View::Wide
    }
}

/// Whether the Path column is hidden: the width falls in the last tier.
fn path_hidden_at(width: f32, scale: f32) -> bool {
    projects_tier_at(view_of(width), width, scale) >= 3
}

/// The column list in force for a screen `width` px wide at `scale`.
fn projects_tracks(width: f32, scale: f32) -> Vec<String> {
    projects_tracks_at(view_of(width), width, scale)
}

#[test]
fn projects_table_fits_at_1024_and_1440_for_every_text_size() {
    for width in [MAIN_1024, MAIN_1440] {
        for scale in STEPS {
            let tracks = projects_tracks(width, scale);
            // The table sits on the page gutter: 16 px under 1200 px wide,
            // 20 px from there up; the rows pad 8 px each side.
            let gutters = if width == MAIN_1024 { 32.0 } else { 40.0 };
            let vars = actions_vars(width);
            let room = width - gutters - 16.0 - css::min_sum_with(&tracks, scale, &vars);
            // The buttons are a track of their own in the narrow lists and
            // the `1fr` in the wide one; Path and Services (or Services
            // alone, once Path is gone) keep some room.
            let buttons = 160.0 * scale + vars[0].1;
            let (need, what) = match projects_tier_at(view_of(width), width, scale) {
                0 => (buttons, "the buttons"),
                1 => (216.0, "Path and Services"),
                2 => (250.0, "Path and Services"),
                _ => (96.0, "Services"),
            };
            assert!(
                room >= need,
                "{width} px at {scale}: {room} px left for {what}, needs {need} ({tracks:?})"
            );
        }
    }
}

#[test]
fn projects_path_column_is_hidden_at_1024_from_130_percent_up_only() {
    // Hidden: 1024 px at 130 % and 150 %.
    for scale in [1.3, 1.5] {
        assert!(path_hidden_at(MAIN_1024, scale), "1024 x {scale}");
    }
    // Shown: 1024 px at every step up to 120 %, and 1440 px at every step.
    for scale in STEPS.into_iter().filter(|s| *s <= 1.2) {
        assert!(!path_hidden_at(MAIN_1024, scale), "1024 x {scale}");
    }
    for scale in STEPS {
        assert!(!path_hidden_at(MAIN_1440, scale), "1440 x {scale}");
    }
    // The tracks agree: Path keeps a track exactly where it is shown.
    for (width, scale, tracks) in [
        (MAIN_1024, 1.2, 7),
        (MAIN_1024, 1.3, 6),
        (MAIN_1024, 1.5, 6),
        (MAIN_1440, 1.5, 7),
        (MAIN_1440, 1.0, 9),
    ] {
        assert_eq!(
            projects_tracks(width, scale).len(),
            tracks,
            "{width} x {scale}"
        );
    }
}

#[test]
fn projects_no_path_tier_hides_path_and_gives_services_its_share() {
    // The header's second cell and the row's Path cell, hidden by the
    // generated tier (here at 130 %).
    let hidden = ":root[data-text-scale=\"1.3\"] .projects-head > :nth-child(2),\n    \
                  :root[data-text-scale=\"1.3\"] .project-cell-path {\n      display: none;";
    assert!(PROJECTS_CSS.contains(hidden), "{PROJECTS_CSS}");
    // Child 2 is Path while the markup keeps that order; the row's cell
    // carries the class hidden above.
    let html = scene("Projects / default / 1440");
    let head = html.split("class=\"projects-head\"").nth(1).expect("head");
    let second = head
        .split("table-header-label\">")
        .nth(2)
        .and_then(|s| s.split('<').next());
    assert_eq!(second, Some("Path"));
    assert!(html.contains("project-cell project-cell-path"));
    // Six visible columns: the nine less Path, Engine and Branch. Name and
    // Services keep the 116:216 split of what is left (Services is Path's
    // 104 plus its own 112), Ports stays closed, the numeric tracks and the
    // buttons keep their size.
    let tracks = tier_tracks(3);
    assert_eq!(tracks.len(), 6, "{tracks:?}");
    assert_eq!(tracks[1], "minmax(0, 216fr)", "Services");
    assert_eq!(tracks[2], "0px", "Ports");
    for i in [3, 4, 5] {
        assert!(css::has_scale(&tracks[i]), "track {i} `{}`", tracks[i]);
    }
    assert!(tracks[0].ends_with(", 116fr)"), "Name `{}`", tracks[0]);
}

#[test]
fn projects_no_path_tier_keeps_names_14_characters_wide_and_nothing_clips() {
    let tracks = tier_tracks(3);
    for scale in [1.3, 1.5] {
        // The Name track asks for 14 characters (a digit is 0.6 em, em is
        // the 13 px body size times the scale) plus the folder icon, the gap
        // after it and the cell's right padding.
        let name = css::min_px(&tracks[0], scale);
        let want = 14.0 * 0.6 * 13.0 * scale + 16.0 * scale + 8.0 + 8.0;
        assert!(
            (name - want).abs() < 0.5,
            "Name asks for {name} px at {scale}, wants {want}"
        );
        // Services gives up room for it but never pushes the table wider
        // than the window, and CPU, Memory and the buttons keep their size.
        let vars = actions_vars(MAIN_1024);
        let room = MAIN_1024 - 32.0 - 16.0 - css::min_sum_with(&tracks, scale, &vars);
        assert!(room >= 96.0, "{room} px left for Services at {scale}");
        for (i, base) in [(3, 48.0), (4, 56.0)] {
            assert_eq!(css::min_px(&tracks[i], scale), base * scale, "track {i}");
        }
        assert!(css::min_px_with(&tracks[5], scale, &vars) >= 160.0 * scale + 56.0);
    }
    // The minimum grows with the text, it is not a fixed px width.
    assert!(css::min_px(&tracks[0], 1.5) > css::min_px(&tracks[0], 1.3) + 20.0);
}

#[test]
fn projects_name_cell_carries_the_full_path_as_its_tooltip() {
    let html = scene("Projects / default / 1440");
    // At any size: the cell, whatever the collapsed text in the Path column.
    assert!(
        html.contains("project-cell-name\" role=\"cell\" title=\"~/Dev/clients/acme/shop-api\""),
        "{html}"
    );
    assert!(html.contains("title=\"~/Dev/analytics\""), "{html}");
}

#[test]
fn projects_name_cell_of_unassigned_has_a_dash_for_its_tooltip() {
    let html = scene("Projects / with unassigned / 1440");
    let at = html
        .find("<span class=\"project-name-text\">Unassigned</span>")
        .expect("Unassigned row");
    let cell = &html[..at];
    let cell = &cell[cell.rfind("project-cell-name").expect("name cell")..];
    assert!(cell.contains("title=\"\u{2014}\""), "{cell}");
}

// --- The Unassigned row (loose containers; the owner keeps it) ---

#[test]
fn projects_sort_puts_the_unassigned_project_after_every_named_one() {
    let w = world();
    let mut projects = w.projects.clone();
    projects.extend(crate::scenes::loose_projects(&w));
    projects.reverse();
    projects.sort_by_key(|p| sort_rank(p, &w.engines));
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
    // Last even though "landing" sits on an engine that is down.
    assert_eq!(names.last(), Some(&"Unassigned"), "{names:?}");
    assert_eq!(names.iter().filter(|n| **n == "Unassigned").count(), {
        crate::scenes::loose_projects(&w).len()
    });
    let landing = names.iter().position(|n| *n == "landing").expect("landing");
    let first_loose = names
        .iter()
        .position(|n| *n == "Unassigned")
        .expect("unassigned");
    assert!(landing < first_loose, "{names:?}");
}

#[test]
fn projects_scene_with_unassigned_shows_the_row_last_and_the_sidebar_hides_it() {
    let html = scene("Projects / with unassigned / 1440");
    let table = &html[html.find("projects-table").expect("table")..];
    let name = |n: &str| format!("<span class=\"project-name-text\">{n}</span>");
    let landing = table.find(&name("landing")).expect("landing row");
    let loose = table.find(&name("Unassigned")).expect("Unassigned row");
    assert!(landing < loose, "{table}");
    assert_eq!(table.matches(&name("Unassigned")).count(), 1);
    // Its row: path is a dash, and the buttons are the group ones.
    let row = &table[loose..];
    let row = &row[..row.find("project-row-wrap").unwrap_or(row.len())];
    assert!(
        row.contains("project-cell-path\" role=\"cell\" title=\"—\">—<"),
        "{row}"
    );
    assert!(row.contains(">Stop all<"), "{row}");
    assert!(row.contains(">Restart<"), "{row}");
    assert!(!row.contains("Stop project"), "{row}");
    assert!(!row.contains("disabled"), "{row}");
    // The sidebar lists projects, not this bucket.
    let sidebar = &html[..html.find("projects-screen").expect("screen")];
    assert!(!sidebar.contains("Unassigned"), "{sidebar}");
    assert!(sidebar.contains("docker-manger"), "{sidebar}");
}

// --- AD5: the Unassigned row is a holding row, not a project ---

/// The mock world's projects plus its default-engine Unassigned row, and
/// the containers the row groups.
fn world_with_unassigned() -> (
    Vec<berth_core::Project>,
    Vec<berth_core::Container>,
    Vec<berth_core::Engine>,
) {
    let w = world();
    let mut projects = w.projects.clone();
    projects.extend(
        crate::scenes::loose_projects(&w)
            .into_iter()
            .filter(|p| p.engine.0 == "colima:default"),
    );
    (projects, w.containers.clone(), w.engines.clone())
}

fn listed(
    projects: &[berth_core::Project],
    containers: &[berth_core::Container],
    engines: &[berth_core::Engine],
    query: &str,
    filter: ProjectFilter,
) -> (usize, Vec<String>) {
    let mut ordered: Vec<&berth_core::Project> = projects.iter().collect();
    ordered.sort_by_key(|p| sort_rank(p, engines));
    let l = list_projects(&ordered, engines, containers, query, filter);
    (
        l.matched,
        l.visible.iter().map(|p| p.name.clone()).collect(),
    )
}

#[test]
fn projects_scene_with_unassigned_counts_eight_projects_not_nine() {
    let html = scene("Projects / with unassigned / 1440");
    // The row is there, last, and the numbers still say 8.
    assert!(html.contains("<span class=\"project-name-text\">Unassigned</span>"));
    assert!(html.contains("badge badge-count\">8<"), "{html}");
    assert!(!html.contains("badge badge-count\">9<"), "{html}");
    for (label, count) in [("All", 8), ("Running", 3), ("Failing", 1), ("Stopped", 2)] {
        assert!(
            html.contains(&format!(
                "<span>{label}</span><span class=\"segment-count\">{count}</span>"
            )),
            "segment {label} {count}: {html}"
        );
    }
    // The sidebar's Projects badge (it sits before the screen) is 8 as well.
    let sidebar = &html[..html.find("projects-screen").expect("screen")];
    assert!(sidebar.contains("sidebar-count\">8<"), "{sidebar}");
    assert!(!sidebar.contains("sidebar-count\">9<"), "{sidebar}");
}

#[test]
fn projects_filter_counts_leave_the_unassigned_row_out_of_every_segment() {
    let (projects, _, _) = world_with_unassigned();
    assert_eq!(projects.len(), 9);
    let all: Vec<&berth_core::Project> = projects.iter().collect();
    assert_eq!(filter_counts(&all), [8, 3, 1, 2]);
    assert_eq!(real_count(&all), 8);
    // Even when its own status would match a segment.
    let loose = projects.iter().find(|p| is_unassigned(p)).expect("loose");
    assert!(ProjectFilter::ALL.iter().any(|f| f.matches(loose.status)));
}

#[test]
fn projects_status_filters_hide_the_unassigned_row_and_all_shows_it_last() {
    let (projects, containers, engines) = world_with_unassigned();
    for filter in [
        ProjectFilter::Running,
        ProjectFilter::Failing,
        ProjectFilter::Stopped,
    ] {
        let (_, names) = listed(&projects, &containers, &engines, "", filter);
        assert!(!names.is_empty(), "{filter:?} lists real projects");
        assert!(
            !names.iter().any(|n| n == "Unassigned"),
            "{filter:?}: {names:?}"
        );
    }
    let (_, names) = listed(&projects, &containers, &engines, "", ProjectFilter::All);
    assert_eq!(names.len(), 9, "{names:?}");
    assert_eq!(names.last().map(String::as_str), Some("Unassigned"));
}

#[test]
fn projects_filter_text_shows_unassigned_for_its_name_or_a_container_name() {
    let (projects, containers, engines) = world_with_unassigned();
    let loose = projects.iter().find(|p| is_unassigned(p)).expect("loose");
    let member = containers
        .iter()
        .find(|c| loose.services.iter().any(|s| s.containers.contains(&c.id)))
        .expect("a container in the bucket");
    let member_name = member.name.clone();
    let shows = |q: &str| {
        listed(&projects, &containers, &engines, q, ProjectFilter::All)
            .1
            .iter()
            .any(|n| n == "Unassigned")
    };
    assert!(shows(""));
    assert!(shows("unassign"));
    assert!(shows("  UNASSIGNED "));
    assert!(shows(&member_name));
    assert!(shows(&member_name.to_uppercase()));
    // Not for what a named project matches on: its engine or a folder.
    assert!(!shows("shop-api"));
    assert!(!shows("zzz-no-such-thing"));
    // A container of a named project does not bring the row in.
    let other = containers
        .iter()
        .find(|c| c.name.starts_with("shop-api-db"))
        .expect("shop-api container");
    assert!(!shows(&other.name));
}

#[test]
fn projects_filter_text_count_counts_projects_only() {
    let (projects, containers, engines) = world_with_unassigned();
    // "unassigned" matches only the row: zero of eight, one row listed.
    let (matched, names) = listed(
        &projects,
        &containers,
        &engines,
        "unassigned",
        ProjectFilter::All,
    );
    assert_eq!(matched, 0);
    assert_eq!(names, ["Unassigned"]);
    // The empty query matches the eight real projects, not nine.
    let (matched, _) = listed(&projects, &containers, &engines, "", ProjectFilter::All);
    assert_eq!(matched, 8);
}

#[test]
fn projects_needs_attention_is_one_bordered_list_with_dividers() {
    let html = scene("Projects / default / 1440");
    assert_eq!(
        html.matches("class=\"attention-list\"").count(),
        1,
        "{html}"
    );
    let list = html
        .split("class=\"attention-list\"")
        .nth(1)
        .expect("the list");
    assert_eq!(list.matches("class=\"attention-card\"").count(), 3);
    let card = css::block(PROJECTS_CSS, "\n.attention-card {");
    assert!(card.contains("height: var(--size-row);"), "{card}");
    assert!(!card.contains("margin"), "rows do not float apart: {card}");
    assert!(PROJECTS_CSS.contains(
        ".attention-card + .attention-card {\n  border-top: 1px solid var(--color-line);"
    ));
    let list = css::block(PROJECTS_CSS, "\n.attention-list {");
    assert!(
        list.contains("border: 1px solid var(--color-line);"),
        "{list}"
    );
    // Nothing below the list: the next section's 32 px above is the whole gap.
    let section = css::block(PROJECTS_CSS, "\n.projects-attention {");
    assert!(
        section.contains("padding: var(--space-32) var(--gutter) 0;"),
        "{section}"
    );
    let recent = css::block(PROJECTS_CSS, "\n.projects-recent {");
    assert!(
        recent.contains("padding: var(--space-32) var(--gutter) 0;"),
        "{recent}"
    );
}

#[test]
fn projects_rows_use_the_large_row_token_and_a_divider() {
    // The rule that sets the height (the grid rule before it names the row too).
    let row = css::block(PROJECTS_CSS, "\n.project-row {\n  height");
    assert!(row.contains(": var(--size-row-lg);"), "{row}");
    assert!(
        row.contains("border-bottom: 1px solid var(--color-line);"),
        "{row}"
    );
    // The row's border is part of its height (grid rows are border-box).
    let shared = css::block(PROJECTS_CSS, ".projects-head,\n.project-row {");
    assert!(shared.contains("box-sizing: border-box;"), "{shared}");
    assert!(!PROJECTS_CSS.contains("--projects-row"), "one row token");
}

#[test]
fn projects_button_gap_is_4_under_1200_px_wide_and_8_from_there_up() {
    let screen = css::block(PROJECTS_CSS, "\n.projects-screen {");
    assert_eq!(
        css::declaration(screen, "--projects-actions-gap"),
        "var(--space-4)"
    );
    let wide = css::block(PROJECTS_CSS, "@media (min-width: 1200px) {");
    assert!(
        wide.contains("--projects-actions-gap: var(--space-8);"),
        "{wide}"
    );
    let cell = css::block(PROJECTS_CSS, "\n.project-cell-actions {");
    assert!(cell.contains("gap: var(--projects-actions-gap);"), "{cell}");
}

/// Projects of one rank keep their places when an engine's slice is
/// rewritten (every container event rewrites it, at the end of the list).
#[test]
fn projects_rows_of_one_rank_keep_their_places_when_an_engine_slice_moves() {
    let w = world();
    let running = w
        .engines
        .iter()
        .find(|e| e.state == berth_core::EngineState::Running)
        .expect("a running engine")
        .clone();
    let engine = |id: &str| berth_core::Engine {
        id: berth_core::EngineId(id.to_string()),
        ..running.clone()
    };
    let engines = [engine("one"), engine("two")];
    let on = |engine: &str, name: &str| berth_core::Project {
        engine: berth_core::EngineId(engine.to_string()),
        name: name.to_string(),
        id: berth_core::ProjectId(format!("{engine}/{name}")),
        status: berth_core::ProjectStatus::AllRunning,
        ..project(&w, "berth-site").clone()
    };
    let one = vec![on("one", "alpha"), on("one", "zulu")];
    let two = vec![on("two", "bravo")];
    let names = |projects: &[berth_core::Project]| -> Vec<String> {
        ordered_projects(projects, &engines)
            .iter()
            .map(|p| p.name.clone())
            .collect()
    };
    let before = names(&[one.clone(), two.clone()].concat());
    assert_eq!(before, ["alpha", "zulu", "bravo"]);
    // What replace_engine_slice does after an event on engine one: its
    // rows go to the end of the list.
    assert_eq!(names(&[two.clone(), one.clone()].concat()), before);
}
