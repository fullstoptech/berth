//! The Find images service (P4-T3, PLAN §6.8 and §6.17, D23): what the
//! "Find images" tab reads, and the I/O behind its search field and its tag
//! list. The UI reads the store; this file owns every Docker Hub call.
//!
//! Search goes through [`berth_registry::HubClient`] only, behind the small
//! [`HubApi`] trait so a test or `BERTH_MOCK=1` can answer without a network.
//! "Did you mean" and the cached results shown while Docker Hub limits
//! anonymous searches live here, app-side: `berth-registry` keeps neither.
//! Pulls are not here: they run through `ImageStore::pull`, which keeps the
//! per-layer progress (the rows and the panel of the tab read it).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_core::{Arch, Container, Project};
use berth_registry::{ImageHit, Page, Publisher, RegistryError, TagInfo};

use crate::services::images::io;
use crate::services::Services;

/// Docker Hub's API host.
pub const HUB_URL: &str = "https://hub.docker.com";
/// How long typing waits before a search goes out.
pub const DEBOUNCE: Duration = Duration::from_millis(300);
/// Hits per search; the tab shows them all.
pub const PAGE_SIZE: u32 = 25;
/// Tags the detail panel lists.
pub const TAG_PAGE_SIZE: u32 = 12;

// ---------------------------------------------------------------------
// The backend
// ---------------------------------------------------------------------

/// What the tab asks of Docker Hub.
#[async_trait::async_trait]
pub trait HubApi: Send + Sync {
    async fn search(&self, query: &str, page_size: u32) -> Result<Page<ImageHit>, RegistryError>;
    async fn tags(&self, repo: &str, page_size: u32) -> Result<Page<TagInfo>, RegistryError>;
}

/// The real Docker Hub.
pub struct LiveHub(berth_registry::HubClient);

impl LiveHub {
    /// `base` is `https://hub.docker.com`.
    #[must_use]
    pub fn new(base: url::Url) -> Self {
        Self(berth_registry::HubClient::new(base))
    }
}

/// Stands in when the Hub address does not parse (it always does).
struct Unavailable;

#[async_trait::async_trait]
impl HubApi for Unavailable {
    async fn search(&self, _: &str, _: u32) -> Result<Page<ImageHit>, RegistryError> {
        Err(RegistryError::Http(
            "Docker Hub address is invalid".to_string(),
        ))
    }
    async fn tags(&self, _: &str, _: u32) -> Result<Page<TagInfo>, RegistryError> {
        Err(RegistryError::Http(
            "Docker Hub address is invalid".to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl HubApi for LiveHub {
    async fn search(&self, query: &str, page_size: u32) -> Result<Page<ImageHit>, RegistryError> {
        self.0.search(query, page_size).await
    }
    async fn tags(&self, repo: &str, page_size: u32) -> Result<Page<TagInfo>, RegistryError> {
        self.0.tags(repo, page_size).await
    }
}

/// `BERTH_MOCK=1` and scenes: the frames' five postgres images, no network.
pub struct MockHub;

#[async_trait::async_trait]
impl HubApi for MockHub {
    async fn search(&self, query: &str, _page_size: u32) -> Result<Page<ImageHit>, RegistryError> {
        let needle = query.trim().to_lowercase();
        let items = mock_hits(Utc::now())
            .into_iter()
            .filter(|h| h.name.to_lowercase().contains(&needle))
            .collect();
        Ok(Page { items, next: None })
    }
    async fn tags(&self, repo: &str, _page_size: u32) -> Result<Page<TagInfo>, RegistryError> {
        let items = mock_tags(repo, Utc::now());
        if items.is_empty() {
            return Err(RegistryError::NotFound);
        }
        Ok(Page { items, next: None })
    }
}

/// The backend the app uses: the real Hub, or [`MockHub`] under
/// `BERTH_MOCK=1` and `BERTH_SCENE`.
#[must_use]
pub fn default_backend() -> Arc<dyn HubApi> {
    static LIVE: std::sync::OnceLock<Arc<dyn HubApi>> = std::sync::OnceLock::new();
    let mock = std::env::var("BERTH_MOCK").is_ok_and(|v| v == "1")
        || std::env::var_os("BERTH_SCENE").is_some();
    if mock {
        return Arc::new(MockHub);
    }
    Arc::clone(LIVE.get_or_init(|| match url::Url::parse(HUB_URL) {
        Ok(base) => Arc::new(LiveHub::new(base)),
        Err(_) => Arc::new(Unavailable),
    }))
}

/// The five hits "Images / find / 1440" draws.
#[must_use]
pub fn mock_hits(now: DateTime<Utc>) -> Vec<ImageHit> {
    let days = |n: i64| Some(now - chrono::Duration::days(n));
    let hit = |name: &str,
               publisher: Publisher,
               stars: u32,
               pulls: u64,
               updated: Option<DateTime<Utc>>,
               description: &str,
               arm64: bool| ImageHit {
        name: name.to_string(),
        publisher,
        stars,
        pulls,
        updated,
        description: description.to_string(),
        arm64: Some(arm64),
    };
    vec![
        hit(
            "postgres",
            Publisher::Official,
            14_000,
            1_000_000_000,
            days(2),
            "The PostgreSQL object-relational database system provides reliability and data integrity.",
            true,
        ),
        hit(
            "bitnami/postgresql",
            Publisher::Verified,
            850,
            500_000_000,
            days(1),
            "Bitnami container image for PostgreSQL",
            true,
        ),
        hit(
            "postgis/postgis",
            Publisher::Community,
            1_100,
            100_000_000,
            days(6),
            "PostgreSQL with the PostGIS extension",
            false,
        ),
        hit(
            "timescale/timescaledb",
            Publisher::Verified,
            1_300,
            100_000_000,
            days(3),
            "PostgreSQL for time series, built for performance",
            true,
        ),
        hit(
            "supabase/postgres",
            Publisher::Verified,
            180,
            50_000_000,
            days(5),
            "Unmodified Postgres with some useful plugins",
            true,
        ),
    ]
}

/// The four tags of the frames for `postgres`; none for another repository.
#[must_use]
pub fn mock_tags(repo: &str, now: DateTime<Utc>) -> Vec<TagInfo> {
    if repo != "postgres" {
        return Vec::new();
    }
    let tag = |name: &str, arm: u64, amd: u64, days: i64| TagInfo {
        name: name.to_string(),
        digest: None,
        pushed: Some(now - chrono::Duration::days(days)),
        per_arch: vec![
            (Arch::Aarch64, ByteSize::mb(arm)),
            (Arch::X86_64, ByteSize::mb(amd)),
        ],
    };
    vec![
        tag("17-alpine", 105, 107, 2),
        tag("17", 157, 160, 2),
        tag("16-alpine", 97, 99, 9),
        tag("16", 151, 154, 9),
    ]
}

// ---------------------------------------------------------------------
// Words and numbers (pure)
// ---------------------------------------------------------------------

/// Stars as the frame writes them: `850`, `1.1k`, `14k`, `1.2M`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn count_label(n: u64) -> String {
    let (value, unit) = if n >= 1_000_000 {
        (n as f64 / 1_000_000.0, "M")
    } else if n >= 1_000 {
        (n as f64 / 1_000.0, "k")
    } else {
        return n.to_string();
    };
    // One digit under 10, none from 10 up; a trailing ".0" goes.
    let text = if value >= 10.0 {
        format!("{value:.0}")
    } else {
        let one = format!("{value:.1}");
        one.strip_suffix(".0").unwrap_or(&one).to_string()
    };
    format!("{text}{unit}")
}

/// Pulls as Docker Hub shows them, a lower bound: `1B+`, `500M+`, `10K+`.
#[must_use]
pub fn pulls_label(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{}B+", n / 1_000_000_000)
    } else if n >= 1_000_000 {
        format!("{}M+", n / 1_000_000)
    } else if n >= 1_000 {
        format!("{}K+", n / 1_000)
    } else {
        n.to_string()
    }
}

/// The publisher column's words.
#[must_use]
pub fn publisher_label(publisher: Publisher) -> &'static str {
    match publisher {
        Publisher::Official => "Docker Official Image",
        Publisher::Verified => "Verified Publisher",
        Publisher::SponsoredOss => "Sponsored OSS",
        Publisher::Community => "Community",
    }
}

/// The image has no arm64 build: it runs under emulation on this Mac.
#[must_use]
pub fn amd64_only(hit: &ImageHit) -> bool {
    hit.arm64 == Some(false)
}

/// The amber note of a row and of the detail panel.
pub const AMD64_ONLY_ROW: &str = "amd64 only \u{b7} emulated";
pub const AMD64_ONLY_NOTE: &str = "amd64 only \u{b7} runs under emulation";

/// How long ago, as the frames write it: "just now", "6 min ago",
/// "3 hours ago", "2 days ago", "4 months ago".
#[must_use]
pub fn ago(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = now.signed_duration_since(at).num_seconds().max(0);
    let plural = |n: i64, unit: &str| {
        if n == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{n} {unit}s ago")
        }
    };
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => plural(secs / 3600, "hour"),
        86_400..=2_591_999 => plural(secs / 86_400, "day"),
        2_592_000..=31_535_999 => plural(secs / 2_592_000, "month"),
        _ => plural(secs / 31_536_000, "year"),
    }
}

/// "6 min ago" for a cached answer.
#[must_use]
pub fn cached_ago(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    ago(at, now)
}

/// The `docker pull` line the panel shows and Copy copies. The platform
/// appears only when it is not the engine's own.
#[must_use]
pub fn pull_command(reference: &str, platform: Option<&str>) -> String {
    match platform {
        Some(p) if !p.is_empty() => format!("docker pull --platform {p} {reference}"),
        _ => format!("docker pull {reference}"),
    }
}

/// `postgres` + `17-alpine` is `postgres:17-alpine`.
#[must_use]
pub fn reference(name: &str, tag: &str) -> String {
    format!("{name}:{tag}")
}

/// The Hub name a pull reference belongs to: `postgres:17-alpine` and
/// `docker.io/library/postgres:17` are `postgres`.
#[must_use]
pub fn repo_of_reference(reference: &str) -> String {
    let no_digest = reference.split('@').next().unwrap_or(reference);
    let last_slash = no_digest.rfind('/').map_or(0, |i| i + 1);
    let repo = match no_digest[last_slash..].find(':') {
        Some(i) => &no_digest[..last_slash + i],
        None => no_digest,
    };
    let repo = repo.strip_prefix("docker.io/").unwrap_or(repo);
    repo.strip_prefix("library/").unwrap_or(repo).to_string()
}

/// The `--platform` value of an engine architecture ("linux/arm64").
#[must_use]
pub fn platform_of(arch: &Arch) -> String {
    format!("linux/{}", arch.docker_name())
}

/// The platform choices the panel offers: the engine's own first.
#[must_use]
pub fn platform_options(engine: &Arch) -> Vec<String> {
    let own = platform_of(engine);
    let mut options = vec![own];
    for other in ["linux/arm64", "linux/amd64"] {
        if !options.iter().any(|o| o == other) {
            options.push(other.to_string());
        }
    }
    options
}

/// The architecture a `linux/arm64` style platform names.
#[must_use]
pub fn arch_of_platform(platform: &str) -> Arch {
    Arch::parse(platform.rsplit('/').next().unwrap_or(platform))
}

/// The Download column: the size of the tag for `platform`, else the first
/// architecture it has.
#[must_use]
pub fn download_size(tag: &TagInfo, platform: &str) -> Option<ByteSize> {
    let wanted = arch_of_platform(platform);
    tag.per_arch
        .iter()
        .find(|(arch, _)| *arch == wanted)
        .or_else(|| tag.per_arch.first())
        .map(|(_, size)| *size)
}

/// The Platforms column: "arm64 · amd64".
#[must_use]
pub fn platforms_label(tag: &TagInfo) -> String {
    tag.per_arch
        .iter()
        .map(|(arch, _)| arch.docker_name().to_string())
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// Does the tag have an image for `platform`?
#[must_use]
pub fn tag_has_platform(tag: &TagInfo, platform: &str) -> bool {
    let wanted = arch_of_platform(platform);
    tag.per_arch.iter().any(|(arch, _)| *arch == wanted)
}

// ---------------------------------------------------------------------
// Results (pure)
// ---------------------------------------------------------------------

/// An exact name match goes first; the rest keep Docker Hub's order.
#[must_use]
pub fn rank(mut hits: Vec<ImageHit>, query: &str) -> Vec<ImageHit> {
    let query = query.trim().to_lowercase();
    // `sort_by_key` is stable: false sorts before true.
    hits.sort_by_key(|h| h.name.to_lowercase() != query);
    hits
}

/// The hits the two switches leave: Official only keeps official images,
/// arm64 only drops what has no arm64 build (an unknown stays).
#[must_use]
pub fn visible_hits(hits: &[ImageHit], official_only: bool, arm64_only: bool) -> Vec<ImageHit> {
    hits.iter()
        .filter(|h| !official_only || h.publisher == Publisher::Official)
        .filter(|h| !arm64_only || h.arm64 != Some(false))
        .cloned()
        .collect()
}

/// Official images people look for most; "Did you mean" also reads the
/// images the engine has and the ones the projects use.
pub const POPULAR: &[&str] = &[
    "alpine",
    "busybox",
    "caddy",
    "composer",
    "debian",
    "elasticsearch",
    "golang",
    "grafana/grafana",
    "httpd",
    "influxdb",
    "mariadb",
    "memcached",
    "minio/minio",
    "mongo",
    "mysql",
    "nats",
    "nginx",
    "node",
    "php",
    "postgres",
    "prom/prometheus",
    "python",
    "rabbitmq",
    "redis",
    "ruby",
    "rust",
    "traefik",
    "ubuntu",
    "vault",
    "wordpress",
];

/// The Levenshtein distance of two short strings.
#[must_use]
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ca != cb);
            current.push(substitute.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

/// The nearest known name to a query that found nothing, when one is close
/// (two edits, three from eight letters) and is not the query itself.
#[must_use]
pub fn did_you_mean(query: &str, candidates: &[String]) -> Option<String> {
    let query = query.trim().to_lowercase();
    if query.chars().count() < 3 {
        return None;
    }
    let limit = if query.chars().count() >= 8 { 3 } else { 2 };
    candidates
        .iter()
        .filter_map(|name| {
            // Compare with the repository part too: "nginx" for "ngnix",
            // not only "library/nginx".
            let lower = name.to_lowercase();
            let short = lower.rsplit('/').next().unwrap_or(&lower).to_string();
            let d = edit_distance(&query, &lower).min(edit_distance(&query, &short));
            (1..=limit).contains(&d).then_some((d, name))
        })
        .min_by_key(|(d, _)| *d)
        .map(|(_, name)| name.clone())
}

/// Names "Did you mean" may offer: the popular list, the images on the
/// engine and the suggestions.
#[must_use]
pub fn name_candidates(local_repos: &[String], suggestions: &[Suggestion]) -> Vec<String> {
    let mut names: Vec<String> = POPULAR.iter().map(|s| (*s).to_string()).collect();
    for repo in local_repos
        .iter()
        .cloned()
        .chain(suggestions.iter().map(|s| s.name.clone()))
    {
        if !names.contains(&repo) {
            names.push(repo);
        }
    }
    names
}

/// A Hub name for a local image repository: `library/` goes, and an image
/// of another registry (`ghcr.io/…`) is not on Docker Hub.
#[must_use]
pub fn hub_name(repo: &str) -> Option<String> {
    let repo = repo.strip_prefix("docker.io/").unwrap_or(repo);
    let first = repo.split('/').next().unwrap_or(repo);
    if repo.contains('/') && (first.contains('.') || first.contains(':')) {
        return None;
    }
    let name = repo.strip_prefix("library/").unwrap_or(repo);
    (!name.is_empty() && !name.starts_with("sha256")).then(|| name.to_string())
}

/// An image pulled before, from the activity feed.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentPull {
    pub reference: String,
    pub at: DateTime<Utc>,
}

/// One line of the no-query state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// What a click searches for.
    pub name: String,
    /// "Used by shop-api", "Pulled 2 days ago".
    pub reason: String,
}

/// What to offer before anything is typed: images the containers of known
/// projects run, then images pulled lately. An image built from the
/// project's own Dockerfile (`<project>-<service>`) is not on Docker Hub.
#[must_use]
pub fn suggestions(
    containers: &[Container],
    projects: &[Project],
    recent: &[RecentPull],
    now: DateTime<Utc>,
) -> Vec<Suggestion> {
    const LIMIT: usize = 8;
    let mut out: Vec<Suggestion> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for container in containers {
        let Some(project_id) = &container.project else {
            continue;
        };
        let Some(name) = hub_name(container.image.repo()) else {
            continue;
        };
        let built_here = container
            .compose_project
            .as_ref()
            .zip(container.service.as_ref())
            .is_some_and(|(project, service)| {
                [
                    format!("{project}-{service}"),
                    format!("{project}_{service}"),
                ]
                .contains(&name)
            });
        if built_here || seen.contains(&name) {
            continue;
        }
        let project = projects
            .iter()
            .find(|p| p.id == *project_id)
            .map_or_else(|| project_id.0.clone(), |p| p.name.clone());
        seen.push(name.clone());
        out.push(Suggestion {
            name,
            reason: format!("Used by {project}"),
        });
    }
    for pull in recent {
        let Some(name) = hub_name(&repo_of_reference(&pull.reference)) else {
            continue;
        };
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());
        out.push(Suggestion {
            name,
            reason: format!("Pulled {}", ago(pull.at, now)),
        });
    }
    out.truncate(LIMIT);
    out
}

// ---------------------------------------------------------------------
// The search view (pure state)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum SearchStatus {
    /// Nothing typed.
    Idle,
    Searching,
    Done,
    RateLimited {
        reset_in: Option<Duration>,
    },
    Offline,
    Failed(String),
}

/// The answer to one query: what the table draws.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchView {
    /// The query the hits answer.
    pub query: String,
    pub status: SearchStatus,
    pub hits: Vec<ImageHit>,
    /// Set when the hits come from the cache of an earlier search.
    pub cached_at: Option<DateTime<Utc>>,
    pub did_you_mean: Option<String>,
}

impl SearchView {
    #[must_use]
    pub fn idle() -> Self {
        Self {
            query: String::new(),
            status: SearchStatus::Idle,
            hits: Vec::new(),
            cached_at: None,
            did_you_mean: None,
        }
    }
}

impl Default for SearchView {
    fn default() -> Self {
        Self::idle()
    }
}

/// Docker Hub's answer to `query` as the view shows it. A search that
/// failed shows the last good answer of the same query when there is one.
#[must_use]
pub fn answer(
    query: &str,
    result: Result<Page<ImageHit>, RegistryError>,
    cached: Option<Cached<ImageHit>>,
    candidates: &[String],
) -> SearchView {
    let from_cache = |status: SearchStatus| {
        let (hits, cached_at) = match cached.clone() {
            Some((hits, at)) => (hits, Some(at)),
            None => (Vec::new(), None),
        };
        SearchView {
            query: query.to_string(),
            status,
            hits,
            cached_at,
            did_you_mean: None,
        }
    };
    match result {
        Ok(page) => {
            let hits = rank(page.items, query);
            let did_you_mean = hits
                .is_empty()
                .then(|| did_you_mean(query, candidates))
                .flatten();
            SearchView {
                query: query.to_string(),
                status: SearchStatus::Done,
                hits,
                cached_at: None,
                did_you_mean,
            }
        }
        Err(RegistryError::RateLimited { reset_in }) => {
            from_cache(SearchStatus::RateLimited { reset_in })
        }
        Err(RegistryError::Offline) => from_cache(SearchStatus::Offline),
        Err(other) => from_cache(SearchStatus::Failed(other.to_string())),
    }
}

/// What the panel knows about the tags of one repository.
#[derive(Debug, Clone, PartialEq)]
pub enum TagsEntry {
    Loading,
    Ready {
        tags: Vec<TagInfo>,
        /// Set when Docker Hub refused and the tags are from the cache.
        cached_at: Option<DateTime<Utc>>,
    },
    Failed(String),
}

/// An earlier answer and when it came.
pub type Cached<T> = (Vec<T>, DateTime<Utc>);

/// One request's settings.
pub struct SearchRequest {
    pub text: String,
    pub debounce: Duration,
    /// The app's root token; the store derives its own from it.
    pub parent: CancellationToken,
    pub backend: Arc<dyn HubApi>,
    /// Names "Did you mean" may offer.
    pub candidates: Vec<String>,
}

// ---------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------

/// The Find images tab's state and I/O. Provided as context from the App
/// root (and by a scene root); every field is a signal, so the store is
/// `Copy`.
#[derive(Clone, Copy)]
pub struct FindStore {
    pub query: Signal<String>,
    /// Bumped to remount the search field (clear, a suggestion's click).
    pub query_rev: Signal<u32>,
    pub official_only: Signal<bool>,
    pub arm64_only: Signal<bool>,
    pub view: Signal<SearchView>,
    /// The hit whose panel is open: its Hub name.
    pub selected: Signal<Option<String>>,
    /// The tag the panel's Pull button names; the first tag when unset.
    pub selected_tag: Signal<Option<String>>,
    /// The panel's platform; the engine's own when unset.
    pub platform: Signal<Option<String>>,
    pub tags: Signal<HashMap<String, TagsEntry>>,
    pub recent_pulls: Signal<Vec<RecentPull>>,
    recents_loaded: Signal<bool>,
    searches: Signal<HashMap<String, Cached<ImageHit>>>,
    tag_cache: Signal<HashMap<String, Cached<TagInfo>>>,
    generation: Signal<u64>,
    cancel: Signal<Option<CancellationToken>>,
}

impl FindStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            query: Signal::new(String::new()),
            query_rev: Signal::new(0),
            official_only: Signal::new(false),
            arm64_only: Signal::new(false),
            view: Signal::new(SearchView::idle()),
            selected: Signal::new(None),
            selected_tag: Signal::new(None),
            platform: Signal::new(None),
            tags: Signal::new(HashMap::new()),
            recent_pulls: Signal::new(Vec::new()),
            recents_loaded: Signal::new(false),
            searches: Signal::new(HashMap::new()),
            tag_cache: Signal::new(HashMap::new()),
            generation: Signal::new(0),
            cancel: Signal::new(None),
        }
    }

    /// Put an answer in as if it had been fetched (scenes, tests): the
    /// view, the tags of the selected hit, and the cache both fill.
    pub fn seed(&self, view: SearchView, selected: Option<&str>, tags: Vec<(String, TagsEntry)>) {
        let mut query = self.query;
        query.set(view.query.clone());
        let mut searches = self.searches;
        if view.cached_at.is_none() && !view.hits.is_empty() {
            searches
                .write()
                .insert(cache_key(&view.query), (view.hits.clone(), Utc::now()));
        }
        let mut shown = self.view;
        shown.set(view);
        let mut chosen = self.selected;
        chosen.set(selected.map(str::to_string));
        let mut map = self.tags;
        map.write().extend(tags);
    }

    /// Forget the query and its answer; a search on its way is dropped.
    pub fn clear(&self) {
        let mut generation = self.generation;
        let next = *generation.peek() + 1;
        generation.set(next);
        let mut cancel = self.cancel;
        if let Some(previous) = cancel.write().take() {
            previous.cancel();
        }
        let mut query = self.query;
        query.set(String::new());
        let mut view = self.view;
        view.set(SearchView::idle());
    }

    /// Search for `request.text`: the view says "Searching" at once, the
    /// request goes out after the debounce, and an answer to an older text
    /// is dropped. An empty text clears the view.
    pub fn search(&self, request: SearchRequest) {
        let mut generation = self.generation;
        let id = *generation.peek() + 1;
        generation.set(id);
        let mut cancel = self.cancel;
        if let Some(previous) = cancel.write().take() {
            previous.cancel();
        }
        let text = request.text.trim().to_string();
        let mut query = self.query;
        query.set(request.text.clone());
        let mut view = self.view;
        if text.is_empty() {
            self.clear();
            return;
        }
        {
            let mut current = view.write();
            current.query = text.clone();
            current.status = SearchStatus::Searching;
            current.did_you_mean = None;
        }
        let token = request.parent.child_token();
        cancel.set(Some(token.clone()));
        let store = *self;
        let backend = request.backend.clone();
        let debounce = request.debounce;
        let asked = text.clone();
        let candidates = request.candidates;
        io(
            async move {
                if !debounce.is_zero() {
                    tokio::select! {
                        () = token.cancelled() => return None,
                        () = tokio::time::sleep(debounce) => {}
                    }
                }
                tokio::select! {
                    () = token.cancelled() => None,
                    result = backend.search(&asked, PAGE_SIZE) => Some(result),
                }
            },
            move |result| {
                let Some(result) = result else { return };
                if *store.generation.peek() != id {
                    return;
                }
                store.land(
                    &text,
                    result,
                    &candidates,
                    &request.backend,
                    &request.parent,
                );
            },
        );
    }

    /// Fold an answer in: cache a good one, show a cached one for a refusal,
    /// and open the first hit when nothing the user chose is in the list.
    fn land(
        &self,
        text: &str,
        result: Result<Page<ImageHit>, RegistryError>,
        candidates: &[String],
        backend: &Arc<dyn HubApi>,
        parent: &CancellationToken,
    ) {
        let key = cache_key(text);
        let cached = self.searches.peek().get(&key).cloned();
        let view = answer(text, result, cached, candidates);
        if view.status == SearchStatus::Done && !view.hits.is_empty() {
            let mut searches = self.searches;
            searches
                .write()
                .insert(key, (view.hits.clone(), Utc::now()));
        }
        let keep = self
            .selected
            .peek()
            .as_ref()
            .is_some_and(|name| view.hits.iter().any(|h| h.name == *name));
        let first = view.hits.first().map(|h| h.name.clone());
        let mut shown = self.view;
        shown.set(view);
        if !keep {
            let mut selected = self.selected;
            selected.set(first.clone());
            let mut tag = self.selected_tag;
            tag.set(None);
            if let Some(name) = first {
                self.ensure_tags(&name, backend, parent);
            }
        }
    }

    /// Open a hit's panel and list its tags.
    pub fn select(&self, name: &str, backend: &Arc<dyn HubApi>, parent: &CancellationToken) {
        let mut selected = self.selected;
        if selected.peek().as_deref() != Some(name) {
            selected.set(Some(name.to_string()));
            let mut tag = self.selected_tag;
            tag.set(None);
        }
        self.ensure_tags(name, backend, parent);
    }

    /// Ask for a repository's tags unless they are known or on the way.
    pub fn ensure_tags(&self, name: &str, backend: &Arc<dyn HubApi>, parent: &CancellationToken) {
        let mut tags = self.tags;
        if matches!(
            tags.peek().get(name),
            Some(
                TagsEntry::Loading
                    | TagsEntry::Ready {
                        cached_at: None,
                        ..
                    }
            )
        ) {
            return;
        }
        tags.write().insert(name.to_string(), TagsEntry::Loading);
        let cached = self.tag_cache.peek().get(name).cloned();
        let backend = backend.clone();
        let token = parent.child_token();
        let repo = name.to_string();
        let store = *self;
        io(
            async move {
                tokio::select! {
                    () = token.cancelled() => None,
                    result = backend.tags(&repo, TAG_PAGE_SIZE) => Some((repo, result)),
                }
            },
            move |done| {
                let Some((repo, result)) = done else { return };
                let entry = match result {
                    Ok(page) => {
                        let mut cache = store.tag_cache;
                        cache
                            .write()
                            .insert(repo.clone(), (page.items.clone(), Utc::now()));
                        TagsEntry::Ready {
                            tags: page.items,
                            cached_at: None,
                        }
                    }
                    Err(err) => match cached {
                        Some((tags, at)) => TagsEntry::Ready {
                            tags,
                            cached_at: Some(at),
                        },
                        None => TagsEntry::Failed(failure_text(&err)),
                    },
                };
                let mut tags = store.tags;
                tags.write().insert(repo, entry);
            },
        );
    }

    /// Read the activity feed once for the images pulled lately.
    pub fn ensure_recent_pulls(&self, services: &Services) {
        let mut loaded = self.recents_loaded;
        if *loaded.peek() {
            return;
        }
        loaded.set(true);
        let store = services.store.clone();
        let mut recent = self.recent_pulls;
        io(
            async move {
                tokio::task::spawn_blocking(move || recent_pulls(&store))
                    .await
                    .unwrap_or_default()
            },
            move |pulls| recent.set(pulls),
        );
    }
}

impl Default for FindStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A search is cached under its trimmed, lower-case text.
#[must_use]
pub fn cache_key(text: &str) -> String {
    text.trim().to_lowercase()
}

/// What the user reads for a Hub refusal.
#[must_use]
pub fn failure_text(err: &RegistryError) -> String {
    match err {
        RegistryError::RateLimited { .. } => "Docker Hub limits anonymous requests".to_string(),
        RegistryError::Offline => "Docker Hub cannot be reached".to_string(),
        other => other.to_string(),
    }
}

/// "Try again in 2 min" for a refusal that says when the limit resets.
#[must_use]
pub fn reset_text(reset_in: Duration) -> String {
    let secs = reset_in.as_secs();
    if secs < 90 {
        format!("Try again in {} s.", secs.max(1))
    } else {
        format!("Try again in {} min.", secs.div_ceil(60))
    }
}

/// The images pulled lately: successful `image.pull` entries, newest first.
/// Blocking SQLite: run it off the UI thread.
fn recent_pulls(store: &berth_store::Store) -> Vec<RecentPull> {
    crate::services::activity::load_recent(store, 300)
        .into_iter()
        .filter(|e| e.action == "image.pull" && e.outcome == berth_core::Outcome::Ok)
        .map(|e| RecentPull {
            reference: e.subject_name,
            at: e.ts,
        })
        .collect()
}

/// Provide one [`FindStore`] from the App root (or a scene root).
pub fn provide_find_store() -> FindStore {
    use_hook(|| provide_context(FindStore::new()))
}

#[cfg(test)]
#[path = "find_images_tests.rs"]
mod tests;
