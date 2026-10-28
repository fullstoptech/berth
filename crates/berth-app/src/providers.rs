//! The per-screen view and store contexts the App root and a scene root
//! provide. Each screen has one function here; a new screen adds its
//! function and one call in [`provide_screen_views`].

use crate::services::detail::DetailStore;
use crate::services::images::ImageStore;
use crate::state::AppState;
use crate::ui::dock::DockView;

/// The handles a caller keeps after [`provide_screen_views`]: the dock (a
/// scene sets its window width) and the stores a scene seeds. `plans` is
/// the one PlanStore the screens and the tool executor share (P7-T5).
#[derive(Clone)]
pub struct ScreenViews {
    pub dock: DockView,
    pub details: DetailStore,
    pub images: ImageStore,
    pub plans: crate::services::plans::PlanStore,
}

/// Provide every screen's context, in the order the screens need it (the
/// dock reads the Containers view). Call once from a component body.
pub fn provide_screen_views(state: AppState) -> ScreenViews {
    containers();
    volumes();
    let dock = dock(state);
    let details = details();
    let images = images();
    find_images();
    first_run();
    updates();
    engine_dashboard();
    folder_view();
    git_deploy();
    plugins();
    let plans = plans();
    ScreenViews {
        dock,
        details,
        images,
        plans,
    }
}

fn containers() {
    crate::screens::containers::provide_containers_view();
}

fn volumes() {
    crate::screens::volumes::provide_volumes_view();
}

fn dock(state: AppState) -> DockView {
    crate::ui::dock::use_dock_view(state)
}

fn details() -> DetailStore {
    crate::services::detail::provide_detail_store()
}

fn images() -> ImageStore {
    crate::services::images::provide_image_store()
}

fn find_images() {
    crate::services::find_images::provide_find_store();
}

fn first_run() {
    crate::services::first_run::provide_first_run_store();
}

fn updates() {
    crate::services::updates::provide_updates_view();
}

fn engine_dashboard() {
    crate::services::engine_dashboard::provide_dashboard_store();
}

fn folder_view() {
    crate::sheets::open_folder::provide_folder_view();
}

fn git_deploy() {
    crate::services::git_deploy::provide_git_deploy_view();
}

fn plugins() {
    crate::services::plugins::provide_plugins_view();
}

fn plans() -> crate::services::plans::PlanStore {
    crate::services::plans::provide_plan_store()
}
