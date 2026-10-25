//! The container detail's view model (P3-T3): pure functions from a
//! container, its detail and the app state to what the inspector draws.
//! Nothing here reads a signal or calls an engine, so tests build the
//! inputs by hand.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use berth_core::{
    Container, ContainerAction, ContainerDetail, ContainerState, EngineAction, EnvVar, FixEffect,
    FixSource, Health, Mount, MountKind, Problem, ProblemKind, RestartPolicy, Subject,
};

use crate::services::actions::{container_action_id, disabled_reason, StateView};
use crate::services::group_ops::is_active;
use crate::state::{PendingKey, PendingState};
use crate::ui::tabs::TabItem;
use crate::ui::{FixButton, HeaderButton, HeaderTone, KeyValue, MenuEntry, ProblemView};

// ---------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DetailTab {
    #[default]
    Overview,
    Logs,
    Terminal,
    Stats,
    Inspect,
    Env,
    Mounts,
    Files,
    Limits,
}

impl DetailTab {
    /// In strip order: the first five fit the frame's 360 px, the rest sit
    /// under ⋯ ("Containers / selected / 1440").
    pub const ALL: [DetailTab; 9] = [
        Self::Overview,
        Self::Logs,
        Self::Terminal,
        Self::Stats,
        Self::Inspect,
        Self::Env,
        Self::Mounts,
        Self::Files,
        Self::Limits,
    ];

    /// Logs and Terminal live in the bottom panel's own tabs, so the
    /// inspector hides them while that panel sits beside it (D22).
    pub const STREAMS: [DetailTab; 2] = [Self::Logs, Self::Terminal];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Logs => "logs",
            Self::Terminal => "terminal",
            Self::Stats => "stats",
            Self::Inspect => "inspect",
            Self::Env => "env",
            Self::Mounts => "mounts",
            Self::Files => "files",
            Self::Limits => "limits",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Logs => "Logs",
            Self::Terminal => "Terminal",
            Self::Stats => "Stats",
            Self::Inspect => "Inspect",
            Self::Env => "Env",
            Self::Mounts => "Mounts",
            Self::Files => "Files",
            Self::Limits => "Limits",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.id() == id)
    }
}

/// How many tabs a strip `pane_px` wide holds at a text scale. A tab is its
/// label, which grows with the text (about 41 px at 100 %), and 24 px of
/// padding, which does not; the ⋯ button takes 28 px. Measured against the
/// frame's 360 px at each step this gives five tabs up to 100 %, four up to
/// 130 %, three above that; a wider pane shows more, a narrower one fewer.
#[must_use]
pub fn tab_capacity(pane_px: f32, text_scale: f32) -> usize {
    let room = pane_px - 28.0;
    let per_tab = 24.0 + 41.0 * text_scale;
    // Truncation is the point: whole tabs only.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fit = (room / per_tab).floor().max(0.0) as usize;
    fit.clamp(2, DetailTab::ALL.len())
}

/// The tabs in the strip: the first `capacity` that are not `hidden`, with a
/// selected tab that lives under ⋯ taking the last place so the selection
/// is always seen. A hidden tab is never shown, selected or not.
#[must_use]
pub fn visible_tabs(
    selected: DetailTab,
    pane_px: f32,
    text_scale: f32,
    hidden: &[DetailTab],
) -> Vec<DetailTab> {
    let capacity = tab_capacity(pane_px, text_scale);
    let mut tabs: Vec<DetailTab> = DetailTab::ALL
        .into_iter()
        .filter(|tab| !hidden.contains(tab))
        .take(capacity)
        .collect();
    if !tabs.contains(&selected) && !hidden.contains(&selected) {
        if let Some(last) = tabs.last_mut() {
            *last = selected;
        }
    }
    tabs
}

/// The strip's items and the ⋯ menu's entries.
#[must_use]
pub fn tab_strip(
    selected: DetailTab,
    pane_px: f32,
    text_scale: f32,
    hidden: &[DetailTab],
) -> (Vec<TabItem>, Vec<MenuEntry>) {
    let visible = visible_tabs(selected, pane_px, text_scale, hidden);
    let items = visible
        .iter()
        .map(|tab| TabItem::new(tab.id(), tab.label()))
        .collect();
    let overflow = DetailTab::ALL
        .iter()
        .filter(|tab| !visible.contains(tab) && !hidden.contains(tab))
        .map(|tab| MenuEntry::new(tab.id(), tab.label()))
        .collect();
    (items, overflow)
}

// ---------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------

/// The twelve characters the header shows of a container id.
#[must_use]
pub fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

fn times(n: u32) -> String {
    if n == 1 {
        "1 time".to_string()
    } else {
        format!("{n} times")
    }
}

/// The muted line next to the status chip, in Docker's own wording where
/// it has one ("Restarting (1) 3 seconds ago · restarted 14 times").
#[must_use]
pub fn state_note(c: &Container, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    let restarted = (c.restart_count > 0).then(|| format!("restarted {}", times(c.restart_count)));
    let since = |prefix: &str| {
        c.started
            .map(|t| format!("{prefix} {}", berth_core::fmt::uptime(t, now)))
    };
    let head = match c.state {
        ContainerState::Restarting => {
            let code = c.exit_code.unwrap_or(0);
            Some(match c.started {
                Some(t) => format!("Restarting ({code}) {}", berth_core::fmt::age(t, now)),
                None => format!("Restarting ({code})"),
            })
        }
        ContainerState::Running => since("Up"),
        ContainerState::Paused => since("Paused, up"),
        ContainerState::Exited | ContainerState::Dead => c
            .started
            .map(|t| format!("Last started {}", berth_core::fmt::age(t, now))),
        ContainerState::Created => {
            Some(format!("Created {}", berth_core::fmt::age(c.created, now)))
        }
        ContainerState::Removing => None,
    };
    match (head, restarted) {
        (Some(head), Some(restarted)) => Some(format!("{head} · {restarted}")),
        (Some(head), None) => Some(head),
        (None, Some(restarted)) => Some(restarted),
        (None, None) => None,
    }
}

fn running_pending<'a>(
    pending: &'a BTreeMap<PendingKey, PendingState>,
    c: &Container,
) -> Option<(&'static str, &'a str)> {
    pending
        .iter()
        .find_map(|(key, value)| match (&key.subject, value) {
            (Subject::Container(id), PendingState::Running { label }) if *id == c.id => {
                Some((key.action, label.as_str()))
            }
            _ => None,
        })
}

/// The labelled buttons: Start or Stop, Restart, Pause or Resume, Kill,
/// Delete. A button that cannot run now stays and carries its reason.
#[must_use]
pub fn header_buttons(c: &Container, view: &StateView<'_>) -> Vec<HeaderButton> {
    let subject = Subject::Container(c.id.clone());
    let pending = running_pending(view.pending, c);
    let button = |id: &'static str,
                  label: &str,
                  icon: &'static str,
                  action: &'static str,
                  tone: HeaderTone| {
        let running = pending
            .filter(|(running, _)| *running == action || (id == "start-stop" && is_toggle(running)))
            .map(|(_, text)| text.to_string());
        HeaderButton::new(
            id,
            running.clone().unwrap_or_else(|| label.to_string()),
            icon,
        )
        .tone(tone)
        .reason(disabled_reason(&subject, action, view))
        .loading(running.is_some())
    };
    let (toggle_label, toggle_icon, toggle_action) = if is_active(c.state) {
        ("Stop", "square", "container.stop")
    } else {
        ("Start", "play", "container.start")
    };
    let (pause_label, pause_icon, pause_action) = if c.state == ContainerState::Paused {
        ("Resume", "play", "container.resume")
    } else {
        ("Pause", "pause", "container.pause")
    };
    vec![
        button(
            "start-stop",
            toggle_label,
            toggle_icon,
            toggle_action,
            HeaderTone::Neutral,
        ),
        button(
            "restart",
            "Restart",
            "rotate-cw",
            "container.restart",
            HeaderTone::Neutral,
        ),
        button(
            "pause-resume",
            pause_label,
            pause_icon,
            pause_action,
            HeaderTone::Neutral,
        ),
        button(
            "kill",
            "Kill",
            "zap",
            "container.kill",
            HeaderTone::DangerIcon,
        ),
        button(
            "delete",
            "Delete…",
            "trash-2",
            "container.delete",
            HeaderTone::Danger,
        ),
    ]
}

fn is_toggle(action: &str) -> bool {
    matches!(action, "container.start" | "container.stop")
}

/// The ⋯ menu: Edit, Duplicate, Recreate with latest image and Rename work
/// (P3-T7); the later tasks' items stay present and disabled with the task
/// that enables them (D34: disabled stays visible, with a reason).
/// A Compose-managed container is never recreated outside Compose (D17).
#[must_use]
pub fn header_menu(c: &Container, backing: crate::services::backing::Backing) -> Vec<MenuEntry> {
    let later = |id: &str, label: &str, task: &str| {
        MenuEntry::new(id, label).disabled(format!("Built in {task}"))
    };
    // A mock engine cannot create or recreate: the entries that would end
    // in its raw error stay visible and say why.
    let creates = |entry: MenuEntry, action: &str| match backing.unsupported_action(action) {
        Some(reason) => entry.disabled(reason),
        None => entry,
    };
    vec![
        creates(MenuEntry::new("edit", "Edit…").kbd("E"), "container.edit"),
        creates(
            MenuEntry::new("duplicate", "Duplicate"),
            "container.duplicate",
        ),
        if c.labels.contains_key("com.docker.compose.project") {
            MenuEntry::new("recreate", "Recreate with latest image").disabled("Managed by Compose")
        } else {
            creates(
                MenuEntry::new("recreate", "Recreate with latest image"),
                "container.recreate_latest",
            )
        },
        MenuEntry::new("rename", "Rename…"),
        later("save-image", "Save as image…", "P8-T4"),
        MenuEntry::new("copy-id", "Copy container ID")
            .icon("copy")
            .separator_before(),
    ]
}

// ---------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------

/// "unless-stopped", as Docker names the policies.
#[must_use]
pub fn restart_label(policy: &RestartPolicy) -> String {
    match policy {
        RestartPolicy::No => "no".to_string(),
        RestartPolicy::Always => "always".to_string(),
        RestartPolicy::UnlessStopped => "unless-stopped".to_string(),
        RestartPolicy::OnFailure { max_retries: 0 } => "on-failure".to_string(),
        RestartPolicy::OnFailure { max_retries } => format!("on-failure:{max_retries}"),
    }
}

/// `./worker → /app (bind)`: a bind source under the Compose folder reads
/// relative to it, other paths with `~`.
#[must_use]
pub fn mount_line(m: &Mount, compose_dir: Option<&str>, home: &std::path::Path) -> String {
    let kind = match m.kind {
        MountKind::Bind => "bind",
        MountKind::Volume => "volume",
        MountKind::Tmpfs => "tmpfs",
        MountKind::Other => "other",
    };
    let source = match m.kind {
        MountKind::Volume => m.volume_name.clone().unwrap_or_else(|| m.source.clone()),
        MountKind::Tmpfs => "tmpfs".to_string(),
        _ => {
            let path = std::path::Path::new(&m.source);
            match compose_dir
                .map(std::path::Path::new)
                .and_then(|dir| path.strip_prefix(dir).ok())
            {
                Some(rest) if rest.as_os_str().is_empty() => ".".to_string(),
                Some(rest) => format!("./{}", rest.display()),
                None => berth_core::fmt::tilde(path, home),
            }
        }
    };
    let mode = if m.read_only { ", read-only" } else { "" };
    format!("{source} → {} ({kind}{mode})", m.destination)
}

fn command_of(detail: &ContainerDetail) -> String {
    if let Some(command) = &detail.container.command {
        return command.clone();
    }
    detail
        .entrypoint
        .iter()
        .flatten()
        .chain(detail.cmd.iter().flatten())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The Overview's key-value rows, in the frame's order.
#[must_use]
pub fn overview_rows(detail: &ContainerDetail, home: &std::path::Path) -> Vec<KeyValue> {
    let c = &detail.container;
    let compose_dir = c
        .labels
        .get("com.docker.compose.project.working_dir")
        .map(String::as_str);
    let ports = c
        .ports
        .iter()
        .map(berth_core::fmt::port)
        .collect::<Vec<_>>()
        .join("\n");
    let mounts = c
        .mounts
        .iter()
        .map(|m| mount_line(m, compose_dir, home))
        .collect::<Vec<_>>()
        .join("\n");
    let mut rows = vec![
        KeyValue::mono("Image", c.image.reference.clone()),
        KeyValue::mono("Command", command_of(detail)),
        KeyValue::mono("Ports", ports).empty_text("–"),
        KeyValue::mono("Mounts", mounts).empty_text("–"),
        KeyValue::mono("Networks", c.networks.join("\n")).empty_text("–"),
        KeyValue::text("Restart policy", restart_label(&detail.restart)),
    ];
    if let Some(health) = c.health {
        rows.push(KeyValue::text(
            "Health",
            match health {
                Health::Starting => "Starting",
                Health::Healthy => "Healthy",
                Health::Unhealthy => "Unhealthy",
            },
        ));
    }
    let shows_exit = matches!(
        c.state,
        ContainerState::Exited | ContainerState::Dead | ContainerState::Restarting
    );
    if let (true, Some(code)) = (shows_exit, c.exit_code) {
        let row = KeyValue::text("Exit code", code.to_string());
        rows.push(if code == 0 { row } else { row.danger() });
    }
    rows
}

// ---------------------------------------------------------------------
// Problems in the inspector
// ---------------------------------------------------------------------

/// The Problems of this container, as the inspector's compact items.
/// Fix ids are `<problem id>#<index>`; "Go to X" is `goto:<container id>`.
#[must_use]
pub fn problem_views(
    c: &Container,
    problems: &[Problem],
    siblings: &[Container],
    view: &StateView<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<ProblemView> {
    problems
        .iter()
        .filter(|p| p.subject == Subject::Container(c.id.clone()))
        .map(|p| problem_view(p, siblings, view, now))
        .collect()
}

fn problem_view(
    p: &Problem,
    siblings: &[Container],
    view: &StateView<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> ProblemView {
    let mut fixes: Vec<FixButton> = Vec::new();
    for (index, fix) in p.fixes.iter().enumerate() {
        let id = format!("{}#{index}", p.id.0);
        let (disabled_reason, loading) = match (&fix.source, &fix.effect) {
            (FixSource::Harness(_), _) => (Some("Needs a harness (P7-T6)".to_string()), false),
            (
                FixSource::Rule,
                FixEffect::RunAction {
                    action: EngineAction::Container { id, action, .. },
                    ..
                },
            ) => {
                let key = container_action_id(action);
                let loading = view.pending.iter().any(|(k, v)| {
                    k.action == key
                        && k.subject == Subject::Container(id.clone())
                        && matches!(v, PendingState::Running { .. })
                });
                (
                    disabled_reason(&Subject::Container(id.clone()), key, view),
                    loading,
                )
            }
            (FixSource::Rule, _) => (Some("Built in P5-T7".to_string()), false),
        };
        let harness = matches!(fix.source, FixSource::Harness(_));
        fixes.push(FixButton {
            id,
            label: fix.label.clone(),
            primary: index == 0 && !harness,
            icon: harness.then_some("sparkles"),
            split: harness,
            disabled_reason,
            loading,
        });
    }
    if let ProblemKind::DependencyFailed { service } = &p.kind {
        if let Some(dependency) = dependency_of(p, service, siblings) {
            let go = FixButton {
                id: format!("goto:{}", dependency.id.0),
                label: format!("Go to {}", dependency.name),
                primary: false,
                icon: None,
                split: false,
                disabled_reason: None,
                loading: false,
            };
            let at = fixes.iter().position(|f| f.split).unwrap_or(fixes.len());
            fixes.insert(at, go);
        }
    }
    ProblemView {
        severity: p.severity,
        title: p.title.clone(),
        age: berth_core::fmt::uptime(p.first_seen, now),
        explanation: p.explanation.clone(),
        evidence: p.evidence.clone(),
        fixes,
    }
}

/// The container of the service a failed dependency names, in the same
/// Compose project as the problem's container.
fn dependency_of<'a>(
    p: &Problem,
    service: &str,
    siblings: &'a [Container],
) -> Option<&'a Container> {
    let Subject::Container(id) = &p.subject else {
        return None;
    };
    let project = siblings
        .iter()
        .find(|c| c.id == *id)?
        .compose_project
        .as_ref()?;
    siblings.iter().find(|c| {
        c.compose_project.as_ref() == Some(project) && c.service.as_deref() == Some(service)
    })
}

/// The card for a restart loop nobody has diagnosed yet: shown when the
/// container restarts and no Problem names it.
#[must_use]
pub fn restart_loop_view(c: &Container, view: &StateView<'_>) -> ProblemView {
    let subject = Subject::Container(c.id.clone());
    let code = c
        .exit_code
        .map_or_else(String::new, |code| format!(" with code {code}"));
    ProblemView {
        severity: berth_core::Severity::Warning,
        title: format!("{} keeps restarting", c.name),
        age: String::new(),
        explanation: format!(
            "It exits{code} and Docker starts it again; it has restarted {}.",
            times(c.restart_count)
        ),
        evidence: Vec::new(),
        fixes: vec![FixButton {
            id: format!("stop-loop:{}", c.id.0),
            label: "Stop the restart loop".to_string(),
            primary: true,
            icon: None,
            split: false,
            disabled_reason: disabled_reason(&subject, "container.stop", view),
            loading: view.pending.iter().any(|(k, v)| {
                k.action == "container.stop"
                    && k.subject == subject
                    && matches!(v, PendingState::Running { .. })
            }),
        }],
    }
}

/// Does any problem item already offer to stop this container's loop?
#[must_use]
pub fn offers_stop(problems: &[Problem], c: &Container) -> bool {
    problems.iter().any(|p| {
        p.subject == Subject::Container(c.id.clone())
            && p.fixes.iter().any(|f| {
                matches!(
                    &f.effect,
                    FixEffect::RunAction {
                        action: EngineAction::Container {
                            action: ContainerAction::Stop { .. },
                            ..
                        },
                        ..
                    }
                )
            })
    })
}

// ---------------------------------------------------------------------
// Env
// ---------------------------------------------------------------------

/// How long a revealed value stays readable (D79).
pub const REVEAL_FOR: Duration = Duration::from_secs(30);

/// Which env values are revealed, per key, until when. UI-only state: it
/// lives in the panel and is gone when the panel is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reveals {
    until: BTreeMap<String, Instant>,
}

impl Reveals {
    pub fn reveal(&mut self, key: &str, now: Instant) {
        self.until.insert(key.to_string(), now + REVEAL_FOR);
    }

    pub fn hide(&mut self, key: &str) {
        self.until.remove(key);
    }

    /// Mask every value whose time is up.
    pub fn expire(&mut self, now: Instant) {
        self.until.retain(|_, until| *until > now);
    }

    #[must_use]
    pub fn is_revealed(&self, key: &str, now: Instant) -> bool {
        self.until.get(key).is_some_and(|until| *until > now)
    }
}

/// What one env row shows. `Masked` carries no value at all, so a masked
/// row cannot put one in the markup.
#[derive(Clone, PartialEq, Eq)]
pub enum EnvShown {
    Masked,
    Revealed(String),
}

// Not derived: a derived `Debug` would print a revealed value.
impl std::fmt::Debug for EnvShown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Masked => f.write_str("Masked"),
            Self::Revealed(_) => f.write_str("Revealed(..)"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvRow {
    pub key: String,
    pub shown: EnvShown,
}

#[must_use]
pub fn env_rows(env: &[EnvVar], reveals: &Reveals, now: Instant) -> Vec<EnvRow> {
    env.iter()
        .map(|var| EnvRow {
            key: var.key.clone(),
            shown: if reveals.is_revealed(&var.key, now) {
                EnvShown::Revealed(var.value.clone())
            } else {
                EnvShown::Masked
            },
        })
        .collect()
}
