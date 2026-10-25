//! "Recent activity" under the Projects table (frame "Projects / default /
//! 1440"): the newest few entries of the activity feed, one line each.
//! The screen holds its own feed signal (no `AppState` field): history from
//! the store, then the activity tap, exactly like the Activity screen but
//! trimmed to [`RECENT_MAX`] lines. An entry carries no free text, so the
//! line is a sentence built from its action and subject.

use std::cell::RefCell;

use chrono::{DateTime, Duration, Local, Utc};
use dioxus::prelude::*;

use super::activity::{action_label, engine_name, format_time, is_gap, Feed, FeedSeed};
use crate::services::{activity, Services};
use crate::state::AppState;
use berth_core::{ActivityEntry, ActivityOrigin, Actor, EngineId, Outcome, Subject};

/// Lines shown (the frame shows eight).
pub const RECENT_MAX: usize = 8;

/// Entries read from the store: more than shown, so dropped exec events
/// do not leave the section short.
const LOAD: usize = 4 * RECENT_MAX;

/// How loudly a line is drawn: the frame tints the entries that need a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Danger,
    Warning,
    Neutral,
}

impl Tone {
    fn class(self) -> &'static str {
        match self {
            Self::Danger => "recent-danger",
            Self::Warning => "recent-warning",
            Self::Neutral => "recent-neutral",
        }
    }
}

/// A failed action and an out-of-memory kill are errors; a failing health
/// check and a dropped event stream are warnings. The entry has no exit
/// code (attributes are never stored), so a plain `die` is not tinted.
pub fn tone(entry: &ActivityEntry) -> Tone {
    if matches!(entry.outcome, Outcome::Failed { .. }) {
        return Tone::Danger;
    }
    if is_gap(entry) {
        return Tone::Warning;
    }
    let action = activity::clean_action(&entry.action).unwrap_or_default();
    if action.ends_with(".oom") {
        Tone::Danger
    } else if action.starts_with("container.health_status") && action.ends_with("unhealthy") {
        Tone::Warning
    } else {
        Tone::Neutral
    }
}

/// The sentence for one entry ("shop-api-worker-1 restarted", "Started
/// project berth-site"); a failed action says why.
pub fn message(entry: &ActivityEntry, engines: &[berth_core::Engine]) -> String {
    let name = &entry.subject_name;
    if is_gap(entry) {
        return format!(
            "{} disconnected; events during the gap are missing",
            engine_name(engines, &entry.engine)
        );
    }
    let action = activity::clean_action(&entry.action).unwrap_or_default();
    if let Outcome::Failed { reason } = &entry.outcome {
        return format!("{name} {} failed: {reason}", action_label(&action));
    }
    match action.split_once('.') {
        Some(("container", rest)) => container_sentence(name, rest),
        Some(("project", verb)) => {
            let past = match verb {
                "start" | "up" => "Started",
                "stop" | "down" => "Stopped",
                "restart" => "Restarted",
                "remove" | "delete" => "Removed",
                other => return format!("{} project {name}", capitalise(&other.replace('_', " "))),
            };
            format!("{past} project {name}")
        }
        Some(("image", "pull")) => format!("Pulled {name}"),
        Some(("image", "prune")) => "Cleaned up unused images".to_string(),
        Some(("engine", "clean_up")) => format!("Cleaned up {name}"),
        _ => format!("{} {name}", capitalise(&action_label(&action))),
    }
}

fn container_sentence(name: &str, rest: &str) -> String {
    if let Some(status) = rest.strip_prefix("health_status: ") {
        return match status {
            "unhealthy" => format!("{name} health check failing"),
            "healthy" => format!("{name} is healthy"),
            other => format!("{name} health check {other}"),
        };
    }
    let what = match rest {
        "start" => "started",
        "stop" => "stopped",
        "die" => "exited",
        "kill" => "was killed",
        "oom" => "ran out of memory",
        "restart" => "restarted",
        "create" => "was created",
        "destroy" | "delete" => "was removed",
        "pause" => "paused",
        "unpause" | "resume" => "resumed",
        other => return format!("{name} {}", other.replace('_', " ")),
    };
    format!("{name} {what}")
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// The second column: Berth and plugin entries name their origin; an
/// engine event names the Compose project of its container, else "Engine".
pub fn source(entry: &ActivityEntry, containers: &[berth_core::Container]) -> String {
    match &entry.origin {
        ActivityOrigin::Engine => match &entry.subject {
            Subject::Container(id) => containers
                .iter()
                .find(|c| c.id == *id)
                .and_then(|c| c.compose_project.clone())
                .unwrap_or_else(|| "Engine".to_string()),
            _ => "Engine".to_string(),
        },
        other => super::activity::origin_label(other),
    }
}

/// The lines shown: the newest [`RECENT_MAX`] of the feed.
pub fn recent(feed: &Feed) -> Vec<&ActivityEntry> {
    recent_for(feed, RECENT_MAX, None)
}

/// The newest `max` lines of the feed, of one engine when `engine` is set
/// (the engine dashboard shows its own).
pub fn recent_for<'a>(
    feed: &'a Feed,
    max: usize,
    engine: Option<&EngineId>,
) -> Vec<&'a ActivityEntry> {
    (0..feed.len())
        .filter_map(|i| feed.get(i))
        .filter(|entry| engine.is_none_or(|id| entry.engine == *id))
        .take(max)
        .collect()
}

// ------------------------------------------------------------- scene seed

thread_local! {
    static SEED: RefCell<Option<Vec<ActivityEntry>>> = const { RefCell::new(None) };
}

/// Hand the next mounted section a feed instead of the store (frame scenes
/// and tests, D58). Consumed once, on the UI thread.
pub fn seed_recent(entries: Vec<ActivityEntry>) {
    SEED.with(|slot| *slot.borrow_mut() = Some(entries));
}

fn take_seed() -> Option<Vec<ActivityEntry>> {
    SEED.with(|slot| slot.borrow_mut().take())
}

/// The mock story the Projects frame shows, aged relative to `now`.
pub fn mock_entries(world: &berth_engine::MockWorld, now: DateTime<Utc>) -> Vec<ActivityEntry> {
    let default = EngineId("colima:default".to_string());
    let container = |name: &str| {
        world
            .containers
            .iter()
            .find(|c| c.name == name)
            .map_or_else(
                || Subject::Container(berth_core::ContainerId(name.to_string())),
                |c| Subject::Container(c.id.clone()),
            )
    };
    let project = |name: &str| {
        world
            .projects
            .iter()
            .find(|p| p.name == name)
            .map_or_else(|| berth_core::ProjectId(name.to_string()), |p| p.id.clone())
    };
    let make = |ago_s: i64,
                origin: ActivityOrigin,
                action: &str,
                subject: Subject,
                name: &str,
                outcome: Outcome| ActivityEntry {
        id: uuid::Uuid::now_v7(),
        ts: now - Duration::seconds(ago_s),
        engine: default.clone(),
        actor: Actor::Local,
        origin,
        action: action.to_string(),
        project: match &subject {
            Subject::Project(id) => Some(id.clone()),
            _ => None,
        },
        subject,
        subject_name: name.to_string(),
        outcome,
    };
    let engine = |ago_s, action: &str, container_name: &str| {
        make(
            ago_s,
            ActivityOrigin::Engine,
            action,
            container(container_name),
            container_name,
            Outcome::Ok,
        )
    };
    let project_action = |ago_s, action: &str, name: &str| {
        make(
            ago_s,
            ActivityOrigin::Berth,
            action,
            Subject::Project(project(name)),
            name,
            Outcome::Ok,
        )
    };
    vec![
        engine(
            2,
            "container.health_status: unhealthy",
            "analytics-ingest-1",
        ),
        engine(3, "container.restart", "shop-api-worker-1"),
        engine(14, "container.die", "shop-api-db-1"),
        make(
            26,
            ActivityOrigin::Berth,
            "container.stop",
            container("analytics-grafana-1"),
            "analytics-grafana-1",
            Outcome::Failed {
                reason: "The engine did not finish within 30 s".to_string(),
            },
        ),
        project_action(261, "project.start", "berth-site"),
        project_action(2_484, "project.stop", "docs-site"),
        make(
            9_044,
            ActivityOrigin::Engine,
            "image.pull",
            Subject::Engine(default.clone()),
            "postgres:17-alpine",
            Outcome::Ok,
        ),
        make(
            10_091,
            ActivityOrigin::Berth,
            "image.prune",
            Subject::Engine(default.clone()),
            "default",
            Outcome::Ok,
        ),
    ]
}

// -------------------------------------------------------------- component

/// Newest entries from the store, off the UI thread, merged into the feed.
async fn load_history(store: std::sync::Arc<berth_store::Store>, mut feed: Signal<Feed>) {
    let history = tokio::task::spawn_blocking(move || activity::load_recent(&store, LOAD))
        .await
        .unwrap_or_default();
    let history = history
        .into_iter()
        .filter(|e| activity::clean_action(&e.action).is_some());
    feed.write().extend(history);
}

async fn follow(services: Services, mut feed: Signal<Feed>) {
    use tokio::sync::broadcast::error::RecvError;
    // Subscribe first so nothing falls between history and live.
    let mut rx = activity::subscribe();
    load_history(std::sync::Arc::clone(&services.store), feed).await;
    loop {
        match rx.recv().await {
            Ok(entry) => {
                if activity::clean_action(&entry.action).is_some() {
                    feed.write().push(entry);
                }
            }
            Err(RecvError::Lagged(_)) => {
                load_history(std::sync::Arc::clone(&services.store), feed).await;
            }
            Err(RecvError::Closed) => return,
        }
    }
}

#[component]
pub fn RecentActivity(
    /// How many lines to show.
    #[props(default = RECENT_MAX)]
    max: usize,
    /// Only this engine's entries.
    #[props(default)]
    engine: Option<EngineId>,
) -> Element {
    let state = use_context::<AppState>();
    let services = try_use_context::<Services>();
    let seed = use_hook(take_seed);
    let seeded = seed.is_some();
    let feed = use_signal(|| {
        seed.clone()
            .map(|entries| {
                Feed::from_seed(FeedSeed {
                    entries,
                    ..FeedSeed::default()
                })
            })
            .unwrap_or_default()
    });
    use_future(move || {
        let services = services.clone();
        async move {
            if let (Some(services), false) = (services, seeded) {
                follow(services, feed).await;
            }
        }
    });

    let feed = feed.read();
    let lines = recent_for(&feed, max, engine.as_ref());
    if lines.is_empty() {
        return rsx! {};
    }
    let engines = state.engines.read();
    let containers = state.containers.read();
    let now = Local::now();
    rsx! {
        div { class: "projects-recent",
            h2 { class: "projects-section-title", "Recent activity" }
            div { class: "recent-list", role: "list",
                for entry in lines {
                    div {
                        key: "{entry.id}",
                        class: "recent-row {tone(entry).class()}",
                        role: "listitem",
                        span { class: "recent-bar", "aria-hidden": "true" }
                        span { class: "recent-time", "{format_time(entry.ts.with_timezone(&Local), now)}" }
                        span { class: "recent-source", "{source(entry, &containers)}" }
                        span { class: "recent-message", "{message(entry, &engines)}" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "recent_activity_tests.rs"]
mod tests;
