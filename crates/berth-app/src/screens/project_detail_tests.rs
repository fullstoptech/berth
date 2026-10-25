//! P5-T6 tests for the Project detail screen: the header model, the tabs
//! and their states, the order of the rows (AD31), `.env` values staying
//! hidden, and the Files tab reading a real folder.

use berth_core::{Container, Project, ProjectStatus};
use std::collections::BTreeMap;

use super::tree::{initial_selection, tree_rows, Load, TreeData, TreeRow};
use super::*;
use crate::scenes;
use crate::services::project_files::{DirListing, MemoryFs};

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn project_named(world: &berth_engine::MockWorld, name: &str) -> Project {
    world
        .projects
        .iter()
        .find(|p| p.name == name)
        .cloned()
        .expect("mock project")
}

/// An error-severity problem on the named container.
fn error_problem(world: &berth_engine::MockWorld, container: &str) -> berth_core::Problem {
    use berth_core::{Problem, ProblemId, ProblemKind, Severity, Subject};
    let id = world
        .containers
        .iter()
        .find(|c| c.name == container)
        .map(|c| c.id.clone())
        .expect("mock container");
    let kind = ProblemKind::PortAllocated { port: 5432 };
    let subject = Subject::Container(id);
    Problem {
        id: ProblemId::new(&kind, &subject),
        kind,
        severity: Severity::Error,
        subject,
        title: "Port 5432 is already in use".to_string(),
        explanation: String::new(),
        evidence: Vec::new(),
        fixes: Vec::new(),
        first_seen: chrono::DateTime::from_timestamp(1_792_000_000, 0).unwrap_or_default(),
    }
}

fn model_of(world: &berth_engine::MockWorld, name: &str) -> DetailModel {
    let project = project_named(world, name);
    let pending = BTreeMap::new();
    let view = StateView {
        engines: &world.engines,
        containers: &world.containers,
        projects: &world.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending: &pending,
        backing: crate::services::backing::Backing::default(),
    };
    detail_model(&project, &view, &[], std::path::Path::new("/Users/berth"))
}

fn assert_in_order(html: &str, needles: &[&str]) {
    let mut last = 0;
    for needle in needles {
        let position = html[last..]
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} is missing or out of order"));
        last += position + needle.len();
    }
}

// ---------------------------------------------------------------- header

#[test]
fn project_detail_status_labels_name_each_state() {
    assert_eq!(status_label(ProjectStatus::AllRunning), "Running");
    assert_eq!(
        status_label(ProjectStatus::Partial {
            running: 3,
            total: 4
        }),
        "3 of 4 running"
    );
    assert_eq!(status_label(ProjectStatus::Stopped), "Stopped");
    assert_eq!(status_label(ProjectStatus::Failing), "Failing");
    assert_eq!(status_label(ProjectStatus::NotCreated), "Not created");
}

#[test]
fn project_detail_tabs_round_trip_and_unknown_ids_open_services() {
    for tab in Tab::ALL {
        assert_eq!(Tab::from_id(tab.id()), tab);
    }
    assert_eq!(
        Tab::ALL.map(Tab::label),
        ["Services", "Files", "Logs", "Activity"]
    );
    assert_eq!(Tab::from_id("nonesuch"), Tab::Services);
}

#[test]
fn project_detail_model_of_a_failing_project_offers_stop_and_counts_problems() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let pending = BTreeMap::new();
    let view = StateView {
        engines: &world.engines,
        containers: &world.containers,
        projects: &world.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending: &pending,
        backing: crate::services::backing::Backing::default(),
    };
    let problems = vec![error_problem(&world, "shop-api-db-1")];
    let model = detail_model(
        &project,
        &view,
        &problems,
        std::path::Path::new("/Users/berth"),
    );
    assert_eq!(model.row.primary_label, "Stop project");
    assert_eq!(model.status_label, "Failing");
    assert_eq!(model.branch.as_deref(), Some("feat/checkout-v2 *"));
    assert_eq!(
        model.subline,
        "default engine \u{b7} 5 services \u{b7} 1 problem"
    );
    assert_eq!(model.row.restart_reason, None);
    assert_eq!(model.row.delete_reason, None);
}

#[test]
fn project_detail_model_of_a_stopped_project_offers_start_and_no_restart() {
    let world = world();
    let model = model_of(&world, "ml-notebooks");
    assert_eq!(model.row.primary_label, "Start project");
    assert_eq!(model.status_label, "Stopped");
    assert_eq!(
        model.row.restart_reason.as_deref(),
        Some("Nothing is running")
    );
}

#[test]
fn project_detail_model_of_a_project_not_created_cannot_restart_or_be_deleted() {
    let world = world();
    let model = model_of(&world, "docker-manger");
    assert_eq!(model.status_label, "Not created");
    assert!(model.row.restart_reason.is_some());
    assert!(model.row.delete_reason.is_some());
}

#[test]
fn project_detail_menu_lists_pull_folder_editor_reveal_and_a_last_danger_delete() {
    let world = world();
    let model = model_of(&world, "shop-api");
    let editor = Editor {
        program: Some(std::path::PathBuf::from("/usr/local/bin/code")),
        label: "Visual Studio Code (detected)".to_string(),
    };
    let entries = menu_entries(&model, &editor);
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        ["pull", "open-folder", "open-editor", "reveal", "delete"]
    );
    assert_eq!(entries[2].label, "Open in Visual Studio Code");
    let delete = entries.last().expect("delete");
    assert!(delete.danger);
    assert!(delete.separator_before);
    assert!(
        entries.iter().all(|e| e.disabled_reason.is_none()),
        "{entries:?}"
    );
}

#[test]
fn project_detail_menu_disables_what_a_project_cannot_do_and_says_why() {
    let world = world();
    let mut model = model_of(&world, "docker-manger");
    let editor = Editor {
        program: None,
        label: "the default app".to_string(),
    };
    let entries = menu_entries(&model, &editor);
    let delete = entries.iter().find(|e| e.id == "delete").expect("delete");
    assert_eq!(
        delete.disabled_reason.as_deref(),
        Some("No Compose file is known for this project")
    );
    assert_eq!(
        entries
            .iter()
            .find(|e| e.id == "open-editor")
            .map(|e| e.label.as_str()),
        Some("Open in the default app")
    );
    model.folder = None;
    let entries = menu_entries(&model, &editor);
    for id in ["open-folder", "open-editor", "reveal"] {
        let entry = entries.iter().find(|e| e.id == id).expect(id);
        assert_eq!(
            entry.disabled_reason.as_deref(),
            Some("This project has no folder on this Mac"),
            "{id}"
        );
    }
}

#[test]
fn project_detail_screen_state_is_loading_gone_or_ready() {
    let world = world();
    let project = project_named(&world, "shop-api");
    assert_eq!(
        screen_state(Some(&project), &world.engines, false),
        ScreenState::Ready
    );
    assert_eq!(screen_state(None, &[], false), ScreenState::Loading);
    assert_eq!(
        screen_state(None, &world.engines, true),
        ScreenState::Loading
    );
    assert_eq!(screen_state(None, &world.engines, false), ScreenState::Gone);
}

#[test]
fn project_detail_open_actions_are_registered() {
    for id in [
        "project.open_folder",
        "project.open_in_editor",
        "project.delete",
    ] {
        assert!(crate::actions::handler(id).is_some(), "{id} has no handler");
    }
}

// ----------------------------------------------------------- files scene

#[test]
fn project_detail_files_scene_matches_the_frame_in_text() {
    let html = scenes::render_scene_html("Project / files / 1440").expect("scene");
    assert_in_order(
        &html,
        &[
            "<h1 class=\"pd-title\">shop-api</h1>",
            "Failing",
            "Stop project",
            "Restart project",
            "Rebuild",
            "~/Dev/clients/acme/shop-api",
            "feat/checkout-v2 *",
            "default engine \u{b7} 5 services \u{b7} 2 problems",
        ],
    );
    assert!(
        html.contains("aria-selected=\"true\"><span>Files</span>"),
        "Files tab is open"
    );
    assert_in_order(
        &html,
        &[
            "<span>Services</span>",
            "<span>Files</span>",
            "<span>Logs</span>",
            "<span>Activity</span>",
        ],
    );
    assert!(html.contains("Opens in Visual Studio Code (detected)"));
    for label in [
        "Copy path",
        "Open terminal here",
        "Reveal in Finder",
        "Open in editor",
    ] {
        assert!(html.contains(&format!(">{label}<")), "{label}");
    }
    // The tree, in the order the frame shows it, with `.env` open.
    assert_in_order(
        &html,
        &[
            "pd-row-name\">compose.yaml<",
            "pd-row-name\">.env<",
            "6 keys",
            "DATABASE_URL",
            "REDIS_URL",
            "SMTP_HOST",
            "SMTP_PORT",
            "JWT_SECRET",
            "STRIPE_KEY",
            "pd-row-name\">.env.example<",
            "pd-row-name\">Dockerfile<",
            "pd-row-name\">src/<",
            "pd-row-name\">worker/<",
            "pd-row-name\">package.json<",
        ],
    );
    // The selected file is compose.yaml, shown with line numbers.
    assert!(html.contains(
        "pd-row pd-row-selected\" type=\"button\" role=\"treeitem\" aria-selected=\"true\""
    ));
    assert!(html.contains(
        "<span class=\"pd-card-name\">compose.yaml</span><span class=\"pd-card-meta\">20 lines"
    ));
    assert_in_order(
        &html,
        &[
            "pd-line-no\">1<",
            "services:",
            "pd-line-no\">20<",
            "restart: always",
        ],
    );
}

#[test]
fn project_detail_files_scene_shows_env_names_with_hidden_values() {
    let html = scenes::render_scene_html("Project / files / 1440").expect("scene");
    assert_eq!(
        html.matches("aria-label=\"Value hidden\"").count(),
        6,
        "one mask per variable"
    );
    assert_eq!(
        html.matches("class=\"pd-key-value\" aria-label=\"Value hidden\">\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}<").count(),
        6,
        "each hidden value is eight dots"
    );
    for key in [
        "DATABASE_URL",
        "REDIS_URL",
        "SMTP_HOST",
        "SMTP_PORT",
        "JWT_SECRET",
        "STRIPE_KEY",
    ] {
        assert!(
            html.contains(&format!("aria-label=\"Reveal value of {key}\"")),
            "{key} has an eye"
        );
    }
    // No value of the file reaches the page, not even in a title or a label.
    assert!(
        !html.contains("class=\"pd-key-value pd-key-shown\""),
        "nothing is revealed"
    );
    for value in ["made-up-1", "made-up-2", "made-up-3", "shop:made-up"] {
        assert!(!html.contains(value), "{value} is on the page");
    }
}

#[test]
fn project_detail_files_scene_keeps_the_tree_in_one_order_across_renders() {
    let first = scenes::render_scene_html("Project / files / 1440").expect("scene");
    let second = scenes::render_scene_html("Project / files / 1440").expect("scene");
    let names = |html: &str| -> Vec<String> {
        html.split("pd-row-name\">")
            .skip(1)
            .filter_map(|rest| rest.split('<').next())
            .map(str::to_string)
            .collect()
    };
    assert_eq!(names(&first), names(&second));
    assert_eq!(
        names(&first),
        [
            "compose.yaml",
            ".env",
            ".env.example",
            "Dockerfile",
            "src/",
            "worker/",
            "package.json"
        ]
    );
}

#[test]
fn project_detail_invalid_scene_shows_the_line_and_message_and_marks_the_line() {
    let html = scenes::render_scene_html("Project / compose invalid / 1440").expect("scene");
    assert!(
        html.contains(
            "compose.yaml does not load, line 4: mapping values are not allowed in this context"
        ),
        "{html}"
    );
    // The banner has no "Show the file" button on the Files tab itself.
    assert!(!html.contains("Show the file"));
    // Only line 4 is marked.
    assert_eq!(html.matches("pd-line pd-line-error").count(), 1);
    assert_in_order(
        &html,
        &["pd-line pd-line-error\"><span class=\"pd-line-no\">4<"],
    );
}

#[test]
fn project_detail_invalid_message_names_the_file_compose_named_else_the_first_one() {
    let problem = ComposeProblem {
        line: Some(7),
        message: "did not find expected key".to_string(),
        file: Some("compose.override.yaml".to_string()),
    };
    assert_eq!(
        invalid_message(&problem, &[std::path::PathBuf::from("/p/compose.yaml")]),
        "compose.override.yaml does not load, line 7: did not find expected key"
    );
    let unnamed = ComposeProblem {
        line: None,
        message: "services.api.image must be a string".to_string(),
        file: None,
    };
    assert_eq!(
        invalid_message(
            &unnamed,
            &[std::path::PathBuf::from("/p/docker-compose.yml")]
        ),
        "docker-compose.yml does not load, services.api.image must be a string"
    );
    assert_eq!(
        invalid_message(&unnamed, &[]),
        "The Compose file does not load, services.api.image must be a string"
    );
}

// ------------------------------------------------------------ tree rows

fn listing_of(fs: &MemoryFs, rel: &str) -> DirListing {
    use crate::services::project_files::ProjectFs;
    fs.list(rel).expect("lists")
}

fn data_of(fs: &MemoryFs) -> TreeData {
    use crate::services::project_files::ProjectFs;
    let mut data = TreeData::default();
    data.listings
        .insert(String::new(), Load::Ready(listing_of(fs, "")));
    data.keys.insert(
        ".env".to_string(),
        Load::Ready(fs.env_keys(".env").expect("keys")),
    );
    data
}

fn entry_names(rows: &[TreeRow]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            TreeRow::Entry { entry, .. } => entry.rel.clone(),
            TreeRow::Key { file, key, .. } => format!("{file}#{key}"),
            TreeRow::Loading { .. } => "…".to_string(),
            TreeRow::Failed { .. } => "!".to_string(),
            TreeRow::More { .. } => "+".to_string(),
            TreeRow::Empty { .. } => "·".to_string(),
        })
        .collect()
}

#[test]
fn project_detail_tree_opening_a_folder_adds_rows_below_it_and_moves_none() {
    let fs = MemoryFs::new()
        .file("compose.yaml", "x")
        .file(".env", "A=1\nB=2\n")
        .file("src/index.js", "x")
        .file("src/util.js", "x")
        .file("package.json", "{}");
    let mut data = data_of(&fs);
    let closed = entry_names(&tree_rows(&data));
    data.expanded.insert("src".to_string());
    data.listings
        .insert("src".to_string(), Load::Ready(listing_of(&fs, "src")));
    let open = entry_names(&tree_rows(&data));
    // Everything that was there is still there, in the same order, with the
    // new rows between "src" and what followed it.
    let without_new: Vec<String> = open
        .iter()
        .filter(|name| !name.starts_with("src/"))
        .cloned()
        .collect();
    assert_eq!(without_new, closed);
    assert_eq!(
        &open[open.iter().position(|n| n == "src").expect("src") + 1..][..2],
        ["src/index.js", "src/util.js"]
    );
}

#[test]
fn project_detail_tree_an_env_file_lists_its_names_when_open_and_hides_them_when_closed() {
    let fs = MemoryFs::new().file(".env", "A=1\nB=2\n");
    let mut data = data_of(&fs);
    assert_eq!(entry_names(&tree_rows(&data)), [".env"]);
    data.expanded.insert(".env".to_string());
    assert_eq!(entry_names(&tree_rows(&data)), [".env", ".env#A", ".env#B"]);
    let rows = tree_rows(&data);
    assert!(matches!(
        &rows[0],
        TreeRow::Entry {
            key_count: Some(2),
            expanded: true,
            ..
        }
    ));
}

#[test]
fn project_detail_tree_shows_reading_failed_and_empty_states_in_place() {
    let mut data = TreeData::default();
    assert!(matches!(
        tree_rows(&data)[..],
        [TreeRow::Loading { depth: 0 }]
    ));
    data.listings.insert(
        String::new(),
        Load::Failed(crate::services::project_files::FsError::NotFound),
    );
    let rows = tree_rows(&data);
    assert!(
        matches!(&rows[..], [TreeRow::Failed { message, .. }] if message == "this folder is not on this Mac"),
        "{rows:?}"
    );
    data.listings
        .insert(String::new(), Load::Ready(DirListing::default()));
    assert!(matches!(
        tree_rows(&data)[..],
        [TreeRow::Empty { depth: 0 }]
    ));
    data.listings.insert(
        String::new(),
        Load::Ready(DirListing {
            entries: Vec::new(),
            more: 12,
        }),
    );
    assert!(tree_rows(&data)
        .iter()
        .any(|r| matches!(r, TreeRow::More { count: 12, .. })));
}

#[test]
fn project_detail_initial_selection_is_the_projects_compose_file_inside_the_folder() {
    let fs = MemoryFs::new().file("compose.yaml", "x").file("a.txt", "x");
    let root = listing_of(&fs, "");
    let folder = std::path::Path::new("/p/shop");
    assert_eq!(
        initial_selection(
            Some(folder),
            &[std::path::PathBuf::from("/p/shop/deploy/compose.yaml")],
            Some(&root)
        )
        .as_deref(),
        Some("deploy/compose.yaml")
    );
    // A Compose file elsewhere is not inside the folder: the listing decides.
    assert_eq!(
        initial_selection(
            Some(folder),
            &[std::path::PathBuf::from("/elsewhere/compose.yaml")],
            Some(&root)
        )
        .as_deref(),
        Some("compose.yaml")
    );
    assert_eq!(initial_selection(Some(folder), &[], None), None);
}

// -------------------------------------------------------- services tab

fn shuffled(mut containers: Vec<Container>) -> Vec<Container> {
    containers.reverse();
    let len = containers.len();
    containers.rotate_left(len / 3);
    containers
}

#[test]
fn project_detail_services_order_is_service_then_name_whatever_the_list_order() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let expected = project_container_ids(&project, &world.containers);
    let names: Vec<String> = expected
        .iter()
        .filter_map(|id| world.containers.iter().find(|c| &c.id == id))
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(
        names,
        [
            "shop-api-api-1",
            "shop-api-db-1",
            "shop-api-mailpit-1",
            "shop-api-redis-1",
            "shop-api-worker-1"
        ]
    );
    assert_eq!(
        project_container_ids(&project, &shuffled(world.containers.clone())),
        expected
    );
    // A container changing state does not move its row.
    let mut changed = world.containers.clone();
    for container in &mut changed {
        if container.name == "shop-api-db-1" {
            container.state = berth_core::ContainerState::Running;
        }
    }
    assert_eq!(project_container_ids(&project, &changed), expected);
}

#[test]
fn project_detail_services_leave_out_other_projects_containers() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let ids = project_container_ids(&project, &world.containers);
    assert_eq!(ids.len(), 5);
    assert!(ids.iter().all(|id| world
        .containers
        .iter()
        .find(|c| &c.id == id)
        .is_some_and(|c| c.name.starts_with("shop-api-"))));
}

#[test]
fn project_detail_down_note_names_what_is_not_running() {
    let world = world();
    let analytics = project_named(&world, "analytics");
    assert_eq!(
        down_note(&analytics, &world.containers).as_deref(),
        Some("grafana is not running.")
    );
    let shop = project_named(&world, "shop-api");
    let note = down_note(&shop, &world.containers).expect("two are down");
    assert!(note.ends_with("are not running."), "{note}");
    assert!(note.contains("db") && note.contains("worker"), "{note}");
    let berth_site = project_named(&world, "berth-site");
    assert_eq!(down_note(&berth_site, &world.containers), None);
}

#[test]
fn project_detail_services_scene_draws_rows_in_service_order_with_the_partial_note() {
    let html = scenes::render_scene_html("Project / services / 1440").expect("scene");
    assert!(html.contains("3 of 4 running"));
    assert!(html.contains("grafana is not running."));
    assert_in_order(
        &html,
        &[
            "analytics-clickhouse-1",
            "analytics-grafana-1",
            "analytics-ingest-1",
            "analytics-legacy-exporter-1",
        ],
    );
    assert!(html.contains("aria-selected=\"true\"><span>Services</span>"));
    // Other projects' containers are not here.
    assert!(!html.contains("shop-api-api-1"));
    // The Start/Stop button reads Stop project: three of four run.
    assert!(html.contains("Stop project"));
}

#[test]
fn project_detail_not_created_scene_says_so_and_does_not_offer_start_without_a_file() {
    let html = scenes::render_scene_html("Project / not created / 1440").expect("scene");
    assert!(html.contains("Not created yet"));
    assert!(
        html.contains("no Compose file Berth knows of")
            || html.contains("This folder has no Compose file Berth knows of")
    );
    assert!(html.contains("No Compose file is known for this project"));
    assert!(
        !html.contains("empty-state-action"),
        "nothing to start without a Compose file"
    );
    assert!(html.contains(">Not created<"));
}

// --------------------------------------------------------- activity tab

#[test]
fn project_detail_activity_scene_lists_only_this_projects_entries() {
    let html = scenes::render_scene_html("Project / activity / 1440").expect("scene");
    let rows = html.matches("class=\"pd-act ").count();
    assert!(rows >= 1, "{html}");
    assert!(html.contains("shop-api-worker-1"), "{html}");
    for other in ["analytics-ingest-1", "berth-site-web-1"] {
        assert!(
            !html.contains(&format!("pd-act-message\">{other}")),
            "{other} belongs to another project"
        );
    }
    assert!(html.contains("aria-selected=\"true\"><span>Activity</span>"));
}

// ----------------------------------------- the Files tab on a real folder

#[component]
fn DiskHarness(folder: std::path::PathBuf) -> Element {
    rsx! {
        files::FilesTab {
            folder: folder.clone(),
            folder_text: "~/disk".to_string(),
            compose_files: vec![folder.join("compose.yaml")],
            problem: None,
            editor: Editor { program: None, label: "the default app".to_string() },
            editor_setting: None,
            terminal_setting: None,
            on_refresh: move |()| {},
        }
    }
}

/// Let the tab's reads finish, then draw: the page once it holds every `until`.
async fn settled_html(dom: &mut VirtualDom, until: &[&str]) -> String {
    for _ in 0..60 {
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work()).await;
        dom.render_immediate(&mut dioxus::core::NoOpMutations);
        let html = dioxus_ssr::render(dom);
        if until.iter().all(|marker| html.contains(marker)) {
            return html;
        }
    }
    dioxus_ssr::render(dom)
}

#[tokio::test]
async fn project_detail_files_tab_reads_a_real_folder_and_hides_its_env_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("compose.yaml"),
        "services:\n  web:\n    image: nginx\n",
    )
    .expect("compose");
    std::fs::write(
        dir.path().join(".env"),
        "TOKEN=not-a-real-token-9\nMODE=dev\n",
    )
    .expect("env");
    std::fs::create_dir(dir.path().join("sub")).expect("sub");
    let mut dom = VirtualDom::new_with_props(
        DiskHarness,
        DiskHarnessProps {
            folder: dir.path().to_path_buf(),
        },
    );
    let _ = dom.rebuild_to_vec();
    let html = settled_html(&mut dom, &["image: nginx", "2 keys"]).await;
    assert_in_order(
        &html,
        &[
            "pd-row-name\">compose.yaml<",
            "pd-row-name\">.env<",
            "2 keys",
            "TOKEN",
            "MODE",
            "pd-row-name\">sub/<",
        ],
    );
    assert!(
        html.contains("<span class=\"pd-card-name\">compose.yaml</span>"),
        "{html}"
    );
    assert!(html.contains("image: nginx"));
    assert!(!html.contains("not-a-real-token-9"));
}

#[tokio::test]
async fn project_detail_files_tab_says_when_the_folder_is_not_on_this_mac() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gone = dir.path().join("deleted");
    let mut dom = VirtualDom::new_with_props(DiskHarness, DiskHarnessProps { folder: gone });
    let _ = dom.rebuild_to_vec();
    let html = settled_html(&mut dom, &["is not on this Mac"]).await;
    assert!(html.contains("this folder is not on this Mac"), "{html}");
}

// ------------------------------------------------------------ routing

#[test]
fn project_detail_the_project_section_draws_the_selected_project_and_nothing_else() {
    let html = scenes::render_scene_html("Project / services / 1440").expect("scene");
    assert!(html.contains("class=\"project-detail\""));
    assert!(html.contains("aria-label=\"Project analytics\""));
    assert!(
        !html.contains("class=\"projects-screen"),
        "the Projects table is not drawn"
    );
}
