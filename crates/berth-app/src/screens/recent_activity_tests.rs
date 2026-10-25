use super::*;

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn entry(action: &str, name: &str, outcome: Outcome) -> ActivityEntry {
    ActivityEntry {
        id: uuid::Uuid::now_v7(),
        ts: Utc::now(),
        engine: EngineId("colima:default".to_string()),
        actor: Actor::Local,
        origin: ActivityOrigin::Engine,
        action: action.to_string(),
        subject: Subject::Container(berth_core::ContainerId("c1".to_string())),
        subject_name: name.to_string(),
        project: None,
        outcome,
    }
}

fn msg(action: &str, name: &str) -> String {
    message(&entry(action, name, Outcome::Ok), &[])
}

#[test]
fn recent_activity_sentences_read_like_the_frame() {
    assert_eq!(
        msg("container.health_status: unhealthy", "analytics-ingest-1"),
        "analytics-ingest-1 health check failing"
    );
    assert_eq!(
        msg("container.health_status: healthy", "web-1"),
        "web-1 is healthy"
    );
    assert_eq!(msg("container.die", "db-1"), "db-1 exited");
    assert_eq!(msg("container.restart", "w-1"), "w-1 restarted");
    assert_eq!(msg("container.oom", "w-1"), "w-1 ran out of memory");
    assert_eq!(
        msg("project.start", "berth-site"),
        "Started project berth-site"
    );
    assert_eq!(
        msg("project.stop", "docs-site"),
        "Stopped project docs-site"
    );
    assert_eq!(
        msg("image.pull", "postgres:17-alpine"),
        "Pulled postgres:17-alpine"
    );
    assert_eq!(msg("image.prune", "default"), "Cleaned up unused images");
    // An action the table does not know still reads as words, never a code.
    assert_eq!(msg("container.top_secret", "c"), "c top secret");
    assert_eq!(msg("volume.create", "data"), "Volume create data");
}

#[test]
fn recent_activity_a_failed_action_says_why_and_a_gap_names_the_engine() {
    let failed = entry(
        "container.stop",
        "grafana-1",
        Outcome::Failed {
            reason: "The engine did not finish within 30 s".to_string(),
        },
    );
    assert_eq!(
        message(&failed, &[]),
        "grafana-1 stop failed: The engine did not finish within 30 s"
    );
    let gap = activity::entry_for_gap(EngineId("ssh:hetzner-1".to_string()), Utc::now());
    assert_eq!(
        message(&gap, &[]),
        "hetzner-1 disconnected; events during the gap are missing"
    );
}

#[test]
fn recent_activity_tone_tints_only_what_needs_a_look() {
    let tone_of = |action: &str, outcome: Outcome| tone(&entry(action, "c", outcome));
    assert_eq!(
        tone_of(
            "container.stop",
            Outcome::Failed {
                reason: "x".to_string()
            }
        ),
        Tone::Danger
    );
    assert_eq!(tone_of("container.oom", Outcome::Ok), Tone::Danger);
    assert_eq!(
        tone_of("container.health_status: unhealthy", Outcome::Ok),
        Tone::Warning
    );
    assert_eq!(
        tone_of("container.health_status: healthy", Outcome::Ok),
        Tone::Neutral
    );
    assert_eq!(tone_of("container.die", Outcome::Ok), Tone::Neutral);
    let gap = activity::entry_for_gap(EngineId("ssh:hetzner-1".to_string()), Utc::now());
    assert_eq!(tone(&gap), Tone::Warning);
}

#[test]
fn recent_activity_source_names_the_project_of_an_engine_event() {
    let world = world();
    let worker = world
        .containers
        .iter()
        .find(|c| c.name == "shop-api-worker-1")
        .expect("worker");
    let mut event = entry("container.die", &worker.name, Outcome::Ok);
    event.subject = Subject::Container(worker.id.clone());
    assert_eq!(source(&event, &world.containers), "shop-api");
    // A container nobody lists, or a non-container subject: just Engine.
    assert_eq!(
        source(
            &entry("container.die", "ghost", Outcome::Ok),
            &world.containers
        ),
        "Engine"
    );
    // Berth and plugin entries name their origin.
    let mut berth = event.clone();
    berth.origin = ActivityOrigin::Berth;
    assert_eq!(source(&berth, &world.containers), "Berth");
    berth.origin = ActivityOrigin::Plugin("coolify".to_string());
    assert_eq!(source(&berth, &world.containers), "Plugin \u{00b7} coolify");
}

#[test]
fn recent_activity_shows_the_newest_eight_whatever_the_feed_holds() {
    let now = Utc::now();
    let entries: Vec<ActivityEntry> = (0..12)
        .map(|i| {
            let mut e = entry("container.start", &format!("c{i}"), Outcome::Ok);
            e.ts = now - Duration::seconds(i);
            e
        })
        .collect();
    let feed = Feed::from_seed(FeedSeed {
        entries,
        ..FeedSeed::default()
    });
    let names: Vec<&str> = recent(&feed)
        .iter()
        .map(|e| e.subject_name.as_str())
        .collect();
    assert_eq!(names, ["c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7"]);
    assert!(recent(&Feed::default()).is_empty());
}

#[test]
fn recent_activity_history_comes_from_the_store_without_exec_commands() {
    let store = berth_store::Store::open_in_memory().expect("store");
    for (i, action) in [
        "container.start",
        "container.exec_start: psql -U x",
        "container.die",
    ]
    .into_iter()
    .enumerate()
    {
        let mut e = entry(action, &format!("c{i}"), Outcome::Ok);
        e.ts = Utc::now() - Duration::seconds(10 - i as i64);
        store.activity_push(&e).expect("push");
    }
    let loaded = activity::load_recent(&store, LOAD);
    assert_eq!(loaded.len(), 3);
    let feed = Feed::from_seed(FeedSeed {
        entries: loaded
            .into_iter()
            .filter(|e| activity::clean_action(&e.action).is_some())
            .collect(),
        ..FeedSeed::default()
    });
    let shown: Vec<&str> = recent(&feed).iter().map(|e| e.action.as_str()).collect();
    assert_eq!(shown, ["container.die", "container.start"]);
}

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

#[test]
fn recent_activity_scene_lists_eight_lines_under_needs_attention() {
    let html = strip_style(
        &crate::scenes::render_scene_html("Projects / default / 1440").expect("scene exists"),
    );
    let attention = html.find("Needs attention").expect("needs attention");
    let recent = html.find("Recent activity").expect("recent activity");
    assert!(attention < recent, "{html}");
    assert_eq!(html.matches("class=\"recent-row ").count(), 8, "{html}");
    // Newest first; engine events carry their project, Berth its name.
    let lines = [
        "analytics-ingest-1 health check failing",
        "shop-api-worker-1 restarted",
        "shop-api-db-1 exited",
        "analytics-grafana-1 stop failed: The engine did not finish within 30 s",
        "Started project berth-site",
        "Stopped project docs-site",
        "Pulled postgres:17-alpine",
        "Cleaned up unused images",
    ];
    let mut at = recent;
    for line in lines {
        let found = html[at..]
            .find(line)
            .unwrap_or_else(|| panic!("{line} out of order or missing"));
        at += found + 1;
    }
    assert!(html.contains("recent-row recent-warning"), "{html}");
    assert!(html.contains("recent-row recent-danger"), "{html}");
    assert!(html.contains("class=\"recent-source\">shop-api<"), "{html}");
    assert!(html.contains("class=\"recent-source\">Berth<"), "{html}");
}

#[test]
fn recent_activity_is_not_drawn_on_the_empty_projects_screen() {
    // The empty scene has no projects, so no table and no activity section;
    // a Rows screen with an empty feed also draws nothing.
    let html = strip_style(
        &crate::scenes::render_scene_html("Projects / empty / 1440").expect("scene exists"),
    );
    assert!(!html.contains("Recent activity"), "{html}");
}
