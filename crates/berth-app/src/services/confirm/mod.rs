//! Confirm models (D33): the one place that builds the `EffectSummary` of
//! a container or engine action that asks first. The dialog's button, its
//! text and the result message all render from that one value
//! ([`button_label`], the lines, [`result_label`]); `search.rs` (palette),
//! the row, the bulk bar and ⌫ all come here.
//!
//! [`run`] is the other half: it sends a confirmed model to the actions
//! service and returns the pending keys, so the caller can watch for the
//! outcome ([`watch_result`]) and say it from the same summary.
//!
//! One file per resource builds its effects and models and starts its own
//! kind of confirmed action; this file holds what every kind shares: the
//! line and summary helpers, the labels, and the rule (D2, D25) that a
//! confirmed model only acts on the resources its action names. A new
//! kind adds a file, a `pub use` line and an entry in `RUNNERS`.

use berth_core::{
    ColimaOp, Container, ContainerAction, ContainerId, ContainerState, EffectKind, EffectLine,
    EffectSummary, EngineAction, EngineId, Network, ResourceRef, Subject, Volume,
};
use dioxus::prelude::{ReadableExt, WritableExt};

use crate::services::actions::{container_action_id, ActionsService, ContainerTarget};
use crate::state::{AppState, ConfirmModel, PendingKey, PendingState, Toast, ToastTone};

mod containers;
mod engines;
mod images;
mod networks;
mod remove;
mod upload;
mod volumes;

pub use containers::*;
pub use engines::*;
pub use images::*;
pub use networks::*;
pub use upload::*;
pub use volumes::*;

/// "1 container" or "3 containers".
#[must_use]
pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

fn line(kind: EffectKind, text: impl Into<String>) -> EffectLine {
    EffectLine {
        kind,
        text: text.into(),
    }
}

fn summary(
    verb: &str,
    noun_one: &str,
    noun_many: &str,
    count: usize,
    lines: Vec<EffectLine>,
) -> EffectSummary {
    EffectSummary {
        verb: verb.to_string(),
        noun_one: noun_one.to_string(),
        noun_many: noun_many.to_string(),
        count: u32::try_from(count).unwrap_or(u32::MAX),
        bytes: None,
        lines,
        typed_name: None,
    }
}

fn is_active(state: ContainerState) -> bool {
    matches!(
        state,
        ContainerState::Running | ContainerState::Restarting | ContainerState::Paused
    )
}

/// The confirm button's text, from the summary alone: "Delete container",
/// "Delete 4 containers", "Stop and delete" when something runs.
#[must_use]
pub fn button_label(effect: &EffectSummary) -> String {
    let stops_first =
        effect.verb == "Delete" && effect.lines.iter().any(|l| l.kind == EffectKind::Changes);
    match (stops_first, effect.count) {
        (true, 1) => "Stop and delete".to_string(),
        (true, n) => format!("Stop and delete {n} {}", effect.noun_many),
        (false, 1) => format!("{} {}", effect.verb, effect.noun_one),
        (false, _) => effect.button_label(),
    }
}

/// The result message when `done` of the summary's items went through:
/// the same count and nouns as the button, in the past tense.
#[must_use]
pub fn result_label(effect: &EffectSummary, done: usize) -> String {
    let mut done_effect = effect.clone();
    done_effect.count = u32::try_from(done).unwrap_or(u32::MAX);
    done_effect.bytes = None;
    done_effect.result_label()
}

// ---------------------------------------------------------------------
// Running a confirmed model
// ---------------------------------------------------------------------

/// What [`run`] started: the pending keys to watch and the reasons the
/// actions service refused the rest.
#[derive(Debug, Default, PartialEq)]
pub struct Launched {
    pub keys: Vec<PendingKey>,
    pub refused: Vec<String>,
}

/// A confirmed model being started: the state and service it runs against,
/// the dialog's checkbox and what has been started so far.
pub(super) struct Run<'a> {
    pub state: &'a mut AppState,
    pub actions: &'a ActionsService,
    pub model: &'a ConfirmModel,
    pub checked: bool,
    pub out: Launched,
}

impl Run<'_> {
    /// D2, D25: every target of the model must be a resource the action
    /// names (`names` says so). A target that is not is refused with
    /// `refusal` and never started, so a dialog opened for one resource
    /// cannot act on another. `start` begins one accepted target.
    pub(super) fn each_named(
        &mut self,
        names: impl Fn(&Subject) -> bool,
        refusal: &str,
        mut start: impl FnMut(&mut Self, &Subject),
    ) {
        let model = self.model;
        for target in &model.targets {
            if names(target) {
                start(self, target);
            } else {
                self.out.refused.push(refusal.to_string());
            }
        }
    }
}

/// One kind of confirmed action. Returns `false` when `ctx.model.action`
/// is not this kind's, so the next runner can try.
type Runner = fn(&mut Run<'_>) -> bool;

const RUNNERS: &[Runner] = &[
    containers::run,
    containers::run_recreate,
    engines::run,
    remove::run,
];

/// Send a confirmed model to the actions service. `checked` is the state
/// of the dialog's checkbox (it means "also delete anonymous volumes" on a
/// delete).
pub fn run(
    state: &mut AppState,
    actions: &ActionsService,
    model: &ConfirmModel,
    checked: bool,
) -> Launched {
    let mut ctx = Run {
        state,
        actions,
        model,
        checked,
        out: Launched::default(),
    };
    if !RUNNERS.iter().any(|runner| runner(&mut ctx)) {
        ctx.out
            .refused
            .push("This action cannot run from here yet".to_string());
    }
    ctx.out
}

/// How many of `keys` finished without a failure, once none is running.
/// `None` while one still runs.
#[must_use]
pub fn finished_count(
    pending: &std::collections::BTreeMap<PendingKey, PendingState>,
    keys: &[PendingKey],
) -> Option<usize> {
    if keys
        .iter()
        .any(|k| matches!(pending.get(k), Some(PendingState::Running { .. })))
    {
        return None;
    }
    Some(
        keys.iter()
            .filter(|k| !matches!(pending.get(*k), Some(PendingState::Failed { .. })))
            .count(),
    )
}

fn push_toast(state: &mut AppState, text: String, tone: ToastTone) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone,
        action: None,
    });
}

/// After [`run`]: say what was refused, then wait for the started actions
/// and say what went through, from the same summary the dialog showed.
/// Failures stay on their rows (D34), so only successes are counted.
pub async fn watch_result(mut state: AppState, effect: EffectSummary, launched: Launched) {
    for reason in launched.refused {
        push_toast(&mut state, reason, ToastTone::Danger);
    }
    if launched.keys.is_empty() {
        return;
    }
    let done = loop {
        if let Some(done) = finished_count(&state.pending.peek(), &launched.keys) {
            break done;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    if done > 0 {
        push_toast(&mut state, result_label(&effect, done), ToastTone::Success);
    }
}

#[cfg(test)]
mod tests;
