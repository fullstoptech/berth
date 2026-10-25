//! P4-T3 tests: the three frames of "Find images" rendered from their
//! scenes, the view model (body, count, banner, platform, tag, Pull and Run
//! rules), the "1 pulling" count, the results table's column tiers, and one
//! live check (ignored) against Docker Hub and the real engine.

use super::*;

use crate::scenes;
use crate::screens::css_tracks as css;
use crate::screens::table_tiers::find_images::{find_state_at, panel_width, FindCol};
use crate::screens::table_tiers::View;
use crate::services::find_images::{mock_hits, mock_tags, SearchView};
use crate::text_scale::STEPS;

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).expect("the frame has a scene")
}

/// Drop every `<style>` block so class names in assertions mean elements,
/// not rules of the screen's own stylesheet.
fn markup(frame: &str) -> String {
    let mut out = scene(frame);
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

fn count(html: &str, needle: &str) -> usize {
    html.matches(needle).count()
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_792_000_000, 0).unwrap_or_default()
}

fn engine() -> Engine {
    berth_engine::MockWorld::bundled().engines.remove(0)
}

const FIND: &str = "Images / find / 1440";
const PULLING: &str = "Images / find / pulling / 1440";
const LIMITED: &str = "Images / find / rate limited / 1440";

// ---------------------------------------------------------------------
// The frames
// ---------------------------------------------------------------------

#[test]
fn find_images_scene_lists_the_five_hits_with_their_publisher_and_platforms() {
    let html = markup(FIND);
    // The Images header and the two tabs; Find is the open one.
    assert!(html.contains(">Images<"), "{html}");
    assert!(
        html.contains("aria-selected=\"true\"><span>Find images</span>"),
        "{html}"
    );
    assert_eq!(count(&html, "class=\"fi-row"), 5, "{html}");
    for name in [
        "postgres",
        "bitnami/postgresql",
        "postgis/postgis",
        "timescale/timescaledb",
        "supabase/postgres",
    ] {
        assert!(
            html.contains(&format!("title=\"{name}\"")),
            "{name}: {html}"
        );
    }
    // Publisher, stars, pulls, updated.
    assert_eq!(
        count(&html, ">Docker Official Image<"),
        2,
        "row and panel: {html}"
    );
    assert_eq!(count(&html, ">Verified Publisher<"), 3, "{html}");
    assert_eq!(count(&html, ">Community<"), 1, "{html}");
    for text in [
        "14k", "850", "1.1k", "1.3k", "180", "1B+", "500M+", "100M+", "50M+",
    ] {
        assert!(html.contains(&format!(">{text}<")), "{text}: {html}");
    }
    assert!(html.contains(">2 days ago<"), "{html}");
    assert!(html.contains(">1 day ago<"), "{html}");
    // The one image with no arm64 build carries the amber note, in its row.
    assert_eq!(count(&html, "amd64 only \u{b7} emulated"), 1, "{html}");
    assert_eq!(count(&html, "<span>arm64</span>"), 4, "{html}");
    // The count, the registry, the two switches.
    assert!(html.contains(">5 results<"), "{html}");
    assert!(html.contains("Docker Hub"), "{html}");
    assert!(html.contains("Official only"), "{html}");
    assert!(html.contains("arm64 only"), "{html}");
}

#[test]
fn find_images_scene_opens_the_first_hit_with_tags_per_architecture() {
    let html = markup(FIND);
    assert_eq!(count(&html, "class=\"fi-row fi-selected\""), 1, "{html}");
    assert!(
        html.contains("The PostgreSQL object-relational database system"),
        "{html}"
    );
    for tag in ["17-alpine", "17", "16-alpine", "16"] {
        assert!(
            html.contains(&format!("fd-tag-name\">{tag}<")),
            "{tag}: {html}"
        );
    }
    // The size of the default platform (arm64), both architectures listed.
    for size in ["105 MB", "157 MB", "97 MB", "151 MB"] {
        assert!(html.contains(&format!(">{size}<")), "{size}: {html}");
    }
    assert_eq!(count(&html, "<span>arm64 \u{b7} amd64</span>"), 4, "{html}");
    // The first tag is the one named, with the engine's own platform.
    assert!(html.contains("docker pull postgres:17-alpine"), "{html}");
    assert!(html.contains("linux/arm64"), "{html}");
    assert!(html.contains("Pull postgres:17-alpine"), "{html}");
    assert!(html.contains(">Copy<"), "{html}");
    assert!(html.contains(">Run\u{2026}<"), "{html}");
    assert!(html.contains(">Add to project\u{2026}<"), "{html}");
    // Nothing is pulling and nothing is cached.
    assert!(!html.contains("pulling"), "{html}");
    assert!(!html.contains("Cached"), "{html}");
    assert!(!html.contains("Pulls may also be limited"), "{html}");
}

#[test]
fn find_images_scene_pulling_shows_progress_in_the_row_the_panel_and_the_tab() {
    let html = markup(PULLING);
    // "1 pulling" beside the first tab.
    assert!(html.contains("tab-note\">1 pulling<"), "{html}");
    // The row replaces its description by "Pulling 17-alpine" and a bar.
    assert!(html.contains("Pulling 17-alpine"), "{html}");
    assert_eq!(
        count(&html, "progress progress-thin"),
        5 + 1,
        "5 layers and the row: {html}"
    );
    // The panel: title and percent, bytes and time left, one bar per layer.
    assert!(html.contains("Pulling postgres:17-alpine"), "{html}");
    assert!(html.contains(">65%<"), "{html}");
    assert!(html.contains("68 MB of 105 MB \u{b7} 12 s left"), "{html}");
    for layer in [
        "a2318d6c47ec",
        "1f8c0ab51b3e",
        "5d2b9e1c7a40",
        "e90a1c47d6b2",
        "7c13f08ea9d5",
    ] {
        assert!(html.contains(layer), "{layer}: {html}");
    }
    assert_eq!(count(&html, "Pull complete"), 3, "{html}");
    assert!(html.contains("31.1 MB of 41.5 MB"), "{html}");
    assert!(html.contains("Waiting"), "{html}");
    assert!(html.contains(">Cancel pull<"), "{html}");
    // The tags stay; the form that would start another pull does not.
    assert!(html.contains("fd-tag-name\">17-alpine<"), "{html}");
    assert!(!html.contains("docker pull postgres"), "{html}");
    assert!(!html.contains("Add to project"), "{html}");
}

#[test]
fn find_images_scene_rate_limited_shows_the_banner_and_names_the_cache() {
    let html = markup(LIMITED);
    assert!(
        html.contains(
            "Docker Hub limits anonymous searches and pulls. Log in to Docker Hub to raise the limit."
        ),
        "{html}"
    );
    assert!(html.contains(">Log in to Docker Hub<"), "{html}");
    assert!(
        html.contains("Token stored in the macOS Keychain"),
        "{html}"
    );
    assert!(html.contains("5 results \u{b7} cached 6 min ago"), "{html}");
    assert!(html.contains("Cached 6 min ago"), "{html}");
    assert!(
        html.contains("Pulls may also be limited until you log in"),
        "{html}"
    );
    // The cached answer is still a table you can read and pull from.
    assert_eq!(count(&html, "class=\"fi-row"), 5, "{html}");
    assert!(html.contains("Pull postgres:17-alpine"), "{html}");
}

#[test]
fn find_images_the_other_two_frames_have_no_banner_and_the_first_has_no_count_note() {
    for frame in [FIND, PULLING] {
        let html = markup(frame);
        assert!(!html.contains("Docker Hub limits anonymous"), "{frame}");
        assert!(!html.contains("cached"), "{frame}");
    }
    assert!(!markup(FIND).contains("tab-note"));
}

// ---------------------------------------------------------------------
// The view model
// ---------------------------------------------------------------------

#[test]
fn find_images_body_follows_the_query_the_status_and_what_is_left() {
    use SearchStatus::{Done, Idle, Offline, RateLimited, Searching};
    // Nothing typed: suggestions, whatever the status says.
    assert_eq!(body(&Idle, 0, 0, false), Body::Suggestions);
    assert_eq!(body(&Searching, 5, 5, false), Body::Suggestions);
    assert_eq!(body(&Done, 5, 3, true), Body::Rows);
    // Hits found, the switches leave none.
    assert_eq!(body(&Done, 5, 0, true), Body::Filtered);
    assert_eq!(body(&Done, 0, 0, true), Body::NoResults);
    assert_eq!(body(&Searching, 0, 0, true), Body::Searching);
    // A refusal with nothing cached has nothing to show.
    assert_eq!(body(&Offline, 0, 0, true), Body::Unavailable);
    assert_eq!(
        body(&RateLimited { reset_in: None }, 0, 0, true),
        Body::Unavailable
    );
    // With a cache the table stays.
    assert_eq!(
        body(&RateLimited { reset_in: None }, 5, 5, true),
        Body::Rows
    );
}

#[test]
fn find_images_count_text_names_filters_the_cache_and_the_search() {
    let done = SearchStatus::Done;
    assert_eq!(count_text(&done, 5, 5, None, now()), "5 results");
    assert_eq!(count_text(&done, 1, 1, None, now()), "1 result");
    assert_eq!(count_text(&done, 5, 3, None, now()), "3 of 5 results");
    assert_eq!(
        count_text(
            &done,
            5,
            5,
            Some(now() - chrono::Duration::minutes(6)),
            now()
        ),
        "5 results \u{b7} cached 6 min ago"
    );
    assert_eq!(
        count_text(&SearchStatus::Searching, 0, 0, None, now()),
        "Searching\u{2026}"
    );
    // A new search over old hits keeps the old count.
    assert_eq!(
        count_text(&SearchStatus::Searching, 5, 5, None, now()),
        "5 results"
    );
}

#[test]
fn find_images_banner_spec_says_what_happened_and_what_to_do() {
    let limited =
        banner_spec(&SearchStatus::RateLimited { reset_in: None }, None, now()).expect("a banner");
    assert_eq!(limited.tone, BannerTone::Warning);
    assert_eq!(limited.action, "Log in to Docker Hub");
    assert!(!limited.retry);
    assert_eq!(limited.note, Some("Token stored in the macOS Keychain"));
    // A reset time is appended.
    let timed = banner_spec(
        &SearchStatus::RateLimited {
            reset_in: Some(std::time::Duration::from_secs(120)),
        },
        None,
        now(),
    )
    .expect("a banner");
    assert!(
        timed.message.ends_with("Try again in 2 min."),
        "{}",
        timed.message
    );
    let offline = banner_spec(
        &SearchStatus::Offline,
        Some(now() - chrono::Duration::hours(3)),
        now(),
    )
    .expect("a banner");
    assert!(offline.retry && offline.action == "Try again");
    assert!(offline
        .message
        .contains("Showing results cached 3 hours ago."));
    let failed =
        banner_spec(&SearchStatus::Failed("HTTP 500".to_string()), None, now()).expect("a banner");
    assert_eq!(failed.tone, BannerTone::Danger);
    assert!(failed.message.contains("HTTP 500"));
    assert!(banner_spec(&SearchStatus::Done, None, now()).is_none());
    assert!(banner_spec(&SearchStatus::Searching, None, now()).is_none());
}

#[test]
fn find_images_platform_defaults_to_the_engines_and_to_amd64_for_an_image_with_no_arm64() {
    let arm = mock_hits(now()).remove(0);
    let amd_only = mock_hits(now()).remove(2);
    assert_eq!(
        effective_platform(None, &Arch::Aarch64, &arm),
        "linux/arm64"
    );
    assert_eq!(effective_platform(None, &Arch::X86_64, &arm), "linux/amd64");
    assert_eq!(
        effective_platform(None, &Arch::Aarch64, &amd_only),
        "linux/amd64"
    );
    // A choice wins.
    assert_eq!(
        effective_platform(Some("linux/amd64"), &Arch::Aarch64, &arm),
        "linux/amd64"
    );
    // The pull names a platform only when it is not the engine's own.
    assert_eq!(pull_platform("linux/arm64", &Arch::Aarch64), None);
    assert_eq!(
        pull_platform("linux/amd64", &Arch::Aarch64),
        Some("linux/amd64".to_string())
    );
}

#[test]
fn find_images_the_tag_is_the_choice_else_the_first_else_latest() {
    let tags = mock_tags("postgres", now());
    assert_eq!(effective_tag(None, &tags), "17-alpine");
    assert_eq!(effective_tag(Some("16"), &tags), "16");
    // A choice from another repository's list is not kept.
    assert_eq!(effective_tag(Some("nope"), &tags), "17-alpine");
    // Tags not known (loading, failed): what was chosen, else latest.
    assert_eq!(effective_tag(None, &[]), "latest");
    assert_eq!(effective_tag(Some("16"), &[]), "16");
}

#[test]
fn find_images_pull_is_off_without_an_engine_while_pulling_and_for_a_missing_platform() {
    let engine = engine();
    let tag = mock_tags("postgres", now()).remove(0);
    assert_eq!(
        pull_blocked(Some(&engine), Some(&tag), "linux/arm64", false),
        None
    );
    assert_eq!(
        pull_blocked(Some(&engine), None, "linux/arm64", false),
        None
    );
    assert!(pull_blocked(None, Some(&tag), "linux/arm64", false)
        .is_some_and(|r| r.contains("No running engine")));
    assert!(pull_blocked(Some(&engine), Some(&tag), "linux/arm64", true)
        .is_some_and(|r| r.contains("Already pulling")));
    let amd_only = TagInfo {
        per_arch: vec![(Arch::X86_64, bytesize::ByteSize::mb(50))],
        ..tag
    };
    assert_eq!(
        pull_blocked(Some(&engine), Some(&amd_only), "linux/arm64", false),
        Some("This tag has no linux/arm64 image".to_string())
    );
}

#[test]
fn find_images_run_opens_on_the_local_image_of_that_tag_only() {
    let world = berth_engine::MockWorld::bundled();
    let by_engine: std::collections::BTreeMap<_, _> = world
        .images_by_engine
        .iter()
        .map(|(k, v)| (berth_core::EngineId(k.clone()), v.clone()))
        .collect();
    let default = berth_core::EngineId("colima:default".to_string());
    let id = local_image_id(&by_engine, &default, "postgres:17-alpine");
    assert!(id.is_some(), "the mock engine has postgres:17-alpine");
    // The tag matters; so does the engine.
    assert_eq!(local_image_id(&by_engine, &default, "postgres:9"), None);
    assert_eq!(
        local_image_id(
            &by_engine,
            &berth_core::EngineId("nowhere".to_string()),
            "postgres:17-alpine"
        ),
        None
    );
}

#[test]
fn find_images_a_pull_belongs_to_a_hit_by_repository_and_a_running_one_wins() {
    let engine = berth_core::EngineId("colima:default".to_string());
    let mut done = PullState::new(engine.clone(), "postgres:16", None);
    done.phase = crate::services::images::PullPhase::Done;
    let running = PullState::new(engine.clone(), "postgres:17-alpine", None);
    let other = PullState::new(engine, "redis:7", None);
    let pulls = vec![done, other, running];
    assert_eq!(
        pull_of(&pulls, "postgres").map(|p| p.reference.as_str()),
        Some("postgres:17-alpine")
    );
    assert_eq!(
        pull_of(&pulls, "redis").map(|p| p.reference.as_str()),
        Some("redis:7")
    );
    assert!(pull_of(&pulls, "bitnami/postgresql").is_none());
    assert_eq!(tag_of_reference("postgres:17-alpine"), "17-alpine");
    assert_eq!(tag_of_reference("localhost:5000/app"), "latest");
    assert_eq!(tag_of_reference("localhost:5000/app:2"), "2");
}

#[test]
fn find_images_the_tab_counts_running_pulls_by_their_pending_key() {
    use crate::state::{PendingKey, PendingState};
    let mut pending = std::collections::BTreeMap::new();
    assert_eq!(crate::screens::images::pulling_count(&pending), 0);
    let key = |subject: berth_core::Subject, action: &'static str| PendingKey { subject, action };
    pending.insert(
        key(
            berth_core::Subject::Image("postgres:17-alpine".to_string()),
            "image.pull",
        ),
        PendingState::Running {
            label: "Pulling postgres:17-alpine".to_string(),
        },
    );
    // Another action on an image, a failed pull and a volume do not count.
    pending.insert(
        key(
            berth_core::Subject::Image("sha256:aa".to_string()),
            "image.remove",
        ),
        PendingState::Running {
            label: "Deleting".to_string(),
        },
    );
    pending.insert(
        key(
            berth_core::Subject::Image("redis:7".to_string()),
            "image.pull",
        ),
        PendingState::Failed {
            message: "no".to_string(),
        },
    );
    pending.insert(
        key(berth_core::Subject::Volume("v".to_string()), "image.pull"),
        PendingState::Running {
            label: "x".to_string(),
        },
    );
    assert_eq!(crate::screens::images::pulling_count(&pending), 1);
}

#[test]
fn find_images_an_idle_view_has_no_hits_and_no_cache_mark() {
    let view = SearchView::idle();
    assert_eq!(view.status, SearchStatus::Idle);
    assert!(view.hits.is_empty() && view.cached_at.is_none());
}

// ---------------------------------------------------------------------
// Column tiers
// ---------------------------------------------------------------------

/// The table's room: the screen next to the 420 px panel (AD24 sweep over
/// the three windows and every text size).
fn table_width(window: f32, scale: f32) -> (f32, bool) {
    let body = window - 220.0;
    (body - panel_width(body, scale), window < 1200.0)
}

#[test]
fn find_images_table_fits_every_window_and_text_size() {
    for window in [1024.0, 1220.0, 1440.0] {
        for scale in STEPS {
            let (width, narrow) = table_width(window, scale);
            let view = if narrow { View::Narrow } else { View::Wide };
            let state = find_state_at(view, width, scale);
            let gutters = if narrow { 32.0 } else { 40.0 };
            let used = css::min_sum(&state.tracks, scale)
                + 12.0 * (state.tracks.len() as f32 - 1.0)
                + gutters;
            assert!(
                used <= width + 0.5,
                "{window} px at {scale}: the table is {width} px and needs {used} ({:?})",
                state.tracks
            );
        }
    }
}

#[test]
fn find_images_panel_is_420_px_wherever_the_table_keeps_its_room() {
    // From a 1220 window every text size leaves the panel its 420 px; the
    // 1024 window does up to 100 %.
    for scale in STEPS {
        for window in [1220.0, 1440.0] {
            assert!(
                (panel_width(window - 220.0, scale) - 420.0).abs() < f32::EPSILON,
                "{window} at {scale}"
            );
        }
    }
    for scale in [0.85, 0.9, 1.0] {
        assert!((panel_width(1024.0 - 220.0, scale) - 420.0).abs() < f32::EPSILON);
    }
    // Large text in a small window takes room from the panel, never below 300.
    assert!(panel_width(804.0, 1.5) < 420.0);
    assert!(panel_width(500.0, 1.5) >= 300.0);
}

#[test]
fn find_images_table_keeps_every_column_at_1440_and_drops_them_in_order() {
    let (width, _) = table_width(1440.0, 1.0);
    let full = find_state_at(View::Wide, width, 1.0);
    assert_eq!((full.step, full.hidden.clone()), (0, vec![]));
    let mut seen: Vec<Vec<FindCol>> = Vec::new();
    for width in (300..1000).rev().step_by(2) {
        let hidden = find_state_at(View::Wide, width as f32, 1.0).hidden;
        if seen.last() != Some(&hidden) {
            seen.push(hidden);
        }
    }
    assert_eq!(
        seen,
        vec![
            vec![],
            vec![FindCol::Updated],
            vec![FindCol::Updated, FindCol::Pulls],
            vec![FindCol::Updated, FindCol::Pulls, FindCol::Stars],
            vec![
                FindCol::Updated,
                FindCol::Pulls,
                FindCol::Stars,
                FindCol::Publisher
            ],
        ]
    );
}

#[test]
fn find_images_stylesheet_uses_only_tokens_and_scales_every_text_track() {
    let tiers = include_str!("../../assets/css/find_images_tiers.css");
    assert!(
        tiers.contains("@container findtable (max-width:"),
        "{tiers}"
    );
    let hand = include_str!("../../assets/css/find_images.css");
    assert!(hand.contains("container: findtable / inline-size"));
    let body = css::block(hand, "\n.fi-table {");
    for name in [
        "--fi-cols",
        "--fi-cols-no-updated",
        "--fi-cols-no-pulls",
        "--fi-cols-no-stars",
        "--fi-cols-compact",
    ] {
        let list = css::declaration(body, name);
        let tracks = css::tracks(&list);
        // The name track flexes; every other track holds text or numbers.
        for (i, track) in tracks.iter().enumerate().skip(1) {
            assert!(
                css::has_scale(track),
                "track {i} `{track}` of {name} needs a scale term"
            );
        }
    }
    // No colour is written out: tokens only.
    for line in hand.lines() {
        assert!(!line.contains('#') || line.contains("/*"), "{line}");
    }
}

// ---------------------------------------------------------------------
// A pull from the tab: the pending key, the row and the end
// ---------------------------------------------------------------------

fn services_with_mock() -> Services {
    let id = berth_core::EngineId("colima:default".to_string());
    let mock = Arc::new(
        berth_engine::MockEngine::from_world(berth_engine::MockWorld::bundled(), &id)
            .expect("engine"),
    );
    let clients = crate::services::engines::client_map();
    if let Ok(mut map) = clients.lock() {
        map.insert(id, mock as Arc<dyn berth_engine::EngineClient>);
    }
    Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients,
        cancel: CancellationToken::new(),
    }
}

#[tokio::test]
async fn find_images_a_pull_holds_its_pending_key_until_it_ends() {
    use crate::state::{PendingKey, PendingState};
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (state, _find, images) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the stores");
    let services = services_with_mock();
    let engine = berth_core::EngineId("colima:default".to_string());
    let key = PendingKey {
        subject: berth_core::Subject::Image("berthfx/never:1".to_string()),
        action: "image.pull",
    };
    dom.in_scope(ScopeId::APP, || {
        images
            .pull(&services, &engine, "berthfx/never:1", None)
            .expect("the pull starts");
        // Cancelled before its first poll, so it ends at once and pulls nothing.
        images.cancel_pull(&(engine.clone(), "berthfx/never:1".to_string()));
    });
    // The tab counts it from the moment it starts.
    assert_eq!(
        dom.in_runtime(|| state.pending.peek().get(&key).cloned()),
        Some(PendingState::Running {
            label: "Pulling berthfx/never:1".to_string()
        })
    );
    assert_eq!(
        dom.in_runtime(|| crate::screens::images::pulling_count(&state.pending.peek())),
        1
    );
    until(&mut dom, "the end of the pull", 5, |dom| {
        dom.in_runtime(|| images.pulls.peek().iter().all(|p| !p.is_running()))
    })
    .await;
    // And not after it: the count goes with the pull, whatever its end.
    assert!(dom.in_runtime(|| !state.pending.peek().contains_key(&key)));
    assert_eq!(
        dom.in_runtime(|| crate::screens::images::pulling_count(&state.pending.peek())),
        0
    );
}

// ---------------------------------------------------------------------
// Live: Docker Hub and the real engine (BERTH_IT=1 BERTH_NET=1)
// ---------------------------------------------------------------------

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, FindStore, ImageStore)>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = crate::state::use_app_state();
    use_context_provider(|| state);
    let find = FindStore::new();
    let images = ImageStore::new();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some((state, find, images)));
    });
    rsx! { div {} }
}

async fn until(
    dom: &mut VirtualDom,
    what: &str,
    secs: u64,
    mut done: impl FnMut(&VirtualDom) -> bool,
) {
    let waited = tokio::time::timeout(std::time::Duration::from_secs(secs), async {
        loop {
            if done(dom) {
                return;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work())
                .await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    assert!(waited.is_ok(), "{what} did not happen");
}

/// Searches the live Docker Hub for "postgres" (the Official image is first
/// with its badge), then pulls `postgres:17-alpine` to the real
/// `colima:default` through the store the tab uses, and finds it under "On
/// this engine". The image is one the fixtures use: it is not deleted.
#[tokio::test]
#[ignore = "live: needs Docker Hub and Colima (BERTH_IT=1 BERTH_NET=1)"]
async fn find_images_it_search_postgres_then_pull_17_alpine() {
    if std::env::var("BERTH_IT").as_deref() != Ok("1")
        || std::env::var("BERTH_NET").as_deref() != Ok("1")
    {
        eprintln!("skipped: set BERTH_IT=1 BERTH_NET=1");
        return;
    }
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (_state, find, images) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the stores");

    // 1. The live Hub lists postgres first, as the Official image.
    let hub: Arc<dyn HubApi> = Arc::new(crate::services::find_images::LiveHub::new(
        url::Url::parse(crate::services::find_images::HUB_URL).expect("the Hub url"),
    ));
    dom.in_scope(ScopeId::APP, || {
        find.search(SearchRequest {
            text: "postgres".to_string(),
            debounce: std::time::Duration::ZERO,
            parent: CancellationToken::new(),
            backend: hub,
            candidates: Vec::new(),
        });
    });
    until(&mut dom, "the live answer", 30, |dom| {
        dom.in_runtime(|| {
            !matches!(
                find.view.peek().status,
                SearchStatus::Idle | SearchStatus::Searching
            )
        })
    })
    .await;
    let view = dom.in_runtime(|| find.view.peek().clone());
    assert_eq!(view.status, SearchStatus::Done, "{view:?}");
    let first = view.hits.first().expect("hits");
    println!(
        "first hit: {} {:?} stars={} pulls={} arm64={:?}",
        first.name, first.publisher, first.stars, first.pulls, first.arm64
    );
    assert_eq!(first.name, "postgres");
    assert_eq!(first.publisher, Publisher::Official);
    until(&mut dom, "the live tags", 30, |dom| {
        dom.in_runtime(|| {
            matches!(
                find.tags.peek().get("postgres"),
                Some(TagsEntry::Ready { .. })
            )
        })
    })
    .await;

    // 2. Pull postgres:17-alpine on the real engine.
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima = berth_engine::ColimaCli::new(search);
    let engines = berth_engine::discovery::discover(&colima, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running");
    let client = berth_engine::BollardEngine::connect(engine).expect("connect");
    let clients = crate::services::engines::client_map();
    if let Ok(mut map) = clients.lock() {
        map.insert(engine.id.clone(), Arc::new(client));
    }
    let services = Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients,
        cancel: CancellationToken::new(),
    };
    dom.in_scope(ScopeId::APP, || {
        images
            .pull(&services, &engine.id, "postgres:17-alpine", None)
            .expect("the pull starts");
    });
    assert_eq!(dom.in_runtime(|| images.pulling()), 1);
    let mut longest_layers = 0;
    until(&mut dom, "the end of the pull", 300, |dom| {
        dom.in_runtime(|| {
            let pulls = images.pulls.peek();
            longest_layers = longest_layers.max(pulls.first().map_or(0, |p| p.layers.len()));
            pulls.iter().all(|p| !p.is_running())
        })
    })
    .await;
    let pull = dom.in_runtime(|| images.pulls.peek()[0].clone());
    println!(
        "pull: {:?} layers={} note={:?} percent={:?}",
        pull.phase,
        pull.layers.len().max(longest_layers),
        pull.note,
        pull.percent()
    );
    assert_eq!(pull.phase, crate::services::images::PullPhase::Done);
    assert!(
        !pull.layers.is_empty() || !pull.note.is_empty(),
        "the stream said something"
    );

    // The image of the fixtures is usually there already ("Image is up to
    // date"); a tiny public image shows the layers of a real pull.
    dom.in_scope(ScopeId::APP, || {
        images
            .pull(&services, &engine.id, "hello-world:latest", None)
            .expect("the second pull starts");
    });
    let mut seen_layers = 0;
    until(&mut dom, "the end of the second pull", 120, |dom| {
        dom.in_runtime(|| {
            let pulls = images.pulls.peek();
            let hello = pulls.iter().find(|p| p.reference == "hello-world:latest");
            seen_layers = seen_layers.max(hello.map_or(0, |p| p.layers.len()));
            hello.is_some_and(|p| !p.is_running())
        })
    })
    .await;
    let hello = dom.in_runtime(|| {
        images
            .pulls
            .peek()
            .iter()
            .find(|p| p.reference == "hello-world:latest")
            .cloned()
    });
    let hello = hello.expect("the second pull");
    println!(
        "hello-world: {:?} layers={seen_layers} note={:?}",
        hello.phase, hello.note
    );
    assert_eq!(hello.phase, crate::services::images::PullPhase::Done);
    assert!(seen_layers > 0 || hello.note.contains("up to date"));

    // 3. It is listed under "On this engine".
    dom.in_scope(ScopeId::APP, || images.refresh(&services));
    until(&mut dom, "the refreshed list", 30, |dom| {
        dom.in_runtime(|| {
            images.by_engine.peek().get(&engine.id).is_some_and(|list| {
                list.iter()
                    .any(|i| i.repo_tags.iter().any(|t| t == "postgres:17-alpine"))
            })
        })
    })
    .await;
    let by_engine = dom.in_runtime(|| images.by_engine.peek().clone());
    assert!(local_image_id(&by_engine, &engine.id, "postgres:17-alpine").is_some());
}
