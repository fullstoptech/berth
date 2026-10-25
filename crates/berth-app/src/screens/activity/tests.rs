//! Activity screen tests (P2-T10): pause counts new events, filters keep
//! the right rows, gap markers survive filtering, and the rendered screen
//! follows the store and the live tap.

use super::*;
use crate::services::activity as svc;
use crate::state::use_app_state;
use berth_core::{ActivityEntry, ActivityOrigin, Outcome, Subject};
use chrono::TimeZone;

fn engine(id: &str) -> berth_core::EngineId {
    berth_core::EngineId(id.to_string())
}

fn at(secs: i64) -> DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_790_000_000 + secs, 0).expect("ts")
}

fn entry(
    secs: i64,
    engine_id: &str,
    origin: ActivityOrigin,
    action: &str,
    container: &str,
    outcome: Outcome,
) -> ActivityEntry {
    ActivityEntry {
        id: uuid::Uuid::now_v7(),
        ts: at(secs),
        engine: engine(engine_id),
        actor: berth_core::Actor::Local,
        origin,
        action: action.to_string(),
        subject: Subject::Container(berth_core::ContainerId(container.to_string())),
        subject_name: container.to_string(),
        project: None,
        outcome,
    }
}

fn ev(secs: i64, engine_id: &str, container: &str) -> ActivityEntry {
    entry(
        secs,
        engine_id,
        ActivityOrigin::Engine,
        "container.die",
        container,
        Outcome::Ok,
    )
}

fn names(feed: &Feed, rows: &[usize]) -> Vec<String> {
    rows.iter()
        .filter_map(|&i| feed.get(i))
        .map(|e| e.subject_name.clone())
        .collect()
}

fn all(feed: &Feed) -> Vec<String> {
    let rows = feed.visible(&Filters::default(), &ProjectOf::new());
    names(feed, &rows)
}

fn filtered(feed: &Feed, filters: &Filters, projects: &ProjectOf) -> Vec<String> {
    let rows = feed.visible(filters, projects);
    names(feed, &rows)
}

// ---- pause ----

#[test]
fn activity_pause_counts_new_events_and_freezes_the_table() {
    let mut feed = Feed::from_seed(FeedSeed {
        entries: vec![ev(10, "e1", "a"), ev(20, "e1", "b")],
        ..FeedSeed::default()
    });
    feed.pause();
    feed.push(ev(30, "e1", "c"));
    feed.push(ev(40, "e1", "d"));
    let filters = Filters::default();
    let projects = ProjectOf::new();

    assert_eq!(all(&feed), ["b", "a"], "table does not move while paused");
    assert_eq!(feed.held_count(&filters, &projects), 2);
    assert_eq!(
        new_events_label(feed.held_count(&filters, &projects)),
        "2 new events"
    );

    assert_eq!(feed.resume(), 2);
    assert!(!feed.is_paused());
    assert_eq!(
        all(&feed),
        ["d", "c", "b", "a"],
        "newest first after resume"
    );
    assert_eq!(feed.held_count(&filters, &projects), 0);
    feed.push(ev(50, "e1", "e"));
    assert_eq!(all(&feed)[0], "e", "live again after resume");
}

#[test]
fn activity_pause_counter_counts_only_what_the_filters_would_show() {
    let mut feed = Feed::default();
    feed.pause();
    feed.push(ev(1, "e1", "a"));
    feed.push(ev(2, "e2", "b"));
    feed.push(ev(3, "e2", "c"));
    let projects = ProjectOf::new();
    let on_e2 = Filters {
        engine: Some(engine("e2")),
        ..Filters::default()
    };
    assert_eq!(feed.held_count(&Filters::default(), &projects), 3);
    assert_eq!(feed.held_count(&on_e2, &projects), 2);
}

#[test]
fn activity_new_events_label_reads_for_zero_one_many() {
    assert_eq!(new_events_label(0), "No new events");
    assert_eq!(new_events_label(1), "1 new event");
    assert_eq!(new_events_label(23), "23 new events");
}

// ---- feed ----

#[test]
fn activity_feed_ignores_an_entry_seen_twice() {
    let e = ev(1, "e1", "a");
    let mut feed = Feed::default();
    feed.push(e.clone());
    feed.push(e.clone());
    feed.extend(vec![e.clone()]);
    feed.pause();
    feed.push(e);
    assert_eq!(feed.len(), 1);
    assert_eq!(feed.held_count(&Filters::default(), &ProjectOf::new()), 0);
}

#[test]
fn activity_feed_keeps_time_order_for_late_and_history_entries() {
    let mut feed = Feed::default();
    feed.push(ev(20, "e1", "b"));
    feed.push(ev(10, "e1", "a"));
    feed.push(ev(30, "e1", "c"));
    assert_eq!(all(&feed), ["c", "b", "a"]);
    feed.extend(vec![ev(25, "e1", "x"), ev(5, "e1", "y")]);
    assert_eq!(all(&feed), ["c", "x", "b", "a", "y"]);
}

#[test]
fn activity_feed_holds_at_most_max_loaded_entries() {
    let mut feed = Feed::default();
    let extra = 5_i64;
    feed.extend((0..(MAX_LOADED as i64 + extra)).map(|i| ev(i, "e1", &format!("c{i}"))));
    assert_eq!(feed.len(), MAX_LOADED);
    let newest = feed.get(0).expect("newest").subject_name.clone();
    assert_eq!(newest, format!("c{}", MAX_LOADED as i64 + extra - 1));
}

// ---- filters ----

fn sample() -> Feed {
    Feed::from_seed(FeedSeed {
        entries: vec![
            ev(1, "e1", "engine-e1"),
            ev(2, "e2", "engine-e2"),
            entry(
                3,
                "e1",
                ActivityOrigin::Berth,
                "container.restart",
                "berth-ok",
                Outcome::Ok,
            ),
            entry(
                4,
                "e2",
                ActivityOrigin::Berth,
                "container.stop",
                "berth-failed",
                Outcome::Failed {
                    reason: "timeout".to_string(),
                },
            ),
            entry(
                5,
                "e1",
                ActivityOrigin::Plugin("coolify".to_string()),
                "deploy.trigger",
                "plugin-call",
                Outcome::Ok,
            ),
        ],
        ..FeedSeed::default()
    })
}

#[test]
fn activity_filters_by_engine() {
    let feed = sample();
    let filters = Filters {
        engine: Some(engine("e2")),
        ..Filters::default()
    };
    assert_eq!(
        filtered(&feed, &filters, &ProjectOf::new()),
        ["berth-failed", "engine-e2"]
    );
}

#[test]
fn activity_filters_by_origin() {
    let feed = sample();
    let by = |origin| Filters {
        origin,
        ..Filters::default()
    };
    let projects = ProjectOf::new();
    assert_eq!(
        filtered(&feed, &by(OriginFilter::Engine), &projects),
        ["engine-e2", "engine-e1"]
    );
    assert_eq!(
        filtered(&feed, &by(OriginFilter::Berth), &projects),
        ["berth-failed", "berth-ok"]
    );
    assert_eq!(
        filtered(&feed, &by(OriginFilter::Plugin), &projects),
        ["plugin-call"]
    );
    assert_eq!(filtered(&feed, &by(OriginFilter::All), &projects).len(), 5);
}

#[test]
fn activity_filters_by_outcome() {
    let feed = sample();
    let by = |outcome| Filters {
        outcome,
        ..Filters::default()
    };
    let projects = ProjectOf::new();
    assert_eq!(
        filtered(&feed, &by(OutcomeFilter::Failed), &projects),
        ["berth-failed"]
    );
    assert_eq!(filtered(&feed, &by(OutcomeFilter::Ok), &projects).len(), 4);
}

#[test]
fn activity_filters_combine() {
    let feed = sample();
    let filters = Filters {
        engine: Some(engine("e1")),
        origin: OriginFilter::Berth,
        outcome: OutcomeFilter::Ok,
        ..Filters::default()
    };
    assert_eq!(filtered(&feed, &filters, &ProjectOf::new()), ["berth-ok"]);
}

#[test]
fn activity_project_filter_resolves_engine_events_through_their_container() {
    let web = berth_core::ContainerId("web-id".to_string());
    let shop = berth_core::ProjectId("shop".to_string());
    let mut projects = ProjectOf::new();
    projects.insert(web.clone(), shop.clone());

    // An engine event names only the container; a Berth action carries its
    // project; a project action names the project itself.
    let mut engine_event = ev(1, "e1", "web-1");
    engine_event.subject = Subject::Container(web);
    let mut berth_action = ev(2, "e1", "api-1");
    berth_action.project = Some(shop.clone());
    let mut project_action = ev(3, "e1", "shop");
    project_action.subject = Subject::Project(shop.clone());
    let other = ev(4, "e1", "unrelated");

    let feed = Feed::from_seed(FeedSeed {
        entries: vec![engine_event, berth_action, project_action, other],
        ..FeedSeed::default()
    });
    let filters = Filters {
        project: Some(shop),
        ..Filters::default()
    };
    assert_eq!(
        filtered(&feed, &filters, &projects),
        ["shop", "api-1", "web-1"]
    );
}

#[test]
fn activity_gap_marker_answers_only_to_the_engine_filter() {
    let gap = svc::entry_for_gap(engine("e1"), at(5));
    assert!(is_gap(&gap));
    let feed = Feed::from_seed(FeedSeed {
        entries: vec![gap, ev(1, "e1", "a")],
        ..FeedSeed::default()
    });
    let projects = ProjectOf::new();
    let failed_berth = Filters {
        origin: OriginFilter::Berth,
        outcome: OutcomeFilter::Failed,
        project: Some(berth_core::ProjectId("p".to_string())),
        ..Filters::default()
    };
    assert_eq!(
        filtered(&feed, &failed_berth, &projects),
        ["e1"],
        "the gap stays visible whatever else is filtered"
    );
    let other_engine = Filters {
        engine: Some(engine("e2")),
        ..Filters::default()
    };
    assert!(filtered(&feed, &other_engine, &projects).is_empty());
}

#[test]
fn activity_filters_know_when_they_are_active() {
    assert!(!Filters::default().is_active());
    assert!(Filters {
        outcome: OutcomeFilter::Failed,
        ..Filters::default()
    }
    .is_active());
}

// ---- row text ----

#[test]
fn activity_time_shows_the_date_only_for_other_days() {
    let local = |d, h, m, s| {
        Local
            .with_ymd_and_hms(2026, 10, d, h, m, s)
            .single()
            .expect("local time")
    };
    let now = local(25, 12, 0, 0);
    assert_eq!(format_time(local(25, 9, 5, 7), now), "09:05:07");
    assert_eq!(format_time(local(24, 23, 59, 1), now), "Oct 24 23:59:01");
}

#[test]
fn activity_action_and_origin_labels() {
    assert_eq!(action_label("container.die"), "die");
    assert_eq!(action_label("container.health_status"), "health status");
    assert_eq!(action_label("image.pull"), "image pull");
    assert_eq!(action_label("project.pull_images"), "project pull images");
    assert_eq!(action_label("oddity"), "oddity");
    assert_eq!(
        action_label("container.exec_start: wget -qO- http://127.0.0.1/"),
        "exec start",
        "the exec command stays out of the column"
    );
    assert_eq!(origin_label(&ActivityOrigin::Engine), "Engine");
    assert_eq!(origin_label(&ActivityOrigin::Berth), "Berth");
    assert_eq!(
        origin_label(&ActivityOrigin::Plugin("coolify".to_string())),
        "Plugin \u{00b7} coolify"
    );
}

#[test]
fn activity_engine_name_prefers_the_engine_then_the_id_tail() {
    let world = berth_engine::MockWorld::bundled();
    assert_eq!(
        engine_name(&world.engines, &engine("colima:default")),
        "default"
    );
    assert_eq!(engine_name(&[], &engine("ssh:hetzner-1")), "hetzner-1");
    assert_eq!(engine_name(&[], &engine("plain")), "plain");
}

// ---- rendered screen ----

thread_local! {
    static SCOPE: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

#[component]
fn Host() -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    use_hook(move || {
        let world = berth_engine::MockWorld::bundled();
        state.engines.set(world.engines.clone());
        state.containers.set(world.containers.clone());
        state.projects.set(world.projects.clone());
        if let Some(id) = SCOPE.with(std::cell::Cell::take) {
            state
                .engine_scope
                .set(crate::state::EngineScope::One(engine(id)));
        }
    });
    rsx! { ActivityScreen {} }
}

fn render_host() -> String {
    let mut dom = VirtualDom::new(Host);
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn activity_scene_live_renders_the_table_with_a_gap_marker_and_a_failure() {
    let html = crate::scenes::render_scene_html("Activity / live / 1440").expect("scene");
    for needle in [
        "Activity",
        "shop-api-worker-1",
        "Time",
        "Outcome",
        "Pause feed",
        "hetzner-1 disconnected. Events during the gap are missing.",
        "The engine did not finish within 30 s",
        "Plugin \u{00b7} coolify",
        "All engines",
    ] {
        assert!(html.contains(needle), "missing {needle:?}");
    }
    assert!(!html.contains("Feed paused"), "live feed shows no banner");
    assert!(!html.contains("No activity yet"));
}

#[test]
fn activity_scene_paused_shows_the_counter_and_keeps_held_rows_out() {
    let html = crate::scenes::render_scene_html("Activity / paused / 1440").expect("scene");
    assert!(html.contains("Feed paused. 23 new events"), "{html}");
    assert!(html.contains("Resume feed"));
    // The 23 held events are the only `health_status` ones with these
    // names, and `shop-api-db-1` appears nowhere else in the history.
    assert!(!html.contains("shop-api-db-1"), "held rows stay hidden");
    assert!(html.contains("shop-api-worker-1"), "history still shown");
}

#[test]
fn activity_screen_starts_on_the_engine_scope_of_the_title_bar() {
    let world = berth_engine::MockWorld::bundled();
    crate::screens::activity_scene::seed(&world, false);
    SCOPE.with(|s| s.set(Some("colima:default")));
    let html = render_host();
    assert!(html.contains("shop-api-worker-1"));
    assert!(
        !html.contains("landing-app-1"),
        "hetzner events hidden under the default engine scope"
    );
    assert!(!html.contains("crm-api-1"), "work events hidden too");
}

#[test]
fn activity_screen_without_history_says_so() {
    seed_feed(FeedSeed::default());
    let html = render_host();
    assert!(html.contains("No activity yet"), "{html}");
}

// ---- the live path (store -> tap -> feed -> DOM) ----

#[derive(Clone)]
struct LiveProps(Services);

impl PartialEq for LiveProps {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[component]
fn LiveHost(props: LiveProps) -> Element {
    let services = props.0.clone();
    let state = use_app_state();
    use_context_provider(|| state);
    use_context_provider(|| services);
    rsx! { ActivityScreen {} }
}

async fn settle(dom: &mut VirtualDom, until: impl Fn(&str) -> bool) -> String {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let html = dioxus_ssr::render(dom);
        if until(&html) || tokio::time::Instant::now() > deadline {
            return html;
        }
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(100), dom.wait_for_work()).await;
        dom.render_immediate(&mut dioxus::core::NoOpMutations);
    }
}

#[tokio::test]
async fn activity_screen_shows_history_then_entries_recorded_while_open() {
    let store = std::sync::Arc::new(berth_store::Store::open_in_memory().expect("store"));
    svc::record(&store, &ev(0, "colima:default", "from-history"));
    let services = Services {
        store: std::sync::Arc::clone(&store),
        clients: crate::services::engines::client_map(),
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let mut dom = VirtualDom::new_with_props(LiveHost, LiveProps(services));
    let _ = dom.rebuild_to_vec();

    let html = settle(&mut dom, |h| h.contains("from-history")).await;
    assert!(html.contains("from-history"), "history loaded: {html}");

    svc::record(&store, &ev(5, "colima:default", "arrived-live"));
    let html = settle(&mut dom, |h| h.contains("arrived-live")).await;
    assert!(html.contains("arrived-live"), "live entry shown: {html}");
    assert!(html.contains("from-history"));
}
