//! The update check (D62, P6-T2): once a day Berth asks GitHub for the latest release of its own
//! repository and compares it with the running version. Nothing is downloaded or installed; the
//! result is a line in Settings with a link. The switch in Settings turns the check off.
//!
//! The network is `berth_registry::latest_release` (PLAN 6.8). Everything else here is pure or
//! takes its inputs: [`due`] decides from the last check and the clock, [`check_if_due`] runs
//! one check against any [`Feed`], and [`start`] is the loop the App root starts after the first
//! paint. The time of the last answer persists as `Settings::update_last_checked`, so a relaunch
//! does not ask again within the day.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use dioxus::prelude::*;

use berth_registry::{RegistryError, Release};

use crate::state::AppState;

/// "Once a day" (D62).
#[must_use]
pub fn interval() -> Duration {
    Duration::hours(24)
}

/// The first check waits this long, so it never competes with the first paint and discovery.
pub const STARTUP_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

/// A long-running window looks again this often whether a day has passed.
pub const RECHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// The GitHub API the check talks to; `BERTH_UPDATE_API` points it at a local feed for tests.
pub const DEFAULT_API: &str = "https://api.github.com";

/// What the check knows, as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    /// No check has answered in this run.
    #[default]
    Unknown,
    /// The running version is the latest (or newer).
    UpToDate,
    /// A newer release exists.
    Available { tag: String, url: String },
    /// The last attempt could not reach GitHub.
    Offline,
    /// GitHub answered something Berth could not use.
    Failed(String),
}

/// What the Settings screen reads. Provided as context by `providers`.
#[derive(Clone, Copy)]
pub struct UpdatesView {
    pub status: Signal<UpdateStatus>,
}

/// Create the view's signal; called once from `providers`.
pub fn provide_updates_view() -> UpdatesView {
    use_context_provider(|| UpdatesView {
        status: Signal::new(UpdateStatus::Unknown),
    })
}

/// A source of "the latest release". [`GithubFeed`] in the app, a fake in tests.
#[async_trait::async_trait]
pub trait Feed: Send + Sync {
    async fn latest(&self) -> Result<Option<Release>, RegistryError>;
}

/// The real feed: `berth_registry::latest_release` against the repository in `Cargo.toml`.
pub struct GithubFeed {
    pub api_base: url::Url,
    pub slug: String,
}

impl GithubFeed {
    /// The feed for this build: the workspace `repository`, the API from `BERTH_UPDATE_API` or
    /// GitHub. `None` when the repository is not a GitHub URL (a fork building with another
    /// field has nothing to ask).
    #[must_use]
    pub fn for_this_build() -> Option<Self> {
        let slug = slug_of(env!("CARGO_PKG_REPOSITORY"))?;
        let base = std::env::var("BERTH_UPDATE_API").unwrap_or_else(|_| DEFAULT_API.to_string());
        Some(Self {
            api_base: url::Url::parse(&base).ok()?,
            slug,
        })
    }
}

#[async_trait::async_trait]
impl Feed for GithubFeed {
    async fn latest(&self) -> Result<Option<Release>, RegistryError> {
        berth_registry::latest_release(&self.api_base, &self.slug).await
    }
}

/// "https://github.com/fullstoptech/berth" gives "fullstoptech/berth".
#[must_use]
pub fn slug_of(repository: &str) -> Option<String> {
    let path = repository
        .trim()
        .strip_prefix("https://github.com/")
        .or_else(|| repository.trim().strip_prefix("http://github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = path.split_once('/')?;
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/'))
        .then(|| format!("{owner}/{repo}"))
}

/// Is a check due? On, and either never run, a day or more ago, or recorded in the future (the
/// clock was set back, so the record cannot be trusted).
#[must_use]
pub fn due(enabled: bool, last_checked: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    if !enabled {
        return false;
    }
    match last_checked {
        None => true,
        Some(last) => last > now || now - last >= interval(),
    }
}

/// The running version against a release: newer tag, same or older, or a tag that is not a
/// version.
#[must_use]
pub fn judge(current: &str, release: Option<&Release>) -> UpdateStatus {
    let Some(release) = release else {
        return UpdateStatus::UpToDate;
    };
    let tag = release.tag.trim();
    let latest = semver::Version::parse(tag.trim_start_matches('v'));
    let running = semver::Version::parse(current.trim_start_matches('v'));
    match (latest, running) {
        (Ok(latest), Ok(running)) if latest > running => UpdateStatus::Available {
            tag: tag.to_string(),
            url: release.url.to_string(),
        },
        (Ok(_), Ok(_)) => UpdateStatus::UpToDate,
        _ => UpdateStatus::Failed(format!("The latest release is called {tag:?}")),
    }
}

/// What one check came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub status: UpdateStatus,
    /// Set when GitHub answered (or refused with a rate limit): the time to record as the last
    /// check. `None` after an offline or failed attempt, so the next hour tries again.
    pub checked_at: Option<DateTime<Utc>>,
}

/// Run a check when one is due; `None` when it is not (off, or done within the day).
pub async fn check_if_due(
    feed: &dyn Feed,
    current: &str,
    enabled: bool,
    last_checked: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<Outcome> {
    if !due(enabled, last_checked, now) {
        return None;
    }
    Some(match feed.latest().await {
        Ok(release) => Outcome {
            status: judge(current, release.as_ref()),
            checked_at: Some(now),
        },
        Err(RegistryError::Offline) => Outcome {
            status: UpdateStatus::Offline,
            checked_at: None,
        },
        // GitHub said stop: do not ask again until tomorrow.
        Err(RegistryError::RateLimited { .. }) => Outcome {
            status: UpdateStatus::Failed("GitHub's rate limit is used up".to_string()),
            checked_at: Some(now),
        },
        Err(other) => Outcome {
            status: UpdateStatus::Failed(other.to_string()),
            checked_at: None,
        },
    })
}

/// The version of this build.
#[must_use]
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Start the loop (call inside a hook, after the first render): wait, then check when due, and
/// look again every hour. The answer goes to the view and, when GitHub answered, the time to
/// `Settings::update_last_checked`, which the settings saver persists. The request runs on a
/// tokio task and stops with `cancel`.
pub fn start(
    mut state: AppState,
    view: UpdatesView,
    feed: Arc<dyn Feed>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let mut status = view.status;
    spawn(async move {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(STARTUP_DELAY) => {}
        }
        loop {
            let (enabled, last) = {
                let settings = state.settings.peek();
                (settings.update_check, settings.update_last_checked)
            };
            let task_feed = Arc::clone(&feed);
            let task_cancel = cancel.clone();
            let outcome = tokio::spawn(async move {
                tokio::select! {
                    () = task_cancel.cancelled() => None,
                    outcome = check_if_due(task_feed.as_ref(), current_version(), enabled, last, Utc::now()) => outcome,
                }
            })
            .await
            .ok()
            .flatten();
            if let Some(outcome) = outcome {
                status.set(outcome.status);
                if let Some(at) = outcome.checked_at {
                    state.settings.write().update_last_checked = Some(at);
                }
            }
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(RECHECK_EVERY) => {}
            }
        }
    });
}

#[cfg(test)]
#[path = "updates_tests.rs"]
mod tests;
