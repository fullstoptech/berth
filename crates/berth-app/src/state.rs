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
    /// Opens from Templates or Find images without a project, or from a project (AD33).
    AddService(AddServiceSeed),
    /// Two checkboxes: delete the folder's containers, remove the project's volumes (AD33).
    DeleteProject(berth_core::ProjectId),
}
/// What the Add service sheet starts from: the project it adds to (none when opened from Templates or Find
/// images), a built-in or custom template id, or an image reference (AD33).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AddServiceSeed {
    pub project: Option<berth_core::ProjectId>,
    pub template: Option<String>,
    pub image: Option<String>,
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
            height_px: 320,
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
    /// The subjects the dialog acts on, fixed when it opens
    /// (`effect.count == targets.len()`). `confirm::run` never re-reads the
    /// selection; a target gone by confirm time becomes a refused line.
    pub targets: Vec<berth_core::Subject>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastTone {
    Neutral,
    Success,
    Danger,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionScope {
    Global,
    Container,
    Project,
    Image,
    Volume,
    Network,
    Engine,
    Logs,
    Assistant,
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
    /// What backs the engines (mock data or real Docker); set once at boot.
    pub backing: Signal<crate::services::backing::Backing>,
    // P2-T2: dev-only primitives gallery ("Components / primitives" scene).
    // Not a PLAN 6.11 field — architect-accepted at integration (option a).
    pub dev_gallery: Signal<bool>,
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
        backing: use_signal(crate::services::backing::Backing::default),
        dev_gallery: use_signal(|| false),
    }
}

/// The JS the theme effect runs (D7): sets or clears data-theme, data-density
/// and the `--text-scale` property on <html>.
pub fn theme_script(
    theme: berth_store::Theme,
    density: berth_store::Density,
    text_scale: f32,
) -> String {
    let theme_js = match theme {
        berth_store::Theme::Light => "document.documentElement.dataset.theme = 'light';",
        berth_store::Theme::Dark => "document.documentElement.dataset.theme = 'dark';",
        berth_store::Theme::System => "delete document.documentElement.dataset.theme;",
    };
    let density_js = match density {
        berth_store::Density::Compact => "document.documentElement.dataset.density = 'compact';",
        berth_store::Density::Comfortable => "delete document.documentElement.dataset.density;",
    };
    format!(
        "{theme_js} {density_js} {}",
        crate::text_scale::script(text_scale)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_script_sets_and_clears_attributes_on_document_element() {
        let dark = theme_script(berth_store::Theme::Dark, berth_store::Density::Compact, 1.3);
        assert!(
            dark.contains("documentElement.dataset.theme = 'dark'"),
            "{dark}"
        );
        assert!(dark.contains("dataset.density = 'compact'"));
        assert!(dark.contains("documentElement.style.setProperty('--text-scale', '1.3')"));
        let system = theme_script(
            berth_store::Theme::System,
            berth_store::Density::Comfortable,
            1.0,
        );
        assert!(system.contains("delete document.documentElement.dataset.theme"));
        assert!(system.contains("delete document.documentElement.dataset.density"));
        assert!(system.contains("removeProperty('--text-scale')"));
    }

    #[test]
    fn add_service_overlays_with_different_seeds_are_different_overlays() {
        let none = Overlay::AddService(AddServiceSeed::default());
        let from_template = Overlay::AddService(AddServiceSeed {
            template: Some("postgres".to_string()),
            ..AddServiceSeed::default()
        });
        let in_project = Overlay::AddService(AddServiceSeed {
            project: Some(berth_core::ProjectId("p1".to_string())),
            template: Some("postgres".to_string()),
            image: None,
        });
        assert_ne!(none, from_template);
        assert_ne!(from_template, in_project);
        assert_eq!(
            Overlay::DeleteProject(berth_core::ProjectId("p1".to_string())),
            Overlay::DeleteProject(berth_core::ProjectId("p1".to_string()))
        );
        assert_ne!(
            Overlay::DeleteProject(berth_core::ProjectId("p1".to_string())),
            Overlay::DeleteProject(berth_core::ProjectId("p2".to_string()))
        );
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
