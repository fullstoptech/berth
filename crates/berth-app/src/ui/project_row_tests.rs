use super::*;
use berth_core::StatusKind as S;

struct World {
    engines: Vec<berth_core::Engine>,
    containers: Vec<berth_core::Container>,
    projects: Vec<berth_core::Project>,
    pending: std::collections::BTreeMap<PendingKey, PendingState>,
}

fn world() -> World {
    let w = berth_engine::MockWorld::bundled();
    World {
        engines: w.engines.clone(),
        containers: w.containers.clone(),
        projects: w.projects.clone(),
        pending: std::collections::BTreeMap::new(),
    }
}

fn model_of(w: &World, name: &str, problems: &[berth_core::Problem]) -> ProjectRowModel {
    let project = w
        .projects
        .iter()
        .find(|p| p.name == name)
        .expect("project in the mock world");
    let view = StateView {
        engines: &w.engines,
        containers: &w.containers,
        projects: &w.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending: &w.pending,
    };
    row_model(
        project,
        &view,
        problems,
        std::path::Path::new("/Users/berth"),
    )
}

fn error_on(container: &str, title: &str, w: &World) -> berth_core::Problem {
    let id = w
        .containers
        .iter()
        .find(|c| c.name == container)
        .expect("container")
        .id
        .clone();
    let kind = berth_core::ProblemKind::PortAllocated { port: 5432 };
    let subject = berth_core::Subject::Container(id);
    berth_core::Problem {
        id: berth_core::ProblemId::new(&kind, &subject),
        kind,
        severity: berth_core::Severity::Error,
        subject,
        title: title.to_string(),
        explanation: String::new(),
        evidence: Vec::new(),
        fixes: Vec::new(),
        first_seen: chrono::DateTime::default(),
    }
}

#[test]
fn projects_row_collapse_path_keeps_first_two_and_last() {
    assert_eq!(
        collapse_path("~/Dev/clients/acme/shop-api"),
        "~/Dev/…/shop-api"
    );
    assert_eq!(
        collapse_path("~/Dev/app-ideas/docker-manger"),
        "~/Dev/…/docker-manger"
    );
    assert_eq!(collapse_path("~/Dev/analytics"), "~/Dev/analytics");
    assert_eq!(collapse_path("/srv/landing"), "/srv/landing");
}

#[test]
fn projects_row_ports_are_running_services_sorted_once() {
    let w = world();
    let shop = w
        .projects
        .iter()
        .find(|p| p.name == "shop-api")
        .expect("shop");
    // db (5432) is exited, so it is not listed; the rest ascend.
    assert_eq!(running_ports(shop), vec![3000, 6379, 8025]);
    let analytics = w
        .projects
        .iter()
        .find(|p| p.name == "analytics")
        .expect("analytics");
    assert_eq!(running_ports(analytics), vec![8123, 9104]);
    let stopped = w
        .projects
        .iter()
        .find(|p| p.name == "ml-notebooks")
        .expect("ml");
    assert!(running_ports(stopped).is_empty());
}

#[test]
fn projects_row_service_note_names_the_worst_service() {
    let n = |items: &[(&str, S)]| {
        service_note(
            &items
                .iter()
                .map(|(name, s)| ((*name).to_string(), *s))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        n(&[("a", S::Running), ("b", S::Healthy)]),
        Some(("all running".to_string(), NoteTone::Muted))
    );
    assert_eq!(
        n(&[("a", S::Running)]),
        Some(("running".to_string(), NoteTone::Muted))
    );
    assert_eq!(
        n(&[("a", S::Exited(0)), ("b", S::Exited(0))]),
        Some(("stopped".to_string(), NoteTone::Muted))
    );
    assert_eq!(
        n(&[
            ("api", S::Running),
            ("db", S::Exited(1)),
            ("w", S::Restarting)
        ]),
        Some(("db failing".to_string(), NoteTone::Danger))
    );
    assert_eq!(
        n(&[("ch", S::Running), ("grafana", S::Exited(0))]),
        Some(("grafana exited".to_string(), NoteTone::Warning))
    );
    // A service that is down explains the project better than one that is
    // merely unhealthy (frame: analytics reads "grafana exited").
    assert_eq!(
        n(&[
            ("ingest", S::Unhealthy),
            ("grafana", S::Exited(0)),
            ("ch", S::Running)
        ]),
        Some(("grafana exited".to_string(), NoteTone::Warning))
    );
    assert_eq!(
        n(&[("ingest", S::Unhealthy), ("ch", S::Running)]),
        Some(("ingest unhealthy".to_string(), NoteTone::Warning))
    );
    assert_eq!(n(&[]), None);
}

#[test]
fn projects_row_model_of_the_failing_project_matches_the_frame() {
    let w = world();
    let problems = vec![error_on("shop-api-db-1", "Port 5432 is already in use", &w)];
    let m = model_of(&w, "shop-api", &problems);
    assert_eq!(m.kind, RowKind::Normal);
    assert_eq!(m.path_short, "~/Dev/…/shop-api");
    assert_eq!(m.engine_name, "default");
    assert_eq!(m.branch.as_deref(), Some("feat/checkout-v2"));
    assert_eq!(m.services_label, "5 services");
    assert_eq!(m.note, Some(("db failing".to_string(), NoteTone::Danger)));
    assert_eq!(m.ports, vec![3000, 6379, 8025]);
    assert_eq!(m.cpu.as_deref(), Some("3.7%"));
    assert_eq!(m.problem_count, 1);
    assert_eq!(
        m.problem_summary.as_deref(),
        Some("1 problem · Port 5432 is already in use")
    );
    assert_eq!(m.primary, PrimaryOp::Stop);
    assert_eq!(m.restart_reason, None);
    assert_eq!(m.primary_reason, None);
}

#[test]
fn projects_row_only_error_problems_count_on_the_row() {
    let w = world();
    let mut warning = error_on("analytics-ingest-1", "ingest health check is failing", &w);
    warning.severity = berth_core::Severity::Warning;
    let m = model_of(&w, "analytics", &[warning]);
    assert_eq!(m.problem_count, 0);
    assert_eq!(m.problem_summary, None);
}

#[test]
fn projects_row_stopped_project_offers_start_and_disables_restart_with_a_reason() {
    let w = world();
    let m = model_of(&w, "ml-notebooks", &[]);
    assert_eq!(m.primary, PrimaryOp::Start);
    assert_eq!(m.restart_reason.as_deref(), Some("Nothing is running"));
    assert_eq!(m.primary_reason, None);
    assert_eq!(m.cpu, None);
    assert_eq!(m.mem, None);
    assert_eq!(m.note, Some(("stopped".to_string(), NoteTone::Muted)));
}

#[test]
fn projects_row_unreachable_engine_disables_everything_with_the_engine_reason() {
    let w = world();
    let m = model_of(&w, "landing", &[]);
    assert_eq!(m.kind, RowKind::Unreachable);
    assert_eq!(m.primary, PrimaryOp::Start);
    let reason = m.primary_reason.expect("start is disabled");
    assert!(reason.contains("unreachable"), "{reason}");
    assert!(m.restart_reason.is_some());
    assert!(m.dots.iter().all(|d| d.tone == berth_core::Tone::Danger));
    assert!(m.note.is_none());
}

#[test]
fn projects_row_pinned_folder_without_compose_is_not_set_up() {
    let w = world();
    let m = model_of(&w, "docker-manger", &[]);
    assert_eq!(m.kind, RowKind::NotSetUp);
    assert_eq!(m.services_label, "No compose file yet");
    assert_eq!(m.dots.len(), 1);
    assert_eq!(m.dots[0].shape, DotShape::Hollow);
}

#[test]
fn projects_row_pending_and_failed_actions_show_in_the_model() {
    let mut w = world();
    let id = w
        .projects
        .iter()
        .find(|p| p.name == "berth-site")
        .expect("p")
        .id
        .clone();
    w.pending.insert(
        PendingKey {
            subject: berth_core::Subject::Project(id.clone()),
            action: "project.stop",
        },
        PendingState::Running {
            label: "Stopping…".to_string(),
        },
    );
    let m = model_of(&w, "berth-site", &[]);
    assert_eq!(
        m.pending,
        Some(RowPending {
            action: "project.stop",
            label: "Stopping…".to_string()
        })
    );
    // Other buttons are refused while it runs (one pure rule, D34).
    assert_eq!(
        m.restart_reason.as_deref(),
        Some("Stopping… is still in progress")
    );

    w.pending.clear();
    w.pending.insert(
        PendingKey {
            subject: berth_core::Subject::Project(id),
            action: "project.stop",
        },
        PendingState::Failed {
            message: "Compose failed: no such service".to_string(),
        },
    );
    let m = model_of(&w, "berth-site", &[]);
    assert_eq!(m.pending, None);
    assert_eq!(
        m.failure,
        Some(RowFailure {
            action: "project.stop",
            message: "Compose failed: no such service".to_string()
        })
    );
    // A failure does not block a retry.
    assert_eq!(m.restart_reason, None);
}
