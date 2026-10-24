use dioxus::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Projects,
    Project,
    Containers,
    Logs,
    Images,
    Volumes,
    Networks,
    Templates,
    Engines,
    Registries,
    Activity,
    Plugin,
    Settings,
    Plugins,
    FirstRun,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BottomTab {
    Logs,
    Terminal,
    Problems,
    Assistant,
}
// ConfirmModel is large by definition (PLAN 6.11); boxing would change the interface.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Overlay {
    Palette,
    Confirm(ConfirmModel),
    ReviewChanges(uuid::Uuid),
    Generate(std::path::PathBuf),
    Detected(std::path::PathBuf),
    OpenFolder,
    DropFolder(Vec<std::path::PathBuf>),
    NewProject,
    EditRecreate(berth_core::ContainerId),
    Run(String),
    CleanUp(berth_core::EngineId),
    CreateVolume,
    CreateNetwork,
    EditResources(String),
    AddEngine,
    AddRegistry,
    BuildImage,
    AddService(berth_core::ProjectId),
}
#[derive(Debug, Clone, PartialEq)]
pub enum EngineScope {
    One(berth_core::EngineId),
    All,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PendingKey {
    pub subject: berth_core::Subject,
    pub action: &'static str,
}
// BTreeMap needs Ord; Subject has no Ord in berth-core, so PendingKey orders
// by its debug-shaped subject string plus the action (decision in STATUS).
impl PendingKey {
    pub fn sort_key(&self) -> (String, &'static str) {
        (format!("{:?}", self.subject), self.action)
    }
}
impl PartialOrd for PendingKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Eq for PendingKey {}
impl Ord for PendingKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum PendingState {
    Running { label: String },
    Failed { message: String },
}
#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    None,
    Containers(Vec<berth_core::ContainerId>),
    Project(berth_core::ProjectId),
    Image(String),
    Volume(String),
    Network(String),
    Engine(berth_core::EngineId),
}
#[derive(Debug, Clone, PartialEq)]
pub struct BottomPanel {
    pub open: bool,
    pub height_px: u32,
    pub tab: BottomTab,
    pub terminals: Vec<TerminalTab>,
}
impl Default for BottomPanel {
    fn default() -> Self {
        Self {
            open: false,
            height_px: 260,
            tab: BottomTab::Logs,
            terminals: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalTab {
    pub id: uuid::Uuid,
    pub title: String,
    pub session: Option<berth_core::ExecSession>,
    pub exited: bool,
    pub unread: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmModel {
    pub title: String,
    pub effect: berth_core::EffectSummary,
    pub action: berth_core::EngineAction,
    pub checkbox: Option<(String, bool)>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastTone {
    Neutral,
    Success,
    Danger,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: uuid::Uuid,
    pub text: String,
    pub tone: ToastTone,
    pub action: Option<(String, &'static str)>,
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub settings: Signal<berth_store::Settings>,
    pub engines: Signal<Vec<berth_core::Engine>>,
    pub engine_scope: Signal<EngineScope>,
    pub containers: Signal<Vec<berth_core::Container>>,
    pub projects: Signal<Vec<berth_core::Project>>,
    pub images: Signal<Vec<berth_core::Image>>,
    pub volumes: Signal<Vec<berth_core::Volume>>,
    pub networks: Signal<Vec<berth_core::Network>>,
    pub problems: Signal<Vec<berth_core::Problem>>,
    pub pending: Signal<std::collections::BTreeMap<PendingKey, PendingState>>,
    pub section: Signal<Section>,
    pub selection: Signal<Selection>,
    pub overlays: Signal<Vec<Overlay>>,
    pub bottom: Signal<BottomPanel>,
    pub harnesses: Signal<Vec<berth_core::Harness>>,
    pub toasts: Signal<Vec<Toast>>,
    pub snapshot_stale: Signal<bool>,
}

/// Create the AppState signals once. Call from the root component only.
pub fn use_app_state() -> AppState {
    let settings = use_signal(berth_store::Settings::default);
    AppState {
        settings,
        engines: use_signal(Vec::new),
        engine_scope: use_signal(|| EngineScope::All),
        containers: use_signal(Vec::new),
        projects: use_signal(Vec::new),
        images: use_signal(Vec::new),
        volumes: use_signal(Vec::new),
        networks: use_signal(Vec::new),
        problems: use_signal(Vec::new),
        pending: use_signal(std::collections::BTreeMap::new),
        section: use_signal(|| Section::Projects),
        selection: use_signal(|| Selection::None),
        overlays: use_signal(Vec::new),
        bottom: use_signal(BottomPanel::default),
        harnesses: use_signal(Vec::new),
        toasts: use_signal(Vec::new),
        snapshot_stale: use_signal(|| false),
    }
}

/// The JS the theme effect runs for a theme (D7): sets or clears data-theme on <html>.
pub fn theme_script(theme: berth_store::Theme, density: berth_store::Density) -> String {
    let theme_js = match theme {
        berth_store::Theme::Light => "document.documentElement.dataset.theme = 'light';",
        berth_store::Theme::Dark => "document.documentElement.dataset.theme = 'dark';",
        berth_store::Theme::System => "delete document.documentElement.dataset.theme;",
    };
    let density_js = match density {
        berth_store::Density::Compact => "document.documentElement.dataset.density = 'compact';",
        berth_store::Density::Comfortable => "delete document.documentElement.dataset.density;",
    };
    format!("{theme_js} {density_js}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_script_sets_and_clears_attributes_on_document_element() {
        let dark = theme_script(berth_store::Theme::Dark, berth_store::Density::Compact);
        assert!(
            dark.contains("documentElement.dataset.theme = 'dark'"),
            "{dark}"
        );
        assert!(dark.contains("dataset.density = 'compact'"));
        let system = theme_script(
            berth_store::Theme::System,
            berth_store::Density::Comfortable,
        );
        assert!(system.contains("delete document.documentElement.dataset.theme"));
        assert!(system.contains("delete document.documentElement.dataset.density"));
    }

    #[test]
    fn pending_keys_order_without_panicking() {
        let a = PendingKey {
            subject: berth_core::Subject::Volume("b".to_string()),
            action: "remove",
        };
        let b = PendingKey {
            subject: berth_core::Subject::Volume("a".to_string()),
            action: "remove",
        };
        assert!(b < a);
    }
}
