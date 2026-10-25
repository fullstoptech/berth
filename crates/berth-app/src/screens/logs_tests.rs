use super::*;
use crate::scenes;

fn frame(name: &str) -> String {
    scenes::render_scene_html(name).expect("the frame has a scene")
}

fn count(html: &str, needle: &str) -> usize {
    html.matches(needle).count()
}

fn container(id: &str, name: &str, project: Option<&str>) -> Container {
    let world = berth_engine::MockWorld::bundled();
    let mut c = world.containers.first().cloned().expect("a mock container");
    c.id = ContainerId(id.to_string());
    c.name = name.to_string();
    c.project = project.map(|p| ProjectId(p.to_string()));
    c
}

fn project(id: &str, name: &str) -> Project {
    let mut project = berth_engine::MockWorld::bundled()
        .projects
        .first()
        .cloned()
        .expect("a mock project");
    project.id = ProjectId(id.to_string());
    project.name = name.to_string();
    project.services.clear();
    project
}

#[test]
fn logs_short_name_drops_the_project_prefix_and_replica_number() {
    assert_eq!(short_name("shop-api-worker-1", Some("shop-api")), "worker");
    assert_eq!(
        short_name("shop-api-mailpit-1", Some("shop-api")),
        "mailpit"
    );
    assert_eq!(short_name("shop-api-api-1", Some("shop-api")), "api");
    // No project: the name stays whole, minus a replica number.
    assert_eq!(short_name("ferry-pg", None), "ferry-pg");
    assert_eq!(short_name("worker-2", None), "worker");
    // A name that is only the prefix is not emptied.
    assert_eq!(short_name("shop-api", Some("shop-api")), "shop-api");
}

#[test]
fn logs_title_names_the_project_when_the_set_is_one_project() {
    let a = container("a", "shop-api-api-1", Some("p1"));
    let b = container("b", "shop-api-db-1", Some("p1"));
    let c = container("c", "other-1", Some("p2"));
    let project = project("p1", "shop-api");
    let all = [a.clone(), b.clone(), c.clone()];
    let id = |s: &str| ContainerId(s.to_string());
    assert_eq!(
        view_title(&all, std::slice::from_ref(&project), &[id("a"), id("b")]),
        "Logs \u{b7} shop-api (2 containers)"
    );
    assert_eq!(
        view_title(&all, std::slice::from_ref(&project), &[id("a"), id("c")]),
        "Logs \u{b7} 2 containers"
    );
    assert_eq!(
        view_title(&all, std::slice::from_ref(&project), &[id("c")]),
        "Logs \u{b7} other-1"
    );
    assert_eq!(view_title(&all, &[], &[]), "Logs");
}

#[test]
fn logs_start_from_the_selected_containers_or_the_selected_projects() {
    let a = container("a", "x-1", Some("p1"));
    let b = container("b", "y-1", Some("p1"));
    let c = container("c", "z-1", None);
    let all = [a, b, c];
    let project = project("p1", "shop-api");
    let id = |s: &str| ContainerId(s.to_string());
    assert_eq!(
        ids_from_selection(&Selection::Containers(vec![id("c")]), &all, &[]),
        vec![id("c")]
    );
    assert_eq!(
        ids_from_selection(
            &Selection::Project(ProjectId("p1".to_string())),
            &all,
            std::slice::from_ref(&project)
        ),
        vec![id("a"), id("b")]
    );
    assert!(ids_from_selection(&Selection::None, &all, &[]).is_empty());
    assert!(ids_from_selection(&Selection::Image("i".to_string()), &all, &[]).is_empty());
}

#[test]
fn logs_targets_skip_a_container_that_is_gone() {
    let a = container("a", "x-1", None);
    let id = |s: &str| ContainerId(s.to_string());
    let targets = targets_for(&[a], &[id("a"), id("gone")]);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].name, "x-1");
}

#[test]
fn logs_add_menu_offers_projects_then_containers_not_followed_yet() {
    let a = container("a", "x-1", Some("p1"));
    let b = container("b", "y-1", Some("p1"));
    let c = container("c", "z-1", None);
    let project = project("p1", "shop-api");
    let all = [a, b, c];
    let id = |s: &str| ContainerId(s.to_string());
    let entries = add_entries(&all, std::slice::from_ref(&project), &[id("a")]);
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["p:p1", "c:b", "c:c"]);
    assert_eq!(entries[0].label, "Whole project \u{b7} shop-api (1)");
    assert_eq!(
        ids_for_entry("p:p1", &all, std::slice::from_ref(&project)),
        vec![id("a"), id("b")]
    );
    assert_eq!(ids_for_entry("c:c", &all, &[]), vec![id("c")]);
    assert!(add_entries(&all, &[project], &[id("a"), id("b"), id("c")]).is_empty());
}

#[test]
fn logs_errors_frame_shows_chips_toolbar_selection_and_search() {
    let html = frame("Logs / errors / 1440");
    assert!(
        html.contains("Logs \u{b7} shop-api (5 containers)"),
        "{html}"
    );
    // One chip per container, in its series colour (api = 1 … mailpit = 5).
    for (hue, name) in [
        (1, "api"),
        (2, "worker"),
        (3, "db"),
        (4, "redis"),
        (5, "mailpit"),
    ] {
        assert!(
            html.contains(&format!(
                "style=\"color: var(--color-series-{hue});\"><span class=\"log-chip-icon\""
            )),
            "chip colour {hue} for {name}: {html}"
        );
        assert!(
            html.contains(&format!("<span class=\"log-chip-name\">{name}</span>")),
            "{name}"
        );
    }
    // Toolbar: Follow and Time on, Wrap and Raw off.
    assert!(html.contains("log-toggle log-toggle-on\" type=\"button\" aria-pressed=\"true\" data-tooltip=\"Keep the newest line in view\""), "{html}");
    assert!(html.contains("aria-pressed=\"false\" data-tooltip=\"Wrap long lines\""));
    assert!(html.contains("aria-pressed=\"true\" data-tooltip=\"Show timestamps\""));
    // Search: the text, and "4 of 4".
    assert!(html.contains("value=\"ECONNREFUSED\""), "{html}");
    assert!(html.contains(">4 of 4<"), "{html}");
    // Level chips with counts; stream and range selects; Next error with its key.
    assert!(
        html.contains("<span>Error</span><span class=\"segment-count\">5</span>"),
        "{html}"
    );
    assert!(html.contains("<span>Warn</span><span class=\"segment-count\">1</span>"));
    assert!(html.contains("All streams"));
    assert!(html.contains("Last 15 min"));
    assert!(html.contains("Next error"));
    assert!(html.contains(">F8<"));
    // Two lines selected: Explain is live (the rules run for the lines), Ask is disabled and says why.
    assert!(html.contains("2 lines selected"), "{html}");
    assert_eq!(count(&html, "log-selected"), 2, "{html}");
    assert!(!html.contains("Explain arrives with Problems"));
    let explain = html.find(">Explain<").expect("the Explain button");
    let button_start = html[..explain].rfind("<button").expect("its tag");
    assert!(
        !html[button_start..explain].contains("disabled"),
        "{}",
        &html[button_start..explain]
    );
    assert!(html.contains("Ask needs a harness"));
    // Follow is on: no pill.
    assert!(!html.contains("new lines"));
    assert!(html.contains("Ask Claude Code"), "{html}");
}

#[test]
fn logs_explained_scene_shows_what_the_rules_say_under_the_selection_bar() {
    let html = frame("Logs / explained / 1440");
    // The explanation sits between the selection bar and the lines, with the fix the rules offer
    // and a harness fix that waits for a harness.
    let bar = html.find("log-selbar").expect("the selection bar");
    let explain = html.find("class=\"explain\"").expect("the explanation");
    let rows = html.find("log-list").expect("the lines");
    assert!(bar < explain && explain < rows);
    let panel = &html[explain..rows];
    assert!(panel.contains("shop-api-worker-1 cannot reach db"));
    assert!(panel.contains("Restart shop-api-db-1"));
    assert!(panel.contains("Needs a harness (P7-T6)"));
    // Without a result nothing is drawn: the plain frame has no explanation.
    assert!(!frame("Logs / errors / 1440").contains("class=\"explain\""));
}

#[test]
fn logs_errors_frame_rows_tint_fold_and_highlight() {
    let html = frame("Logs / errors / 1440");
    // Five error heads plus the three trace lines of the open first trace.
    assert_eq!(count(&html, "log-row log-error"), 5 + 3, "{html}");
    assert_eq!(count(&html, "log-row log-warn"), 1);
    // The three closed traces say how many lines they hide; the open one offers to close.
    assert_eq!(count(&html, "11 more lines"), 3, "{html}");
    assert_eq!(count(&html, "3 more lines"), 1, "{html}");
    assert_eq!(count(&html, "aria-expanded=\"true\""), 1);
    // Four matches highlighted.
    assert_eq!(count(&html, "class=\"log-match\""), 4, "{html}");
    // Info JSON collapses; the warn line shows its text.
    assert!(html.contains("{ level: info, msg: GET /health, status: 200, ms: 2.1 }"));
    assert!(
        html.contains("{&#34;level&#34;:&#34;warn&#34;,&#34;msg&#34;:&#34;cache miss storm&#34;,&#34;keys&#34;:412}"),
        "{html}"
    );
    // Merged view: the container column, names in full.
    assert!(html.contains("shop-api-worker-1"));
    assert!(html.contains("14:02:12.031"));
    assert!(html.contains("stderr"));
}

#[test]
fn logs_paused_frame_shows_the_new_lines_pill_and_no_selection() {
    let html = frame("Logs / paused / 1440");
    assert!(html.contains("142 new lines"), "{html}");
    assert!(!html.contains("lines selected"));
    assert_eq!(count(&html, "log-selected"), 0);
    // Follow is off.
    assert!(
        html.contains("aria-pressed=\"false\" data-tooltip=\"Keep the newest line in view\""),
        "{html}"
    );
    assert!(html.contains(">4 of 4<"));
    assert!(html.contains("data-follow=\"0\""), "{html}");
}

#[test]
fn logs_frames_have_no_pill_while_following_and_no_dropped_notice() {
    let html = frame("Logs / errors / 1440");
    assert!(html.contains("data-follow=\"1\""));
    assert!(!html.contains("Older lines dropped"));
}
