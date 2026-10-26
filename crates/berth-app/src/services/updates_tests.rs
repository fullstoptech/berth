use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn at(hours_after_epoch_day: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap_or_default()
        + Duration::hours(hours_after_epoch_day)
}

fn release(tag: &str) -> Release {
    Release {
        tag: tag.to_string(),
        url: url::Url::parse(&format!(
            "https://github.com/fullstoptech/berth/releases/tag/{tag}"
        ))
        .expect("url"),
        published: at(0),
    }
}

/// A feed that answers one thing and counts how often it was asked.
struct FakeFeed {
    answer: fn() -> Result<Option<Release>, RegistryError>,
    asked: AtomicUsize,
}

impl FakeFeed {
    fn new(answer: fn() -> Result<Option<Release>, RegistryError>) -> Self {
        Self {
            answer,
            asked: AtomicUsize::new(0),
        }
    }
    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl Feed for FakeFeed {
    async fn latest(&self) -> Result<Option<Release>, RegistryError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        (self.answer)()
    }
}

// --- Once a day ---

#[test]
fn updates_due_when_never_checked_and_on() {
    assert!(due(true, None, at(0)));
}

#[test]
fn updates_not_due_when_off_however_old_the_last_check() {
    assert!(!due(false, None, at(0)));
    assert!(!due(false, Some(at(-1000)), at(0)));
}

#[test]
fn updates_not_due_within_a_day_due_at_a_day() {
    let last = at(0);
    assert!(!due(true, Some(last), at(1)));
    assert!(!due(
        true,
        Some(last),
        last + Duration::hours(23) + Duration::minutes(59)
    ));
    assert!(due(true, Some(last), at(24)));
    assert!(due(true, Some(last), at(24 * 9)));
}

#[test]
fn updates_a_last_check_in_the_future_cannot_be_trusted_so_a_check_is_due() {
    assert!(due(true, Some(at(5)), at(0)));
}

#[tokio::test]
async fn updates_the_daily_cycle_asks_once_then_waits_a_day() {
    // The injected clock walks through three launches; only the first and the third may ask.
    let feed = FakeFeed::new(|| Ok(Some(release("v0.4.0"))));
    let first = check_if_due(&feed, "0.5.0", true, None, at(0))
        .await
        .expect("first launch asks");
    assert_eq!(first.checked_at, Some(at(0)));
    let last = first.checked_at;
    assert_eq!(
        check_if_due(&feed, "0.5.0", true, last, at(3)).await,
        None,
        "three hours later: no second ask"
    );
    assert_eq!(feed.asked(), 1);
    let third = check_if_due(&feed, "0.5.0", true, last, at(25))
        .await
        .expect("a day later asks again");
    assert_eq!(third.checked_at, Some(at(25)));
    assert_eq!(feed.asked(), 2);
}

#[tokio::test]
async fn updates_off_never_asks() {
    let feed = FakeFeed::new(|| Ok(Some(release("v9.0.0"))));
    assert_eq!(check_if_due(&feed, "0.5.0", false, None, at(0)).await, None);
    assert_eq!(feed.asked(), 0);
}

// --- What a release means ---

#[test]
fn updates_a_newer_release_is_available_with_its_link() {
    let status = judge("0.5.0", Some(&release("v0.6.0")));
    assert_eq!(
        status,
        UpdateStatus::Available {
            tag: "v0.6.0".to_string(),
            url: "https://github.com/fullstoptech/berth/releases/tag/v0.6.0".to_string(),
        }
    );
}

#[test]
fn updates_the_same_or_an_older_release_is_up_to_date() {
    assert_eq!(
        judge("0.5.0", Some(&release("v0.5.0"))),
        UpdateStatus::UpToDate
    );
    assert_eq!(
        judge("0.5.0", Some(&release("0.5.0"))),
        UpdateStatus::UpToDate
    );
    assert_eq!(
        judge("0.5.0", Some(&release("v0.4.9"))),
        UpdateStatus::UpToDate
    );
    assert_eq!(judge("0.5.0", None), UpdateStatus::UpToDate);
}

#[test]
fn updates_versions_compare_as_versions_not_as_text() {
    // As text "0.10.0" < "0.9.0"; as versions it is newer.
    assert!(matches!(
        judge("0.9.0", Some(&release("v0.10.0"))),
        UpdateStatus::Available { .. }
    ));
}

#[test]
fn updates_a_tag_that_is_not_a_version_is_a_failure_not_a_crash() {
    assert!(matches!(
        judge("0.5.0", Some(&release("nightly"))),
        UpdateStatus::Failed(_)
    ));
}

// --- What a failed ask leaves behind ---

#[tokio::test]
async fn updates_offline_is_shown_but_not_recorded_so_the_next_look_retries() {
    let feed = FakeFeed::new(|| Err(RegistryError::Offline));
    let outcome = check_if_due(&feed, "0.5.0", true, None, at(0))
        .await
        .expect("asked");
    assert_eq!(outcome.status, UpdateStatus::Offline);
    assert_eq!(outcome.checked_at, None);
}

#[tokio::test]
async fn updates_a_rate_limit_is_recorded_so_github_is_left_alone_until_tomorrow() {
    let feed = FakeFeed::new(|| Err(RegistryError::RateLimited { reset_in: None }));
    let outcome = check_if_due(&feed, "0.5.0", true, None, at(0))
        .await
        .expect("asked");
    assert!(matches!(outcome.status, UpdateStatus::Failed(_)));
    assert_eq!(outcome.checked_at, Some(at(0)));
}

#[tokio::test]
async fn updates_an_http_error_is_not_recorded() {
    let feed = FakeFeed::new(|| Err(RegistryError::Http("500".to_string())));
    let outcome = check_if_due(&feed, "0.5.0", true, None, at(0))
        .await
        .expect("asked");
    assert!(matches!(outcome.status, UpdateStatus::Failed(_)));
    assert_eq!(outcome.checked_at, None);
}

#[test]
fn updates_the_time_of_the_last_check_survives_a_relaunch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("berth.db");
    {
        let store = berth_store::Store::open(&file).expect("open");
        let mut settings = store.settings().expect("settings");
        assert_eq!(settings.update_last_checked, None);
        settings.update_last_checked = Some(at(7));
        store.set_settings(&settings).expect("save");
    }
    let reopened = berth_store::Store::open(&file).expect("reopen");
    let settings = reopened.settings().expect("settings");
    assert_eq!(settings.update_last_checked, Some(at(7)));
    assert!(!due(
        settings.update_check,
        settings.update_last_checked,
        at(8)
    ));
}

// --- The real feed against a mock release server ---

fn feed_for(server: &MockServer) -> GithubFeed {
    GithubFeed {
        api_base: url::Url::parse(&server.uri()).expect("uri"),
        slug: "fullstoptech/berth".to_string(),
    }
}

async fn serve_release(server: &MockServer, tag: &str) {
    Mock::given(method("GET"))
        .and(path("/repos/fullstoptech/berth/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/fullstoptech/berth/releases/tag/{tag}"),
            "published_at": "2026-10-20T10:00:00Z",
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn updates_feed_newer_release_is_available() {
    let server = MockServer::start().await;
    serve_release(&server, "v99.0.0").await;
    let outcome = check_if_due(&feed_for(&server), "0.5.0", true, None, at(0))
        .await
        .expect("asked");
    assert_eq!(
        outcome.status,
        UpdateStatus::Available {
            tag: "v99.0.0".to_string(),
            url: "https://github.com/fullstoptech/berth/releases/tag/v99.0.0".to_string(),
        }
    );
    assert_eq!(outcome.checked_at, Some(at(0)));
}

#[tokio::test]
async fn updates_feed_older_release_is_up_to_date() {
    let server = MockServer::start().await;
    serve_release(&server, "v0.1.0").await;
    let outcome = check_if_due(&feed_for(&server), "0.5.0", true, None, at(0))
        .await
        .expect("asked");
    assert_eq!(outcome.status, UpdateStatus::UpToDate);
    assert_eq!(outcome.checked_at, Some(at(0)));
}

#[tokio::test]
async fn updates_feed_with_no_release_yet_is_up_to_date() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let outcome = check_if_due(&feed_for(&server), "0.5.0", true, None, at(0))
        .await
        .expect("asked");
    assert_eq!(outcome.status, UpdateStatus::UpToDate);
}

#[tokio::test]
async fn updates_feed_not_due_makes_no_request() {
    let server = MockServer::start().await;
    serve_release(&server, "v99.0.0").await;
    let skipped = check_if_due(&feed_for(&server), "0.5.0", true, Some(at(0)), at(2)).await;
    assert_eq!(skipped, None);
    assert!(server
        .received_requests()
        .await
        .expect("requests")
        .is_empty());
}

#[tokio::test]
async fn updates_feed_offline_is_offline() {
    // A port nobody listens on.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let feed = GithubFeed {
        api_base: url::Url::parse(&format!("http://127.0.0.1:{port}")).expect("url"),
        slug: "fullstoptech/berth".to_string(),
    };
    let outcome = check_if_due(&feed, "0.5.0", true, None, at(0))
        .await
        .expect("asked");
    assert_eq!(outcome.status, UpdateStatus::Offline);
    assert_eq!(outcome.checked_at, None);
}

// --- The repository this build asks about ---

#[test]
fn updates_slug_comes_from_a_github_url() {
    assert_eq!(
        slug_of("https://github.com/fullstoptech/berth").as_deref(),
        Some("fullstoptech/berth")
    );
    assert_eq!(
        slug_of("https://github.com/fullstoptech/berth.git").as_deref(),
        Some("fullstoptech/berth")
    );
    assert_eq!(
        slug_of("https://github.com/fullstoptech/berth/").as_deref(),
        Some("fullstoptech/berth")
    );
    assert_eq!(slug_of("https://gitlab.com/a/b"), None);
    assert_eq!(slug_of("https://github.com/only-owner"), None);
}

#[test]
fn updates_this_build_asks_about_the_workspace_repository() {
    let feed = GithubFeed::for_this_build().expect("a GitHub repository in Cargo.toml");
    assert_eq!(feed.slug, "fullstoptech/berth");
    assert_eq!(feed.api_base.as_str(), "https://api.github.com/");
}
