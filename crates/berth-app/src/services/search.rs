//! Palette and global search (P2-T9, PLAN §6.11, brief §7.3).
//!
//! One [`Index`] is built from the engine state whenever it changes
//! (the palette memoizes it); every keystroke then runs [`Index::search`],
//! which allocates no per-container strings and matches with
//! `nucleo-matcher`. The module has three layers:
//!
//! * pure search (`Index`, `Results`, `SearchItem`), unit-tested without a
//!   DOM;
//! * recents (`RecentKey` and the MRU helpers; activity entries seed them);
//! * `run`, which carries out the chosen item through `ActionsService` or
//!   the navigation state, so the UI itself never calls an engine (D23).
//!
//! Query grammar. The query is split on whitespace into atoms and every
//! atom must match.
//!
//! * `restart berthfx-web`: an atom that matches a verb's label
//!   ("Restart", "Restart project") is the verb, the other atoms must match
//!   the subject's name. The result is the action on that subject.
//! * A verb alone (`rest`) acts on the subject in context: the selection,
//!   the project and engine it belongs to, else the one that most needs
//!   attention (a restarting or failing container before a healthy one).
//! * Every engine is searched (AD4). The engine selected in the title-bar
//!   switcher ranks first, a little below the gap acting gets over
//!   opening, and results from the other engines carry the engine name.
//!   A bare verb prefers the selected engine's subjects.
//! * `:5432` (or a bare number of two digits or more) matches published
//!   and container ports; other atoms match names, then image, project,
//!   folder, labels and engine name at a lower rank.
//! * Acting beats opening: a verb match outranks an "open" match of the
//!   same strength.

use dioxus::prelude::{ReadableExt, WritableExt};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use berth_core::{
    ColimaOp, ContainerAction, ContainerState, EngineKind, EngineState, ProjectStatus, Subject,
};

use crate::actions::{action_by_id, ActionId};
use crate::services::actions::{ActionsService, ContainerTarget, ProjectOp, StateView};
use crate::services::confirm;
use crate::state::{
    AppState, ConfirmModel, EngineScope, Overlay, Section, Selection, Toast, ToastTone,
};

// ---------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------

/// Result groups in tie order (the order the palette lists equal groups).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Recent,
    Actions,
    Containers,
    Projects,
    Images,
    Volumes,
    Networks,
    Engine,
    /// Commands enabled plugins contribute (P9-T5); the palette adds them to a typed query.
    Plugins,
}

impl Group {
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Actions => "Actions",
            Self::Containers => "Containers",
            Self::Projects => "Projects",
            Self::Images => "Images",
            Self::Volumes => "Volumes",
            Self::Networks => "Networks",
            Self::Engine => "Engine",
            Self::Plugins => "Plugins",
        }
    }
}

/// What Enter does with an item.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// An `ActionDef` id, on a subject or (for global actions) on nothing.
    Run {
        action: ActionId,
        subject: Option<Subject>,
    },
    /// Select the record and show its section.
    Open(Subject),
    /// A command an enabled plugin contributes (P9-T5).
    Plugin(crate::services::plugin_views::PluginCommand),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchItem {
    pub group: Group,
    /// Sprite icon name.
    pub icon: &'static str,
    /// "Restart shop-api-worker-1", "Restart engine default…", "pg-scratch".
    pub title: String,
    /// Muted text after the title: "Container · Restarting".
    pub meta: String,
    /// The `ActionDef` shortcut, shown only when it acts on the selection.
    pub shortcut: Option<&'static str>,
    pub danger: bool,
    /// Enter opens a confirm dialog instead of acting at once.
    pub confirm: bool,
    pub target: Target,
    pub score: u32,
}

impl SearchItem {
    /// The identity recents remember.
    #[must_use]
    pub fn recent_key(&self) -> RecentKey {
        match &self.target {
            Target::Run { action, subject } => RecentKey {
                action: (*action).to_string(),
                subject: subject.clone(),
            },
            Target::Open(subject) => RecentKey {
                action: OPEN.to_string(),
                subject: Some(subject.clone()),
            },
            Target::Plugin(command) => RecentKey {
                action: format!("plugin:{}:{}", command.plugin, command.id),
                subject: None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultGroup {
    pub group: Group,
    pub items: Vec<SearchItem>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Results {
    pub groups: Vec<ResultGroup>,
    /// Set when a non-empty query matched nothing: the text the palette
    /// offers to run as a docker command (a terminal tab, P3-T6).
    pub docker_command: Option<String>,
}

impl Results {
    /// Every item in display order (what the arrow keys walk).
    #[must_use]
    pub fn flat(&self) -> Vec<&SearchItem> {
        self.groups.iter().flat_map(|g| g.items.iter()).collect()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(|g| g.items.is_empty())
    }
}

// ---------------------------------------------------------------------
// Tuning
// ---------------------------------------------------------------------

/// A verb match outranks an equally strong "open" match.
const ACTION_BONUS: u32 = 32;
/// A result on the engine the title-bar switcher selects outranks an equal
/// one on another engine, but not an action over an open (AD4).
const HOME_BONUS: u32 = 16;
const _: () = assert!(HOME_BONUS < ACTION_BONUS);
const PORT_EXACT: u32 = 240;
const PORT_PREFIX: u32 = 120;
/// Matches in image, folder, labels and the like rank below name matches.
const SECONDARY_PENALTY: u32 = 24;
/// A fuzzy match may skip this many characters per three query characters
/// (at least one in total): "shp" finds "shop-api", but "rest" does not
/// find "redis-scratch", which holds the same letters far apart. Score
/// alone cannot tell the two apart (nucleo rewards word starts).
const GAP_PER_THREE_CHARS: u32 = 1;
const GROUP_CAP: usize = 6;
const RECENT_CAP: usize = 5;
const EMPTY_ACTIONS_CAP: usize = 8;

const OPEN: &str = "open";

// ---------------------------------------------------------------------
// Verbs
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Verb {
    id: ActionId,
    /// What the palette says and what the query's verb atom matches.
    label: &'static str,
    icon: &'static str,
    confirm: bool,
}

const fn verb(id: ActionId, label: &'static str, icon: &'static str, confirm: bool) -> Verb {
    Verb {
        id,
        label,
        icon,
        confirm,
    }
}

const CONTAINER_VERBS: &[Verb] = &[
    verb("container.start", "Start", "play", false),
    verb("container.stop", "Stop", "square", false),
    verb("container.restart", "Restart", "rotate-cw", false),
    verb("container.pause", "Pause", "pause", false),
    verb("container.resume", "Resume", "play", false),
    verb("container.kill", "Kill", "zap", true),
    verb("container.delete", "Delete", "trash-2", true),
    verb("container.logs", "Show logs of", "logs", false),
    verb("container.terminal", "Open terminal in", "terminal", false),
    verb("container.edit", "Edit and recreate", "settings", false),
];

const PROJECT_VERBS: &[Verb] = &[
    verb("project.start", "Start project", "play", false),
    verb("project.stop", "Stop project", "square", false),
    verb("project.restart", "Restart project", "rotate-cw", false),
    verb("project.rebuild", "Rebuild project", "layers", false),
    verb(
        "project.pull_images",
        "Pull images of project",
        "arrow-down-to-line",
        false,
    ),
    verb("project.open_folder", "Open folder of", "folder", false),
    verb(
        "project.open_in_editor",
        "Open in editor",
        "open-external",
        false,
    ),
    verb("project.delete", "Delete project", "trash-2", false),
];

const ENGINE_VERBS: &[Verb] = &[
    verb("engine.start", "Start engine", "play", false),
    verb("engine.stop", "Stop engine", "square", true),
    verb("engine.restart", "Restart engine", "rotate-cw", true),
    verb(
        "engine.edit_resources",
        "Edit resources of",
        "settings",
        false,
    ),
    verb("engine.clean_up", "Clean up", "trash-2", false),
];

fn verbs_of(group: Group) -> &'static [Verb] {
    match group {
        Group::Containers => CONTAINER_VERBS,
        Group::Projects => PROJECT_VERBS,
        Group::Engine => ENGINE_VERBS,
        _ => &[],
    }
}

/// Global actions the palette offers, in the order an empty query lists
/// them. An `ActionDef` the palette does not list is named, with its other
/// way in, in `NOT_IN_PALETTE` (search_tests.rs); a palette row that does
/// nothing is worse than none, so an action joins this table with its
/// handler.
const GLOBALS: &[(ActionId, &str, &str)] = &[
    ("global.open_folder", "Open folder…", "folder"),
    ("global.new_project", "New project", "plus"),
    ("global.settings", "Settings", "settings"),
    ("section.projects", "Go to Projects", "folder"),
    ("section.containers", "Go to Containers", "box"),
    ("section.logs", "Go to Logs", "logs"),
    ("section.images", "Go to Images", "layers"),
    ("section.volumes", "Go to Volumes", "hard-drive"),
    ("section.networks", "Go to Networks", "network"),
    ("section.templates", "Go to Templates", "file-text"),
    ("section.engines", "Go to Engines", "server"),
    ("section.activity", "Go to Activity", "activity"),
    (
        "global.toggle_bottom_panel",
        "Toggle bottom panel",
        "terminal",
    ),
    ("assistant.open", "Open assistant", "sparkles"),
    ("volume.create", "Create volume", "plus"),
    ("network.create", "Create network", "plus"),
    ("engine.add", "Add engine", "plus"),
    ("view.text_bigger", "Bigger text", "plus"),
    ("view.text_smaller", "Smaller text", "minus"),
    ("view.text_reset", "Actual text size", "eye"),
    ("global.toggle_inspector", "Toggle inspector", "eye"),
    (
        "view.dock_inspector",
        "Dock inspector at the other side",
        "corner-down-left",
    ),
];

// ---------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    subject: Subject,
    group: Group,
    name: String,
    /// Image, project, folder, labels, engine name: matched at a penalty.
    secondary: Vec<String>,
    ports: Vec<String>,
    /// Muted text of an "open" item (without the group name).
    open_meta: String,
    /// Muted text of verb items: "Container · Restarting".
    verb_meta: String,
    /// 2 selected, 1 belongs to the selection, 0 not in context.
    ctx: u8,
    /// Lower first: what most needs attention.
    attention: u8,
    caps: Vec<ActionId>,
    /// For the engine's confirm text: running containers on it.
    running: usize,
    /// On the engine the title-bar switcher selects: ranks above the rest
    /// by [`HOME_BONUS`].
    home: bool,
}

/// Everything search needs, built once per state change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Index {
    entries: Vec<Entry>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

fn container_attention(
    state: ContainerState,
    health: Option<berth_core::Health>,
    exit: Option<i32>,
) -> u8 {
    use berth_core::status_of;
    use berth_core::StatusKind as S;
    match status_of(state, health, exit) {
        S::Restarting => 0,
        S::Unhealthy => 1,
        S::Dead => 2,
        S::Exited(code) if code != 0 => 2,
        S::Running | S::Healthy | S::Starting => 3,
        S::Paused => 4,
        S::Created => 5,
        S::Exited(_) => 6,
        S::Removing => 7,
    }
}

fn container_caps(state: ContainerState, engine_up: bool) -> Vec<ActionId> {
    use ContainerState as S;
    if !engine_up || state == S::Removing {
        return Vec::new();
    }
    let running = matches!(state, S::Running | S::Restarting);
    let paused = state == S::Paused;
    let mut caps = vec!["container.restart", "container.delete", "container.logs"];
    if !running && !paused {
        caps.push("container.start");
    }
    if running || paused {
        caps.push("container.stop");
        caps.push("container.kill");
    }
    if state == S::Running {
        caps.push("container.pause");
        caps.push("container.terminal");
    }
    caps.push("container.edit");
    if paused {
        caps.push("container.resume");
    }
    caps
}

fn project_caps(project: &berth_core::Project, engine_up: bool) -> Vec<ActionId> {
    if project.folder.is_none() || project.compose_files.is_empty() {
        return Vec::new();
    }
    // Folder, editor and delete work on files and the project's record: no
    // engine needed.
    let mut caps = vec![
        "project.open_folder",
        "project.open_in_editor",
        "project.delete",
    ];
    if !engine_up {
        return caps;
    }
    caps.extend(["project.rebuild", "project.pull_images"]);
    if project.status != ProjectStatus::AllRunning {
        caps.push("project.start");
    }
    if !matches!(
        project.status,
        ProjectStatus::Stopped | ProjectStatus::NotCreated
    ) {
        caps.push("project.stop");
        caps.push("project.restart");
    }
    caps
}

fn project_attention(status: ProjectStatus) -> u8 {
    match status {
        ProjectStatus::Failing => 0,
        ProjectStatus::Partial { .. } => 1,
        ProjectStatus::AllRunning => 2,
        ProjectStatus::Stopped => 3,
        ProjectStatus::NotCreated => 4,
    }
}

fn engine_caps(engine: &berth_core::Engine) -> Vec<ActionId> {
    if !matches!(engine.kind, EngineKind::Colima { .. }) {
        return Vec::new();
    }
    match engine.state {
        EngineState::Stopped => vec!["engine.start"],
        EngineState::Running => vec![
            "engine.stop",
            "engine.restart",
            "engine.edit_resources",
            "engine.clean_up",
        ],
        _ => Vec::new(),
    }
}

fn state_label(state: &EngineState) -> &'static str {
    match state {
        EngineState::Running => "Running",
        EngineState::Stopped => "Stopped",
        EngineState::Starting => "Starting",
        EngineState::Stopping => "Stopping",
        EngineState::Unreachable { .. } => "Unreachable",
        EngineState::Unsupported { .. } => "Unsupported",
    }
}

impl Index {
    /// Index every engine's records (AD4: the palette always searches all
    /// engines). `scope` is the title-bar switcher: the engine it selects
    /// ranks first ([`HOME_BONUS`]) and every other engine's results carry
    /// the engine name; under "All engines" nothing is preferred and
    /// results are tagged when there is more than one engine (D81).
    /// `selection` decides which records are "in context".
    #[must_use]
    pub fn build(view: &StateView<'_>, selection: &Selection, scope: &EngineScope) -> Self {
        let selected_engine = match scope {
            EngineScope::All => None,
            EngineScope::One(id) => Some(id),
        };
        let is_home = |engine: &berth_core::EngineId| selected_engine == Some(engine);
        let engine_name = |id: &berth_core::EngineId| {
            view.engines
                .iter()
                .find(|e| e.id == *id)
                .map_or_else(|| id.0.clone(), |e| e.name.clone())
        };
        let engine_up = |id: &berth_core::EngineId| {
            view.engines
                .iter()
                .find(|e| e.id == *id)
                .is_some_and(|e| e.state == EngineState::Running)
        };
        // A tag only when the result could be confused with another engine:
        // anything off the selected engine, or everything under "All
        // engines" when there are several.
        let multi = view.engines.len() > 1;
        let tag = |id: &berth_core::EngineId| match selected_engine {
            Some(selected) => (selected != id).then(|| engine_name(id)),
            None => multi.then(|| engine_name(id)),
        };
        // Images, volumes and networks know their engine (AD27). Off the
        // selected engine, the result names it.
        let resource_home = |engine: &berth_core::EngineId| is_home(engine);
        let resource_suffix = |engine: &berth_core::EngineId| {
            if selected_engine.is_some() && !is_home(engine) {
                format!(" · {}", engine_name(engine))
            } else {
                String::new()
            }
        };

        let selected_containers: &[berth_core::ContainerId] = match selection {
            Selection::Containers(ids) => ids,
            _ => &[],
        };
        let project_of = |c: &berth_core::Container| {
            view.projects.iter().find(|p| {
                p.engine == c.engine
                    && (c.project.as_ref() == Some(&p.id)
                        || c.compose_project.as_deref() == Some(p.name.as_str()))
            })
        };
        let selected_project_ids: Vec<&berth_core::ProjectId> = match selection {
            Selection::Project(id) => vec![id],
            _ => view
                .containers
                .iter()
                .filter(|c| selected_containers.contains(&c.id))
                .filter_map(|c| project_of(c).map(|p| &p.id))
                .collect(),
        };
        let selected_engines: Vec<&berth_core::EngineId> = match selection {
            Selection::Engine(id) => vec![id],
            Selection::Containers(_) => view
                .containers
                .iter()
                .filter(|c| selected_containers.contains(&c.id))
                .map(|c| &c.engine)
                .collect(),
            Selection::Project(id) => view
                .projects
                .iter()
                .filter(|p| p.id == *id)
                .map(|p| &p.engine)
                .collect(),
            _ => Vec::new(),
        };

        let mut entries = Vec::with_capacity(
            view.containers.len()
                + view.projects.len()
                + view.images.len()
                + view.volumes.len()
                + view.networks.len()
                + view.engines.len(),
        );

        for c in view.containers {
            let status = berth_core::status_of(c.state, c.health, c.exit_code).label();
            let project = project_of(c);
            let mut secondary = vec![c.image.reference.clone()];
            if let Some(name) = &c.compose_project {
                secondary.push(name.clone());
            }
            if let Some(service) = &c.service {
                secondary.push(service.clone());
            }
            if let Some(folder) = project.and_then(|p| p.folder.as_ref()) {
                secondary.push(folder.display().to_string());
            }
            for (key, value) in &c.labels {
                if !key.starts_with("com.docker.compose.") {
                    secondary.push(format!("{key}={value}"));
                }
            }
            secondary.push(engine_name(&c.engine));
            let mut ports: Vec<String> = Vec::new();
            for p in &c.ports {
                if let Some(host) = p.host_port {
                    ports.push(host.to_string());
                }
                ports.push(p.container_port.to_string());
            }
            ports.dedup();
            let port_text = c
                .ports
                .iter()
                .find_map(|p| p.host_port.map(|h| format!(" · :{h}")))
                .unwrap_or_default();
            let ctx = if selected_containers.contains(&c.id) {
                2
            } else {
                0
            };
            let engine_tag = tag(&c.engine);
            let suffix = engine_tag
                .as_ref()
                .map_or_else(String::new, |t| format!(" · {t}"));
            entries.push(Entry {
                subject: Subject::Container(c.id.clone()),
                group: Group::Containers,
                name: c.name.clone(),
                secondary,
                ports,
                open_meta: format!(
                    "Container · {status} · {}{port_text}{suffix}",
                    c.image.reference
                ),
                verb_meta: format!("Container · {status}{suffix}"),
                ctx,
                attention: container_attention(c.state, c.health, c.exit_code),
                caps: container_caps(c.state, engine_up(&c.engine)),
                running: 0,
                home: is_home(&c.engine),
            });
        }

        for p in view.projects {
            let count = view
                .containers
                .iter()
                .filter(|c| {
                    c.engine == p.engine
                        && (c.project.as_ref() == Some(&p.id)
                            || c.compose_project.as_deref() == Some(p.name.as_str()))
                })
                .count();
            let mut secondary = Vec::new();
            if let Some(folder) = &p.folder {
                secondary.push(folder.display().to_string());
            }
            secondary.push(engine_name(&p.engine));
            let ctx = if matches!(selection, Selection::Project(id) if *id == p.id) {
                2
            } else if selected_project_ids.contains(&&p.id) {
                1
            } else {
                0
            };
            let engine_tag = tag(&p.engine);
            let suffix = engine_tag
                .as_ref()
                .map_or_else(String::new, |t| format!(" · {t}"));
            entries.push(Entry {
                subject: Subject::Project(p.id.clone()),
                group: Group::Projects,
                name: p.name.clone(),
                secondary,
                ports: Vec::new(),
                open_meta: format!(
                    "Project · {}{suffix}",
                    plural(count, "container", "containers")
                ),
                verb_meta: format!("{}{suffix}", plural(count, "container", "containers")),
                ctx,
                attention: project_attention(p.status),
                caps: project_caps(p, engine_up(&p.engine)),
                running: 0,
                home: is_home(&p.engine),
            });
        }

        for e in view.engines {
            let running = view
                .containers
                .iter()
                .filter(|c| c.engine == e.id && c.state == ContainerState::Running)
                .count();
            let ctx = if matches!(selection, Selection::Engine(id) if *id == e.id) {
                2
            } else if selected_engines.contains(&&e.id)
                || matches!(scope, EngineScope::One(id) if *id == e.id)
            {
                1
            } else {
                0
            };
            let attention = match e.state {
                EngineState::Running => 0,
                EngineState::Stopped => 1,
                _ => 2,
            };
            entries.push(Entry {
                subject: Subject::Engine(e.id.clone()),
                group: Group::Engine,
                name: e.name.clone(),
                secondary: Vec::new(),
                ports: Vec::new(),
                open_meta: format!("Engine · {}", state_label(&e.state)),
                verb_meta: String::new(),
                ctx,
                attention,
                caps: engine_caps(e),
                running,
                home: is_home(&e.id),
            });
        }

        // Images, volumes and networks are always searchable. Verbs arrive
        // with P4.
        for image in view.images {
            let name = image
                .repo_tags
                .first()
                .cloned()
                .unwrap_or_else(|| image.id.clone());
            let ctx = u8::from(matches!(selection, Selection::Image(id) if *id == image.id)) * 2;
            entries.push(Entry {
                subject: Subject::Image(image.id.clone()),
                group: Group::Images,
                name,
                secondary: image.repo_tags.iter().skip(1).cloned().collect(),
                ports: Vec::new(),
                open_meta: format!(
                    "Image · {}{}",
                    berth_core::fmt::bytes(image.size),
                    resource_suffix(&image.engine)
                ),
                verb_meta: String::new(),
                ctx,
                attention: 0,
                caps: Vec::new(),
                running: 0,
                home: resource_home(&image.engine),
            });
        }
        for volume in view.volumes {
            let ctx = u8::from(matches!(selection, Selection::Volume(n) if *n == volume.name)) * 2;
            entries.push(Entry {
                subject: Subject::Volume(volume.name.clone()),
                group: Group::Volumes,
                name: volume.name.clone(),
                secondary: volume
                    .labels
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect(),
                ports: Vec::new(),
                open_meta: match volume.size {
                    Some(size) => format!(
                        "Volume · {}{}",
                        berth_core::fmt::bytes(size),
                        resource_suffix(&volume.engine)
                    ),
                    None => format!("Volume{}", resource_suffix(&volume.engine)),
                },
                verb_meta: String::new(),
                ctx,
                attention: 0,
                caps: Vec::new(),
                running: 0,
                home: resource_home(&volume.engine),
            });
        }
        for network in view.networks {
            let ctx = u8::from(
                matches!(selection, Selection::Network(n) if *n == network.id || *n == network.name),
            ) * 2;
            entries.push(Entry {
                subject: Subject::Network(network.id.clone()),
                group: Group::Networks,
                name: network.name.clone(),
                secondary: network.subnet.iter().cloned().collect(),
                ports: Vec::new(),
                open_meta: format!(
                    "Network · {}{}",
                    network.driver,
                    resource_suffix(&network.engine)
                ),
                verb_meta: String::new(),
                ctx,
                attention: 0,
                caps: Vec::new(),
                running: 0,
                home: resource_home(&network.engine),
            });
        }
        Self { entries }
    }

    /// Number of indexed records (tests, status text).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entry_of(&self, subject: &Subject) -> Option<&Entry> {
        self.entries.iter().find(|e| e.subject == *subject)
    }
}

// ---------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------

struct Atom {
    atom: nucleo_matcher::pattern::Atom,
    chars: u32,
    /// Digits to match against ports (`:5432` or a bare number).
    port: Option<String>,
    /// `:5432` matches ports only.
    port_only: bool,
}

struct Matching {
    matcher: Matcher,
    buf: Vec<char>,
    indices: Vec<u32>,
    /// How many haystacks were scored; tests count work with it instead of
    /// timing it (AD26).
    #[cfg(test)]
    scored: usize,
}

impl Matching {
    fn new() -> Self {
        let mut config = Config::DEFAULT;
        config.prefer_prefix = true;
        Self {
            matcher: Matcher::new(config),
            buf: Vec::new(),
            indices: Vec::new(),
            #[cfg(test)]
            scored: 0,
        }
    }

    fn fuzzy(&mut self, atom: &Atom, haystack: &str) -> Option<u32> {
        #[cfg(test)]
        {
            self.scored += 1;
        }
        let score = atom
            .atom
            .score(Utf32Str::new(haystack, &mut self.buf), &mut self.matcher)?;
        let score = u32::from(score);
        // Survivors only: measure how far apart the matched letters sit.
        self.indices.clear();
        atom.atom.indices(
            Utf32Str::new(haystack, &mut self.buf),
            &mut self.matcher,
            &mut self.indices,
        )?;
        self.indices.sort_unstable();
        self.indices.dedup();
        let gaps: u32 = self.indices.windows(2).map(|w| w[1] - w[0] - 1).sum();
        let allowed = (atom.chars / 3 * GAP_PER_THREE_CHARS).max(1);
        (gaps <= allowed).then_some(score)
    }

    fn name_score(&mut self, atom: &Atom, entry: &Entry) -> Option<u32> {
        let port = atom.port.as_deref().and_then(|digits| {
            entry.ports.iter().find_map(|p| {
                if p == digits {
                    Some(PORT_EXACT)
                } else if p.starts_with(digits) {
                    Some(PORT_PREFIX)
                } else {
                    None
                }
            })
        });
        if atom.port_only {
            return port;
        }
        let name = self.fuzzy(atom, &entry.name);
        match (name, port) {
            (Some(n), Some(p)) => Some(n.max(p)),
            (n, p) => n.or(p),
        }
    }

    fn any_score(&mut self, atom: &Atom, entry: &Entry) -> Option<u32> {
        if let Some(score) = self.name_score(atom, entry) {
            return Some(score);
        }
        if atom.port_only {
            return None;
        }
        entry
            .secondary
            .iter()
            .filter_map(|text| self.fuzzy(atom, text))
            .max()
            .map(|s| s.saturating_sub(SECONDARY_PENALTY))
    }
}

fn parse_atoms(query: &str) -> Vec<Atom> {
    query
        .split_whitespace()
        .filter_map(|text| {
            let (body, port_only) = match text.strip_prefix(':') {
                Some(rest) => (rest, true),
                None => (text, false),
            };
            if body.is_empty() {
                return None;
            }
            let digits = body.chars().all(|c| c.is_ascii_digit());
            let port = (digits && (port_only || body.len() >= 2)).then(|| body.to_string());
            if port_only && port.is_none() {
                // ":abc" is not a port query; match it as text.
                return Some(make_atom(text, None, false));
            }
            Some(make_atom(body, port, port_only))
        })
        .collect()
}

fn make_atom(text: &str, port: Option<String>, port_only: bool) -> Atom {
    Atom {
        atom: nucleo_matcher::pattern::Atom::new(
            text,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
            false,
        ),
        chars: u32::try_from(text.chars().count()).unwrap_or(u32::MAX),
        port,
        port_only,
    }
}

thread_local! {
    static MATCHING: std::cell::RefCell<Option<Matching>> = const { std::cell::RefCell::new(None) };
}

fn with_matching<R>(f: impl FnOnce(&mut Matching) -> R) -> R {
    MATCHING.with(|cell| {
        let mut slot = cell.borrow_mut();
        let matching = slot.get_or_insert_with(Matching::new);
        f(matching)
    })
}

// ---------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------

fn shortcut_of(id: ActionId) -> Option<&'static str> {
    action_by_id(id).and_then(|def| def.shortcut)
}

impl Entry {
    /// The rank bonus of a result on the selected engine.
    fn home_bonus(&self) -> u32 {
        if self.home {
            HOME_BONUS
        } else {
            0
        }
    }

    fn verb_item(&self, verb: &Verb, score: u32) -> SearchItem {
        let ellipsis = if verb.confirm { "…" } else { "" };
        let danger = action_by_id(verb.id).is_some_and(|d| d.danger);
        let confirm_note = match (self.group, verb.confirm) {
            (Group::Engine, true) if verb.id != "engine.start" => {
                if self.running == 0 {
                    "Asks to confirm".to_string()
                } else {
                    format!(
                        "Asks to confirm · stops {}",
                        plural(self.running, "container", "containers")
                    )
                }
            }
            (_, true) => format!("{} · Asks to confirm", self.verb_meta),
            _ => self.verb_meta.clone(),
        };
        SearchItem {
            group: self.group,
            icon: verb.icon,
            title: format!("{} {}{ellipsis}", verb.label, self.name),
            meta: confirm_note,
            shortcut: if self.ctx == 2 {
                shortcut_of(verb.id)
            } else {
                None
            },
            danger,
            confirm: verb.confirm,
            target: Target::Run {
                action: verb.id,
                subject: Some(self.subject.clone()),
            },
            score,
        }
    }

    fn open_item(&self, score: u32) -> SearchItem {
        let icon = match self.group {
            Group::Containers => "box",
            Group::Projects => "folder",
            Group::Images => "layers",
            Group::Volumes => "hard-drive",
            Group::Networks => "network",
            Group::Engine => "server",
            Group::Recent | Group::Actions | Group::Plugins => "search",
        };
        SearchItem {
            group: self.group,
            icon,
            title: self.name.clone(),
            meta: self.open_meta.clone(),
            shortcut: None,
            danger: false,
            confirm: false,
            target: Target::Open(self.subject.clone()),
            score,
        }
    }
}

fn global_item(id: ActionId, title: &str, icon: &'static str, score: u32) -> SearchItem {
    SearchItem {
        group: Group::Actions,
        icon,
        title: title.to_string(),
        meta: String::new(),
        shortcut: shortcut_of(id),
        danger: false,
        confirm: false,
        target: Target::Run {
            action: id,
            subject: None,
        },
        score,
    }
}

fn order(a: &(SearchItem, u8, u8, &str), b: &(SearchItem, u8, u8, &str)) -> std::cmp::Ordering {
    b.0.score
        .cmp(&a.0.score)
        .then(b.1.cmp(&a.1))
        .then(a.2.cmp(&b.2))
        .then(a.3.cmp(b.3))
}

impl Index {
    /// Run `query`. An empty query lists recent items then the global
    /// actions. `recents` is most recent first.
    #[must_use]
    pub fn search(&self, query: &str, recents: &[RecentKey]) -> Results {
        let atoms = parse_atoms(query);
        if atoms.is_empty() {
            return self.suggestions(recents);
        }
        with_matching(|m| self.search_with(m, &atoms, query))
    }

    fn suggestions(&self, recents: &[RecentKey]) -> Results {
        let mut groups = Vec::new();
        let recent: Vec<SearchItem> = recents
            .iter()
            .filter_map(|key| self.resolve(key))
            .take(RECENT_CAP)
            .collect();
        if !recent.is_empty() {
            groups.push(ResultGroup {
                group: Group::Recent,
                items: recent,
            });
        }
        let actions: Vec<SearchItem> = GLOBALS
            .iter()
            .take(EMPTY_ACTIONS_CAP)
            .map(|(id, title, icon)| global_item(id, title, icon, 0))
            .collect();
        groups.push(ResultGroup {
            group: Group::Actions,
            items: actions,
        });
        Results {
            groups,
            docker_command: None,
        }
    }

    fn search_with(&self, m: &mut Matching, atoms: &[Atom], query: &str) -> Results {
        // (item, ctx, attention, name) so ties break the same way each time.
        let mut found: Vec<(SearchItem, u8, u8, &str)> = Vec::new();

        // Global actions: all atoms must match the title.
        for (id, title, icon) in GLOBALS {
            let mut total = 0u32;
            let all = atoms.iter().all(|atom| {
                let score = m
                    .fuzzy(atom, title)
                    .or_else(|| action_by_id(id).and_then(|d| m.fuzzy(atom, d.label)));
                score.is_some_and(|s| {
                    total += s;
                    true
                })
            });
            if all {
                found.push((
                    global_item(id, title, icon, total + ACTION_BONUS),
                    0,
                    0,
                    title,
                ));
            }
        }

        // Per verb: which atoms match the label, computed once.
        for group in [Group::Containers, Group::Projects, Group::Engine] {
            let verbs = verbs_of(group);
            let verb_scores: Vec<Vec<Option<u32>>> = verbs
                .iter()
                .map(|v| atoms.iter().map(|a| m.fuzzy(a, v.label)).collect())
                .collect();
            let live: Vec<usize> = verb_scores
                .iter()
                .enumerate()
                .filter(|(_, s)| s.iter().any(Option::is_some))
                .map(|(i, _)| i)
                .collect();
            if live.is_empty() {
                continue;
            }
            let entries: Vec<&Entry> = self.entries.iter().filter(|e| e.group == group).collect();
            // Name scores per atom, once per entry (names only).
            let name_scores: Vec<Vec<Option<u32>>> = entries
                .iter()
                .map(|e| atoms.iter().map(|a| m.name_score(a, e)).collect())
                .collect();
            for &vi in &live {
                let verb = &verbs[vi];
                let mut explicit: Vec<(&Entry, u32)> = Vec::new();
                let mut context: Vec<(&Entry, u32)> = Vec::new();
                let mut candidates: Vec<(&Entry, u32)> = Vec::new();
                for (entry, names) in entries.iter().zip(&name_scores) {
                    if !entry.caps.contains(&verb.id) {
                        continue;
                    }
                    let mut total = 0u32;
                    let mut verb_hit = false;
                    let mut name_hit = false;
                    let mut ok = true;
                    for (ai, name) in names.iter().enumerate() {
                        match (verb_scores[vi][ai], *name) {
                            (None, None) => {
                                ok = false;
                                break;
                            }
                            (Some(v), None) => {
                                verb_hit = true;
                                total += v;
                            }
                            (None, Some(n)) => {
                                name_hit = true;
                                total += n;
                            }
                            (Some(v), Some(n)) => {
                                if v >= n {
                                    verb_hit = true;
                                    total += v;
                                } else {
                                    name_hit = true;
                                    total += n;
                                }
                            }
                        }
                    }
                    if !ok || !verb_hit {
                        continue;
                    }
                    if name_hit {
                        explicit.push((entry, total));
                    } else {
                        if entry.ctx > 0 {
                            context.push((entry, total));
                        }
                        candidates.push((entry, total));
                    }
                }
                // A verb alone acts on the context, else on whatever most
                // needs attention.
                let bare: Vec<(&Entry, u32)> = if context.is_empty() {
                    candidates
                        .into_iter()
                        .min_by(|a, b| {
                            b.0.home
                                .cmp(&a.0.home)
                                .then(a.0.attention.cmp(&b.0.attention))
                                .then(a.0.name.cmp(&b.0.name))
                        })
                        .into_iter()
                        .collect()
                } else {
                    context
                };
                for (entry, total) in explicit.into_iter().chain(bare) {
                    found.push((
                        entry.verb_item(verb, total + ACTION_BONUS + entry.home_bonus()),
                        entry.ctx,
                        entry.attention,
                        &entry.name,
                    ));
                }
            }
        }

        // Records: every atom must match the name, a port or a secondary
        // field.
        for entry in &self.entries {
            let mut total = 0u32;
            let mut ok = true;
            for atom in atoms {
                match m.any_score(atom, entry) {
                    Some(score) => total += score,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                found.push((
                    entry.open_item(total + entry.home_bonus()),
                    entry.ctx,
                    entry.attention,
                    &entry.name,
                ));
            }
        }

        found.sort_by(order);
        let mut groups: Vec<ResultGroup> = Vec::new();
        for (item, _, _, _) in found {
            let group = item.group;
            if let Some(existing) = groups.iter_mut().find(|g| g.group == group) {
                if existing.items.len() < GROUP_CAP {
                    existing.items.push(item);
                }
            } else {
                groups.push(ResultGroup {
                    group,
                    items: vec![item],
                });
            }
        }
        // Best group first; equal groups keep the fixed Group order.
        groups.sort_by(|a, b| {
            let best = |g: &ResultGroup| g.items.first().map_or(0, |i| i.score);
            best(b).cmp(&best(a)).then(a.group.cmp(&b.group))
        });
        let docker_command = groups.is_empty().then(|| query.trim().to_string());
        Results {
            groups,
            docker_command,
        }
    }

    /// The item a recent key stands for, if it still exists and can run.
    #[must_use]
    pub fn resolve(&self, key: &RecentKey) -> Option<SearchItem> {
        match (&key.subject, key.action.as_str()) {
            (Some(subject), OPEN) => self.entry_of(subject).map(|e| e.open_item(0)),
            (Some(subject), action) => {
                let entry = self.entry_of(subject)?;
                let verb = verbs_of(entry.group).iter().find(|v| v.id == action)?;
                entry
                    .caps
                    .contains(&verb.id)
                    .then(|| entry.verb_item(verb, 0))
            }
            (None, action) => GLOBALS
                .iter()
                .find(|(id, _, _)| *id == action)
                .map(|(id, title, icon)| global_item(id, title, icon, 0)),
        }
    }
}

// ---------------------------------------------------------------------
// Recents
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentKey {
    /// An `ActionDef` id, or "open".
    pub action: String,
    pub subject: Option<Subject>,
}

const RECENT_KEEP: usize = 20;

/// Move `key` to the front of `list` (most recent first), once.
pub fn remember(list: &mut Vec<RecentKey>, key: RecentKey) {
    list.retain(|k| *k != key);
    list.insert(0, key);
    list.truncate(RECENT_KEEP);
}

/// What the user ran from Berth, newest first, from the activity log
/// (entries are newest first). Engine events and failures are not recents.
#[must_use]
pub fn recents_from_activity(entries: &[berth_core::ActivityEntry]) -> Vec<RecentKey> {
    let mut list = Vec::new();
    let mut seen = Vec::new();
    for entry in entries {
        if entry.origin != berth_core::ActivityOrigin::Berth
            || entry.outcome != berth_core::Outcome::Ok
        {
            continue;
        }
        let key = RecentKey {
            action: entry.action.clone(),
            subject: Some(entry.subject.clone()),
        };
        if !seen.contains(&key) {
            seen.push(key.clone());
            list.push(key);
        }
        if list.len() >= RECENT_KEEP {
            break;
        }
    }
    list
}

static SESSION_RECENTS: std::sync::Mutex<Vec<RecentKey>> = std::sync::Mutex::new(Vec::new());

/// The palette's recents of this session, newest first.
#[must_use]
pub fn session_recents() -> Vec<RecentKey> {
    SESSION_RECENTS
        .lock()
        .map(|list| list.clone())
        .unwrap_or_default()
}

pub fn remember_in_session(key: RecentKey) {
    if let Ok(mut list) = SESSION_RECENTS.lock() {
        remember(&mut list, key);
    }
}

// ---------------------------------------------------------------------
// Running an item
// ---------------------------------------------------------------------

/// What `run` did, for the palette to react to.
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    /// Started or navigated; the palette closes.
    Done,
    /// Opened a confirm dialog; the palette closes.
    Confirm,
    /// Refused with the reason the action's own check gave.
    Refused(String),
}

fn container_action_of(id: &str) -> Option<ContainerAction> {
    match id {
        "container.start" => Some(ContainerAction::Start),
        "container.stop" => Some(ContainerAction::Stop { timeout_s: None }),
        "container.restart" => Some(ContainerAction::Restart { timeout_s: None }),
        "container.pause" => Some(ContainerAction::Pause),
        "container.resume" => Some(ContainerAction::Unpause),
        "container.kill" => Some(ContainerAction::Kill { signal: None }),
        "container.delete" => Some(ContainerAction::Remove {
            force: false,
            volumes: false,
        }),
        _ => None,
    }
}

fn project_op_of(id: &str) -> Option<ProjectOp> {
    match id {
        "project.start" => Some(ProjectOp::Start),
        "project.stop" => Some(ProjectOp::Stop),
        "project.restart" => Some(ProjectOp::Restart),
        "project.rebuild" => Some(ProjectOp::Rebuild),
        "project.pull_images" => Some(ProjectOp::PullImages),
        _ => None,
    }
}

fn colima_op_of(id: &str) -> Option<ColimaOp> {
    match id {
        "engine.start" => Some(ColimaOp::Start),
        "engine.stop" => Some(ColimaOp::Stop),
        "engine.restart" => Some(ColimaOp::Restart),
        _ => None,
    }
}

/// The confirm dialog model for an action that asks first. `None` when the
/// subject is gone or the action never confirms. The summaries come from
/// `services::confirm`, the one place that builds them (D33).
fn confirm_model(state: &AppState, action: ActionId, subject: &Subject) -> Option<ConfirmModel> {
    match (subject, action) {
        (Subject::Engine(id), "engine.stop" | "engine.restart") => {
            let engine = state.engines.peek().iter().find(|e| e.id == *id)?.clone();
            let profile = crate::services::colima::profile_of(&engine)?.to_string();
            let running = state
                .containers
                .peek()
                .iter()
                .filter(|c| c.engine == engine.id && c.state == ContainerState::Running)
                .count();
            Some(confirm::engine_model(
                &engine.id,
                &engine.name,
                profile,
                colima_op_of(action)?,
                running,
            ))
        }
        (Subject::Container(id), "container.kill") => confirm::kill_model(state, id),
        (Subject::Container(id), "container.delete") => {
            confirm::delete_model(state, std::slice::from_ref(id))
        }
        _ => None,
    }
}

fn toast(state: &mut AppState, text: String) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone: ToastTone::Danger,
        action: None,
    });
}

/// Section and selection for opening a record.
pub fn open(state: &mut AppState, subject: &Subject) {
    match subject {
        Subject::Container(id) => {
            state.section.set(Section::Containers);
            state.selection.set(Selection::Containers(vec![id.clone()]));
        }
        Subject::Project(id) => {
            state.section.set(Section::Project);
            state.selection.set(Selection::Project(id.clone()));
        }
        Subject::Image(id) => {
            state.section.set(Section::Images);
            state.selection.set(Selection::Image(id.clone()));
        }
        Subject::Volume(name) => {
            state.section.set(Section::Volumes);
            state.selection.set(Selection::Volume(name.clone()));
        }
        Subject::Network(id) => {
            state.section.set(Section::Networks);
            state.selection.set(Selection::Network(id.clone()));
        }
        Subject::Engine(id) => {
            state.section.set(Section::Engines);
            state.selection.set(Selection::Engine(id.clone()));
        }
        Subject::Plugin(_) => {}
    }
}

/// `results` with a "Plugins" group of the commands whose titles match the
/// typed `query` (P9-T5). Plugin commands answer a typed query only; the
/// "run as docker command" offer is dropped when one matched.
#[must_use]
pub fn with_plugin_commands(
    mut results: Results,
    commands: &[crate::services::plugin_views::PluginCommand],
    query: &str,
) -> Results {
    let items: Vec<SearchItem> = crate::services::plugin_views::match_commands(commands, query)
        .into_iter()
        .map(|command| SearchItem {
            group: Group::Plugins,
            icon: "zap",
            title: command.title.clone(),
            meta: "Plugin".to_string(),
            shortcut: None,
            danger: false,
            confirm: false,
            target: Target::Plugin(command.clone()),
            score: 0,
        })
        .collect();
    if !items.is_empty() {
        results.docker_command = None;
        results.groups.push(ResultGroup {
            group: Group::Plugins,
            items,
        });
    }
    results
}

/// Run the item's target. The caller closes the palette first, so overlays
/// pushed here (confirm, sheets) land on an empty stack.
pub fn run(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    item: &SearchItem,
) -> RunOutcome {
    let (action, subject) = match &item.target {
        Target::Open(subject) => {
            open(state, subject);
            return RunOutcome::Done;
        }
        Target::Run { action, subject } => (*action, subject.clone()),
        Target::Plugin(command) => {
            crate::services::plugin_views::run_command(state, command);
            return RunOutcome::Done;
        }
    };
    let Some(subject) = subject else {
        match action {
            "volume.create" => state.overlays.write().push(Overlay::CreateVolume),
            "network.create" => state.overlays.write().push(Overlay::CreateNetwork),
            "engine.add" => state.overlays.write().push(Overlay::AddEngine),
            other => crate::keys::dispatch(state, other),
        }
        return RunOutcome::Done;
    };
    if item.confirm {
        return match confirm_model(state, action, &subject) {
            Some(model) => {
                state.overlays.write().push(Overlay::Confirm(model));
                RunOutcome::Confirm
            }
            None => RunOutcome::Refused("This is no longer available".to_string()),
        };
    }
    // Actions that work on the selection: select the subject, then run the
    // handler the shortcut would.
    if matches!(
        action,
        "container.logs"
            | "container.terminal"
            | "container.edit"
            | "project.open_folder"
            | "project.open_in_editor"
            | "project.delete"
    ) {
        open(state, &subject);
        crate::keys::dispatch(state, action);
        return RunOutcome::Done;
    }
    // Sheets, not actions: Edit resources and Clean up open over the shell.
    if let Subject::Engine(id) = &subject {
        match action {
            "engine.edit_resources" => {
                crate::sheets::edit_resources::open_sheet(state, id);
                return RunOutcome::Done;
            }
            "engine.clean_up" => {
                state.overlays.write().push(Overlay::CleanUp(id.clone()));
                return RunOutcome::Done;
            }
            _ => {}
        }
    }
    let Some(actions) = actions else {
        return RunOutcome::Refused("Actions are not running".to_string());
    };
    let result = match &subject {
        Subject::Container(id) => {
            let found = state
                .containers
                .peek()
                .iter()
                .find(|c| c.id == *id)
                .cloned();
            match (found, container_action_of(action)) {
                (Some(container), Some(op)) => {
                    actions.container(state, &ContainerTarget::from(&container), op)
                }
                _ => Err("This container no longer exists".to_string()),
            }
        }
        Subject::Project(id) => {
            let found = state.projects.peek().iter().find(|p| p.id == *id).cloned();
            match (found, project_op_of(action)) {
                (Some(project), Some(op)) => actions.project(state, &project, op),
                _ => Err("This project no longer exists".to_string()),
            }
        }
        Subject::Engine(id) => {
            let found = state.engines.peek().iter().find(|e| e.id == *id).cloned();
            match (found, colima_op_of(action)) {
                (Some(engine), Some(op)) => actions.engine(state, &engine, op),
                _ => Err("This engine no longer exists".to_string()),
            }
        }
        _ => Err("Nothing to run here yet".to_string()),
    };
    match result {
        Ok(()) => RunOutcome::Done,
        Err(reason) => {
            toast(state, reason.clone());
            RunOutcome::Refused(reason)
        }
    }
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
