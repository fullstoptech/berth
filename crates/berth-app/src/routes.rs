//! The routing tables of the shell: which component draws a section, an
//! inspector selection and an overlay. A new screen adds one arm to the
//! table it belongs in and touches nothing else in `app.rs`.

use dioxus::prelude::*;

use crate::state::{AppState, Overlay, Section, Selection};

/// The screen the main area draws for `section`. `dev_gallery` is the
/// components gallery (`Components / primitives`), which takes the place of
/// the Projects screen.
pub fn screen(section: Section, dev_gallery: bool) -> Element {
    match section {
        Section::Activity => rsx! { crate::screens::activity::ActivityScreen {} },
        Section::Project => rsx! { ProjectRoute {} },
        Section::Projects if !dev_gallery => rsx! { crate::screens::projects::ProjectsScreen {} },
        Section::Containers => rsx! { crate::screens::containers::ContainersScreen {} },
        Section::Logs => rsx! { crate::screens::logs::LogsScreen {} },
        Section::Networks => rsx! { crate::screens::networks::NetworksScreen {} },
        Section::Templates => rsx! { crate::screens::templates::TemplatesScreen {} },
        Section::Volumes => rsx! { crate::screens::volumes::VolumesScreen {} },
        Section::Images => rsx! { crate::screens::images::ImagesScreen {} },
        Section::Engines => rsx! { crate::screens::engines::EnginesScreen {} },
        Section::Plugin => rsx! { crate::screens::plugin_view::PluginViewScreen {} },
        Section::Plugins => rsx! { crate::screens::plugins::PluginsScreen {} },
        Section::Registries => rsx! { crate::screens::registries::RegistriesScreen {} },
        Section::FirstRun => rsx! { crate::screens::first_run::FirstRunScreen {} },
        Section::Settings => rsx! { crate::screens::settings::SettingsScreen {} },
        _ => rsx! {},
    }
}

/// The selected project's own screen, keyed by the project so a new
/// selection remounts it (its tabs, tree and checks start fresh).
#[component]
fn ProjectRoute() -> Element {
    let state = use_context::<AppState>();
    let selection = state.selection.read();
    match &*selection {
        Selection::Project(id) => rsx! {
            crate::screens::project_detail::ProjectDetailScreen { key: "{id.0}", id: id.clone() }
        },
        _ => rsx! {},
    }
}

/// How the inspector body is drawn: the width of the pane (how many tabs
/// fit before the overflow menu) and whether the bottom panel beside it
/// already shows the stream tabs (AD22).
#[derive(Debug, Clone, Copy)]
pub struct InspectorPane {
    pub pane_px: u32,
    pub hide_stream_tabs: bool,
}

/// The inspector body for the current selection. Each panel is keyed by
/// what it shows, so a new selection remounts it.
pub fn inspector_body(state: &AppState, pane: InspectorPane) -> Element {
    let selection = state.selection.read();
    match &*selection {
        Selection::Containers(ids) => rsx! {
            // Keyed per container: a new selection remounts the panel.
            for id in ids.iter().take(1) {
                crate::screens::container_detail::ContainerDetailPanel {
                    key: "{id.0}",
                    id: id.clone(),
                    pane_px: pane.pane_px,
                    hide_stream_tabs: pane.hide_stream_tabs,
                }
            }
        },
        Selection::Network(id) => rsx! {
            crate::screens::network_detail::NetworkDetailPanel { key: "{id}", id: id.clone() }
        },
        Selection::Volume(name) if *state.section.read() == Section::Volumes => rsx! {
            crate::screens::volume_browser::VolumeDetailPanel { key: "{name}", name: name.clone() }
        },
        Selection::Image(id) => rsx! {
            crate::screens::image_detail::ImageDetailPanel { key: "{id}", id: id.clone() }
        },
        _ => rsx! {},
    }
}

/// What an overlay is called until its own screen exists (P2-T8, P2-T9,
/// P4 and P5 each replace the placeholder body for their variants).
fn overlay_title(overlay: &Overlay) -> &'static str {
    match overlay {
        Overlay::Palette => "Command palette",
        Overlay::Confirm(_) => "Confirm",
        Overlay::ReviewChanges(_) => "Review changes",
        Overlay::Generate(_) => "Generate",
        Overlay::Detected(_) => "Detected",
        Overlay::OpenFolder => "Open folder",
        Overlay::DropFolder(_) => "Add folders",
        Overlay::NewProject => "New project",
        Overlay::EditRecreate(_) => "Edit and recreate",
        Overlay::Run(_) => "Run image",
        Overlay::CleanUp(_) => "Clean up",
        Overlay::CreateVolume => "Create volume",
        Overlay::CreateNetwork => "Create network",
        Overlay::EditResources(_) => "Edit resources",
        Overlay::AddEngine => "Add engine",
        Overlay::AddRegistry(_) => "Add registry",
        Overlay::BuildImage => "Build image",
        Overlay::AddService(_) => "Add service",
        Overlay::DeleteProject(_) => "Delete project",
        Overlay::PluginEnable(_) => "Enable plugin",
        Overlay::PluginConfigure(_) => "Configure plugin",
    }
}

/// The body of one overlay, drawn inside the scrim. Variants whose screen
/// is not built yet fall through to a placeholder card.
pub fn overlay(overlay: &Overlay) -> Element {
    match overlay {
        Overlay::Palette => rsx! { crate::screens::palette::PaletteScreen {} },
        Overlay::Confirm(model) => rsx! {
            crate::ui::ConfirmOverlay { key: "{model.title}", model: model.clone() }
        },
        Overlay::AddService(seed) => rsx! {
            crate::sheets::add_service::AddServiceSheet {
                key: "{seed.project:?}{seed.template:?}{seed.image:?}",
                seed: seed.clone(),
            }
        },
        Overlay::BuildImage => rsx! { crate::sheets::build_image::BuildImageSheet {} },
        Overlay::CleanUp(engine) => rsx! {
            crate::sheets::clean_up::CleanUpSheet { key: "{engine.0}", engine: engine.clone() }
        },
        Overlay::CreateVolume => rsx! { crate::sheets::create_volume::CreateVolumeSheet {} },
        Overlay::Generate(folder) => rsx! {
            crate::sheets::generate::GenerateSheet { key: "{folder.display()}", folder: folder.clone() }
        },
        Overlay::ReviewChanges(id) => rsx! {
            crate::sheets::review_changes::ReviewChangesSheet { key: "{id}", id: *id }
        },
        Overlay::Run(id) => rsx! { crate::sheets::run::RunSheet { key: "{id}", id: id.clone() } },
        Overlay::EditResources(engine) => rsx! {
            crate::sheets::edit_resources::EditResourcesSheet {
                key: "{engine}",
                engine: berth_core::EngineId(engine.clone()),
            }
        },
        Overlay::EditRecreate(id) => rsx! {
            crate::sheets::edit_recreate::EditRecreateSheet { key: "{id.0}", id: id.clone() }
        },
        Overlay::CreateNetwork => rsx! { crate::sheets::create_network::CreateNetworkSheet {} },
        Overlay::DeleteProject(id) => rsx! {
            crate::sheets::delete_project::DeleteProjectSheet { key: "{id.0}", id: id.clone() }
        },
        Overlay::Detected(folder) => rsx! {
            crate::sheets::detected::DetectedSheet { key: "{folder.display()}", folder: folder.clone() }
        },
        Overlay::NewProject => rsx! { crate::sheets::new_project::NewProjectSheet {} },
        Overlay::PluginConfigure(id) => rsx! {
            crate::sheets::plugin_configure::PluginConfigureSheet { key: "{id}", id: id.clone() }
        },
        Overlay::PluginEnable(id) => rsx! {
            crate::sheets::plugin_enable::PluginEnableSheet { key: "{id}", id: id.clone() }
        },
        Overlay::OpenFolder => rsx! { crate::sheets::open_folder::OpenFolderSheet {} },
        Overlay::AddEngine => rsx! { crate::sheets::add_engine::AddEngineSheet {} },
        Overlay::AddRegistry(existing) => rsx! {
            crate::sheets::registry_login::RegistryLoginSheet {
                key: "{existing:?}",
                existing: existing.clone(),
            }
        },
        other => placeholder(other),
    }
}

fn placeholder(overlay: &Overlay) -> Element {
    let title = overlay_title(overlay);
    rsx! {
        div {
            class: "overlay-card",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": title,
            onclick: move |e| e.stop_propagation(),
            h2 { "{title}" }
            p { "This screen is not built yet. Press Esc to close." }
        }
    }
}
