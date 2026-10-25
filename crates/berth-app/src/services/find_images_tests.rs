//! P4-T3 tests for the Find images service: the words and numbers of a
//! result, ranking and the two switches, "Did you mean", the suggestions
//! before anything is typed, how an answer or a refusal becomes the view
//! (cached results while Docker Hub limits anonymous searches), and the
//! store's debounce, stale-answer and tag paths over a fake Hub.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::*;
use crate::state::{use_app_state, AppState};

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_792_000_000 + secs, 0).unwrap_or_default()
}

fn hit(name: &str, publisher: Publisher, arm64: Option<bool>) -> ImageHit {
    ImageHit {
        name: name.to_string(),
        publisher,
        stars: 10,
        pulls: 1_000,
        updated: None,
        description: String::new(),
        arm64,
    }
}

fn names(hits: &[ImageHit]) -> Vec<&str> {
    hits.iter().map(|h| h.name.as_str()).collect()
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

// ---------------------------------------------------------------------
// Words and numbers
// ---------------------------------------------------------------------

#[test]
fn find_images_stars_read_as_the_frame_writes_them() {
    for (n, want) in [
        (0, "0"),
        (180, "180"),
        (850, "850"),
        (999, "999"),
        (1_000, "1k"),
        (1_100, "1.1k"),
        (1_300, "1.3k"),
        (14_000, "14k"),
        (14_600, "15k"),
        (1_200_000, "1.2M"),
        (25_000_000, "25M"),
    ] {
        assert_eq!(count_label(n), want, "{n}");
    }
}

#[test]
fn find_images_pulls_read_as_docker_hub_lower_bounds() {
    for (n, want) in [
        (50, "50"),
        (12_345, "12K+"),
        (50_000_000, "50M+"),
        (100_000_000, "100M+"),
        (500_000_000, "500M+"),
        (1_000_000_000, "1B+"),
        (2_400_000_000, "2B+"),
    ] {
        assert_eq!(pulls_label(n), want, "{n}");
    }
}

#[test]
fn find_images_publishers_have_the_frames_words() {
    assert_eq!(
        publisher_label(Publisher::Official),
        "Docker Official Image"
    );
    assert_eq!(publisher_label(Publisher::Verified), "Verified Publisher");
    assert_eq!(publisher_label(Publisher::SponsoredOss), "Sponsored OSS");
    assert_eq!(publisher_label(Publisher::Community), "Community");
}

#[test]
fn find_images_ago_uses_the_words_of_the_frames() {
    let now = at(0);
    let back = |secs: i64| ago(now - chrono::Duration::seconds(secs), now);
    assert_eq!(back(5), "just now");
    assert_eq!(back(6 * 60), "6 min ago");
    assert_eq!(back(3600), "1 hour ago");
    assert_eq!(back(3 * 3600), "3 hours ago");
    assert_eq!(back(86_400), "1 day ago");
    assert_eq!(back(2 * 86_400), "2 days ago");
    assert_eq!(back(45 * 86_400), "1 month ago");
    assert_eq!(back(400 * 86_400), "1 year ago");
    // The future is not negative time.
    assert_eq!(ago(now + chrono::Duration::hours(1), now), "just now");
}

#[test]
fn find_images_pull_command_names_a_platform_only_when_it_is_not_the_engines() {
    assert_eq!(
        pull_command("postgres:17-alpine", None),
        "docker pull postgres:17-alpine"
    );
    assert_eq!(
        pull_command("postgres:17-alpine", Some("linux/amd64")),
        "docker pull --platform linux/amd64 postgres:17-alpine"
    );
}

#[test]
fn find_images_repo_of_reference_drops_tag_digest_and_hub_prefixes() {
    for (reference, repo) in [
        ("postgres:17-alpine", "postgres"),
        ("postgres", "postgres"),
        ("docker.io/library/postgres:17", "postgres"),
        ("bitnami/postgresql:16", "bitnami/postgresql"),
        ("localhost:5000/team/app:1", "localhost:5000/team/app"),
        ("postgres@sha256:abcdef", "postgres"),
    ] {
        assert_eq!(repo_of_reference(reference), repo, "{reference}");
    }
}

#[test]
fn find_images_platforms_default_to_the_engines_and_offer_both() {
    assert_eq!(platform_of(&Arch::Aarch64), "linux/arm64");
    assert_eq!(
        platform_options(&Arch::Aarch64),
        strings(&["linux/arm64", "linux/amd64"])
    );
    assert_eq!(
        platform_options(&Arch::X86_64),
        strings(&["linux/amd64", "linux/arm64"])
    );
}

#[test]
fn find_images_a_tags_size_and_platforms_follow_the_chosen_platform() {
    let now = at(0);
    let tag = mock_tags("postgres", now).remove(0);
    assert_eq!(
        download_size(&tag, "linux/arm64").map(berth_core::fmt::bytes),
        Some("105 MB".to_string())
    );
    assert_eq!(
        download_size(&tag, "linux/amd64").map(berth_core::fmt::bytes),
        Some("107 MB".to_string())
    );
    assert_eq!(platforms_label(&tag), "arm64 \u{b7} amd64");
    assert!(tag_has_platform(&tag, "linux/arm64"));
    let amd_only = TagInfo {
        per_arch: vec![(Arch::X86_64, ByteSize::mb(50))],
        ..tag
    };
    assert!(!tag_has_platform(&amd_only, "linux/arm64"));
    // No image for the platform: the size of the one it has.
    assert_eq!(
        download_size(&amd_only, "linux/arm64"),
        Some(ByteSize::mb(50))
    );
}

#[test]
fn find_images_amd64_only_means_a_known_missing_arm64_build() {
    assert!(amd64_only(&hit("a", Publisher::Community, Some(false))));
    assert!(!amd64_only(&hit("a", Publisher::Community, Some(true))));
    // Unknown is not a warning.
    assert!(!amd64_only(&hit("a", Publisher::Community, None)));
}

// ---------------------------------------------------------------------
// Ranking and the switches
// ---------------------------------------------------------------------

#[test]
fn find_images_an_exact_name_goes_first_and_the_rest_keep_hubs_order() {
    let hits = vec![
        hit("bitnami/postgresql", Publisher::Verified, Some(true)),
        hit("postgres", Publisher::Official, Some(true)),
        hit("postgis/postgis", Publisher::Community, Some(false)),
    ];
    let ranked = rank(hits, "  Postgres ");
    assert_eq!(
        names(&ranked),
        ["postgres", "bitnami/postgresql", "postgis/postgis"]
    );
    // No exact match: nothing moves.
    let same = rank(ranked.clone(), "postg");
    assert_eq!(names(&same), names(&ranked));
}

#[test]
fn find_images_the_switches_keep_official_and_arm64_images() {
    let hits = vec![
        hit("postgres", Publisher::Official, Some(true)),
        hit("bitnami/postgresql", Publisher::Verified, Some(true)),
        hit("postgis/postgis", Publisher::Community, Some(false)),
        hit("mystery/pg", Publisher::Community, None),
    ];
    assert_eq!(visible_hits(&hits, false, false).len(), 4);
    assert_eq!(names(&visible_hits(&hits, true, false)), ["postgres"]);
    // arm64 only drops what is known to have no arm64 build; unknown stays.
    assert_eq!(
        names(&visible_hits(&hits, false, true)),
        ["postgres", "bitnami/postgresql", "mystery/pg"]
    );
    assert_eq!(names(&visible_hits(&hits, true, true)), ["postgres"]);
}

// ---------------------------------------------------------------------
// Did you mean
// ---------------------------------------------------------------------

#[test]
fn find_images_edit_distance_counts_single_edits() {
    assert_eq!(edit_distance("nginx", "nginx"), 0);
    assert_eq!(edit_distance("ngnix", "nginx"), 2);
    assert_eq!(edit_distance("postgre", "postgres"), 1);
    assert_eq!(edit_distance("", "abc"), 3);
    assert_eq!(edit_distance("kitten", "sitting"), 3);
}

#[test]
fn find_images_did_you_mean_offers_the_nearest_known_name() {
    let known = name_candidates(&[], &[]);
    assert_eq!(did_you_mean("ngnix", &known), Some("nginx".to_string()));
    assert_eq!(
        did_you_mean("postgress", &known),
        Some("postgres".to_string())
    );
    assert_eq!(did_you_mean("redsi", &known), Some("redis".to_string()));
    // A repository name matches on its last part too.
    assert_eq!(
        did_you_mean("prometheous", &known),
        Some("prom/prometheus".to_string())
    );
}

#[test]
fn find_images_did_you_mean_keeps_quiet_when_nothing_is_close_or_it_is_the_name() {
    let known = name_candidates(&[], &[]);
    assert_eq!(did_you_mean("zzzzzz-not-an-image", &known), None);
    // The exact name is not a correction.
    assert_eq!(did_you_mean("nginx", &known), None);
    // Two letters would match anything.
    assert_eq!(did_you_mean("ng", &known), None);
}

#[test]
fn find_images_did_you_mean_also_reads_the_engines_own_images() {
    let local = strings(&["acme/billing-api"]);
    let known = name_candidates(&local, &[]);
    assert_eq!(
        did_you_mean("acme/biling-api", &known),
        Some("acme/billing-api".to_string())
    );
}

// ---------------------------------------------------------------------
// Suggestions
// ---------------------------------------------------------------------

fn container(name: &str, image: &str, project: Option<&str>, service: Option<&str>) -> Container {
    let mut world = berth_engine::MockWorld::bundled();
    let mut c = world.containers.remove(0);
    c.name = name.to_string();
    c.image = berth_core::ImageRef {
        reference: image.to_string(),
        id: None,
    };
    c.project = project.map(|p| berth_core::ProjectId(p.to_string()));
    c.compose_project = project.map(str::to_string);
    c.service = service.map(str::to_string);
    c
}

fn project(id: &str, name: &str) -> Project {
    let mut world = berth_engine::MockWorld::bundled();
    let mut p = world.projects.remove(0);
    p.id = berth_core::ProjectId(id.to_string());
    p.name = name.to_string();
    p
}

#[test]
fn find_images_suggestions_come_from_project_containers_then_recent_pulls() {
    let now = at(0);
    let containers = vec![
        container("shop-db", "postgres:17-alpine", Some("shop"), Some("db")),
        container("shop-cache", "redis:7-alpine", Some("shop"), Some("cache")),
        // The same image in another project is one suggestion.
        container("ml-db", "postgres:16", Some("ml"), Some("db")),
        // No project: not a "known project" image.
        container("loose", "nginx:alpine", None, None),
    ];
    let projects = vec![project("shop", "shop-api"), project("ml", "ml-notebooks")];
    let recent = vec![
        RecentPull {
            reference: "alpine:3".to_string(),
            at: now - chrono::Duration::days(2),
        },
        // Already suggested by a project.
        RecentPull {
            reference: "redis:7".to_string(),
            at: now - chrono::Duration::days(1),
        },
    ];
    let list = suggestions(&containers, &projects, &recent, now);
    let shown: Vec<(&str, &str)> = list
        .iter()
        .map(|s| (s.name.as_str(), s.reason.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            ("postgres", "Used by shop-api"),
            ("redis", "Used by shop-api"),
            ("alpine", "Pulled 2 days ago"),
        ]
    );
}

#[test]
fn find_images_suggestions_skip_images_hub_does_not_have() {
    let containers = vec![
        // Built from the project's own Dockerfile.
        container("shop-api-1", "shop-api", Some("shop"), Some("api")),
        container("shop-web-1", "shop-web", Some("shop"), Some("web")),
        // Another registry.
        container("x", "ghcr.io/acme/tool:1", Some("shop"), Some("tool")),
        container(
            "y",
            "registry.local:5000/team/app:2",
            Some("shop"),
            Some("app"),
        ),
        container("z", "library/mysql:8", Some("shop"), Some("sql")),
    ];
    let projects = vec![project("shop", "shop")];
    let list = suggestions(&containers, &projects, &[], at(0));
    assert_eq!(
        list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["mysql"]
    );
}

#[test]
fn find_images_suggestions_stop_at_eight() {
    let containers: Vec<Container> = (0..12)
        .map(|i| container(&format!("c{i}"), &format!("img{i}:1"), Some("p"), Some("s")))
        .collect();
    let list = suggestions(&containers, &[project("p", "p")], &[], at(0));
    assert_eq!(list.len(), 8);
}

#[test]
fn find_images_hub_name_strips_library_and_refuses_other_registries() {
    assert_eq!(hub_name("library/redis"), Some("redis".to_string()));
    assert_eq!(
        hub_name("docker.io/bitnami/redis"),
        Some("bitnami/redis".to_string())
    );
    assert_eq!(hub_name("ghcr.io/acme/tool"), None);
    assert_eq!(hub_name("localhost:5000/app"), None);
    assert_eq!(hub_name(""), None);
}

// ---------------------------------------------------------------------
// An answer, or a refusal, as the view
// ---------------------------------------------------------------------

fn page(items: Vec<ImageHit>) -> Page<ImageHit> {
    Page { items, next: None }
}

#[test]
fn find_images_an_answer_is_ranked_and_carries_no_cache_mark() {
    let view = answer(
        "postgres",
        Ok(page(mock_hits(at(0)).into_iter().rev().collect())),
        None,
        &[],
    );
    assert_eq!(view.status, SearchStatus::Done);
    assert_eq!(view.hits[0].name, "postgres");
    assert_eq!(view.hits.len(), 5);
    assert_eq!(view.cached_at, None);
    assert_eq!(view.did_you_mean, None);
}

#[test]
fn find_images_an_empty_answer_offers_a_correction() {
    let known = name_candidates(&[], &[]);
    let view = answer("ngnix", Ok(page(Vec::new())), None, &known);
    assert_eq!(view.status, SearchStatus::Done);
    assert!(view.hits.is_empty());
    assert_eq!(view.did_you_mean.as_deref(), Some("nginx"));
}

#[test]
fn find_images_a_rate_limited_search_shows_the_cached_answer_with_its_age() {
    let cached = (mock_hits(at(0)), at(-360));
    let view = answer(
        "postgres",
        Err(RegistryError::RateLimited {
            reset_in: Some(Duration::from_secs(120)),
        }),
        Some(cached),
        &[],
    );
    assert_eq!(
        view.status,
        SearchStatus::RateLimited {
            reset_in: Some(Duration::from_secs(120))
        }
    );
    assert_eq!(view.hits.len(), 5);
    assert_eq!(view.cached_at, Some(at(-360)));
}

#[test]
fn find_images_a_rate_limited_search_with_no_cache_shows_nothing_and_says_so() {
    let view = answer(
        "postgres",
        Err(RegistryError::RateLimited { reset_in: None }),
        None,
        &[],
    );
    assert!(view.hits.is_empty());
    assert_eq!(view.cached_at, None);
    assert!(matches!(view.status, SearchStatus::RateLimited { .. }));
}

#[test]
fn find_images_offline_and_failed_searches_keep_the_cache_too() {
    let cached = (mock_hits(at(0)), at(-60));
    let offline = answer(
        "postgres",
        Err(RegistryError::Offline),
        Some(cached.clone()),
        &[],
    );
    assert_eq!(offline.status, SearchStatus::Offline);
    assert_eq!(offline.hits.len(), 5);
    let failed = answer(
        "postgres",
        Err(RegistryError::Http("HTTP 500".to_string())),
        Some(cached),
        &[],
    );
    assert_eq!(failed.status, SearchStatus::Failed("HTTP 500".to_string()));
    assert_eq!(failed.cached_at, Some(at(-60)));
}

#[test]
fn find_images_reset_text_says_seconds_then_minutes() {
    assert_eq!(reset_text(Duration::from_secs(20)), "Try again in 20 s.");
    assert_eq!(reset_text(Duration::from_secs(0)), "Try again in 1 s.");
    assert_eq!(reset_text(Duration::from_secs(120)), "Try again in 2 min.");
    assert_eq!(reset_text(Duration::from_secs(121)), "Try again in 3 min.");
}

// ---------------------------------------------------------------------
// The store over a fake Hub
// ---------------------------------------------------------------------

type Reply = Box<dyn Fn(&str) -> Result<Page<ImageHit>, RegistryError> + Send + Sync>;
type TagsReply = Box<dyn Fn(&str) -> Result<Page<TagInfo>, RegistryError> + Send + Sync>;

struct FakeHub {
    searches: Mutex<Vec<String>>,
    tag_calls: AtomicUsize,
    search: Reply,
    tags: TagsReply,
    /// Per query: how long the answer takes.
    delay: Box<dyn Fn(&str) -> Duration + Send + Sync>,
}

impl FakeHub {
    fn new(search: Reply) -> Self {
        Self {
            searches: Mutex::new(Vec::new()),
            tag_calls: AtomicUsize::new(0),
            search,
            tags: Box::new(|repo| {
                Ok(Page {
                    items: mock_tags(repo, at(0)),
                    next: None,
                })
            }),
            delay: Box::new(|_| Duration::ZERO),
        }
    }
    fn asked(&self) -> Vec<String> {
        self.searches.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl HubApi for FakeHub {
    async fn search(&self, query: &str, _: u32) -> Result<Page<ImageHit>, RegistryError> {
        if let Ok(mut asked) = self.searches.lock() {
            asked.push(query.to_string());
        }
        let wait = (self.delay)(query);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        (self.search)(query)
    }
    async fn tags(&self, repo: &str, _: u32) -> Result<Page<TagInfo>, RegistryError> {
        self.tag_calls.fetch_add(1, Ordering::SeqCst);
        (self.tags)(repo)
    }
}

fn hub_with_mock_hits() -> FakeHub {
    FakeHub::new(Box::new(|query| {
        let needle = query.to_lowercase();
        Ok(Page {
            items: mock_hits(at(0))
                .into_iter()
                .filter(|h| h.name.contains(&needle))
                .collect(),
            next: None,
        })
    }))
}

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, FindStore)>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let store = FindStore::new();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some((state, store)));
    });
    rsx! { div {} }
}

fn captured() -> (VirtualDom, FindStore) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (_state, store) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the store");
    (dom, store)
}

/// Drive the dom until `done` says so (5 s at most).
async fn until(dom: &mut VirtualDom, what: &str, mut done: impl FnMut(&VirtualDom) -> bool) {
    let waited = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if done(dom) {
                return;
            }
            let _ = tokio::time::timeout(Duration::from_millis(20), dom.wait_for_work()).await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    assert!(waited.is_ok(), "{what} did not happen");
}

fn request(text: &str, debounce: Duration, hub: &Arc<FakeHub>) -> SearchRequest {
    SearchRequest {
        text: text.to_string(),
        debounce,
        parent: CancellationToken::new(),
        backend: hub.clone() as Arc<dyn HubApi>,
        candidates: name_candidates(&[], &[]),
    }
}

fn status(dom: &VirtualDom, store: FindStore) -> SearchStatus {
    dom.in_runtime(|| store.view.peek().status.clone())
}

#[tokio::test]
async fn find_images_typing_searches_once_after_the_pause_and_opens_the_first_hit() {
    let (mut dom, store) = captured();
    let hub = Arc::new(hub_with_mock_hits());
    let wait = Duration::from_millis(80);
    // Three keystrokes inside the debounce: only the last goes out.
    for text in ["p", "po", "postgres"] {
        dom.in_scope(ScopeId::APP, || store.search(request(text, wait, &hub)));
    }
    assert_eq!(status(&dom, store), SearchStatus::Searching);
    until(&mut dom, "the answer", |dom| {
        status(dom, store) == SearchStatus::Done
    })
    .await;
    assert_eq!(hub.asked(), ["postgres"]);
    let (first, count) = dom.in_runtime(|| {
        let view = store.view.peek();
        (view.hits[0].name.clone(), view.hits.len())
    });
    assert_eq!((first.as_str(), count), ("postgres", 3));
    // The first hit is open and its tags are on the way, then known.
    assert_eq!(
        dom.in_runtime(|| store.selected.peek().clone()),
        Some("postgres".to_string())
    );
    until(&mut dom, "the tags", |dom| {
        dom.in_runtime(|| {
            matches!(
                store.tags.peek().get("postgres"),
                Some(TagsEntry::Ready { .. })
            )
        })
    })
    .await;
    assert_eq!(hub.tag_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn find_images_an_answer_to_an_older_text_is_dropped() {
    let (mut dom, store) = captured();
    let mut slow = hub_with_mock_hits();
    // "postgres" answers late, "postgis" at once.
    slow.delay = Box::new(|q| {
        if q == "postgres" {
            Duration::from_millis(300)
        } else {
            Duration::ZERO
        }
    });
    let hub = Arc::new(slow);
    dom.in_scope(ScopeId::APP, || {
        store.search(request("postgres", Duration::ZERO, &hub));
    });
    // Let the first request reach the hub, then ask for something else.
    until(&mut dom, "the first request", |_| hub.asked().len() == 1).await;
    dom.in_scope(ScopeId::APP, || {
        store.search(request("postgis", Duration::ZERO, &hub));
    });
    until(&mut dom, "the second answer", |dom| {
        status(dom, store) == SearchStatus::Done
    })
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    dom.render_immediate(&mut dioxus::core::NoOpMutations);
    let view = dom.in_runtime(|| store.view.peek().clone());
    assert_eq!(view.query, "postgis");
    assert_eq!(names(&view.hits), ["postgis/postgis"]);
}

#[tokio::test]
async fn find_images_a_refused_search_shows_the_answer_cached_before() {
    let (mut dom, store) = captured();
    let refuse = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = refuse.clone();
    let hub = Arc::new(FakeHub::new(Box::new(move |_| {
        if flag.load(Ordering::SeqCst) {
            Err(RegistryError::RateLimited {
                reset_in: Some(Duration::from_secs(90)),
            })
        } else {
            Ok(page(mock_hits(at(0))))
        }
    })));
    dom.in_scope(ScopeId::APP, || {
        store.search(request("postgres", Duration::ZERO, &hub));
    });
    until(&mut dom, "the first answer", |dom| {
        status(dom, store) == SearchStatus::Done
    })
    .await;
    refuse.store(true, Ordering::SeqCst);
    dom.in_scope(ScopeId::APP, || {
        // The cache is keyed by trimmed lower-case text.
        store.search(request(" Postgres ", Duration::ZERO, &hub));
    });
    until(&mut dom, "the refusal", |dom| {
        matches!(status(dom, store), SearchStatus::RateLimited { .. })
    })
    .await;
    let view = dom.in_runtime(|| store.view.peek().clone());
    assert_eq!(view.hits.len(), 5, "the cached hits stay on screen");
    assert!(view.cached_at.is_some());
    // A text never searched has nothing cached.
    dom.in_scope(ScopeId::APP, || {
        store.search(request("redis", Duration::ZERO, &hub));
    });
    until(&mut dom, "the second refusal", |dom| {
        dom.in_runtime(|| store.view.peek().query == "redis")
            && matches!(status(dom, store), SearchStatus::RateLimited { .. })
    })
    .await;
    let view = dom.in_runtime(|| store.view.peek().clone());
    assert!(view.hits.is_empty() && view.cached_at.is_none());
}

#[tokio::test]
async fn find_images_clearing_the_field_drops_the_view_and_an_answer_on_its_way() {
    let (mut dom, store) = captured();
    let mut slow = hub_with_mock_hits();
    slow.delay = Box::new(|_| Duration::from_millis(150));
    let hub = Arc::new(slow);
    dom.in_scope(ScopeId::APP, || {
        store.search(request("postgres", Duration::ZERO, &hub));
    });
    until(&mut dom, "the request", |_| hub.asked().len() == 1).await;
    dom.in_scope(ScopeId::APP, || store.clear());
    tokio::time::sleep(Duration::from_millis(300)).await;
    dom.render_immediate(&mut dioxus::core::NoOpMutations);
    assert_eq!(status(&dom, store), SearchStatus::Idle);
    assert!(dom.in_runtime(|| store.view.peek().hits.is_empty()));
    assert!(dom.in_runtime(|| store.query.peek().is_empty()));
}

#[tokio::test]
async fn find_images_tags_are_asked_once_and_a_refusal_falls_back_to_the_cache() {
    let (mut dom, store) = captured();
    let refuse = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = refuse.clone();
    let mut fake = hub_with_mock_hits();
    fake.tags = Box::new(move |repo| {
        if flag.load(Ordering::SeqCst) {
            Err(RegistryError::RateLimited { reset_in: None })
        } else {
            Ok(Page {
                items: mock_tags(repo, at(0)),
                next: None,
            })
        }
    });
    let hub = Arc::new(fake);
    let backend = hub.clone() as Arc<dyn HubApi>;
    let parent = CancellationToken::new();
    let ready = |dom: &VirtualDom| {
        dom.in_runtime(|| {
            matches!(
                store.tags.peek().get("postgres"),
                Some(TagsEntry::Ready { .. })
            )
        })
    };
    dom.in_scope(ScopeId::APP, || {
        store.ensure_tags("postgres", &backend, &parent);
        store.ensure_tags("postgres", &backend, &parent);
    });
    until(&mut dom, "the tags", ready).await;
    dom.in_scope(ScopeId::APP, || {
        store.ensure_tags("postgres", &backend, &parent);
    });
    assert_eq!(
        hub.tag_calls.load(Ordering::SeqCst),
        1,
        "known tags are not asked again"
    );

    // Forget them in the view only (a new session of the tab), refuse, ask.
    refuse.store(true, Ordering::SeqCst);
    dom.in_runtime(|| {
        let mut tags = store.tags;
        tags.write().clear();
    });
    dom.in_scope(ScopeId::APP, || {
        store.ensure_tags("postgres", &backend, &parent);
    });
    until(&mut dom, "the cached tags", |dom| {
        dom.in_runtime(|| {
            matches!(
                store.tags.peek().get("postgres"),
                Some(TagsEntry::Ready {
                    cached_at: Some(_),
                    ..
                })
            )
        })
    })
    .await;
    // A repository never fetched has nothing to fall back to.
    dom.in_scope(ScopeId::APP, || {
        store.ensure_tags("bitnami/postgresql", &backend, &parent);
    });
    until(&mut dom, "the failure", |dom| {
        dom.in_runtime(|| {
            matches!(
                store.tags.peek().get("bitnami/postgresql"),
                Some(TagsEntry::Failed(_))
            )
        })
    })
    .await;
}

#[tokio::test]
async fn find_images_selecting_a_hit_opens_it_and_forgets_the_old_tag() {
    let (dom, store) = captured();
    let hub = Arc::new(hub_with_mock_hits());
    let backend = hub.clone() as Arc<dyn HubApi>;
    let parent = CancellationToken::new();
    dom.in_runtime(|| {
        let mut tag = store.selected_tag;
        tag.set(Some("16".to_string()));
    });
    dom.in_scope(ScopeId::APP, || {
        store.select("postgres", &backend, &parent);
    });
    assert_eq!(
        dom.in_runtime(|| store.selected.peek().clone()),
        Some("postgres".to_string())
    );
    assert_eq!(dom.in_runtime(|| store.selected_tag.peek().clone()), None);
}
