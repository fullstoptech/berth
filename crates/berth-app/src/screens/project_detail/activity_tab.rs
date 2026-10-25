//! The Activity tab of Project detail (P5-T6): the activity feed filtered to
//! this project, newest first, one sentence per entry (the same sentences as
//! "Recent activity" on the Projects screen).
//!
//! A new entry goes to the top, so the list holds still while the pointer is
//! over it: entries that arrive meanwhile are kept and appear when it leaves
//! (AD31, the same rule as the Activity screen's Pause).

use chrono::Local;
use dioxus::prelude::*;

use berth_core::ProjectId;

use crate::screens::activity::{format_time, project_map, Feed, FeedSeed, Filters};
use crate::screens::recent_activity::{message, source, tone, Tone};
use crate::services::{activity, Services};
use crate::state::AppState;
use crate::ui::EmptyState;

/// Entries read from the store.
const LOAD: usize = 400;
/// Entries drawn.
const SHOWN: usize = 200;

fn tone_class(tone: Tone) -> &'static str {
    match tone {
        Tone::Danger => "pd-act-danger",
        Tone::Warning => "pd-act-warning",
        Tone::Neutral => "pd-act-neutral",
    }
}

async fn load_history(store: std::sync::Arc<berth_store::Store>, mut feed: Signal<Feed>) {
    let history = tokio::task::spawn_blocking(move || activity::load_recent(&store, LOAD))
        .await
        .unwrap_or_default();
    let history = history
        .into_iter()
        .filter(|e| activity::clean_action(&e.action).is_some());
    feed.write().extend(history);
}

async fn follow(services: Services, mut feed: Signal<Feed>, mut loaded: Signal<bool>) {
    use tokio::sync::broadcast::error::RecvError;
    // Subscribe first so nothing falls between history and live.
    let mut rx = activity::subscribe();
    load_history(std::sync::Arc::clone(&services.store), feed).await;
    loaded.set(true);
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
pub fn ActivityTab(id: ProjectId, seed: Option<Vec<berth_core::ActivityEntry>>) -> Element {
    let state = use_context::<AppState>();
    let services = try_use_context::<Services>();
    let seeded = seed.is_some();
    let mut feed = use_signal(|| {
        seed.clone()
            .map(|entries| {
                Feed::from_seed(FeedSeed {
                    entries,
                    ..FeedSeed::default()
                })
            })
            .unwrap_or_default()
    });
    let mut loaded = use_signal(|| seeded);
    use_future(move || {
        let services = services.clone();
        async move {
            match services {
                Some(services) if !seeded => follow(services, feed, loaded).await,
                _ => loaded.set(true),
            }
        }
    });

    // Engine events name a container, not a project: the owner map is the
    // containers' own links plus the ones the project lists as its services'.
    let mut projects = project_map(&state.containers.read());
    if let Some(project) = state.projects.read().iter().find(|p| p.id == id) {
        for container in project.services.iter().flat_map(|s| s.containers.iter()) {
            projects.insert(container.clone(), id.clone());
        }
    }
    let filters = Filters {
        project: Some(id),
        ..Filters::default()
    };
    let feed_read = feed.read();
    let rows: Vec<usize> = feed_read
        .visible(&filters, &projects)
        .into_iter()
        .take(SHOWN)
        .collect();
    if rows.is_empty() {
        return rsx! {
            div { class: "pd-empty",
                if *loaded.read() {
                    EmptyState {
                        title: "No activity for this project yet".to_string(),
                        body: "What you do here and what the engine reports about it appears in this list, kept for 30 days.".to_string(),
                        icon: "activity",
                    }
                }
            }
        };
    }
    let engines = state.engines.read();
    let containers = state.containers.read();
    let now = Local::now();
    rsx! {
        div {
            class: "pd-activity",
            role: "list",
            "aria-label": "Activity of this project",
            onmouseenter: move |_| feed.write().pause(),
            onmouseleave: move |_| {
                feed.write().resume();
            },
            for index in rows {
                if let Some(entry) = feed_read.get(index) {
                    div {
                        key: "{entry.id}",
                        class: "pd-act {tone_class(tone(entry))}",
                        role: "listitem",
                        span { class: "pd-act-bar", "aria-hidden": "true" }
                        span { class: "pd-act-time", "{format_time(entry.ts.with_timezone(&Local), now)}" }
                        span { class: "pd-act-source", "{source(entry, &containers)}" }
                        span { class: "pd-act-message", "{message(entry, &engines)}" }
                    }
                }
            }
        }
    }
}
