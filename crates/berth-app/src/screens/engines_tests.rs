//! P4-T6 tests: both Engines frames render what the frames show from the
//! mock world, the row and dashboard models say what each state needs, and
//! every Colima path (start, stop, restart, apply, failure, progress) runs
//! through `ActionsService` against `MockColima`.

use std::sync::Arc;

use berth_engine::{ColimaControl as _, MockColima, MockColimaCall};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::screens::engine_dashboard::{
    disk_summary, header_sub, info_rows, load_tiles, state_word,
};
use crate::services::actions::{apply_finished, Finished};
use crate::services::engine_dashboard::EngineSeries;
use crate::services::engines::{Applier, EngineUpdate};

fn id(s: &str) -> EngineId {
    EngineId(s.to_string())
}

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn engine(name: &str) -> Engine {
    world()
        .engines
        .into_iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("mock engine {name}"))
}

/// Drop every `<style>` block so class names in assertions mean elements.
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

fn scene(frame: &str) -> String {
    strip_styles(&crate::scenes::render_scene_html(frame).expect("the frame has a scene"))
}

const DASHBOARD: &str = "Engines / dashboard / 1440";
const STARTING: &str = "Engines / starting / 1440";

// ---------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------

#[test]
fn engines_scene_dashboard_shows_the_three_rows_and_the_dashboard() {
    let html = scene(DASHBOARD);
    assert_eq!(html.matches("aria-pressed=").count(), 3, "{}", html.len());
    // Rows: state, versions, counts, VM size, the unreachable one's reason.
    for text in [
        "Colima 0.10.3 \u{b7} Docker 28.4.0 \u{b7} aarch64",
        "15 of 23 running",
        "4 CPU \u{b7} 8 GB \u{b7} 60 GB",
        "0 of 6 running",
        "2 CPU \u{b7} 4 GB \u{b7} 40 GB",
        "SSH \u{b7} deploy@116.203.41.9",
        "7 of 9 running \u{b7} last seen 4 min ago",
        "Connection timed out after 10 s",
    ] {
        assert!(html.contains(text), "{text}: {html}");
    }
    // One button per row: Stop (asks, so the ellipsis), Start, Retry.
    assert!(html.contains("Stop engine\u{2026}"), "{html}");
    assert!(html.contains("Start engine"), "{html}");
    assert!(html.contains(">Retry<"), "{html}");
    // Header and notice.
    assert!(html.contains(">Running<"), "{html}");
    assert!(
        html.contains("Colima 0.10.3 \u{b7} Docker 28.4.0 \u{b7} 4 CPU \u{b7} 8 GB \u{b7} 60 GB"),
        "{html}"
    );
    assert!(html.contains("Edit resources"), "{html}");
    assert!(
        html.contains(
            "Edit resources restarts Colima \u{b7} running containers stop for about 20 s."
        ),
        "{html}"
    );
    // Tiles from the seeded load, the info list, the disk bar and legend.
    assert!(html.contains("23.7"), "{html}");
    assert!(html.contains("of 4 cores \u{b7} last 5 min"), "{html}");
    assert!(html.contains("of 8 GB \u{b7} last 5 min"), "{html}");
    for (label, value) in [
        ("VM type", "vz"),
        ("Rosetta", "on"),
        ("Architecture", "aarch64"),
        ("Docker", "28.4.0"),
    ] {
        assert!(
            html.contains(label) && html.contains(value),
            "{label}: {html}"
        );
    }
    assert!(html.contains("22.5 of 60 GB used"), "{html}");
    assert!(html.contains("5.7 GB reclaimable"), "{html}");
    for text in [
        "Images",
        "11.8 GB",
        "Volumes",
        "6.2 GB",
        "Build cache",
        "3.1 GB",
        "Containers",
        "1.4 GB",
    ] {
        assert!(html.contains(text), "{text}: {html}");
    }
    assert!(html.contains("Clean up\u{2026}"), "{html}");
    assert!(html.contains("Recent activity"), "{html}");
    assert_eq!(html.matches("class=\"recent-row").count(), 13, "{html}");
    // No run panel while nothing runs; the engine has no inspector.
    assert!(!html.contains("run-panel"), "{html}");
}

#[test]
fn engines_scene_starting_shows_progress_steps_and_the_command_output() {
    let html = scene(STARTING);
    assert!(html.contains("Starting Colima"), "{html}");
    assert!(html.contains("40%"), "{html}");
    assert!(
        html.contains("Step 3 of 5 \u{b7} 8 s of about 20 s"),
        "{html}"
    );
    assert!(html.contains("aria-valuenow=\"40\""), "{html}");
    assert!(html.contains("colima start default"), "{html}");
    for line in [
        "+0.0 s",
        "INFO[0000] starting colima [profile=default]",
        "+5.8 s",
        "INFO[0005] provisioning",
        "INFO[0007] waiting for docker socket",
    ] {
        assert!(html.contains(line), "{line}: {html}");
    }
    assert!(
        html.contains("Containers appear here once Docker answers."),
        "{html}"
    );
    // The header says Starting\u{2026} and offers nothing; the row's button is
    // disabled and says what is going on.
    assert!(html.contains(">Starting\u{2026}<"), "{html}");
    assert!(!html.contains("Stop engine"), "{html}");
    assert!(!html.contains("Edit resources"), "{html}");
    assert!(!html.contains("Clean up"), "{html}");
    // The row of the engine that is starting shows the run in small.
    assert!(html.contains("Colima 0.10.3 \u{b7} starting"), "{html}");
    assert!(html.contains("Starting\u{2026} 8 s"), "{html}");
    assert!(html.contains("8 s of about 20 s"), "{html}");
    // The other rows are as before; the chrome says what is going on.
    assert!(html.contains("Connection timed out after 10 s"), "{html}");
    assert!(html.contains("Engine starting\u{2026}"), "{html}");
    assert!(
        html.contains("colima/default \u{b7} Starting\u{2026}"),
        "{html}"
    );
}

// ---------------------------------------------------------------------
// Row and dashboard models
// ---------------------------------------------------------------------

fn at() -> DateTime<Utc> {
    chrono::DateTime::from_timestamp(1_792_850_550, 0).expect("a timestamp")
}

#[test]
fn engines_row_running_colima_offers_stop_and_names_its_sizes() {
    let row = row_model(&engine("default"), Some("0.10.3"), at());
    assert_eq!(row.name, "default");
    assert_eq!(
        row.meta,
        "Colima 0.10.3 \u{b7} Docker 28.4.0 \u{b7} aarch64"
    );
    assert_eq!(row.counts.as_deref(), Some("15 of 23 running"));
    assert_eq!(row.sizes.as_deref(), Some("4 CPU \u{b7} 8 GB \u{b7} 60 GB"));
    assert_eq!(row.problem, None);
    assert_eq!(row.action, RowAction::Stop);
}

#[test]
fn engines_row_stopped_colima_offers_start_and_counts_nothing_running() {
    let row = row_model(&engine("work"), None, at());
    assert_eq!(row.meta, "Colima \u{b7} Docker 28.4.0 \u{b7} aarch64");
    assert_eq!(row.counts.as_deref(), Some("0 of 11 running"));
    assert_eq!(row.action, RowAction::Start);
}

#[test]
fn engines_row_unreachable_says_when_it_was_last_seen_and_offers_retry() {
    let mut remote = engine("hetzner-1");
    remote.state = EngineState::Unreachable {
        reason: "Connection timed out after 10 s".to_string(),
        last_seen: Some(at() - chrono::Duration::minutes(4)),
    };
    let row = row_model(&remote, None, at());
    assert_eq!(row.meta, "SSH \u{b7} deploy@116.203.41.9");
    assert_eq!(
        row.counts.as_deref(),
        Some("7 of 9 running \u{b7} last seen 4 min ago")
    );
    assert_eq!(
        row.problem.as_deref(),
        Some("Connection timed out after 10 s")
    );
    assert_eq!(row.sizes, None);
    assert_eq!(row.action, RowAction::Retry);
    // Never answered: no last seen to age.
    remote.state = EngineState::Unreachable {
        reason: "no route".to_string(),
        last_seen: None,
    };
    remote.counts = berth_core::ResourceCounts::default();
    let row = row_model(&remote, None, at());
    assert_eq!(row.counts.as_deref(), Some("not reached yet"));
}

#[test]
fn engines_row_containerd_and_incus_profiles_are_not_supported() {
    for runtime in [berth_core::Runtime::Containerd, berth_core::Runtime::Incus] {
        let mut profile = engine("work");
        profile.state = EngineState::Unsupported {
            reason: "Not supported in this version".to_string(),
        };
        if let Some(vm) = &mut profile.resources {
            vm.runtime = runtime;
        }
        let row = row_model(&profile, Some("0.10.3"), at());
        assert_eq!(row.action, RowAction::None, "{runtime:?} has no actions");
        assert_eq!(row.counts.as_deref(), Some("Not supported in this version"));
        assert!(
            row.meta.ends_with(match runtime {
                berth_core::Runtime::Containerd => "containerd",
                _ => "incus",
            }),
            "{}",
            row.meta
        );
    }
}

#[test]
fn engines_row_a_running_command_disables_the_button_and_says_what() {
    let mut starting = engine("default");
    starting.state = EngineState::Starting;
    let row = row_model(&starting, Some("0.10.3"), at());
    assert_eq!(row.action, RowAction::Busy("Starting\u{2026}"));
    assert_eq!(row.meta, "Colima 0.10.3 \u{b7} starting");
    starting.state = EngineState::Stopping;
    assert_eq!(
        row_model(&starting, None, at()).action,
        RowAction::Busy("Stopping\u{2026}")
    );
}

#[test]
fn engines_row_a_running_remote_engine_has_no_stop() {
    let mut remote = engine("hetzner-1");
    remote.state = EngineState::Running;
    assert_eq!(row_model(&remote, None, at()).action, RowAction::None);
}

#[test]
fn engines_selection_prefers_the_selected_then_the_scoped_then_the_first() {
    let engines = world().engines;
    let scope = crate::state::EngineScope::One(id("colima:work"));
    assert_eq!(
        selected_engine(&engines, &Selection::Engine(id("ssh:hetzner-1")), &scope),
        Some(id("ssh:hetzner-1"))
    );
    assert_eq!(
        selected_engine(&engines, &Selection::None, &scope),
        Some(id("colima:work"))
    );
    assert_eq!(
        selected_engine(
            &engines,
            &Selection::Engine(id("gone")),
            &crate::state::EngineScope::All
        ),
        Some(id("colima:default"))
    );
    assert_eq!(
        selected_engine(&[], &Selection::None, &crate::state::EngineScope::All),
        None
    );
}

#[test]
fn engines_dashboard_models_say_what_the_frame_says() {
    let default = engine("default");
    assert_eq!(
        header_sub(&default, Some("0.10.3")),
        "Colima 0.10.3 \u{b7} Docker 28.4.0 \u{b7} 4 CPU \u{b7} 8 GB \u{b7} 60 GB"
    );
    assert_eq!(state_word(&default.state), "Running");
    assert_eq!(
        info_rows(&default),
        vec![
            ("VM type", "vz".to_string()),
            ("Rosetta", "on".to_string()),
            ("Architecture", "aarch64".to_string()),
            ("Docker", "28.4.0".to_string()),
        ]
    );
    // A remote engine has no VM: its info is what it knows.
    assert_eq!(
        info_rows(&engine("hetzner-1")),
        vec![("Host", "ssh://deploy@116.203.41.9".to_string())]
    );
    let usage = world().disk_usage_by_engine[&default.id.0].clone();
    let (used, reclaimable) = disk_summary(&usage, None);
    assert_eq!(used, "23.6 of 60 GB used");
    assert_eq!(reclaimable.as_deref(), Some("7.3 GB reclaimable"));
}

#[test]
fn engines_dashboard_tiles_show_the_load_or_the_reason() {
    let default = engine("default");
    let now = Utc::now();
    let mut series = EngineSeries::default();
    series.push(now - chrono::Duration::seconds(10), 20.0, 1.8e9);
    series.push(now, 23.7, 1.9e9);
    let tiles = load_tiles(Some(&series), &default, now, None);
    assert_eq!(tiles.len(), 2);
    assert_eq!(tiles[0].label, "CPU");
    assert_eq!(tiles[0].value.as_deref(), Some("23.7"));
    assert_eq!(tiles[0].unit, "%");
    assert_eq!(tiles[0].caption, "of 4 cores \u{b7} last 5 min");
    assert_eq!(tiles[1].value.as_deref(), Some("1.9"));
    assert_eq!(tiles[1].unit, "GB");
    assert_eq!(tiles[1].caption, "of 8 GB \u{b7} last 5 min");
    assert_eq!(tiles[0].spark.len(), 2);
    // No sample yet: waiting, not a figure.
    let waiting = load_tiles(None, &default, now, None);
    assert!(waiting.iter().all(|t| t.value.is_none() && t.loading));
    // Not running: the reason, never a stale figure.
    let off = load_tiles(Some(&series), &default, now, Some("The engine is stopped"));
    assert!(off.iter().all(|t| t.value.is_none() && !t.loading));
    assert_eq!(off[0].caption, "The engine is stopped");
}

#[test]
fn engines_run_view_reads_progress_from_time() {
    let run = |action: &'static str, elapsed_s: f64| colima::RunView {
        action,
        elapsed_s,
        lines: Vec::new(),
        failure: None,
    };
    assert_eq!(run("engine.start", 8.0).percent(), 40);
    assert_eq!(run("engine.start", 8.0).step(), 3);
    assert_eq!(
        run("engine.start", 8.0).progress_text(),
        "Step 3 of 5 \u{b7} 8 s of about 20 s"
    );
    // The bar waits for the command: never 100 % while it runs.
    assert_eq!(run("engine.start", 90.0).percent(), 95);
    assert_eq!(run("engine.start", 90.0).step(), 5);
    assert_eq!(run("engine.start", 0.0).step(), 1);
    // A stop is short and has no steps.
    assert_eq!(run("engine.stop", 5.0).percent(), 50);
    assert_eq!(run("engine.stop", 5.0).progress_text(), "5 s of about 10 s");
    assert_eq!(colima::run_title("engine.restart"), "Restarting Colima");
    assert_eq!(
        colima::run_command("engine.edit_resources", "default"),
        "colima stop default, then colima start default"
    );
}

#[test]
fn engines_colima_version_is_the_first_line_after_the_name() {
    assert_eq!(
        colima::parse_version("colima version 0.10.3\ngit commit: abc\n"),
        Some("0.10.3".to_string())
    );
    assert_eq!(
        colima::parse_version("colima version v0.9.1"),
        Some("0.9.1".to_string())
    );
    assert_eq!(colima::parse_version("not colima"), None);
    assert_eq!(colima::parse_version(""), None);
}

// ---------------------------------------------------------------------
// Colima actions through the service, against MockColima
// ---------------------------------------------------------------------

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = crate::state::use_app_state();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { div {} }
}

struct Rig {
    dom: VirtualDom,
    state: AppState,
    service: ActionsService,
    rx: tokio::sync::mpsc::UnboundedReceiver<Finished>,
    store: Arc<berth_store::Store>,
    colima: Arc<MockColima>,
}

impl Rig {
    fn new() -> Self {
        let mut dom = VirtualDom::new(Harness);
        let _ = dom.rebuild_to_vec();
        let mut state = SLOT
            .with(|slot| slot.borrow_mut().take())
            .expect("harness captured AppState");
        let world = world();
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let colima = Arc::new(MockColima::bundled());
        let (service, rx) = ActionsService::new(
            crate::services::engines::client_map(),
            Arc::clone(&store),
            Arc::new(berth_engine::MockCompose::bundled()),
            Arc::clone(&colima) as Arc<dyn berth_engine::ColimaControl>,
            CancellationToken::new(),
        );
        dom.in_runtime(|| {
            state.engines.set(world.engines.clone());
            state.containers.set(world.containers.clone());
        });
        Self {
            dom,
            state,
            service,
            rx,
            store,
            colima,
        }
    }

    fn engine_now(&self, name: &str) -> Engine {
        self.dom.in_runtime(|| {
            self.state
                .engines
                .peek()
                .iter()
                .find(|e| e.name == name)
                .cloned()
                .expect("engine")
        })
    }

    fn state_of(&self, name: &str) -> EngineState {
        self.engine_now(name).state
    }

    fn run_op(&self, name: &str, op: berth_core::ColimaOp) -> Result<(), String> {
        let engine = self.engine_now(name);
        let service = self.service.clone();
        let mut state = self.state;
        self.dom
            .in_runtime(|| service.engine(&mut state, &engine, op))
    }

    fn pending(&self, name: &str, action: &'static str) -> Option<PendingState> {
        let key = PendingKey {
            subject: berth_core::Subject::Engine(self.engine_now(name).id),
            action,
        };
        self.dom
            .in_runtime(|| self.state.pending.peek().get(&key).cloned())
    }

    /// Wait for the next finished command and apply it the way the App
    /// root's receiver does.
    async fn settle_one(&mut self) -> Finished {
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), self.rx.recv())
            .await
            .expect("a Finished within 5 s")
            .expect("channel open");
        let key = done.key.clone();
        let ok = done.result.is_ok();
        let copy = Finished {
            key: done.key.clone(),
            result: done.result.clone(),
            replaced_by: None,
            message: None,
        };
        let mut state = self.state;
        let service = self.service.clone();
        self.dom.in_runtime(|| {
            apply_finished(&mut state, done);
            service.settle_engine(&mut state, &key, ok);
        });
        copy
    }

    fn activity_actions(&self) -> Vec<String> {
        self.store
            .activity_query(&berth_store::ActivityFilter::default(), None, 50)
            .expect("activity")
            .into_iter()
            .map(|e| e.action)
            .collect()
    }
}

fn apply_options(cpus: u16) -> berth_core::ColimaStartOptions {
    berth_core::ColimaStartOptions {
        cpus: Some(cpus),
        memory_gib: Some(12),
        disk_gib: None,
        vm_type: None,
        rosetta: None,
        arch: None,
    }
}

#[tokio::test]
async fn engines_start_shows_starting_then_running_and_records_the_command() {
    let mut rig = Rig::new();
    assert_eq!(rig.state_of("work"), EngineState::Stopped);
    rig.run_op("work", berth_core::ColimaOp::Start)
        .expect("accepted");
    // At once: the row says Starting and the pending key says Starting\u{2026}.
    assert_eq!(rig.state_of("work"), EngineState::Starting);
    assert_eq!(
        rig.pending("work", "engine.start"),
        Some(PendingState::Running {
            label: "Starting\u{2026}".to_string()
        })
    );
    let done = rig.settle_one().await;
    assert_eq!(done.result, Ok(()));
    assert_eq!(rig.state_of("work"), EngineState::Running);
    assert_eq!(rig.pending("work", "engine.start"), None);
    assert_eq!(
        rig.colima.calls(),
        vec![MockColimaCall::Start {
            profile: "work".to_string(),
            opts: None
        }]
    );
    assert!(rig.colima.status("work").await.expect("profile").running);
    assert!(rig.activity_actions().contains(&"engine.start".to_string()));
}

#[tokio::test]
async fn engines_stop_goes_through_stopping_to_stopped() {
    let mut rig = Rig::new();
    rig.run_op("default", berth_core::ColimaOp::Stop)
        .expect("accepted");
    assert_eq!(rig.state_of("default"), EngineState::Stopping);
    assert_eq!(
        rig.pending("default", "engine.stop"),
        Some(PendingState::Running {
            label: "Stopping\u{2026}".to_string()
        })
    );
    rig.settle_one().await;
    assert_eq!(rig.state_of("default"), EngineState::Stopped);
    assert!(!rig.colima.status("default").await.expect("profile").running);
    assert_eq!(
        rig.colima.calls(),
        vec![MockColimaCall::Stop {
            profile: "default".to_string()
        }]
    );
}

#[tokio::test]
async fn engines_restart_runs_one_restart_and_ends_running() {
    let mut rig = Rig::new();
    rig.run_op("default", berth_core::ColimaOp::Restart)
        .expect("accepted");
    assert_eq!(rig.state_of("default"), EngineState::Starting);
    rig.settle_one().await;
    assert_eq!(rig.state_of("default"), EngineState::Running);
    assert_eq!(
        rig.colima.calls(),
        vec![MockColimaCall::Restart {
            profile: "default".to_string()
        }]
    );
}

#[tokio::test]
async fn engines_apply_stops_then_starts_with_the_new_options() {
    let mut rig = Rig::new();
    let opts = apply_options(6);
    rig.run_op("default", berth_core::ColimaOp::Apply(opts.clone()))
        .expect("accepted");
    assert_eq!(rig.state_of("default"), EngineState::Starting);
    assert_eq!(
        rig.pending("default", "engine.edit_resources"),
        Some(PendingState::Running {
            label: "Applying\u{2026}".to_string()
        })
    );
    rig.settle_one().await;
    assert_eq!(
        rig.colima.calls(),
        vec![
            MockColimaCall::Stop {
                profile: "default".to_string()
            },
            MockColimaCall::Start {
                profile: "default".to_string(),
                opts: Some(opts)
            },
        ]
    );
    // The profile took the new sizes and the engine runs again.
    let profile = rig.colima.status("default").await.expect("profile");
    assert!(profile.running);
    assert_eq!(profile.cpus, 6);
    assert_eq!(berth_core::fmt::bytes(profile.memory), "12.9 GB");
    assert_eq!(rig.state_of("default"), EngineState::Running);
}

#[tokio::test]
async fn engines_apply_on_a_stopped_profile_only_starts_it() {
    let mut rig = Rig::new();
    rig.run_op("work", berth_core::ColimaOp::Apply(apply_options(3)))
        .expect("accepted");
    rig.settle_one().await;
    assert_eq!(
        rig.colima.calls(),
        vec![MockColimaCall::Start {
            profile: "work".to_string(),
            opts: Some(apply_options(3))
        }]
    );
    assert_eq!(rig.state_of("work"), EngineState::Running);
}

#[tokio::test]
async fn engines_a_failed_start_restores_the_row_and_keeps_the_output() {
    let mut rig = Rig::new();
    rig.colima.fail_next("disk image is locked");
    rig.run_op("work", berth_core::ColimaOp::Start)
        .expect("accepted");
    assert_eq!(rig.state_of("work"), EngineState::Starting);
    let done = rig.settle_one().await;
    assert_eq!(done.result, Err("disk image is locked".to_string()));
    // The row is as it was; the pending key carries the engine's text.
    assert_eq!(rig.state_of("work"), EngineState::Stopped);
    assert_eq!(
        rig.pending("work", "engine.start"),
        Some(PendingState::Failed {
            message: "disk image is locked".to_string()
        })
    );
    assert!(!rig.colima.status("work").await.expect("profile").running);
    // The run stays for the dashboard: its output and why it failed.
    let work = rig.engine_now("work").id;
    let run = rig
        .service
        .engine_run(&work)
        .expect("the failed run is kept");
    assert_eq!(run.failure.as_deref(), Some("disk image is locked"));
    assert_eq!(run.lines.len(), 1);
    assert_eq!(run.lines[0].text, "disk image is locked");
    // The failure is in the activity feed, and dismissing clears the run.
    assert!(rig.activity_actions().contains(&"engine.start".to_string()));
    rig.service.clear_engine_run(&work);
    assert!(rig.service.engine_run(&work).is_none());
    // A retry is allowed after a failure and works.
    rig.run_op("work", berth_core::ColimaOp::Start)
        .expect("a failure does not block a retry");
    rig.settle_one().await;
    assert_eq!(rig.state_of("work"), EngineState::Running);
}

#[tokio::test]
async fn engines_a_failed_stop_leaves_the_engine_running() {
    let mut rig = Rig::new();
    rig.colima.fail_next("vm is busy");
    rig.run_op("default", berth_core::ColimaOp::Stop)
        .expect("accepted");
    rig.settle_one().await;
    assert_eq!(rig.state_of("default"), EngineState::Running);
    assert!(rig.colima.status("default").await.expect("profile").running);
}

#[tokio::test]
async fn engines_progress_lines_arrive_while_the_command_runs() {
    let mut rig = Rig::new();
    rig.colima
        .set_line_delay(std::time::Duration::from_millis(60));
    rig.run_op("work", berth_core::ColimaOp::Start)
        .expect("accepted");
    let work = rig.engine_now("work").id;
    // Wait until a few lines are in while the command is still running.
    let mut seen = None;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if let Some(run) = rig.service.engine_run(&work) {
            if run.lines.len() >= 2 {
                seen = Some(run);
                break;
            }
        }
    }
    let run = seen.expect("lines arrive during the run");
    assert_eq!(run.action, "engine.start");
    assert!(run.failure.is_none());
    assert!(run.elapsed_s > 0.0);
    assert!(run.lines[0].text.starts_with("INFO[0000] starting colima"));
    assert!(run.lines[0].at_s <= run.lines[1].at_s, "{run:?}");
    assert_eq!(
        rig.pending("work", "engine.start"),
        Some(PendingState::Running {
            label: "Starting\u{2026}".to_string()
        }),
        "still running while lines stream"
    );
    rig.settle_one().await;
    assert!(
        rig.service.engine_run(&work).is_none(),
        "a run that succeeded is forgotten"
    );
}

#[tokio::test]
async fn engines_a_second_command_waits_for_the_first() {
    let mut rig = Rig::new();
    rig.run_op("default", berth_core::ColimaOp::Restart)
        .expect("accepted");
    let refused = rig
        .run_op("default", berth_core::ColimaOp::Stop)
        .expect_err("refused while one runs");
    assert_eq!(refused, "Restarting\u{2026} is still in progress");
    rig.settle_one().await;
    assert_eq!(rig.colima.calls().len(), 1);
}

#[tokio::test]
async fn engines_only_colima_engines_take_colima_commands() {
    let rig = Rig::new();
    let refused = rig
        .run_op("hetzner-1", berth_core::ColimaOp::Stop)
        .expect_err("not a Colima engine");
    assert_eq!(
        refused,
        "Only Colima engines can be started or stopped here"
    );
    assert!(rig.colima.calls().is_empty());
}

#[tokio::test]
async fn engines_a_confirmed_apply_reaches_colima_through_confirm_run() {
    let mut rig = Rig::new();
    let default = rig.engine_now("default");
    let running: Vec<berth_core::Container> = rig.dom.in_runtime(|| {
        rig.state
            .containers
            .peek()
            .iter()
            .filter(|c| c.engine == default.id && c.state == berth_core::ContainerState::Running)
            .cloned()
            .collect()
    });
    let refs: Vec<&berth_core::Container> = running.iter().collect();
    let opts = apply_options(6);
    let model = confirm::colima_model(
        &default,
        berth_core::ColimaOp::Apply(opts.clone()),
        &refs,
        &[
            "CPUs 4 \u{2192} 6".to_string(),
            "Memory 8 GiB \u{2192} 12 GiB".to_string(),
        ],
    )
    .expect("apply asks first");
    let service = rig.service.clone();
    let mut state = rig.state;
    let launched = rig
        .dom
        .in_runtime(|| confirm::run(&mut state, &service, &model, false));
    assert!(launched.refused.is_empty(), "{:?}", launched.refused);
    assert_eq!(launched.keys.len(), 1);
    assert_eq!(launched.keys[0].action, "engine.edit_resources");
    rig.settle_one().await;
    assert_eq!(rig.colima.calls().len(), 2);
    assert_eq!(rig.state_of("default"), EngineState::Running);
}

// ---------------------------------------------------------------------
// Confirms name what stops
// ---------------------------------------------------------------------

fn running_on(name: &str) -> Vec<berth_core::Container> {
    let id = engine(name).id;
    world()
        .containers
        .into_iter()
        .filter(|c| c.engine == id && c.state == berth_core::ContainerState::Running)
        .collect()
}

#[test]
fn engines_stop_confirm_names_the_containers_that_stop() {
    let default = engine("default");
    let running = running_on("default");
    let refs: Vec<&berth_core::Container> = running.iter().collect();
    let model = confirm::colima_model(&default, berth_core::ColimaOp::Stop, &refs, &[])
        .expect("stop asks first");
    assert_eq!(model.title, "Stop engine default?");
    assert_eq!(
        model.targets,
        vec![berth_core::Subject::Engine(default.id.clone())]
    );
    assert_eq!(
        model.action,
        berth_core::EngineAction::Colima {
            profile: "default".to_string(),
            op: berth_core::ColimaOp::Stop
        }
    );
    let changes = &model.effect.lines[0];
    assert_eq!(changes.kind, berth_core::EffectKind::Changes);
    assert!(
        changes.text.starts_with(&format!(
            "{} running containers stop when the engine stops: ",
            running.len()
        )),
        "{}",
        changes.text
    );
    // Four names, then the rest as a number.
    assert!(
        changes
            .text
            .contains(&format!("and {} more", running.len() - 4)),
        "{}",
        changes.text
    );
    assert!(changes.text.contains(&running[0].name), "{}", changes.text);
    assert_eq!(confirm::button_label(&model.effect), "Stop engine");
    assert_eq!(confirm::result_label(&model.effect, 1), "Stopped 1 engine");
    assert!(
        model
            .effect
            .lines
            .iter()
            .any(|l| l.kind == berth_core::EffectKind::Stays),
        "what stays is said"
    );
}

#[test]
fn engines_restart_confirm_says_for_about_twenty_seconds() {
    let default = engine("default");
    let running = running_on("default");
    let refs: Vec<&berth_core::Container> = running.iter().take(2).collect();
    let model = confirm::colima_model(&default, berth_core::ColimaOp::Restart, &refs, &[])
        .expect("restart asks first");
    assert_eq!(model.title, "Restart engine default?");
    assert_eq!(
        model.effect.lines[0].text,
        format!(
            "2 running containers stop for about 20 s: {} and {}.",
            running[0].name, running[1].name
        )
    );
    assert_eq!(confirm::button_label(&model.effect), "Restart engine");
}

#[test]
fn engines_confirm_for_an_empty_engine_says_nothing_runs() {
    let work = engine("work");
    let model = confirm::colima_model(&work, berth_core::ColimaOp::Stop, &[], &[])
        .expect("stop asks first");
    assert_eq!(model.effect.lines[0].text, "No containers are running.");
}

#[test]
fn engines_apply_confirm_lists_the_changes_and_names_who_stops() {
    let default = engine("default");
    let running = running_on("default");
    let refs: Vec<&berth_core::Container> = running.iter().take(1).collect();
    let changes = vec![
        "CPUs 4 \u{2192} 6".to_string(),
        "Disk 60 GiB \u{2192} 80 GiB".to_string(),
    ];
    let model = confirm::colima_model(
        &default,
        berth_core::ColimaOp::Apply(apply_options(6)),
        &refs,
        &changes,
    )
    .expect("apply asks first");
    assert_eq!(model.title, "Apply new resources to engine default?");
    assert_eq!(
        confirm::button_label(&model.effect),
        "Apply 2 resource changes"
    );
    assert_eq!(
        confirm::result_label(&model.effect, 2),
        "Applied 2 resource changes"
    );
    assert_eq!(model.effect.lines[0].text, "CPUs 4 \u{2192} 6");
    assert_eq!(model.effect.lines[1].text, "Disk 60 GiB \u{2192} 80 GiB");
    assert_eq!(
        model.effect.lines[2].text,
        format!(
            "1 running container stops for about 20 s: {}.",
            running[0].name
        )
    );
    // Nothing to apply, nothing to ask; a Start stops nothing.
    assert!(confirm::colima_model(
        &default,
        berth_core::ColimaOp::Apply(apply_options(4)),
        &refs,
        &[]
    )
    .is_none());
    assert!(confirm::colima_model(&default, berth_core::ColimaOp::Start, &refs, &[]).is_none());
    // Only Colima engines are asked.
    assert!(
        confirm::colima_model(&engine("hetzner-1"), berth_core::ColimaOp::Stop, &[], &[]).is_none()
    );
}

#[test]
fn engines_stopped_names_spell_out_up_to_four() {
    assert_eq!(confirm::stopped_names(&[]), "");
    assert_eq!(confirm::stopped_names(&["a"]), "a");
    assert_eq!(confirm::stopped_names(&["a", "b"]), "a and b");
    assert_eq!(confirm::stopped_names(&["a", "b", "c"]), "a, b and c");
    assert_eq!(
        confirm::stopped_names(&["a", "b", "c", "d"]),
        "a, b, c and d"
    );
    assert_eq!(
        confirm::stopped_names(&["a", "b", "c", "d", "e", "f"]),
        "a, b, c, d and 2 more"
    );
}

// ---------------------------------------------------------------------
// The engines service: failed refreshes, profile changes, Retry
// ---------------------------------------------------------------------

#[test]
fn engines_a_failed_refresh_of_a_stopped_engine_is_not_reported() {
    let rig = Rig::new();
    let mut applier = Applier::new(Vec::new());
    let mut state = rig.state;
    let work = rig.engine_now("work").id;
    let default = rig.engine_now("default").id;
    rig.dom.in_runtime(|| {
        // Stopped by Colima: the refresh cannot succeed, and nothing is wrong.
        applier.apply(
            &mut state,
            EngineUpdate::EngineFailed {
                engine: work.clone(),
                reason: "socket gone".to_string(),
            },
        );
        // A running engine that stops answering is unreachable, last seen now.
        applier.apply(
            &mut state,
            EngineUpdate::EngineFailed {
                engine: default.clone(),
                reason: "socket gone".to_string(),
            },
        );
    });
    assert_eq!(rig.state_of("work"), EngineState::Stopped);
    match rig.state_of("default") {
        EngineState::Unreachable { reason, last_seen } => {
            assert_eq!(reason, "socket gone");
            let seen = last_seen.expect("it was running a moment ago");
            assert!((Utc::now() - seen).num_seconds().abs() < 5);
        }
        other => panic!("{other:?}"),
    }
    // Still failing: the first last-seen is kept, not moved to now.
    let before = rig.state_of("default");
    rig.dom.in_runtime(|| {
        applier.apply(
            &mut state,
            EngineUpdate::EngineFailed {
                engine: default,
                reason: "still gone".to_string(),
            },
        );
    });
    match (before, rig.state_of("default")) {
        (
            EngineState::Unreachable { last_seen: a, .. },
            EngineState::Unreachable {
                last_seen: b,
                reason,
            },
        ) => {
            assert_eq!(a, b);
            assert_eq!(reason, "still gone");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn engines_a_profile_change_updates_the_sizes_the_row_shows() {
    let rig = Rig::new();
    let mut applier = Applier::new(Vec::new());
    let mut state = rig.state;
    let default = rig.engine_now("default").id;
    let profile = berth_core::ColimaProfile {
        name: "default".to_string(),
        running: true,
        arch: berth_core::Arch::Aarch64,
        cpus: 6,
        memory: bytesize::ByteSize(12 * (1 << 30)),
        disk: bytesize::ByteSize(80 * (1 << 30)),
        runtime: berth_core::Runtime::Docker,
        vm_type: Some(berth_core::VmType::Vz),
        rosetta: Some(false),
        mount_type: None,
        docker_socket: Some(std::path::PathBuf::from("/x/docker.sock")),
    };
    rig.dom.in_runtime(|| {
        applier.apply(
            &mut state,
            EngineUpdate::ProfileChanged {
                engine: default,
                profile,
            },
        );
    });
    let now = rig.engine_now("default");
    assert_eq!(
        sizes_text(now.resources.as_ref().expect("resources")),
        "6 CPU \u{b7} 12 GB \u{b7} 80 GB"
    );
    assert_eq!(now.docker_host.as_deref(), Some("unix:///x/docker.sock"));
}

#[tokio::test]
async fn engines_retry_connects_an_unreachable_engine_and_it_runs_again() {
    let rig = Rig::new();
    let clients = crate::services::engines::client_map();
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let link = EngineLink::new(
        Arc::clone(&clients),
        Arc::clone(&rig.store),
        world().pins,
        tx,
        CancellationToken::new(),
        true,
    );
    let hetzner = rig.engine_now("hetzner-1");
    assert!(matches!(hetzner.state, EngineState::Unreachable { .. }));
    let mut state = rig.state;
    rig.dom
        .in_runtime(|| retry(&mut state, Some(&link), &hetzner));
    // The row says Retrying\u{2026} at once.
    assert_eq!(
        rig.pending("hetzner-1", "engine.retry"),
        Some(PendingState::Running {
            label: "Retrying\u{2026}".to_string()
        })
    );
    // The engine's loop refreshes and sends its data.
    let mut applier = Applier::new(world().pins);
    let mut got_data = false;
    for _ in 0..20 {
        let update = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("an update within 5 s")
            .expect("channel open");
        let is_data = matches!(update, EngineUpdate::EngineData { .. });
        rig.dom.in_runtime(|| applier.apply(&mut state, update));
        if is_data {
            got_data = true;
            break;
        }
    }
    assert!(got_data, "the engine answered");
    assert_eq!(rig.state_of("hetzner-1"), EngineState::Running);
    // The answer ends the row's "Retrying…".
    assert_eq!(rig.pending("hetzner-1", "engine.retry"), None);
    assert!(clients.lock().expect("clients").contains_key(&hetzner.id));
}

#[test]
fn engines_retry_without_a_link_says_so_instead_of_pretending() {
    let rig = Rig::new();
    let hetzner = rig.engine_now("hetzner-1");
    let mut state = rig.state;
    rig.dom.in_runtime(|| retry(&mut state, None, &hetzner));
    assert_eq!(rig.pending("hetzner-1", "engine.retry"), None);
    let toasts = rig.dom.in_runtime(|| state.toasts.peek().clone());
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].text, "Engines cannot be retried here");
}

// ---------------------------------------------------------------------
// The Edit resources sheet, drawn
// ---------------------------------------------------------------------

#[component]
fn SheetHarness(engine: String, stopped: bool) -> Element {
    let state = crate::state::use_app_state();
    use_context_provider(|| state);
    use_hook(move || {
        let mut state = state;
        let mut engines = world().engines;
        if stopped {
            for e in &mut engines {
                if e.name == "work" {
                    e.state = EngineState::Stopped;
                }
            }
        }
        state.engines.set(engines);
    });
    rsx! {
        crate::sheets::edit_resources::EditResourcesSheet { engine: id(&engine) }
    }
}

fn sheet_html(engine: &str, stopped: bool) -> String {
    let mut dom = VirtualDom::new_with_props(
        SheetHarness,
        SheetHarnessProps {
            engine: engine.to_string(),
            stopped,
        },
    );
    let _ = dom.rebuild_to_vec();
    strip_styles(&dioxus_ssr::render(&dom))
}

#[test]
fn engines_sheet_opens_on_the_profiles_values_with_apply_disabled() {
    let html = sheet_html("colima:default", false);
    assert!(html.contains("Edit resources \u{b7} default"), "{html}");
    assert!(
        html.contains("Colima restarts; running containers stop for about 20 s."),
        "{html}"
    );
    for (label, value) in [("CPUs", "4"), ("Memory (GiB)", "8"), ("Disk (GiB)", "60")] {
        assert!(html.contains(label), "{label}: {html}");
        assert!(
            html.contains(&format!("value=\"{value}\"")),
            "{label}={value}: {html}"
        );
    }
    assert!(html.contains("The disk can only grow."), "{html}");
    assert!(html.contains("VM type"), "{html}");
    assert!(html.contains("Rosetta"), "{html}");
    assert!(
        html.contains("aria-checked=\"true\""),
        "Rosetta is on: {html}"
    );
    // Unchanged values: the Apply button is there, disabled, with the reason.
    let apply = html
        .find(">Apply<")
        .map(|i| &html[i.saturating_sub(300)..i])
        .expect("an Apply button");
    assert!(apply.contains("disabled"), "{apply}");
    assert!(
        apply.contains("data-tooltip=\"Nothing has changed\""),
        "{apply}"
    );
    assert!(!html.contains("eres-changes"), "no changes listed: {html}");
}

#[test]
fn engines_sheet_for_a_stopped_profile_has_no_restart_notice() {
    let html = sheet_html("colima:work", true);
    assert!(html.contains("Edit resources \u{b7} work"), "{html}");
    assert!(!html.contains("Colima restarts"), "{html}");
}

#[test]
fn engines_sheet_for_an_engine_without_a_vm_says_so() {
    let html = sheet_html("ssh:hetzner-1", false);
    assert!(
        html.contains("Berth does not know the VM settings of this engine yet."),
        "{html}"
    );
    assert!(!html.contains(">Apply<"), "{html}");
}

// ---------------------------------------------------------------------
// Live, read-only: the real default profile (never started or stopped)
// ---------------------------------------------------------------------

/// Reads the real Colima profiles and the default profile's state, CPU,
/// memory and disk. Only `colima list` and `colima status` run; nothing
/// here starts, stops, restarts or applies anything.
#[tokio::test]
#[ignore = "live: needs Colima (BERTH_IT=1)"]
async fn engines_it_reads_the_default_profile_without_changing_it() {
    if std::env::var("BERTH_IT").as_deref() != Ok("1") {
        eprintln!("skipped: set BERTH_IT=1");
        return;
    }
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let control = berth_engine::ColimaCli::new(search);
    let profiles = control.profiles().await.expect("colima list");
    let default = profiles
        .iter()
        .find(|p| p.name == "default")
        .expect("a default profile");
    let status = control.status("default").await.expect("colima status");
    println!(
        "profile=default running={} cpus={} memory={} disk={} vm_type={:?} rosetta={:?} runtime={:?}",
        status.running,
        status.cpus,
        berth_core::fmt::bytes(status.memory),
        berth_core::fmt::bytes(status.disk),
        status.vm_type,
        status.rosetta,
        status.runtime
    );
    assert_eq!(default.name, status.name);
    assert!(status.cpus >= 1);
    assert!(status.memory.as_u64() > 0 && status.disk.as_u64() > 0);
    let engines = berth_engine::discovery::discover(&control, &[]).await;
    let mine = engines
        .iter()
        .find(|e| e.id == id("colima:default"))
        .expect("discovery lists colima:default");
    let row = row_model(mine, colima::version(false).await.as_deref(), Utc::now());
    println!("row: {row:?}");
    assert_eq!(row.name, "default");
    // The version read is `colima version`, read-only.
    let version = colima::version(false).await;
    println!("colima version={version:?}");
    assert!(version.is_some());
    // The sheet's model reads the same profile and finds nothing to apply.
    if let Some(vm) = &mine.resources {
        let current = crate::sheets::edit_resources::Sizes::of(vm);
        let draft = crate::sheets::edit_resources::Draft::from_sizes(&current);
        assert!(matches!(
            crate::sheets::edit_resources::apply_state(&current, &draft, false),
            crate::sheets::edit_resources::Apply::Blocked(_)
        ));
    }
}

/// Starting, stopping, restarting or applying the real `default` profile
/// stops the owner's other containers. It runs only when
/// `BERTH_COLIMA_RESTART=1` is set, which no script sets: the owner sets it
/// by hand, present, once (docs/QUESTIONS.md Q6).
#[tokio::test]
#[ignore = "restarts the real default Colima profile: owner present only (BERTH_COLIMA_RESTART=1)"]
async fn engines_it_restart_default_profile_owner_present_only() {
    if std::env::var("BERTH_COLIMA_RESTART").as_deref() != Ok("1") {
        eprintln!("skipped: set BERTH_COLIMA_RESTART=1 with the owner present");
        return;
    }
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let control = berth_engine::ColimaCli::new(search);
    let before = control.status("default").await.expect("colima status");
    assert!(before.running, "restart it only while it runs");
    colima::run(
        &control,
        "default",
        berth_core::ColimaOp::Restart,
        CancellationToken::new(),
    )
    .await
    .expect("colima restart");
    assert!(control.status("default").await.expect("status").running);
}
