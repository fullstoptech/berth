//! P4-T1 tests: the Images table from the mock world, the view model
//! (scope, filters, sort, reasons), the pull card, and the column tiers at
//! every window, inspector width and text size.

use super::*;

use crate::scenes;
use crate::screens::css_tracks as css;
use crate::screens::table_tiers::images::{images_least_need, images_state_at, ImageCol};
use crate::screens::table_tiers::View;
use crate::state::{AppState, Overlay, Selection};
use crate::text_scale::STEPS;

fn engine_id(s: &str) -> EngineId {
    EngineId(s.to_string())
}

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).expect("the frame has a scene")
}

/// Drop every `<style>` block so class names in assertions mean elements,
/// not rules of the screen's own stylesheet.
fn strip_styles(html: &str) -> String {
    let mut out = html.to_string();
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

fn markup(frame: &str) -> String {
    strip_styles(&scene(frame))
}

fn count(html: &str, needle: &str) -> usize {
    html.matches(needle).count()
}

fn mock_rows() -> Vec<ImageRow> {
    let w = world();
    build_rows(
        &w.engines,
        &w.images_by_engine
            .iter()
            .map(|(k, v)| (engine_id(k), v.clone()))
            .collect(),
        &EngineScope::One(engine_id("colima:default")),
    )
}

fn image(id: &str, tags: &[&str], mb: u64, arch: Arch, used: usize) -> Image {
    Image {
        id: format!("sha256:{id}"),
        repo_tags: tags.iter().map(|t| (*t).to_string()).collect(),
        digests: Vec::new(),
        size: bytesize::ByteSize::mb(mb),
        created: chrono::DateTime::from_timestamp(
            1_790_000_000 + i64::try_from(mb).unwrap_or(0),
            0,
        )
        .unwrap_or_default(),
        arch,
        os: "linux".to_string(),
        used_by: (0..used).map(|n| ContainerId(format!("c{n}"))).collect(),
        dangling: tags.is_empty(),
        engine: berth_core::EngineId("colima:default".to_string()),
    }
}

// ---------------------------------------------------------------------
// The scene: header, controls, 31 rows
// ---------------------------------------------------------------------

#[test]
fn images_scene_draws_the_header_tabs_controls_and_one_row_per_image() {
    let html = markup("Images / on this engine / 1440");
    assert!(html.contains(">Images</h1>"), "{html}");
    assert!(
        html.contains("badge badge-count\">31<"),
        "the count of the sidebar: {html}"
    );
    // Tabs: On this engine (selected) and Find images.
    assert!(html.contains("On this engine"), "{html}");
    assert!(html.contains("Find images"), "{html}");
    assert!(
        html.contains("tab tab-selected\" type=\"button\" role=\"tab\" aria-selected=\"true\"><span>On this engine"),
        "{html}"
    );
    // Controls.
    assert!(html.contains("Filter images"), "{html}");
    assert!(html.contains("Dangling"), "{html}");
    assert!(html.contains(">Pull image<"), "{html}");
    // The table: header labels, one row per image of the default engine.
    for label in ["Image", "ID", "Size", "Arch", "Created", "Used by"] {
        assert!(
            html.contains(&format!(
                "<span class=\"table-header-label\">{label}</span>"
            )),
            "{label}: {html}"
        );
    }
    assert_eq!(count(&html, "class=\"im-row\""), 31, "{html}");
    assert!(html.contains("shop-api-worker:latest"), "{html}");
    assert!(html.contains("postgres:17-alpine"), "{html}");
    // Sizes are decimal, ids are twelve digits.
    assert!(html.contains("57 MB"), "{html}");
    assert!(!html.contains("sha256:"), "{html}");
}

#[test]
fn images_scene_rows_carry_run_and_a_delete_that_is_off_while_a_container_uses_the_image() {
    let html = markup("Images / on this engine / 1440");
    assert_eq!(
        count(&html, "<span class=\"btn-label\">Run</span>"),
        31,
        "{html}"
    );
    // shop-api-worker:latest is used by two containers.
    assert!(
        html.contains("data-tooltip=\"Used by 2 containers\""),
        "{html}"
    );
    let rows = mock_rows();
    let used = rows.iter().filter(|r| !r.used_by.is_empty()).count();
    assert!(used > 20 && used < 31, "{used} used images");
    let deletes = count(&html, "aria-label=\"Delete ");
    assert_eq!(deletes, 31, "{html}");
    // An unused image's Delete is a live button that says "Delete"; a used
    // one is off and says why.
    assert_eq!(count(&html, "data-tooltip=\"Delete\""), 31 - used, "{html}");
    assert_eq!(
        count(&html, "data-tooltip=\"Used by "),
        used,
        "one reason per used image: {html}"
    );
}

#[test]
fn images_scene_selected_row_opens_the_inspector_panel() {
    let html = markup("Images / selected / 1440");
    assert_eq!(count(&html, "class=\"im-row im-selected\""), 1, "{html}");
    // The panel: name, size and engine, ID, the three tabs, Delete's reason.
    assert!(html.contains("class=\"ih-name\""), "{html}");
    assert!(html.contains("57 MB \u{b7} default"), "{html}");
    for tab in ["Overview", "Layers", "Containers"] {
        assert!(
            html.contains(&format!("<span>{tab}</span>")),
            "{tab}: {html}"
        );
    }
    assert!(
        html.contains("<span class=\"btn-label\">Tag</span>"),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"btn-label\">Pull latest</span>"),
        "{html}"
    );
    assert!(html.contains("Used by 2 containers"), "{html}");
    // Overview rows come from the seeded detail.
    assert!(html.contains("kv-label\">Platform"), "{html}");
    assert!(html.contains("linux/arm64"), "{html}");
}

#[test]
fn images_scene_pulling_shows_a_card_with_per_layer_progress() {
    let html = markup("Images / pulling / 1440");
    assert!(html.contains("Pulling postgres:17-alpine"), "{html}");
    assert!(html.contains(">Cancel pull<"), "{html}");
    // Four layers: three whole, one at 31.1 of 41.5 MB; one waiting.
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
    // 18.3 MB finished plus 31.1 of 41.5 MB: 49.4 of 59.8 MB, 82 %.
    assert!(html.contains("49.4 MB of 59.8 MB"), "{html}");
    assert!(html.contains(">82%<"), "{html}");
    assert!(html.contains("3 of 5 layers"), "{html}");
    // The layer bars are the thin ones.
    assert_eq!(count(&html, "progress progress-thin"), 5, "{html}");
}

#[test]
fn images_scene_run_sheet_is_prefilled_from_the_image() {
    let html = markup("Images / run / 1440");
    assert!(html.contains("Run postgres:17-alpine"), "{html}");
    assert!(html.contains("aria-label=\"Run image\""), "{html}");
    // Name from the repository.
    assert!(html.contains("value=\"postgres\""), "{html}");
    // Environment keys of the image, values hidden and unset.
    assert!(
        html.contains("title=\"POSTGRES_PASSWORD\"") || html.contains("er-env-key"),
        "{html}"
    );
    assert!(html.contains("image default"), "{html}");
    assert!(!html.contains("••••"), "{html}");
}

// ---------------------------------------------------------------------
// The view model
// ---------------------------------------------------------------------

#[test]
fn images_build_rows_follows_the_scope_and_skips_engines_that_are_not_running() {
    let w = world();
    let by_engine: BTreeMap<EngineId, Vec<Image>> = w
        .images_by_engine
        .iter()
        .map(|(k, v)| (engine_id(k), v.clone()))
        .collect();
    let one = build_rows(
        &w.engines,
        &by_engine,
        &EngineScope::One(engine_id("colima:default")),
    );
    assert_eq!(one.len(), 31);
    assert!(one.iter().all(|r| r.engine == engine_id("colima:default")));
    // All: the stopped `work` and the unreachable `hetzner-1` are left out.
    let all = build_rows(&w.engines, &by_engine, &EngineScope::All);
    assert_eq!(all.len(), 31);
    // A stopped engine named by the scope shows nothing, not stale rows.
    let stopped = build_rows(
        &w.engines,
        &by_engine,
        &EngineScope::One(engine_id("colima:work")),
    );
    assert!(stopped.is_empty());
    // An engine the list does not know yet still shows (it answered).
    let unknown = build_rows(&[], &by_engine, &EngineScope::All);
    assert_eq!(unknown.len(), 31 + 6 + 9);
}

#[test]
fn images_rows_mark_an_amd64_image_on_an_arm64_engine() {
    let w = world();
    let default = w
        .engines
        .iter()
        .find(|e| e.id.0 == "colima:default")
        .expect("engine");
    let by_engine = BTreeMap::from([(
        default.id.clone(),
        vec![
            image("a1", &["alpine:3"], 8, Arch::Aarch64, 0),
            image("b2", &["legacy:1"], 90, Arch::X86_64, 0),
            image("c3", &["odd:1"], 5, Arch::Other("unknown".to_string()), 0),
        ],
    )]);
    let rows = build_rows(&w.engines, &by_engine, &EngineScope::All);
    let by_name = |n: &str| rows.iter().find(|r| r.name == n).expect(n);
    assert_eq!(
        (
            by_name("alpine:3").arch.as_str(),
            by_name("alpine:3").foreign_arch
        ),
        ("arm64", false)
    );
    assert_eq!(
        (
            by_name("legacy:1").arch.as_str(),
            by_name("legacy:1").foreign_arch
        ),
        ("amd64", true)
    );
    assert_eq!(
        (
            by_name("odd:1").arch.as_str(),
            by_name("odd:1").foreign_arch
        ),
        ("\u{2014}", false)
    );
    assert_eq!(by_name("alpine:3").short_id, "a1");
}

#[component]
fn RowsHost(rows: Vec<ImageRow>) -> Element {
    let state = crate::state::use_app_state();
    use_context_provider(|| state);
    rsx! {
        for row in rows {
            ImageRowView {
                key: "{row.id}",
                row,
                selected: false,
                used_by: "\u{2014}".to_string(),
                created: "now".to_string(),
                dangling_tag: false,
            }
        }
    }
}

#[test]
fn images_arch_badge_shows_only_for_the_foreign_architecture() {
    let w = world();
    let default = w
        .engines
        .iter()
        .find(|e| e.id.0 == "colima:default")
        .expect("engine");
    let rows = build_rows(
        &w.engines,
        &BTreeMap::from([(
            default.id.clone(),
            vec![
                image("a1", &["alpine:3"], 8, Arch::Aarch64, 0),
                image("b2", &["legacy:1"], 90, Arch::X86_64, 0),
            ],
        )]),
        &EngineScope::All,
    );
    let mut dom = VirtualDom::new_with_props(RowsHost, RowsHostProps { rows });
    let _ = dom.rebuild_to_vec();
    let html = strip_styles(&dioxus_ssr::render(&dom));
    assert_eq!(count(&html, "badge badge-arch\">amd64<"), 1, "{html}");
    assert!(!html.contains("badge badge-arch\">arm64"), "{html}");
    assert!(html.contains(">arm64<"), "{html}");
}

#[test]
fn images_list_rows_filters_by_tag_and_id_and_by_dangling() {
    let rows: Vec<ImageRow> = {
        let w = world();
        let default = w
            .engines
            .iter()
            .find(|e| e.id.0 == "colima:default")
            .expect("engine");
        build_rows(
            &w.engines,
            &BTreeMap::from([(
                default.id.clone(),
                vec![
                    image(
                        "f00dcafe1234",
                        &["postgres:17-alpine", "pg:17"],
                        74,
                        Arch::Aarch64,
                        1,
                    ),
                    image("0123456789ab", &["redis:7"], 40, Arch::Aarch64, 0),
                    image("deadbeef0000", &[], 12, Arch::Aarch64, 0),
                ],
            )]),
            &EngineScope::All,
        )
    };
    let sort = (SortColumn::Name, SortDirection::Ascending);
    let names = |q: &str, f| {
        list_rows(&rows, q, f, sort)
            .iter()
            .map(|r| r.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names("", ImageFilter::All),
        ["postgres:17-alpine", "redis:7", ""]
    );
    assert_eq!(names("POSTGRES", ImageFilter::All), ["postgres:17-alpine"]);
    // Any tag matches, not only the first.
    assert_eq!(names("pg:17", ImageFilter::All), ["postgres:17-alpine"]);
    // The short id matches.
    assert_eq!(names("0123456", ImageFilter::All), ["redis:7"]);
    assert_eq!(names("zzz", ImageFilter::All), Vec::<String>::new());
    // Dangling keeps the untagged one only.
    assert_eq!(names("", ImageFilter::Dangling), [""]);
    assert_eq!(filter_counts(&rows), [3, 1]);
}

#[test]
fn images_list_rows_sorts_by_name_size_and_created_in_both_directions() {
    let w = world();
    let default = w
        .engines
        .iter()
        .find(|e| e.id.0 == "colima:default")
        .expect("engine");
    let rows = build_rows(
        &w.engines,
        &BTreeMap::from([(
            default.id.clone(),
            vec![
                image("a", &["b:1"], 50, Arch::Aarch64, 0),
                image("b", &[], 10, Arch::Aarch64, 0),
                image("c", &["a:1"], 90, Arch::Aarch64, 0),
            ],
        )]),
        &EngineScope::All,
    );
    let order = |column, direction| {
        list_rows(&rows, "", ImageFilter::All, (column, direction))
            .iter()
            .map(|r| r.short_id.clone())
            .collect::<Vec<_>>()
    };
    // Names ascending; the untagged image sorts last either way.
    assert_eq!(
        order(SortColumn::Name, SortDirection::Ascending),
        ["c", "a", "b"]
    );
    assert_eq!(
        order(SortColumn::Name, SortDirection::Descending),
        ["a", "c", "b"]
    );
    assert_eq!(
        order(SortColumn::Size, SortDirection::Descending),
        ["c", "a", "b"]
    );
    assert_eq!(
        order(SortColumn::Size, SortDirection::Ascending),
        ["b", "a", "c"]
    );
    // `created` follows the size in this fixture (older when smaller).
    assert_eq!(
        order(SortColumn::Created, SortDirection::Descending),
        ["c", "a", "b"]
    );
}

#[test]
fn images_next_sort_flips_the_same_column_and_starts_others_at_their_natural_end() {
    let name_up = (SortColumn::Name, SortDirection::Ascending);
    assert_eq!(
        next_sort(name_up, SortColumn::Name),
        (SortColumn::Name, SortDirection::Descending)
    );
    assert_eq!(
        next_sort(name_up, SortColumn::Size),
        (SortColumn::Size, SortDirection::Descending)
    );
    assert_eq!(
        next_sort(
            (SortColumn::Size, SortDirection::Descending),
            SortColumn::Name
        ),
        name_up
    );
    assert_eq!(
        next_sort(
            (SortColumn::Size, SortDirection::Descending),
            SortColumn::Created
        ),
        (SortColumn::Created, SortDirection::Descending)
    );
}

#[test]
fn images_used_by_names_one_container_and_counts_several() {
    let w = world();
    let containers = w.containers.clone();
    let worker = containers
        .iter()
        .find(|c| c.name == "shop-api-worker-1")
        .expect("worker");
    assert_eq!(used_by_text(&[], &containers), "\u{2014}");
    assert_eq!(
        used_by_text(std::slice::from_ref(&worker.id), &containers),
        "shop-api-worker-1"
    );
    assert_eq!(
        used_by_text(
            &[worker.id.clone(), ContainerId("x".to_string())],
            &containers
        ),
        "2 containers"
    );
    // A container the list no longer has still counts as one.
    assert_eq!(
        used_by_text(&[ContainerId("gone".to_string())], &containers),
        "1 container"
    );
}

#[test]
fn images_delete_reason_uses_the_words_of_the_actions_service() {
    let w = world();
    let used = mock_rows()
        .into_iter()
        .find(|r| r.name == "shop-api-worker:latest")
        .expect("a used image");
    let view_images: Vec<Image> = w.images_by_engine.values().flatten().cloned().collect();
    let pending = std::collections::BTreeMap::new();
    let view = crate::services::actions::StateView {
        engines: &w.engines,
        containers: &w.containers,
        projects: &w.projects,
        images: &view_images,
        volumes: &[],
        networks: &[],
        pending: &pending,
        backing: crate::services::backing::Backing::default(),
    };
    let from_service = crate::services::actions::disabled_reason(
        &berth_core::Subject::Image(used.id.clone()),
        "image.delete",
        &view,
    );
    assert_eq!(from_service.as_deref(), Some("Used by 2 containers"));
    assert_eq!(
        delete_reason(&used).as_deref(),
        Some("Used by 2 containers")
    );
    let mut idle = used.clone();
    idle.used_by.clear();
    assert_eq!(delete_reason(&idle), None);
    let mut one = used;
    one.used_by.truncate(1);
    assert_eq!(delete_reason(&one).as_deref(), Some("Used by 1 container"));
}

#[test]
fn images_pull_target_is_the_scope_engine_or_the_first_running_one() {
    let w = world();
    let default = engine_id("colima:default");
    assert_eq!(
        pull_target(&w.engines, &EngineScope::One(default.clone())).map(|e| e.id),
        Some(default.clone())
    );
    assert_eq!(
        pull_target(&w.engines, &EngineScope::All).map(|e| e.id),
        Some(default)
    );
    assert_eq!(pull_target(&[], &EngineScope::All), None);
    // A stopped engine named by the scope is still the target the user chose:
    // the Pull button explains nothing is running only when none is.
    assert!(pull_target(&w.engines, &EngineScope::One(engine_id("colima:work"))).is_some());
}

#[test]
fn images_screen_state_tells_loading_from_empty() {
    let w = world();
    let none: BTreeMap<EngineId, Vec<Image>> = BTreeMap::new();
    assert_eq!(screen_state(&w.engines, &none, &[]), ScreenState::Loading);
    let answered = BTreeMap::from([(engine_id("colima:default"), Vec::new())]);
    assert_eq!(screen_state(&w.engines, &answered, &[]), ScreenState::Empty);
    // A stopped engine's old answer is not an answer.
    let stale = BTreeMap::from([(engine_id("colima:work"), Vec::new())]);
    assert_eq!(screen_state(&w.engines, &stale, &[]), ScreenState::Loading);
    assert_eq!(
        screen_state(&w.engines, &none, &mock_rows()),
        ScreenState::Rows
    );
}

// ---------------------------------------------------------------------
// The pull card
// ---------------------------------------------------------------------

fn card(pull: PullState) -> String {
    strip_styles(&dioxus_ssr::render_element(rsx! {
        style { "{PROGRESS_BAR_CSS}" }
        PullCard { pull }
    }))
}

#[test]
fn images_pull_card_shows_its_four_phases() {
    let mut running = PullState::new(engine_id("colima:default"), "alpine:3", None);
    running.apply(&berth_core::PullProgress {
        layer: Some("aaaaaaaaaaaa".to_string()),
        status: "Downloading".to_string(),
        current: Some(50),
        total: Some(100),
    });
    let html = card(running.clone());
    assert!(html.contains("Pulling alpine:3"), "{html}");
    assert!(html.contains(">Cancel pull<"), "{html}");
    assert!(html.contains(">50%<"), "{html}");

    let mut done = running.clone();
    done.phase = PullPhase::Done;
    let html = card(done);
    assert!(html.contains("Pulled alpine:3"), "{html}");
    assert!(html.contains(">100%<"), "{html}");
    assert!(
        html.contains(">Dismiss<") && !html.contains("Cancel pull"),
        "{html}"
    );

    let mut failed = running.clone();
    failed.phase = PullPhase::Failed("pull access denied for nope".to_string());
    let html = card(failed);
    assert!(html.contains("Could not pull alpine:3"), "{html}");
    assert!(html.contains("pull access denied for nope"), "{html}");
    assert!(!html.contains("progress-track"), "{html}");

    let mut cancelled = running;
    cancelled.phase = PullPhase::Cancelled;
    let html = card(cancelled);
    assert!(html.contains("Cancelled the pull of alpine:3"), "{html}");
}

// ---------------------------------------------------------------------
// Column tiers
// ---------------------------------------------------------------------

/// The table's width next to the inspector at `stored` px in a `window` px
/// wide window at `scale` text, as the dock gives it (AD21, AD24).
fn table_width_with_inspector(window: f32, stored: u32, scale: f32) -> (f32, bool) {
    match crate::ui::dock::docked_width(f64::from(window), stored, scale) {
        Some(width) => (window - 220.0 - width as f32, true),
        None => (window - 220.0, false),
    }
}

fn assert_table_fits(width: f32, narrow: bool, scale: f32) {
    let view = if narrow { View::Narrow } else { View::Wide };
    let state = images_state_at(view, width, scale);
    let gutters = if narrow { 32.0 } else { 40.0 };
    let used =
        css::min_sum(&state.tracks, scale) + 12.0 * (state.tracks.len() as f32 - 1.0) + gutters;
    assert!(
        used <= width + 0.5,
        "{width} px at {scale} (narrow {narrow}): needs {used} ({:?})",
        state.tracks
    );
}

#[test]
fn images_table_fits_every_window_inspector_width_and_text_size() {
    // The AD24 sweep: windows 1024 / 1220 / 1440, inspector widths 320 to
    // 560, every text size; a docked inspector takes room from the table.
    let mut docked_cases = 0;
    for window in [1024.0, 1220.0, 1440.0] {
        for stored in [320, 360, 400, 480, 560] {
            for scale in STEPS {
                let (width, docked) = table_width_with_inspector(window, stored, scale);
                docked_cases += usize::from(docked);
                assert_table_fits(width, window < 1200.0, scale);
            }
        }
    }
    assert!(docked_cases > 30, "{docked_cases} docked cases");
}

#[test]
fn images_table_fits_the_main_area_floor_at_every_text_size() {
    for scale in STEPS {
        let floor = crate::ui::dock::main_min_px(scale) as f32;
        assert_table_fits(floor, false, scale);
        assert!(
            images_least_need(View::Wide, scale) <= floor + 0.5,
            "{scale}"
        );
    }
}

#[test]
fn images_table_keeps_every_column_at_1440_with_no_inspector_and_drops_them_in_order() {
    let full = images_state_at(View::Wide, 1220.0, 1.0);
    assert_eq!((full.step, full.hidden.clone()), (0, vec![]));
    // Narrower: Created goes first, then ID, then Arch.
    let mut seen: Vec<Vec<ImageCol>> = Vec::new();
    for width in (400..1220).rev().step_by(2) {
        let hidden = images_state_at(View::Wide, width as f32, 1.0).hidden;
        if seen.last() != Some(&hidden) {
            seen.push(hidden);
        }
    }
    assert_eq!(
        seen,
        vec![
            vec![],
            vec![ImageCol::Created],
            vec![ImageCol::Created, ImageCol::Id],
            vec![ImageCol::Created, ImageCol::Id, ImageCol::Arch],
        ]
    );
    // At 150 % the 860 px table beside a 360 px inspector at 1440 is gone
    // (it overlays), but a 1024 window already needs the first drops.
    assert!(images_state_at(View::Narrow, 1024.0 - 220.0, 1.5).step >= 1);
}

#[test]
fn images_stylesheet_uses_only_tokens_and_scales_every_text_track() {
    let tiers = include_str!("../../assets/css/images_tiers.css");
    assert!(tiers.contains("@container images (max-width:"), "{tiers}");
    let hand = include_str!("../../assets/css/images.css");
    let body = css::block(hand, "\n.im-table {");
    for name in [
        "--im-cols",
        "--im-cols-no-created",
        "--im-cols-no-id",
        "--im-cols-compact",
    ] {
        let list = css::declaration(body, name);
        let tracks = css::tracks(&list);
        // The name track flexes; every other track holds text, numbers or
        // buttons and grows with the text size.
        for (i, track) in tracks.iter().enumerate().skip(1) {
            assert!(
                css::has_scale(track),
                "track {i} `{track}` of {name} needs a scale term"
            );
        }
    }
}

// ---------------------------------------------------------------------
// Delete: the dialog and where it opens
// ---------------------------------------------------------------------

#[test]
fn images_delete_model_is_one_remove_of_one_image_with_one_effect() {
    let w = world();
    let default = engine_id("colima:default");
    let mut one = image("aa11bb22cc33dd", &["berthfx/old:1"], 57, Arch::Aarch64, 0);
    let model = crate::services::confirm::delete_image_model(&default, &one);
    assert_eq!(model.title, "Delete image berthfx/old:1?");
    assert_eq!(
        model.action,
        berth_core::EngineAction::Remove {
            engine: default.clone(),
            target: berth_core::ResourceRef::Image(one.id.clone()),
            force: false,
        }
    );
    assert_eq!(
        model.targets,
        vec![berth_core::Subject::Image(one.id.clone())]
    );
    assert_eq!(model.effect.count, 1);
    // The dialog's button, rows and the result all come from one summary.
    assert_eq!(
        crate::services::confirm::button_label(&model.effect),
        "Delete image"
    );
    assert_eq!(
        crate::services::confirm::result_label(&model.effect, 1),
        "Deleted 1 image"
    );
    assert_eq!(model.effect.bytes, Some(bytesize::ByteSize::mb(57)));
    let texts: Vec<&str> = model.effect.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts[0],
        "Image berthfx/old:1 (57 MB) is deleted from this engine."
    );
    // Several tags go together, which Docker calls force.
    one.repo_tags.push("berthfx/old:2".to_string());
    let model = crate::services::confirm::delete_image_model(&default, &one);
    assert!(matches!(
        model.action,
        berth_core::EngineAction::Remove { force: true, .. }
    ));
    assert!(model
        .effect
        .lines
        .iter()
        .any(|l| l.text == "All 2 of its tags go with it."));
    // An untagged image is named by its id.
    let bare = image("0123456789abcdef", &[], 5, Arch::Aarch64, 0);
    let model = crate::services::confirm::delete_image_model(&default, &bare);
    assert_eq!(model.title, "Delete image 0123456789ab?");
    let _ = w;
}

thread_local! {
    static OPEN: std::cell::RefCell<Option<(AppState, ImageStore)>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn OpenHost() -> Element {
    let state = crate::state::use_app_state();
    let store = ImageStore::new();
    use_context_provider(|| state);
    use_context_provider(|| store);
    use_hook(move || OPEN.with(|slot| *slot.borrow_mut() = Some((state, store))));
    rsx! { div {} }
}

fn open_host() -> (VirtualDom, AppState, ImageStore) {
    let mut dom = VirtualDom::new(OpenHost);
    let _ = dom.rebuild_to_vec();
    let (state, store) = OPEN
        .with(|slot| slot.borrow_mut().take())
        .expect("captured");
    (dom, state, store)
}

fn confirm_titles(dom: &VirtualDom, state: AppState) -> Vec<String> {
    dom.in_runtime(|| {
        state
            .overlays
            .peek()
            .iter()
            .filter_map(|o| match o {
                Overlay::Confirm(m) => Some(m.title.clone()),
                _ => None,
            })
            .collect()
    })
}

#[test]
fn images_open_delete_asks_first_for_an_unused_image_and_never_for_a_used_one() {
    let (dom, mut state, store) = open_host();
    let default = engine_id("colima:default");
    let free = image("f1f1f1f1f1f1", &["berthfx/free:1"], 9, Arch::Aarch64, 0);
    let busy = image("b2b2b2b2b2b2", &["berthfx/busy:1"], 9, Arch::Aarch64, 1);
    dom.in_scope(ScopeId::APP, || {
        store.seed(&default, vec![free.clone(), busy.clone()]);
        open_delete(&mut state, &store, &default, &busy.id);
    });
    assert!(
        confirm_titles(&dom, state).is_empty(),
        "a used image has no dialog"
    );
    dom.in_scope(ScopeId::APP, || {
        open_delete(&mut state, &store, &default, &free.id)
    });
    assert_eq!(
        confirm_titles(&dom, state),
        ["Delete image berthfx/free:1?"]
    );
    // A second press over the open dialog adds nothing.
    dom.in_scope(ScopeId::APP, || {
        open_delete(&mut state, &store, &default, &free.id)
    });
    assert_eq!(confirm_titles(&dom, state).len(), 1);
    // An image the engine no longer lists has none.
    let (dom, mut state, store) = open_host();
    dom.in_scope(ScopeId::APP, || {
        open_delete(&mut state, &store, &default, "sha256:gone")
    });
    assert!(confirm_titles(&dom, state).is_empty());
}

#[test]
fn images_backspace_on_the_selected_image_opens_the_dialog_only_in_the_images_section() {
    let (dom, mut state, store) = open_host();
    let default = engine_id("colima:default");
    let free = image("f1f1f1f1f1f1", &["berthfx/free:1"], 9, Arch::Aarch64, 0);
    dom.in_scope(ScopeId::APP, || {
        store.seed(&default, vec![free.clone()]);
        state.selection.set(Selection::Image(free.id.clone()));
        state.section.set(crate::state::Section::Containers);
        crate::keys::dispatch(&mut state, "image.delete");
    });
    assert!(confirm_titles(&dom, state).is_empty());
    dom.in_scope(ScopeId::APP, || {
        state.section.set(crate::state::Section::Images);
        crate::keys::dispatch(&mut state, "image.delete");
    });
    assert_eq!(
        confirm_titles(&dom, state),
        ["Delete image berthfx/free:1?"]
    );
}

#[test]
fn images_scene_confirm_delete_shows_the_effect_and_a_button_that_repeats_it() {
    let html = markup("Images / confirm delete / 1440");
    assert!(html.contains("Delete image postgres:17?"), "{html}");
    assert!(
        html.contains("Image postgres:17 (157 MB) is deleted from this engine."),
        "{html}"
    );
    assert!(html.contains("Removes 157 MB"), "{html}");
    assert!(
        html.contains("<span class=\"btn-label\">Delete image</span>"),
        "{html}"
    );
    assert!(html.contains("Pulling it again brings it back."), "{html}");
}
