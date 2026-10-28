//! P9-T5 tests for plugin views: the two Coolify frames as markup (rendered
//! from the plugin's recorded answers), every `View` kind drawn by the
//! screen's components, and the pure rules behind them.

use berth_plugin_protocol::{
    ActionOption, ActionSpec, Badge as ProtoBadge, Cell, Column, ConfigField, Confirm, FieldKind,
    Row, Tab, Tone, View,
};
use chrono::TimeZone;

use super::*;
use crate::scenes;
use crate::screens::plugin_action::request_action;
use crate::screens::plugin_view_model::{
    banner_tone, confirm_view, facets, failing_seq, filter_rows, grid_template, log_rows,
    row_context, row_summary, status_banner, time_text,
};
use crate::services::plugin_views::{LogState, RowContext};
use crate::state::use_app_state;

const DEPLOYMENTS: &str = "Coolify / deployments / 1440";
const CONFIRM: &str = "Coolify / confirm redeploy / 1440";

/// A scene's markup without its stylesheets (class names in assertions mean
/// elements, not rules).
fn markup(frame: &str) -> String {
    let mut out = scenes::render_scene_html(frame).expect("the frame has a scene");
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

fn strip_styles(mut html: String) -> String {
    while let (Some(a), Some(b)) = (html.find("<style>"), html.find("</style>")) {
        if b < a {
            break;
        }
        html.replace_range(a..b + "</style>".len(), "");
    }
    html
}

/// Occurrences of `needle` in `haystack`.
fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

// -- fixtures -------------------------------------------------------------

fn column(key: &str, label: &str) -> Column {
    Column {
        key: key.to_string(),
        label: label.to_string(),
        mono: false,
        align_right: false,
    }
}

fn action(id: &str, label: &str, danger: bool, confirm: Option<Confirm>) -> ActionSpec {
    ActionSpec {
        id: id.to_string(),
        label: label.to_string(),
        danger,
        params: serde_json::json!({ "uuid": "u-1" }),
        confirm,
        options: Vec::new(),
    }
}

fn row(id: &str, name: &str, status: &str, tone: Tone, actions: Vec<ActionSpec>) -> Row {
    Row {
        id: id.to_string(),
        cells: [
            ("name".to_string(), Cell::Text(name.to_string())),
            (
                "status".to_string(),
                Cell::Badge(ProtoBadge {
                    text: status.to_string(),
                    tone,
                }),
            ),
            ("sha".to_string(), Cell::Mono("c2a7e94".to_string())),
            (
                "when".to_string(),
                Cell::Time("2026-10-24T09:28:00.000000Z".to_string()),
            ),
        ]
        .into_iter()
        .collect(),
        open: None,
        actions,
        group: None,
    }
}

fn table() -> View {
    View::Table {
        title: "Things".to_string(),
        columns: vec![
            column("name", "Name"),
            column("status", "Status"),
            Column {
                mono: true,
                ..column("sha", "Commit")
            },
            column("when", "Started"),
        ],
        rows: vec![
            row(
                "a",
                "alpha",
                "Running",
                Tone::Success,
                vec![action("stop", "Stop", true, None)],
            ),
            row("b", "beta", "Failed", Tone::Danger, Vec::new()),
        ],
        empty: "Nothing here.".to_string(),
        groups: Vec::new(),
    }
}

fn detail() -> View {
    View::Detail {
        title: "landing".to_string(),
        status: Some(ProtoBadge {
            text: "Running".to_string(),
            tone: Tone::Success,
        }),
        fields: vec![
            ("Branch".to_string(), "main".to_string()),
            ("Address".to_string(), "https://landing.example".to_string()),
        ],
        tabs: vec![
            Tab {
                id: "logs".to_string(),
                title: "Logs".to_string(),
                view: "application/u-1/logs".to_string(),
            },
            Tab {
                id: "env".to_string(),
                title: "Env".to_string(),
                view: "application/u-1/envs".to_string(),
            },
        ],
        actions: vec![
            action("deploy", "Deploy", false, None),
            action("stop", "Stop", true, None),
        ],
    }
}

/// Draws `content` as a plugin view inside the app's contexts.
#[component]
fn Harness(content: View, log: Option<LogState>) -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let store = crate::services::plugin_views::provide_plugin_view_store();
    use_hook(|| {
        if let Some(log) = log.clone() {
            store.seed_log("demo", "demo/stream", log);
        }
    });
    rsx! {
        ViewContent { plugin: "demo".to_string(), content }
    }
}

/// The wall clock a UTC instant shows in this machine's zone: the log rows
/// print local time, so the expected text depends on where the tests run.
fn local_clock(utc: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(utc)
        .expect("a valid instant")
        .with_timezone(&chrono::Local)
        .format("%H:%M:%S")
        .to_string()
}

fn draw(content: View, log: Option<LogState>) -> String {
    let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { content, log });
    let _ = dom.rebuild_to_vec();
    strip_styles(dioxus_ssr::render(&dom))
}

fn log_state(texts: &[&str], status: Option<&str>) -> LogState {
    let at = chrono::Utc
        .with_ymd_and_hms(2026, 10, 24, 14, 30, 41)
        .single()
        .expect("a time");
    LogState {
        lines: crate::screens::plugin_view_model::stamped(at, texts),
        final_status: status.map(str::to_string),
        dropped: 0,
        version: 1,
    }
}

// -- the frames -----------------------------------------------------------

#[test]
fn plugin_view_frame_deployments_lists_the_twelve_rows_in_the_section() {
    let html = markup(DEPLOYMENTS);
    assert!(
        html.contains("<h1 class=\"pv-title\">Deployments</h1>"),
        "{html}"
    );
    assert!(html.contains(">12</span>"), "the count badge reads 12");
    assert_eq!(count(&html, "class=\"vt-row\""), 12, "{html}");
    for header in ["App", "Status", "Commit", "Message", "Duration", "Trigger"] {
        assert!(
            html.contains(&format!("columnheader\">{header}<")),
            "column {header}"
        );
    }
    // The in-progress row has its Cancel button, the finished rows none.
    assert_eq!(count(&html, "Cancel deployment"), 1);
    // Ten finished, one in progress, one failed (its badge is also in the pane).
    assert_eq!(count(&html, "status-badge tone-success"), 10);
    assert_eq!(count(&html, "status-badge tone-warning"), 1);
    assert_eq!(count(&html, "status-badge tone-info"), 0);
    assert_eq!(
        count(&html, "status-dot-ring"),
        1,
        "in progress draws the ring"
    );
    assert_eq!(count(&html, "status-badge tone-danger"), 2);
}

#[test]
fn plugin_view_frame_deployments_selects_the_failed_row_and_opens_its_log_pane() {
    let html = markup(DEPLOYMENTS);
    assert_eq!(count(&html, "pv-row pv-selected"), 1, "{html}");
    let pane = &html[html.find("<aside").expect("the inspector pane")..];
    assert!(
        pane.contains("<h2 class=\"pv-pane-title\">landing</h2>"),
        "{pane}"
    );
    assert!(pane.contains("status-badge tone-danger"), "{pane}");
    assert!(
        pane.contains("c2a7e94 \u{b7} fix caddy upstream \u{b7} hetzner-1 \u{b7} webhook"),
        "{pane}"
    );
    // The recorded log, 10 lines, three of them the failure.
    assert_eq!(count(pane, "class=\"log-row"), 10, "{pane}");
    assert_eq!(count(pane, "log-row log-error"), 3, "{pane}");
    assert!(pane.contains("Caddyfile:14: unknown directive"), "{pane}");
}

#[test]
fn plugin_view_frame_deployments_adds_the_plugins_group_to_the_sidebar() {
    let html = markup(DEPLOYMENTS);
    let group = html
        .find("sidebar-group-toggle")
        .expect("the Coolify group is in the sidebar");
    let nav = &html[group..html.find("</nav>").expect("nav closes")];
    assert!(nav.contains("<span>Coolify</span>"), "{nav}");
    for item in ["Resources", "Deployments", "Servers"] {
        assert!(nav.contains(&format!(">{item}</span>")), "{item}: {nav}");
    }
    assert_eq!(
        count(nav, "sidebar-item-selected"),
        1,
        "only Deployments is current"
    );
    assert!(nav.contains("aria-current=\"page\""), "{nav}");
}

#[test]
fn plugin_view_frame_confirm_reads_like_the_frame() {
    let html = markup(CONFIRM);
    let dialog = &html[html
        .find("confirm-dialog confirm-dialog-wide")
        .expect("the dialog is drawn")..];
    assert!(dialog.contains(">Redeploy landing?<"), "{dialog}");
    for (label, value) in [
        ("App", "landing"),
        ("Server", "hetzner-1"),
        ("Branch", "main"),
        ("Commit", "c2a7e94 \u{b7} fix caddy upstream"),
    ] {
        let class = if label == "Commit" {
            "confirm-row-text pv-mono"
        } else {
            "confirm-row-text"
        };
        assert!(
            dialog.contains(&format!(
                "confirm-row-label\">{label}</dt><dd class=\"{class}\">{value}<"
            )),
            "{label} {value}: {dialog}"
        );
    }
    // The commit that failed warns, with the reason from its log.
    assert!(dialog.contains("confirm-warning tone-warning"), "{dialog}");
    assert!(
        dialog.contains("This commit failed (Caddyfile:14: unknown directive"),
        "{dialog}"
    );
    assert!(
        dialog.contains("Force rebuild (ignore the build cache)"),
        "{dialog}"
    );
    assert!(
        dialog.contains("Keeps serving the current version until a new build passes."),
        "{dialog}"
    );
    assert!(dialog.contains("aria-modal=\"true\""), "{dialog}");
    // A warned redeploy is a plain button, not the primary one.
    assert!(
        dialog.contains("btn btn-secondary size-md\" type=\"button\" aria-busy=\"false\"><span class=\"btn-label\">Redeploy anyway"),
        "{dialog}"
    );
    // Cancel, the confirm button (worded for the failed commit) and Fix with.
    for button in ["Cancel", "Redeploy anyway", "Fix with Claude Code"] {
        assert!(
            dialog.contains(&format!(">{button}<")),
            "{button}: {dialog}"
        );
    }
    assert!(dialog.contains("pv-fix-chevron"), "{dialog}");
    // The scene has no assistant service, so the label half is disabled and
    // the chevron half must read disabled with it.
    let chevron = &dialog[dialog.find("pv-fix-chevron").expect("the chevron")..];
    let chevron = &chevron[..chevron.find("</button>").expect("it closes")];
    assert!(chevron.contains("disabled"), "{chevron}");
    // It is an overlay now: the scrim of the stack holds it, and the screen
    // under it is still the Deployments table.
    assert!(html.contains("overlay-scrim"), "{html}");
    assert!(html.contains("<h1 class=\"pv-title\">Deployments</h1>"));
}

#[test]
fn plugin_view_frame_confirm_names_where_fix_with_works() {
    let html = markup(CONFIRM);
    assert!(
        html.contains("Fix with Claude Code opens ~/Dev/landing (git remote matches)."),
        "{html}"
    );
}

#[test]
fn plugin_view_frame_deployments_row_buttons_show_on_the_selected_row_and_cancel_always() {
    let html = markup(DEPLOYMENTS);
    // The in-progress row's Cancel is not hidden until hover.
    assert!(
        html.contains("<span class=\"pv-act\"><button class=\"btn btn-secondary size-sm\" type=\"button\" aria-busy=\"false\"><span class=\"btn-label\">Cancel deployment"),
        "{html}"
    );
    // The failed row has Redeploy\u{2026} and Logs; every plain row button hides until hover.
    let selected = &html[html.find("pv-row pv-selected").expect("selected row")..];
    let row = &selected[..selected
        .find("</div></div></div>")
        .unwrap_or(selected.len())];
    assert!(row.contains(">Redeploy\u{2026}<"), "{row}");
    assert!(row.contains(">Logs<"), "{row}");
    assert_eq!(count(row, "pv-act pv-act-hover"), 2, "{row}");
}

#[test]
fn plugin_view_frame_deployments_pane_links_the_folder_and_offers_fix_and_redeploy() {
    let html = markup(DEPLOYMENTS);
    let pane = &html[html.find("<aside").expect("the pane")..];
    assert!(
        pane.contains("Linked folder ~/Dev/landing \u{b7} git remote matches"),
        "{pane}"
    );
    // Fix with the harness, first; then Redeploy; no Logs button (the log is open).
    let fix = pane
        .find("Fix with Claude Code")
        .expect("Fix with is offered");
    let redeploy = pane
        .find(">Redeploy\u{2026}<")
        .expect("Redeploy is offered");
    assert!(fix < redeploy, "{pane}");
    assert!(!pane.contains(">Logs<"), "{pane}");
    // The scene runs no assistant, so the button says why it cannot.
    assert!(pane.contains("The assistant is not connected"), "{pane}");
    // The caption and the times are the frame's.
    assert!(
        pane.contains("<h3 class=\"pv-log-title\">Deployment log</h3>"),
        "{pane}"
    );
    let first = local_clock("2026-10-24T14:30:41Z");
    assert!(
        pane.contains(&format!(">{first}</span>")),
        "no milliseconds: {pane}"
    );
    assert!(!pane.contains(&format!(">{first}.")), "{pane}");
}

#[test]
fn plugin_view_frame_deployments_has_the_server_chip_with_a_remove_button() {
    let html = markup(DEPLOYMENTS);
    assert!(html.contains("pv-chip"), "{html}");
    assert!(html.contains("<span>Server: hetzner-1</span>"), "{html}");
    assert!(
        html.contains("aria-label=\"Remove the Server filter\""),
        "{html}"
    );
}

#[test]
fn plugin_view_no_dialog_without_a_pending_action() {
    let html = markup(DEPLOYMENTS);
    assert!(!html.contains("confirm-dialog"), "{html}");
}

// -- every View kind --------------------------------------------------------

#[test]
fn plugin_view_table_draws_columns_cells_row_buttons_and_the_count() {
    let html = draw(table(), None);
    assert!(
        html.contains("<h1 class=\"pv-title\">Things</h1>"),
        "{html}"
    );
    assert!(html.contains(">2</span>"), "{html}");
    for header in ["Name", "Status", "Commit", "Started"] {
        assert!(
            html.contains(&format!("columnheader\">{header}<")),
            "{header}"
        );
    }
    assert!(
        html.contains("<span class=\"pv-text\" title=\"alpha\">alpha</span>"),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"pv-mono\" title=\"c2a7e94\">c2a7e94</span>"),
        "{html}"
    );
    assert!(html.contains("status-badge tone-success"), "{html}");
    assert!(html.contains("status-badge tone-danger"), "{html}");
    assert!(
        html.contains("title=\"2026-10-24T09:28:00.000000Z\""),
        "the time keeps its exact text: {html}"
    );
    // A row action is a button; a destructive one stays plain in a row.
    assert!(
        html.contains("<span class=\"btn-label\">Stop</span>"),
        "{html}"
    );
    assert!(!html.contains("btn-danger"), "{html}");
}

#[test]
fn plugin_view_table_without_rows_says_what_the_plugin_said() {
    let View::Table {
        title,
        columns,
        empty,
        groups,
        ..
    } = table()
    else {
        unreachable!()
    };
    let html = draw(
        View::Table {
            title,
            columns,
            rows: Vec::new(),
            empty,
            groups,
        },
        None,
    );
    assert!(html.contains("Nothing here."), "{html}");
    assert!(!html.contains("vt-row"), "{html}");
}

#[test]
fn plugin_view_detail_draws_title_status_fields_tabs_and_actions() {
    let html = draw(detail(), None);
    assert!(
        html.contains("<h2 class=\"pv-pane-title\">landing</h2>"),
        "{html}"
    );
    assert!(html.contains("status-badge tone-success"), "{html}");
    assert!(html.contains("kv-list"), "{html}");
    assert!(
        html.contains(">Branch<") && html.contains(">main<"),
        "{html}"
    );
    assert!(html.contains("role=\"tablist\""), "{html}");
    assert!(html.contains(">Logs<") && html.contains(">Env<"), "{html}");
    // The first action is the primary one, a destructive one is Danger.
    assert!(html.contains("btn btn-primary"), "{html}");
    assert!(html.contains("btn btn-danger"), "{html}");
}

#[test]
fn plugin_view_log_stream_draws_rows_and_marks_the_failure() {
    let log = log_state(
        &[
            "Step 1/2 FROM node:22",
            "Error: build failed",
            "Deployment failed at step 2 of 2",
        ],
        Some("failed"),
    );
    let html = draw(
        View::LogStream {
            stream: "demo/stream".to_string(),
            title: "Deployment logs".to_string(),
        },
        Some(log),
    );
    assert_eq!(count(&html, "class=\"log-row"), 3, "{html}");
    assert_eq!(count(&html, "log-row log-error"), 2, "{html}");
    assert!(
        !html.contains("pv-log-status"),
        "a finished log says nothing about itself: {html}"
    );
    let first = local_clock("2026-10-24T14:30:41Z");
    assert!(html.contains(&format!(">{first}</span>")), "{html}");
    assert!(html.contains("id=\"pv-log-demo-stream\""), "{html}");
}

#[test]
fn plugin_view_log_stream_of_a_running_deployment_says_it_is_streaming() {
    let html = draw(
        View::LogStream {
            stream: "demo/stream".to_string(),
            title: "Deployment logs".to_string(),
        },
        Some(log_state(&["Building"], None)),
    );
    assert!(html.contains("Streaming"), "{html}");
    assert!(!html.contains("log-error"), "{html}");
}

#[test]
fn plugin_view_message_draws_a_banner_and_its_action() {
    let html = draw(
        View::Message {
            tone: Tone::Warning,
            text: "Set the Coolify address in Plugins > Configure.".to_string(),
            action: Some(action("open", "Open Plugins", false, None)),
        },
        None,
    );
    assert!(html.contains("banner tone-warning"), "{html}");
    assert!(html.contains("Set the Coolify address"), "{html}");
    assert!(html.contains("Open Plugins"), "{html}");
}

#[test]
fn plugin_view_form_draws_its_fields_by_kind_and_a_submit_button() {
    let html = draw(
        View::Form {
            title: "Connect".to_string(),
            fields: vec![
                ConfigField {
                    key: "url".to_string(),
                    label: "Address".to_string(),
                    kind: FieldKind::Url,
                    required: true,
                },
                ConfigField {
                    key: "token".to_string(),
                    label: "Token".to_string(),
                    kind: FieldKind::Secret,
                    required: false,
                },
                ConfigField {
                    key: "verbose".to_string(),
                    label: "Verbose".to_string(),
                    kind: FieldKind::Bool,
                    required: false,
                },
            ],
            submit: action("connect", "Connect", false, None),
        },
        None,
    );
    assert!(html.contains("Address (required)"), "{html}");
    assert!(html.contains("type=\"url\""), "{html}");
    assert!(
        html.contains("type=\"password\""),
        "a secret is masked: {html}"
    );
    assert!(html.contains("role=\"switch\""), "{html}");
    assert!(
        html.contains("<span class=\"btn-label\">Connect</span>"),
        "{html}"
    );
    // Uncontrolled inputs: nothing is bound to a value.
    assert!(!html.contains(" value=\""), "{html}");
}

// -- the version banner (AD51) ------------------------------------------------

#[test]
fn plugin_view_status_warning_is_a_visible_banner_and_info_is_not() {
    let warning = View::Message {
        tone: Tone::Warning,
        text: "Coolify 5.0.0 \u{2014} Built for Coolify 4.5; some actions may fail".to_string(),
        action: None,
    };
    let (tone, text) = status_banner(&warning).expect("a warning shows");
    assert_eq!(tone, crate::ui::BannerTone::Warning);
    assert!(text.contains("Built for Coolify 4.5"), "{text}");
    let info = View::Message {
        tone: Tone::Info,
        text: "Coolify v4.5.0".to_string(),
        action: None,
    };
    assert_eq!(status_banner(&info), None);
    assert_eq!(status_banner(&table()), None);
}

#[test]
fn plugin_view_section_shows_the_status_warning_as_a_banner() {
    // The deployments scene with the status view replaced by the plugin's
    // newer-major warning.
    #[component]
    fn Section() -> Element {
        let state = use_app_state();
        use_context_provider(|| state);
        let plugins = crate::services::plugins::provide_plugins_view();
        let store = crate::services::plugin_views::provide_plugin_view_store();
        use_hook(|| {
            let mut entries = plugins.entries;
            entries.set(vec![scenes::enabled_entry()]);
            store.seed_view(
                "coolify",
                "status",
                crate::services::plugin_views::ViewState::Ready(View::Message {
                    tone: Tone::Warning,
                    text: "Coolify 5.0.0 \u{2014} Built for Coolify 4.5; some actions may fail"
                        .to_string(),
                    action: None,
                }),
            );
            store.seed_view(
                "coolify",
                "servers",
                crate::services::plugin_views::ViewState::Loading,
            );
            let mut nav = store.nav;
            nav.set(crate::services::plugin_views::PluginNav {
                plugin: "coolify".to_string(),
                view: "servers".to_string(),
                row: None,
                open: None,
            });
        });
        rsx! { PluginViewScreen {} }
    }
    let mut dom = VirtualDom::new(Section);
    let _ = dom.rebuild_to_vec();
    let html = strip_styles(dioxus_ssr::render(&dom));
    assert!(html.contains("class=\"pv-banner\""), "{html}");
    assert!(html.contains("banner tone-warning"), "{html}");
    assert!(
        html.contains("Built for Coolify 4.5; some actions may fail"),
        "{html}"
    );
}

#[test]
fn plugin_view_section_of_a_plugin_that_is_not_enabled_says_so() {
    #[component]
    fn Section() -> Element {
        let state = use_app_state();
        use_context_provider(|| state);
        crate::services::plugins::provide_plugins_view();
        let store = crate::services::plugin_views::provide_plugin_view_store();
        use_hook(|| {
            let mut nav = store.nav;
            nav.set(crate::services::plugin_views::PluginNav {
                plugin: "coolify".to_string(),
                view: "servers".to_string(),
                row: None,
                open: None,
            });
        });
        rsx! { PluginViewScreen {} }
    }
    let mut dom = VirtualDom::new(Section);
    let _ = dom.rebuild_to_vec();
    let html = strip_styles(dioxus_ssr::render(&dom));
    assert!(html.contains("This plugin is not running"), "{html}");
    assert!(!html.contains("pv-table"), "{html}");
}

// -- pure rules ---------------------------------------------------------------

#[test]
fn plugin_view_filter_matches_every_word_in_any_cell() {
    let View::Table { rows, .. } = table() else {
        unreachable!()
    };
    assert_eq!(filter_rows(&rows, "").len(), 2);
    assert_eq!(filter_rows(&rows, "alpha")[0].id, "a");
    assert_eq!(filter_rows(&rows, "FAILED beta")[0].id, "b");
    assert!(
        filter_rows(&rows, "alpha failed").is_empty(),
        "words must meet in one row"
    );
    assert_eq!(filter_rows(&rows, "c2a7").len(), 2, "mono cells count too");
}

#[test]
fn plugin_view_grid_gives_short_columns_a_fixed_width_and_long_ones_the_rest() {
    let View::Table { columns, rows, .. } = table() else {
        unreachable!()
    };
    let grid = grid_template(&columns, &rows, false);
    assert_eq!(count(&grid, "calc("), 4, "{grid}");
    // A time column and a badge column have the widths their cells need.
    assert!(
        grid.contains("calc(112px * var(--text-scale, 1))"),
        "{grid}"
    );
    assert!(
        grid.contains("calc(89px * var(--text-scale, 1))"),
        "Running (7 chars) needs 7 * 7 + 40: {grid}"
    );
    assert!(
        !grid.contains("fr"),
        "every column of this table is short: {grid}"
    );
    let long = Row {
        cells: [(
            "name".to_string(),
            Cell::Text("a-rather-long-application-name".to_string()),
        )]
        .into_iter()
        .collect(),
        ..rows[0].clone()
    };
    let grid = grid_template(&columns, &[long], false);
    assert!(
        grid.contains(", 30fr)"),
        "a long text column takes the free space, by its length: {grid}"
    );
    // With row buttons there is one more track, as wide for every row.
    let with = grid_template(&columns, &rows, true);
    assert_eq!(with.matches(") ").count() + 1, 5, "{with}");
}

#[test]
fn plugin_view_time_cells_read_as_local_time_and_fall_back_to_the_text() {
    let utc = chrono::Utc;
    assert_eq!(
        time_text("2026-10-24T09:28:00.000000Z", &utc),
        "Oct 24, 09:28"
    );
    assert_eq!(time_text("yesterday", &utc), "yesterday");
}

#[test]
fn plugin_view_row_summary_titles_with_the_first_cell_and_lists_the_rest() {
    let View::Table { columns, rows, .. } = table() else {
        unreachable!()
    };
    let summary = row_summary(&columns, &rows[1]);
    assert_eq!(summary.title, "beta");
    assert_eq!(
        summary.badge.as_ref().map(|b| b.text.as_str()),
        Some("Failed")
    );
    assert!(summary.line.starts_with("c2a7e94"), "{}", summary.line);
}

#[test]
fn plugin_view_confirm_lines_split_into_rows_a_warning_and_notes() {
    let shown = confirm_view(&Confirm {
        title: "Redeploy landing?".to_string(),
        lines: vec![
            "Application: landing".to_string(),
            "Server: hetzner-1".to_string(),
            "Warning: This commit failed 4 min ago at step 3.".to_string(),
            "Keeps serving the old version until a new build passes.".to_string(),
            "Branch: main".to_string(),
        ],
        button: "Redeploy anyway".to_string(),
    });
    assert_eq!(
        shown.rows,
        [
            ("Application", "landing"),
            ("Server", "hetzner-1"),
            ("Branch", "main")
        ]
        .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    assert_eq!(shown.warnings, ["This commit failed 4 min ago at step 3."]);
    assert_eq!(
        shown.notes,
        ["Keeps serving the old version until a new build passes."]
    );
    assert_eq!(shown.button, "Redeploy anyway");
    // A sentence with a colon in it is a note, not a row.
    let sentence = confirm_view(&Confirm {
        title: "t".to_string(),
        lines: vec!["This will do two things: stop it and start it.".to_string()],
        button: "Go".to_string(),
    });
    assert!(sentence.rows.is_empty(), "{sentence:?}");
    assert_eq!(sentence.notes.len(), 1);
}

#[test]
fn plugin_view_failing_step_is_the_first_error_line_of_a_failed_log_only() {
    let failed = log_state(
        &[
            "Step 3/6 RUN caddy validate",
            "Error: Caddyfile:14: unknown directive",
            "Error: validate failed with exit code 1",
            "Deployment failed at step 3 of 6",
            "Rollback: c0ffee1 is still serving traffic",
        ],
        Some("failed"),
    );
    let rows = log_rows(&failed);
    let seq = failing_seq(&rows, &failed).expect("failed logs open on their failure");
    assert_eq!(
        rows[seq as usize].row.text,
        "Error: Caddyfile:14: unknown directive"
    );
    // "failed" in a line marks it once the stream failed, not before.
    assert_eq!(
        rows.iter()
            .filter(|r| r.tone == crate::ui::log_line::RowTone::Error)
            .count(),
        3
    );
    let running = log_state(&["Error: transient", "retrying"], None);
    assert_eq!(failing_seq(&log_rows(&running), &running), None);
    let finished = log_state(&["Error: warned but passed"], Some("finished"));
    assert_eq!(failing_seq(&log_rows(&finished), &finished), None);
    // A failure with nothing that reads as an error opens on the last line.
    let quiet = log_state(&["one", "two"], Some("failed"));
    let rows = log_rows(&quiet);
    assert_eq!(failing_seq(&rows, &quiet), Some(rows[1].row.seq));
}

#[test]
fn plugin_view_banner_tones_follow_the_protocol_tones() {
    assert_eq!(banner_tone(&Tone::Danger), crate::ui::BannerTone::Danger);
    assert_eq!(banner_tone(&Tone::Neutral), crate::ui::BannerTone::Info);
    assert_eq!(banner_tone(&Tone::Success), crate::ui::BannerTone::Success);
}

#[test]
fn plugin_view_scroll_script_targets_the_failing_row_or_the_end_of_a_running_log() {
    let to_failure = scroll_script("pv-log-x", Some(5), false);
    assert!(to_failure.contains("[data-seq=\"5\"]"), "{to_failure}");
    assert!(to_failure.contains("scrollIntoView"), "{to_failure}");
    let follow = scroll_script("pv-log-x", None, true);
    assert!(follow.contains("scrollTop = el.scrollHeight"), "{follow}");
    assert_eq!(scroll_script("pv-log-x", None, false), "");
}

#[test]
fn plugin_view_action_with_a_confirm_waits_and_one_without_does_not() {
    // request_action routes on `confirm`; the dialog's own button and the
    // store decide the rest. The routing is the pure part.
    let needs = action(
        "stop",
        "Stop",
        true,
        Some(Confirm {
            title: "Stop it?".to_string(),
            lines: Vec::new(),
            button: "Stop".to_string(),
        }),
    );
    assert!(needs.confirm.is_some());
    assert!(action("start", "Start", false, None).confirm.is_none());
    let with_option = ActionSpec {
        options: vec![ActionOption {
            key: "force".to_string(),
            label: "Force rebuild".to_string(),
            default: false,
        }],
        ..needs
    };
    assert_eq!(with_option.options.len(), 1);
}

// -- project badges ---------------------------------------------------------------

#[component]
fn RowWithBadge(model: crate::ui::project_row::ProjectRowModel, badge: bool) -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let store = crate::services::plugin_views::provide_plugin_view_store();
    use_hook(|| {
        if badge {
            let mut badges = store.badges;
            badges.write().insert(
                model.id.0.clone(),
                crate::services::plugin_views::ProjectBadge {
                    text: "Deployed on Coolify: running".to_string(),
                    tone: Tone::Success,
                },
            );
        }
    });
    rsx! { crate::ui::project_row::ProjectRow { model } }
}

fn project_row_html(badge: bool) -> String {
    use crate::services::actions::StateView;
    let world = berth_engine::MockWorld::bundled();
    let project = world
        .projects
        .iter()
        .find(|p| p.name == "shop-api")
        .expect("shop-api in the mock world");
    let pending = std::collections::BTreeMap::new();
    let view = StateView {
        engines: &world.engines,
        containers: &world.containers,
        projects: &world.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending: &pending,
        backing: crate::services::backing::Backing::default(),
    };
    let model = crate::ui::project_row::row_model(
        project,
        &view,
        &[],
        std::path::Path::new("/Users/berth"),
    );
    let mut dom = VirtualDom::new_with_props(RowWithBadge, RowWithBadgeProps { model, badge });
    let _ = dom.rebuild_to_vec();
    strip_styles(dioxus_ssr::render(&dom))
}

#[test]
fn plugin_view_project_row_shows_the_badge_a_plugin_gave_it() {
    let html = project_row_html(true);
    assert!(
        html.contains("project-plugin-badge status-badge tone-success"),
        "{html}"
    );
    assert!(html.contains("Deployed on Coolify: running"), "{html}");
}

#[test]
fn plugin_view_project_row_without_a_badge_shows_none() {
    let html = project_row_html(false);
    assert!(!html.contains("project-plugin-badge"), "{html}");
}

// -- rows, chips and actions -------------------------------------------------------

#[test]
fn plugin_view_row_context_reads_title_repository_stream_and_failure() {
    let View::Table {
        columns, mut rows, ..
    } = table()
    else {
        unreachable!()
    };
    rows[1].open = Some("deployment/d1/logs".to_string());
    rows[1].cells.insert(
        "repository".to_string(),
        Cell::Text("example/landing".to_string()),
    );
    let failed = row_context(&columns, &rows[1]);
    assert_eq!(failed.row, "b");
    assert_eq!(failed.title, "beta");
    assert_eq!(failed.repository.as_deref(), Some("example/landing"));
    assert_eq!(failed.stream.as_deref(), Some("deployment/d1/logs"));
    assert!(failed.failed, "its badge is danger");
    let fine = row_context(&columns, &rows[0]);
    assert!(!fine.failed);
    assert_eq!(fine.repository, None);
    assert_eq!(fine.stream, None, "a row that opens no log has no stream");
    // The repository is for linking, not for the muted line.
    assert!(!row_summary(&columns, &rows[1])
        .line
        .contains("example/landing"));
}

#[test]
fn plugin_view_a_fact_every_row_shares_is_a_chip_and_a_varying_one_is_not() {
    let View::Table {
        columns, mut rows, ..
    } = table()
    else {
        unreachable!()
    };
    for row in &mut rows {
        row.cells
            .insert("server".to_string(), Cell::Text("hetzner-1".to_string()));
        row.cells
            .insert("zone".to_string(), Cell::Text(row.id.clone()));
    }
    let chips = facets(&columns, &rows);
    assert_eq!(chips.len(), 1, "{chips:?}");
    assert_eq!(chips[0].label, "Server");
    assert_eq!(chips[0].value, "hetzner-1");
    // A column is not a facet, and an empty table has none.
    assert!(facets(&columns, &[]).is_empty());
}

#[component]
fn ActionProbe(action: ActionSpec, context: Option<RowContext>) -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let store = crate::services::plugin_views::provide_plugin_view_store();
    use_hook(|| {
        request_action(store, None, state, "demo", &action, context.clone());
    });
    let overlays = state.overlays.read();
    let shown = match overlays.last() {
        Some(crate::state::Overlay::PluginAction(p)) => format!("overlay:{}", p.action.id),
        Some(_) => "other overlay".to_string(),
        None => "no overlay".to_string(),
    };
    let nav = store.nav.read();
    rsx! { span { "{shown}|{nav.open:?}|{nav.row:?}" } }
}

fn probe_action(action: ActionSpec, context: Option<RowContext>) -> String {
    let mut dom = VirtualDom::new_with_props(ActionProbe, ActionProbeProps { action, context });
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

fn row_ctx() -> RowContext {
    RowContext {
        row: "dep-1".to_string(),
        title: "landing".to_string(),
        repository: None,
        stream: None,
        failed: false,
    }
}

#[test]
fn plugin_view_an_action_with_a_confirm_opens_the_plugin_action_overlay() {
    let confirm = Confirm {
        title: "Stop it?".to_string(),
        lines: vec!["App: landing".to_string()],
        button: "Stop".to_string(),
    };
    let html = probe_action(action("stop", "Stop", true, Some(confirm)), None);
    assert!(html.contains("overlay:stop"), "{html}");
}

#[test]
fn plugin_view_an_action_that_only_opens_a_view_opens_it_and_keeps_the_row() {
    let mut logs = action("open-logs", "Logs", false, None);
    logs.params = serde_json::json!({ "open": "deployment/d1/logs" });
    let html = probe_action(logs, Some(row_ctx()));
    assert!(
        html.contains("no overlay"),
        "no dialog and no plugin call: {html}"
    );
    assert!(
        html.contains("Some(&#34;deployment/d1/logs&#34;)"),
        "{html}"
    );
    assert!(
        html.contains("Some(&#34;dep-1&#34;)"),
        "the row stays selected: {html}"
    );
}

#[test]
fn plugin_view_an_action_without_a_confirm_asks_nothing() {
    let html = probe_action(action("start", "Start", false, None), None);
    assert!(html.contains("no overlay"), "{html}");
}

fn dialog_html(failed: bool) -> String {
    #[component]
    fn Dialog(failed: bool) -> Element {
        let state = use_app_state();
        use_context_provider(|| state);
        crate::services::plugin_views::provide_plugin_view_store();
        let mut redeploy = action(
            "deploy",
            "Redeploy\u{2026}",
            false,
            Some(Confirm {
                title: "Redeploy landing?".to_string(),
                lines: vec![
                    "App: landing".to_string(),
                    "Commit: c2a7e94 \u{b7} fix".to_string(),
                    "Keeps serving the current version.".to_string(),
                ],
                button: "Redeploy".to_string(),
            }),
        );
        redeploy.options = vec![ActionOption {
            key: "force".to_string(),
            label: "Force rebuild".to_string(),
            default: false,
        }];
        let mut context = row_ctx();
        context.failed = failed;
        rsx! {
            crate::screens::plugin_action::PluginActionDialog {
                pending: crate::services::plugin_views::PendingAction {
                    plugin: "demo".to_string(),
                    action: redeploy,
                    context: Some(context),
                },
            }
        }
    }
    let mut dom = VirtualDom::new_with_props(Dialog, DialogProps { failed });
    let _ = dom.rebuild_to_vec();
    strip_styles(dioxus_ssr::render(&dom))
}

#[test]
fn plugin_view_the_dialog_of_a_good_commit_says_redeploy_and_has_no_fix_button() {
    let html = dialog_html(false);
    assert!(html.contains("btn btn-primary"), "{html}");
    assert!(html.contains(">Redeploy<"), "{html}");
    assert!(!html.contains("Fix with"), "{html}");
    assert!(!html.contains("confirm-warning"), "{html}");
    assert!(html.contains("Force rebuild"), "{html}");
}

#[test]
fn plugin_view_the_dialog_of_a_failed_row_offers_fix_with() {
    let html = dialog_html(true);
    assert!(html.contains("pv-fix"), "{html}");
}
