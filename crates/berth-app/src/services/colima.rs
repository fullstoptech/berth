//! Engine actions through `ColimaControl` (D28, D34): start, stop, restart
//! and apply resources for a Colima profile. The binary is found only by
//! `berth_sys::SearchPath` (inside `ColimaCli`).

use std::sync::Arc;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

/// The `ActionDef` id of a Colima operation; also the activity action and
/// the pending key.
#[must_use]
pub fn action_id(op: &berth_core::ColimaOp) -> &'static str {
    match op {
        berth_core::ColimaOp::Start => "engine.start",
        berth_core::ColimaOp::Stop => "engine.stop",
        berth_core::ColimaOp::Restart => "engine.restart",
        berth_core::ColimaOp::Apply(_) => "engine.edit_resources",
    }
}

/// The profile name when the engine is a Colima profile.
#[must_use]
pub fn profile_of(engine: &berth_core::Engine) -> Option<&str> {
    match &engine.kind {
        berth_core::EngineKind::Colima { profile } => Some(profile.as_str()),
        _ => None,
    }
}

/// How many trailing stderr lines make the failure message.
const TAIL_LINES: usize = 4;

type Lines =
    futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, berth_engine::EngineError>>;

/// What the run log is told about every line of output.
pub type Sink<'a> = &'a (dyn Fn(&berth_sys::ProcessLine) + Send + Sync);

/// Drain one Colima command. `Ok` only on exit code 0; the failure message
/// is the stderr tail, else a sentence naming the exit code.
async fn drain(stream: Lines, sink: Sink<'_>) -> Result<(), String> {
    let mut stream = stream;
    let mut tail: Vec<String> = Vec::new();
    while let Some(item) = stream.next().await {
        if let Ok(line) = &item {
            sink(line);
        }
        match item {
            Ok(berth_sys::ProcessLine::Exit(Some(0))) => return Ok(()),
            Ok(berth_sys::ProcessLine::Exit(Some(code))) => {
                return Err(if tail.is_empty() {
                    format!("colima exited with code {code}")
                } else {
                    tail.join("\n")
                });
            }
            Ok(berth_sys::ProcessLine::Exit(None)) => {
                return Err("colima was stopped by a signal".to_string());
            }
            Ok(berth_sys::ProcessLine::Stderr(text)) => {
                if !text.trim().is_empty() {
                    tail.push(text);
                    if tail.len() > TAIL_LINES {
                        tail.remove(0);
                    }
                }
            }
            Ok(berth_sys::ProcessLine::Stdout(_)) => {}
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("colima ended without an exit code".to_string())
}

/// Run one operation on a profile. `Apply` is stop then start with the new
/// options (just start when the profile is not running), because Colima
/// only changes a profile's resources at start.
pub async fn run(
    control: &dyn berth_engine::ColimaControl,
    profile: &str,
    op: berth_core::ColimaOp,
    cancel: CancellationToken,
) -> Result<(), String> {
    run_with(control, profile, op, cancel, &|_| {}).await
}

/// [`run`], telling `sink` about every line the command prints (the
/// dashboard's progress log).
pub async fn run_with(
    control: &dyn berth_engine::ColimaControl,
    profile: &str,
    op: berth_core::ColimaOp,
    cancel: CancellationToken,
    sink: Sink<'_>,
) -> Result<(), String> {
    match op {
        berth_core::ColimaOp::Start => drain(control.start(profile, None, cancel), sink).await,
        berth_core::ColimaOp::Stop => drain(control.stop(profile, cancel), sink).await,
        berth_core::ColimaOp::Restart => drain(control.restart(profile, cancel), sink).await,
        berth_core::ColimaOp::Apply(options) => {
            // A profile that is not running is only started; stopping it
            // again would be an error for nothing.
            let running = control.status(profile).await.is_ok_and(|p| p.running);
            if running {
                drain(control.stop(profile, cancel.clone()), sink).await?;
            }
            drain(control.start(profile, Some(options), cancel), sink).await
        }
    }
}

// ---------------------------------------------------------------------
// Progress (P4-T6)
// ---------------------------------------------------------------------

/// How many output lines a run keeps (the newest).
pub const RUN_LINES_MAX: usize = 200;

/// The steps the progress line counts.
pub const RUN_STEPS: u32 = 5;

/// How long a Colima operation usually takes, for the progress bar and the
/// "about N s" text. The bar never reaches 100 % before the command exits.
#[must_use]
pub fn expected_secs(action: &str) -> f64 {
    match action {
        "engine.stop" => 10.0,
        "engine.restart" => 30.0,
        "engine.edit_resources" => 40.0,
        _ => 20.0,
    }
}

/// "Starting Colima", "Stopping Colima": what the progress panel says.
#[must_use]
pub fn run_title(action: &str) -> &'static str {
    match action {
        "engine.stop" => "Stopping Colima",
        "engine.restart" => "Restarting Colima",
        "engine.edit_resources" => "Applying resources",
        _ => "Starting Colima",
    }
}

/// The command the log box is headed with.
#[must_use]
pub fn run_command(action: &str, profile: &str) -> String {
    match action {
        "engine.stop" => format!("colima stop {profile}"),
        "engine.restart" => format!("colima restart {profile}"),
        "engine.edit_resources" => format!("colima stop {profile}, then colima start {profile}"),
        _ => format!("colima start {profile}"),
    }
}

/// One output line with the second it arrived at.
#[derive(Debug, Clone, PartialEq)]
pub struct RunLine {
    pub at_s: f64,
    pub text: String,
}

/// A Colima run as the screen shows it: seeded by scenes, built from the
/// live log by [`Runs::view`].
#[derive(Debug, Clone, PartialEq)]
pub struct RunView {
    /// The `ActionDef` id (`engine.start` ...).
    pub action: &'static str,
    pub elapsed_s: f64,
    pub lines: Vec<RunLine>,
    /// Set when the command failed: the run stays on screen with its lines.
    pub failure: Option<String>,
}

impl RunView {
    /// Whole percent, from time against [`expected_secs`]; capped at 95
    /// while the command still runs.
    #[must_use]
    pub fn percent(&self) -> u32 {
        let share = (self.elapsed_s / expected_secs(self.action)).clamp(0.0, 0.95);
        // 0..=95 by the clamp above.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole = (share * 100.0).round() as u32;
        whole
    }

    /// The step the time has reached, 1..=[`RUN_STEPS`].
    #[must_use]
    pub fn step(&self) -> u32 {
        let share = (self.elapsed_s / expected_secs(self.action)).clamp(0.0, 0.999);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let step = (share * f64::from(RUN_STEPS)).floor() as u32 + 1;
        step.min(RUN_STEPS)
    }

    /// "Step 3 of 5 · 8 s of about 20 s".
    #[must_use]
    pub fn progress_text(&self) -> String {
        let secs = format!(
            "{} s of about {} s",
            self.elapsed_s.floor(),
            expected_secs(self.action)
        );
        if self.action == "engine.stop" {
            secs
        } else {
            format!("Step {} of {RUN_STEPS} \u{b7} {secs}", self.step())
        }
    }
}

#[derive(Debug)]
struct RunLog {
    action: &'static str,
    started: std::time::Instant,
    lines: Vec<RunLine>,
    failure: Option<String>,
    /// Elapsed seconds once the command ended, so a failed run stops counting.
    ended_at_s: Option<f64>,
}

type RunMap = std::collections::BTreeMap<berth_core::EngineId, RunLog>;

/// The runs of Colima operations by engine. Written by the action task,
/// read by the UI on its tick (the task never touches a signal).
#[derive(Debug, Clone, Default)]
pub struct Runs {
    inner: Arc<std::sync::Mutex<RunMap>>,
}

impl Runs {
    fn lock(&self) -> std::sync::MutexGuard<'_, RunMap> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// A run begins; the previous one for this engine is forgotten.
    pub fn begin(&self, engine: &berth_core::EngineId, action: &'static str) {
        self.lock().insert(
            engine.clone(),
            RunLog {
                action,
                started: std::time::Instant::now(),
                lines: Vec::new(),
                failure: None,
                ended_at_s: None,
            },
        );
    }

    /// One line of output. Only text is kept; the exit marker is not a line.
    pub fn push(&self, engine: &berth_core::EngineId, line: &berth_sys::ProcessLine) {
        let text = match line {
            berth_sys::ProcessLine::Stdout(text) | berth_sys::ProcessLine::Stderr(text) => {
                text.trim_end().to_string()
            }
            berth_sys::ProcessLine::Exit(_) => return,
        };
        if text.is_empty() {
            return;
        }
        if let Some(run) = self.lock().get_mut(engine) {
            let at_s = run.started.elapsed().as_secs_f64();
            run.lines.push(RunLine { at_s, text });
            if run.lines.len() > RUN_LINES_MAX {
                run.lines.remove(0);
            }
        }
    }

    /// The run ended: success forgets it, failure keeps it with the reason.
    pub fn finish(&self, engine: &berth_core::EngineId, result: &Result<(), String>) {
        let mut runs = self.lock();
        match result {
            Ok(()) => {
                runs.remove(engine);
            }
            Err(reason) => {
                if let Some(run) = runs.get_mut(engine) {
                    run.failure = Some(reason.clone());
                    run.ended_at_s = Some(run.started.elapsed().as_secs_f64());
                }
            }
        }
    }

    /// Forget a kept (failed) run, when the user dismisses it.
    pub fn clear(&self, engine: &berth_core::EngineId) {
        self.lock().remove(engine);
    }

    /// The run now, if there is one.
    #[must_use]
    pub fn view(&self, engine: &berth_core::EngineId) -> Option<RunView> {
        self.lock().get(engine).map(|run| RunView {
            action: run.action,
            elapsed_s: run
                .ended_at_s
                .unwrap_or_else(|| run.started.elapsed().as_secs_f64()),
            lines: run.lines.clone(),
            failure: run.failure.clone(),
        })
    }
}

/// The version `colima version` prints ("0.10.3"), or `None` when Colima is
/// not installed or answers something else. Read-only.
pub async fn version(mock: bool) -> Option<String> {
    if mock {
        return Some("0.10.3".to_string());
    }
    let search = crate::services::login_path::search_path().await;
    let binary = search.find("colima")?;
    let output = berth_sys::run_capture(
        &binary,
        &[std::ffi::OsString::from("version")],
        berth_sys::SpawnOpts {
            path: Some(search.path_var()),
            timeout: Some(std::time::Duration::from_secs(5)),
            ..Default::default()
        },
        CancellationToken::new(),
    )
    .await
    .ok()?;
    if output.code != Some(0) {
        return None;
    }
    parse_version(&output.stdout)
}

/// The first line of `colima version`, "colima version 0.10.3", gives "0.10.3".
#[must_use]
pub fn parse_version(stdout: &str) -> Option<String> {
    let first = stdout.lines().next()?;
    let version = first.strip_prefix("colima version ")?.trim();
    (!version.is_empty()).then(|| version.trim_start_matches('v').to_string())
}

/// The real control (colima from `SearchPath`) or, under `BERTH_MOCK=1`,
/// the in-memory mock.
pub async fn control(mock: bool) -> Arc<dyn berth_engine::ColimaControl> {
    if mock {
        return Arc::new(berth_engine::MockColima::bundled());
    }
    let search = crate::services::login_path::search_path().await;
    Arc::new(berth_engine::ColimaCli::new(search))
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_engine::ColimaControl as _;

    #[tokio::test]
    async fn actions_colima_start_and_stop_flip_the_mock_profile() {
        let mock = berth_engine::MockColima::bundled();
        let before = mock.status("default").await.expect("profile");
        assert!(before.running);
        run(
            &mock,
            "default",
            berth_core::ColimaOp::Stop,
            CancellationToken::new(),
        )
        .await
        .expect("stop");
        assert!(!mock.status("default").await.expect("profile").running);
        run(
            &mock,
            "default",
            berth_core::ColimaOp::Start,
            CancellationToken::new(),
        )
        .await
        .expect("start");
        assert!(mock.status("default").await.expect("profile").running);
    }

    #[tokio::test]
    async fn actions_colima_nonzero_exit_reports_the_stderr_tail() {
        let stream = futures::stream::iter([
            Ok(berth_sys::ProcessLine::Stdout("starting".to_string())),
            Ok(berth_sys::ProcessLine::Stderr(
                "disk image is locked".to_string(),
            )),
            Ok(berth_sys::ProcessLine::Exit(Some(1))),
        ])
        .boxed();
        assert_eq!(
            drain(stream, &|_| {}).await,
            Err("disk image is locked".to_string())
        );
        let bare = futures::stream::iter([Ok(berth_sys::ProcessLine::Exit(Some(2)))]).boxed();
        assert_eq!(
            drain(bare, &|_| {}).await,
            Err("colima exited with code 2".to_string())
        );
        let ok = futures::stream::iter([Ok(berth_sys::ProcessLine::Exit(Some(0)))]).boxed();
        assert_eq!(drain(ok, &|_| {}).await, Ok(()));
    }

    #[test]
    fn actions_colima_op_ids_are_action_def_ids() {
        for op in [
            berth_core::ColimaOp::Start,
            berth_core::ColimaOp::Stop,
            berth_core::ColimaOp::Restart,
            berth_core::ColimaOp::Apply(berth_core::ColimaStartOptions {
                cpus: Some(4),
                memory_gib: None,
                disk_gib: None,
                vm_type: None,
                rosetta: None,
                arch: None,
            }),
        ] {
            assert!(
                crate::actions::action_by_id(action_id(&op)).is_some(),
                "{op:?} has no ActionDef"
            );
        }
    }
}
