//! Scenes of the terminal screens.

use super::*;

/// The Containers screen with the bottom panel open on its Terminal tab
/// (frames "Container / terminal / 1440" and "... / no shell / 1440"):
/// dark, the inspector hidden, every group collapsed but `open_group`.
fn terminal_base(state: &mut AppState, selected: &str, open_group: &str) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Dark;
    state.section.set(Section::Containers);
    let containers = state.containers.peek().clone();
    state.problems.set(mock_problems(&containers));
    if let Some(container) = containers.iter().find(|c| c.name == selected) {
        state
            .selection
            .set(Selection::Containers(vec![container.id.clone()]));
    }
    if let Some(view) = try_consume_context::<crate::screens::containers::ContainersView>() {
        let mut toggled = view.inspector_toggled;
        toggled.set(true);
        let mut collapsed = view.collapsed;
        let mut map = collapsed.write();
        for project in state.projects.peek().iter() {
            map.insert(
                format!("project:{}", project.id.0),
                project.name != open_group,
            );
        }
        map.insert("unassigned".to_string(), open_group != "unassigned");
    }
    let mut bottom = state.bottom.write();
    bottom.open = true;
    bottom.height_px = 408;
    bottom.tab = crate::state::BottomTab::Terminal;
}

/// Seed one terminal tab with its output and no engine behind it.
fn seed_terminal(
    state: &mut AppState,
    container: &str,
    status: crate::services::exec::Status,
    shell: Option<&str>,
    output: &str,
    unread: bool,
) -> Option<uuid::Uuid> {
    use crate::services::exec::{container_spec, ExecCore, Kind};
    let core = try_consume_context::<ExecCore>()?;
    let found = state
        .containers
        .peek()
        .iter()
        .find(|c| c.name == container)
        .cloned()?;
    let project = crate::services::exec::project_name(state, &found);
    let mut spec = container_spec(&found, project.as_deref(), shell.map(str::to_string));
    if let Kind::Exec(exec) = &mut spec.kind {
        exec.wait_running = false;
        exec.workdir = Some("/app".to_string());
    }
    let id = uuid::Uuid::now_v7();
    core.seed(id, spec, status.clone(), shell, output.as_bytes());
    let title = core.view(id).map(|v| v.title)?;
    state
        .bottom
        .write()
        .terminals
        .push(crate::state::TerminalTab {
            id,
            title,
            session: None,
            exited: status.is_ended(),
            unread,
        });
    Some(id)
}

/// Show `tab` in the terminal panel.
fn show_terminal(tab: Option<uuid::Uuid>) {
    if let (Some(tab), Some(view)) = (
        tab,
        try_consume_context::<crate::services::exec::TerminalView>(),
    ) {
        let mut active = view.active;
        active.set(Some(tab));
    }
}

/// Dark; the api's bash session on top, redis (sh) with new output and the
/// worker's ended session behind it.
pub(super) fn terminal_active(state: &mut AppState) {
    use crate::services::exec::Status;
    terminal_base(state, "shop-api-api-1", "shop-api");
    let prompt = "root@a3f9c2e1b7d4:/app#";
    let transcript = format!(
        "{prompt} \x1b[1mls\x1b[0m\r\n\
         \x1b[37mDockerfile  dist  node_modules  package.json  pnpm-lock.yaml  prisma  src  tsconfig.json\x1b[0m\r\n\
         {prompt} \x1b[1mpnpm prisma migrate status\x1b[0m\r\n\
         \x1b[37mPrisma schema loaded from prisma/schema.prisma\r\n\
         Datasource \"db\": PostgreSQL database \"shop\", schema \"public\" at \"db:5432\"\x1b[0m\r\n\r\n\
         \x1b[91mError: P1001: Can't reach database server at `db:5432`\x1b[0m\r\n\
         \x1b[37mPlease make sure your database server is running at `db:5432`.\x1b[0m\r\n\
         {prompt} "
    );
    let first = seed_terminal(
        state,
        "shop-api-api-1",
        Status::Live,
        Some("bash"),
        &transcript,
        false,
    );
    seed_terminal(
        state,
        "shop-api-redis-1",
        Status::Live,
        Some("sh"),
        "/data # ",
        true,
    );
    seed_terminal(
        state,
        "shop-api-worker-1",
        Status::Exited,
        Some("sh"),
        "",
        false,
    );
    show_terminal(first);
}

/// Dark; the distroless metrics exporter has neither bash nor sh.
fn terminal_no_shell(state: &mut AppState) {
    use crate::services::exec::Status;
    terminal_base(state, "metrics-exporter-1", "unassigned");
    let first = seed_terminal(
        state,
        "metrics-exporter-1",
        Status::NoShell {
            tried: vec!["bash".to_string(), "sh".to_string()],
        },
        None,
        "",
        false,
    );
    show_terminal(first);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Container / terminal / 1440", terminal_active),
    Scene::design("Container / terminal / no shell / 1440", terminal_no_shell),
];
