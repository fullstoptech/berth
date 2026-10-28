//! P9-T5 tests for the plugin view service: the pure contribution rules, the
//! log fold, the badge answers and the store's slots. The real Coolify plugin
//! is driven through the service in `tests/coolify_views.rs`.

use berth_plugin_host::InstalledPlugin;
use berth_plugin_protocol::{
    ActionOption, Manifest, PaletteCommand, Permission, SidebarEntry, PROTOCOL_VERSION,
};
use chrono::TimeZone;

use super::*;
use crate::state::use_app_state;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 24, 14, 30, 41)
        .single()
        .expect("a time")
}

fn entry(id: &str, state: PluginState, contributes: Vec<Contribution>) -> PluginEntry {
    PluginEntry {
        plugin: InstalledPlugin {
            manifest: Manifest {
                id: id.to_string(),
                name: id.to_uppercase(),
                version: "1.0.0".to_string(),
                protocol: PROTOCOL_VERSION,
                executable: id.to_string(),
                permissions: vec![Permission::ReadEngines],
                contributes,
                config: Vec::new(),
                secrets: Vec::new(),
            },
            dir: std::path::PathBuf::from("/x"),
            state,
            granted: Vec::new(),
        },
        config: json!({}),
        secrets_set: std::collections::BTreeSet::new(),
        crash_log: Vec::new(),
        restarts: 0,
    }
}

fn section() -> Contribution {
    Contribution::SidebarSection {
        id: "demo".to_string(),
        title: "Demo".to_string(),
        items: vec![
            SidebarEntry {
                id: "things".to_string(),
                title: "Things".to_string(),
                view: "things".to_string(),
            },
            SidebarEntry {
                id: "jobs".to_string(),
                title: "Jobs".to_string(),
                view: "jobs".to_string(),
            },
        ],
    }
}

fn commands() -> Contribution {
    Contribution::PaletteCommands {
        commands: vec![PaletteCommand {
            id: "sync".to_string(),
            title: "Sync now".to_string(),
            action: "sync".to_string(),
        }],
    }
}

// -- views and actions ------------------------------------------------------

#[test]
fn plugin_views_parse_view_reads_a_view_and_words_a_failure() {
    let view = parse_view(json!({ "kind": "message", "tone": "info", "text": "hi" }));
    assert!(matches!(view, Ok(View::Message { .. })), "{view:?}");
    let bad = parse_view(json!({ "kind": "carousel" })).expect_err("unknown kind");
    assert!(
        bad.starts_with("The plugin sent a view Berth cannot read"),
        "{bad}"
    );
}

#[test]
fn plugin_views_action_params_set_each_option_to_the_choice_or_its_default() {
    let action = ActionSpec {
        id: "deploy".to_string(),
        label: "Deploy".to_string(),
        danger: false,
        params: json!({ "uuid": "u-1", "kind": "application" }),
        confirm: None,
        options: vec![
            ActionOption {
                key: "force".to_string(),
                label: "Force rebuild".to_string(),
                default: false,
            },
            ActionOption {
                key: "pull".to_string(),
                label: "Pull".to_string(),
                default: true,
            },
        ],
    };
    let untouched = action_params(&action, &BTreeMap::new());
    assert_eq!(
        untouched,
        json!({ "uuid": "u-1", "kind": "application", "force": false, "pull": true })
    );
    let chosen = action_params(&action, &BTreeMap::from([("force".to_string(), true)]));
    assert_eq!(chosen["force"], true);
    assert_eq!(chosen["uuid"], "u-1", "the action's own params stay");
    // An option the action does not have is not invented.
    let stray = action_params(&action, &BTreeMap::from([("nope".to_string(), true)]));
    assert!(stray.get("nope").is_none());
}

// -- the sidebar and the palette ------------------------------------------------

#[test]
fn plugin_views_sidebar_groups_come_from_enabled_plugins_only() {
    let entries = [
        entry("on", PluginState::Enabled, vec![section()]),
        entry("off", PluginState::Disabled, vec![section()]),
        entry(
            "bad",
            PluginState::Error {
                reason: "gone".to_string(),
            },
            vec![section()],
        ),
        entry("quiet", PluginState::Enabled, vec![commands()]),
    ];
    let groups = sidebar_groups(&entries);
    assert_eq!(groups.len(), 1, "{groups:?}");
    assert_eq!(groups[0].plugin, "on");
    assert_eq!(groups[0].title, "Demo");
    let views: Vec<&str> = groups[0].items.iter().map(|i| i.view.as_str()).collect();
    assert_eq!(views, ["things", "jobs"]);
}

#[test]
fn plugin_views_palette_lists_each_sidebar_item_and_each_contributed_command() {
    let entries = [
        entry("demo", PluginState::Enabled, vec![section(), commands()]),
        entry("off", PluginState::Disabled, vec![section(), commands()]),
    ];
    let all = palette_commands(&entries);
    let titles: Vec<&str> = all.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["DEMO: Things", "DEMO: Jobs", "DEMO: Sync now"]);
    assert_eq!(
        all[0].kind,
        CommandKind::Open {
            view: "things".to_string()
        }
    );
    assert_eq!(
        all[2].kind,
        CommandKind::Invoke {
            action: "sync".to_string()
        }
    );
    assert!(
        all.iter().all(|c| c.plugin == "demo"),
        "a disabled plugin adds nothing"
    );
}

#[test]
fn plugin_views_palette_matches_every_word_and_lists_nothing_for_an_empty_query() {
    let all = palette_commands(&[entry(
        "demo",
        PluginState::Enabled,
        vec![section(), commands()],
    )]);
    assert!(match_commands(&all, "").is_empty());
    assert!(match_commands(&all, "   ").is_empty());
    let jobs = match_commands(&all, "demo JOBS");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].title, "DEMO: Jobs");
    assert_eq!(match_commands(&all, "demo").len(), 3);
    assert!(match_commands(&all, "demo nothing").is_empty());
}

// -- logs -----------------------------------------------------------------

#[test]
fn plugin_views_log_state_folds_lines_and_the_final_status() {
    let mut log = LogState::default();
    assert!(log.apply(&json!({ "stream": "s", "lines": ["a", "b"] }), now()));
    assert_eq!(log.lines.len(), 2);
    assert_eq!(log.final_status, None);
    assert!(!log.failed());
    assert!(log.apply(&json!({ "lines": [], "final_status": "failed" }), now()));
    assert_eq!(log.final_status.as_deref(), Some("failed"));
    assert!(log.failed());
    // Nothing new: nothing changes and the version stays.
    let version = log.version;
    assert!(!log.apply(&json!({ "lines": [], "final_status": "failed" }), now()));
    assert_eq!(log.version, version);
    // `finished` is not a failure; a plugin-side `error` is.
    let mut done = LogState::default();
    done.apply(&json!({ "lines": [], "final_status": "finished" }), now());
    assert!(!done.failed());
    let mut broken = LogState::default();
    broken.apply(&json!({ "lines": ["x"], "final_status": "error" }), now());
    assert!(broken.failed());
}

#[test]
fn plugin_views_log_state_keeps_the_newest_lines_and_counts_the_dropped() {
    let mut log = LogState::default();
    let lines: Vec<String> = (0..MAX_LOG_LINES + 5)
        .map(|i| format!("line {i}"))
        .collect();
    log.apply(&json!({ "lines": lines }), now());
    assert_eq!(log.lines.len(), MAX_LOG_LINES);
    assert_eq!(log.dropped, 5);
    assert_eq!(log.lines[0].1, "line 5");
}

// -- badges -----------------------------------------------------------------

fn project(id: &str, remote: Option<&str>) -> berth_core::Project {
    berth_core::Project {
        id: berth_core::ProjectId(id.to_string()),
        name: id.to_string(),
        folder: None,
        compose_files: Vec::new(),
        engine: berth_core::EngineId("colima:default".to_string()),
        source: berth_core::ProjectSource::Compose,
        services: Vec::new(),
        git: remote.map(|r| berth_core::GitInfo {
            branch: None,
            dirty: false,
            remote: Some(r.to_string()),
            ahead: 0,
            behind: 0,
        }),
        status: berth_core::ProjectStatus::Stopped,
        pinned: false,
    }
}

#[test]
fn plugin_views_badge_requests_ask_only_about_projects_with_a_remote() {
    let asked = badge_requests(&[
        project("a", Some("git@github.com:example/a.git")),
        project("b", None),
    ]);
    assert_eq!(
        asked,
        [BadgeRequest {
            id: "a".to_string(),
            git_remote: Some("git@github.com:example/a.git".to_string())
        }]
    );
}

#[test]
fn plugin_views_parse_badges_reads_each_answer_and_skips_the_malformed() {
    let badges = parse_badges(&json!([
        { "id": "a", "text": "Deployed on Coolify: running", "tone": "success" },
        { "id": "b", "text": "Deployed on Coolify: exited", "tone": "danger" },
        { "id": "c", "text": "odd" },
        { "text": "no id", "tone": "info" },
        "not an object",
    ]));
    assert_eq!(badges.len(), 3);
    assert_eq!(badges["a"].tone, Tone::Success);
    assert_eq!(badges["b"].tone, Tone::Danger);
    assert_eq!(badges["c"].tone, Tone::Neutral, "a missing tone is neutral");
    assert!(parse_badges(&json!({ "not": "a list" })).is_empty());
}

// -- the service without a plugin --------------------------------------------

fn service() -> PluginsService {
    PluginsService::new(crate::services::plugins::PluginsDeps {
        store: std::sync::Arc::new(berth_store::Store::open_in_memory().expect("store")),
        secrets: std::sync::Arc::new(berth_store::InMemorySecretStore::default()),
        dirs: Vec::new(),
        path: Some(std::ffi::OsString::new()),
        cancel: CancellationToken::new(),
        backoff: berth_plugin_host::Backoff { delays: Vec::new() },
    })
}

#[tokio::test]
async fn plugin_views_loading_from_a_plugin_that_is_not_running_fails_in_words() {
    let state = load_view(&service(), "coolify", "resources").await;
    let ViewState::Failed(reason) = state else {
        panic!("expected a failure, got {state:?}");
    };
    assert!(reason.contains("not running"), "{reason}");
}

#[tokio::test]
async fn plugin_views_running_an_action_on_a_plugin_that_is_not_running_is_an_error() {
    let action = ActionSpec {
        id: "stop".to_string(),
        label: "Stop".to_string(),
        danger: true,
        params: json!({}),
        confirm: None,
        options: Vec::new(),
    };
    let result = invoke_action(&service(), "coolify", &action, &BTreeMap::new()).await;
    assert!(result.expect_err("no plugin").contains("not running"));
}

// -- the store ---------------------------------------------------------------

#[component]
fn StoreProbe(step: u8) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    let store = provide_plugin_view_store();
    use_hook(|| match step {
        // A loaded slot is left alone by ensure.
        0 => {
            store.seed_view(
                "demo",
                "things",
                ViewState::Ready(View::Message {
                    tone: Tone::Info,
                    text: "loaded".to_string(),
                    action: None,
                }),
            );
            store.ensure(None, "demo", "things");
        }
        // An empty slot with no service fails at once, in words.
        1 => store.ensure(None, "demo", "things"),
        // A palette command that opens a view moves the section.
        _ => {
            run_command(
                &mut state,
                &PluginCommand {
                    plugin: "demo".to_string(),
                    id: "x/things".to_string(),
                    title: "Demo: Things".to_string(),
                    kind: CommandKind::Open {
                        view: "things".to_string(),
                    },
                },
            );
        }
    });
    let shown = format!(
        "{:?}|{}|{}|{:?}",
        store.view("demo", "things"),
        store.nav.read().view,
        store.nav.read().plugin,
        state.section.read()
    );
    rsx! { span { "{shown}" } }
}

fn probe(step: u8) -> String {
    let mut dom = VirtualDom::new_with_props(StoreProbe, StoreProbeProps { step });
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn plugin_views_ensure_keeps_a_view_that_is_already_loaded() {
    let html = probe(0);
    assert!(html.contains("Ready(Message"), "{html}");
    assert!(html.contains("loaded"), "{html}");
}

#[test]
fn plugin_views_ensure_without_a_service_fails_the_slot_in_words() {
    let html = probe(1);
    assert!(html.contains("Failed("), "{html}");
    assert!(html.contains("Plugins are not running here."), "{html}");
}

#[test]
fn plugin_views_a_palette_command_that_opens_a_view_selects_it_in_the_plugin_section() {
    let html = probe(2);
    assert!(html.contains("|things|demo|Plugin"), "{html}");
}

// -- the palette ------------------------------------------------------------

#[test]
fn plugin_views_palette_group_is_added_for_a_typed_query_after_the_other_results() {
    use crate::services::search::{with_plugin_commands, Group, Results, Target};
    let all = palette_commands(&[entry(
        "demo",
        PluginState::Enabled,
        vec![section(), commands()],
    )]);
    let base = Results {
        groups: Vec::new(),
        docker_command: Some("demo jobs".to_string()),
    };
    let found = with_plugin_commands(base.clone(), &all, "demo jobs");
    assert_eq!(found.groups.len(), 1);
    assert_eq!(found.groups[0].group, Group::Plugins);
    assert_eq!(found.groups[0].items[0].title, "DEMO: Jobs");
    assert!(matches!(found.groups[0].items[0].target, Target::Plugin(_)));
    assert_eq!(
        found.docker_command, None,
        "a match replaces the docker offer"
    );
    // Nothing typed, or nothing matching: the results are untouched.
    assert_eq!(with_plugin_commands(base.clone(), &all, ""), base);
    assert_eq!(with_plugin_commands(base.clone(), &all, "zzz"), base);
}

// -- rows, links and the confirm as one summary --------------------------------

fn spec(id: &str, params: Value) -> ActionSpec {
    ActionSpec {
        id: id.to_string(),
        label: id.to_string(),
        danger: false,
        params,
        confirm: None,
        options: Vec::new(),
    }
}

#[test]
fn plugin_views_an_action_with_only_an_open_param_opens_a_view_itself() {
    let logs = spec("open-logs", json!({ "open": "deployment/d1/logs" }));
    assert_eq!(open_only(&logs), Some("deployment/d1/logs"));
    // Anything else in the params means the plugin has work to do.
    assert_eq!(
        open_only(&spec("x", json!({ "open": "v", "uuid": "u" }))),
        None
    );
    assert_eq!(open_only(&spec("x", json!({ "uuid": "u" }))), None);
    assert_eq!(open_only(&spec("x", json!({ "open": 5 }))), None);
    assert_eq!(open_only(&spec("x", Value::Null)), None);
}

#[test]
fn plugin_views_repositories_read_the_same_however_they_are_written() {
    for remote in [
        "https://github.com/Example/Landing.git",
        "git@github.com:example/landing.git",
        "ssh://git@host.example/example/landing",
        "example/landing",
        "https://github.com/example/landing/",
    ] {
        assert_eq!(
            normalize_repo(remote).as_deref(),
            Some("example/landing"),
            "{remote}"
        );
    }
    assert_eq!(normalize_repo("landing"), None);
    assert_eq!(normalize_repo("a/b/c"), None);
}

#[test]
fn plugin_views_a_project_is_linked_by_its_remote_and_needs_a_folder() {
    let mut linked = project("landing", Some("git@github.com:example/landing.git"));
    linked.folder = Some(std::path::PathBuf::from("/Users/dev/landing"));
    let mut folderless = project("other", Some("git@github.com:example/other.git"));
    folderless.folder = None;
    let projects = [folderless, linked];
    assert_eq!(
        linked_project(&projects, "example/landing").map(|p| p.name.as_str()),
        Some("landing")
    );
    assert!(
        linked_project(&projects, "example/other").is_none(),
        "no folder, no link"
    );
    assert!(linked_project(&projects, "example/missing").is_none());
}

#[test]
fn plugin_views_the_confirm_is_one_effect_summary_the_button_reads_from() {
    let confirm = berth_plugin_protocol::Confirm {
        title: "Redeploy landing?".to_string(),
        lines: vec!["App: landing".to_string(), "Branch: main".to_string()],
        button: "Redeploy anyway".to_string(),
    };
    let effect = plugin_effect(&confirm);
    assert_eq!(effect.verb, "Redeploy anyway");
    assert_eq!(effect.count, 1);
    let texts: Vec<&str> = effect.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["App: landing", "Branch: main"]);
}

// -- Fix with <harness> -------------------------------------------------------

fn harness() -> berth_core::Harness {
    berth_core::Harness {
        id: berth_core::HarnessId("claude-code".to_string()),
        name: "Claude Code".to_string(),
        binary: std::path::PathBuf::from("/bin/claude"),
        version: None,
        transport: berth_core::HarnessTransport::ClaudeStreamJson,
        available: true,
    }
}

#[test]
fn plugin_views_fix_with_attaches_the_failed_log_and_the_project_never_env() {
    use crate::services::assistant_entry::{plugin_log_entry, LOG_LINES_CAP};
    let mut linked = project("landing", Some("git@github.com:example/landing.git"));
    linked.folder = Some(std::path::PathBuf::from("/Users/dev/landing"));
    linked.compose_files = vec![std::path::PathBuf::from("/Users/dev/landing/compose.yaml")];
    let lines: Vec<String> = (0..LOG_LINES_CAP + 10)
        .map(|i| format!("line {i}"))
        .collect();
    let entry = plugin_log_entry(&linked, &harness(), "Coolify", "landing", &lines)
        .expect("a project with a folder");
    assert_eq!(entry.request.project, linked.id);
    assert!(entry.origin.contains("Coolify"), "{}", entry.origin);
    assert!(
        entry.prompt.contains("deployment of landing failed"),
        "{}",
        entry.prompt
    );
    let berth_core::ContextItem::Logs {
        name, lines: kept, ..
    } = &entry.context[0]
    else {
        panic!("the log comes first: {:?}", entry.context);
    };
    assert!(
        name.contains("Coolify") && name.contains("landing"),
        "{name}"
    );
    assert_eq!(kept.len(), LOG_LINES_CAP, "capped like every log");
    assert_eq!(
        kept.last().map(String::as_str),
        Some("line 409"),
        "the newest lines"
    );
    assert!(
        entry.context.iter().all(|item| !matches!(
            item,
            berth_core::ContextItem::File(_) | berth_core::ContextItem::Inspect(_)
        )),
        "{:?}",
        entry.context
    );
    // A project with no folder on this Mac cannot be opened.
    linked.folder = None;
    assert!(plugin_log_entry(&linked, &harness(), "Coolify", "landing", &lines).is_none());
}
