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

/// Drain one Colima command. `Ok` only on exit code 0; the failure message
/// is the stderr tail, else a sentence naming the exit code.
async fn drain(mut stream: Lines) -> Result<(), String> {
    let mut tail: Vec<String> = Vec::new();
    while let Some(item) = stream.next().await {
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
/// options, because Colima only changes a profile's resources at start.
pub async fn run(
    control: &dyn berth_engine::ColimaControl,
    profile: &str,
    op: berth_core::ColimaOp,
    cancel: CancellationToken,
) -> Result<(), String> {
    match op {
        berth_core::ColimaOp::Start => drain(control.start(profile, None, cancel)).await,
        berth_core::ColimaOp::Stop => drain(control.stop(profile, cancel)).await,
        berth_core::ColimaOp::Restart => drain(control.restart(profile, cancel)).await,
        berth_core::ColimaOp::Apply(options) => {
            drain(control.stop(profile, cancel.clone())).await?;
            drain(control.start(profile, Some(options), cancel)).await
        }
    }
}

/// The real control (colima from `SearchPath`) or, under `BERTH_MOCK=1`,
/// the in-memory mock.
pub async fn control(mock: bool) -> Arc<dyn berth_engine::ColimaControl> {
    if mock {
        return Arc::new(berth_engine::MockColima::bundled());
    }
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
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
        assert_eq!(drain(stream).await, Err("disk image is locked".to_string()));
        let bare = futures::stream::iter([Ok(berth_sys::ProcessLine::Exit(Some(2)))]).boxed();
        assert_eq!(
            drain(bare).await,
            Err("colima exited with code 2".to_string())
        );
        let ok = futures::stream::iter([Ok(berth_sys::ProcessLine::Exit(Some(0)))]).boxed();
        assert_eq!(drain(ok).await, Ok(()));
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
