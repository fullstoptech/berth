//! Scenes of the Images screen's "Find images" tab (P4-T3): the three frames
//! "Images / find / 1440", "... / pulling / 1440" and "... / rate limited /
//! 1440". A scene runs no service (D58), so each one puts the search answer,
//! the tags and the pull into the stores as the services would have.

use super::*;

use crate::services::find_images::{
    mock_hits, mock_tags, FindStore, SearchStatus, SearchView, TagsEntry,
};
use crate::services::images::{ImageStore, ImagesTab, PullState, PULL_ACTION};

const DEFAULT_ENGINE: &str = "colima:default";

/// The Images section on the Find tab with "postgres" searched and its first
/// hit open, as every one of the frames shows it.
fn find_base(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Images);
    let containers = state.containers.peek().clone();
    state.problems.set(mock_problems(&containers));
    if let Some(images) = try_consume_context::<ImageStore>() {
        let mut tab = images.tab;
        tab.set(ImagesTab::Find);
    }
}

fn seed_search(status: SearchStatus, cached_at: Option<chrono::DateTime<chrono::Utc>>) {
    let Some(find) = try_consume_context::<FindStore>() else {
        return;
    };
    let now = chrono::Utc::now();
    let view = SearchView {
        query: "postgres".to_string(),
        status,
        hits: mock_hits(now),
        cached_at,
        did_you_mean: None,
    };
    let tags = vec![(
        "postgres".to_string(),
        TagsEntry::Ready {
            tags: mock_tags("postgres", now),
            cached_at,
        },
    )];
    find.seed(view, Some("postgres"), tags);
}

fn find_results(state: &mut AppState) {
    find_base(state);
    seed_search(SearchStatus::Done, None);
}

/// Docker Hub refused: the cached answer of six minutes ago, named in the
/// banner, the count and the panel.
fn find_rate_limited(state: &mut AppState) {
    find_base(state);
    let cached = chrono::Utc::now() - chrono::Duration::minutes(6);
    seed_search(SearchStatus::RateLimited { reset_in: None }, Some(cached));
}

/// `postgres:17-alpine` part way: three layers whole, one at 31.1 of 41.5 MB,
/// one waiting; 68 of 104.6 MB, 65 %, 12 s left.
fn find_pulling(state: &mut AppState) {
    use berth_core::PullProgress;
    find_base(state);
    seed_search(SearchStatus::Done, None);
    let Some(images) = try_consume_context::<ImageStore>() else {
        return;
    };
    let event = |layer: &str, status: &str, current: Option<u64>, total: Option<u64>| {
        PullProgress {
            layer: Some(layer.to_string()),
            status: status.to_string(),
            current,
            total,
        }
    };
    let mut pull = PullState::new(
        berth_core::EngineId(DEFAULT_ENGINE.to_string()),
        "postgres:17-alpine",
        None,
    );
    let whole = |id: &str, bytes: u64| {
        [
            event(id, "Pulling fs layer", None, None),
            event(id, "Downloading", Some(bytes), Some(bytes)),
            event(id, "Pull complete", None, None),
        ]
    };
    let mut events: Vec<PullProgress> = Vec::new();
    events.extend(whole("a2318d6c47ec", 3_600_000));
    events.extend(whole("1f8c0ab51b3e", 5_300_000));
    events.extend(whole("5d2b9e1c7a40", 28_000_000));
    events.extend([
        event("e90a1c47d6b2", "Pulling fs layer", None, None),
        event(
            "e90a1c47d6b2",
            "Downloading",
            Some(31_100_000),
            Some(41_500_000),
        ),
        event("7c13f08ea9d5", "Pulling fs layer", None, None),
        event("7c13f08ea9d5", "Downloading", Some(0), Some(26_200_000)),
        event("7c13f08ea9d5", "Waiting", None, None),
    ]);
    for e in &events {
        pull.apply(e);
    }
    pull.eta_secs = Some(12);
    let mut pulls = images.pulls;
    pulls.write().push(pull);
    state.pending.write().insert(
        crate::state::PendingKey {
            subject: berth_core::Subject::Image("postgres:17-alpine".to_string()),
            action: PULL_ACTION,
        },
        crate::state::PendingState::Running {
            label: "Pulling postgres:17-alpine".to_string(),
        },
    );
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Images / find / 1440", find_results),
    Scene::design("Images / find / pulling / 1440", find_pulling),
    Scene::design("Images / find / rate limited / 1440", find_rate_limited),
];
