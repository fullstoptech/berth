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
