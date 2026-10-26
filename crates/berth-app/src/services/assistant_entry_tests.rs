use std::path::PathBuf;

use berth_core::{
    Confidence, Container, ContainerId, ContextItem, Detection, EngineId, FoundFile, FoundKind,
    Harness, HarnessId, HarnessTransport, Problem, ProblemId, ProblemKind, Project, ProjectId,
    ProposalState, ServiceGuess, ServiceKind, SessionState, Severity, StackGuess, Subject,
    TurnPart, TurnRole,
};
use tokio_util::sync::CancellationToken;

use super::super::assistant::{watch_follow_up, AssistantDeps, AssistantService, OpenRequest};
use super::super::tools::ProposalNotice;
use super::*;

fn harness() -> Harness {
    Harness {
        id: HarnessId("claude-code".to_string()),
        name: "Claude Code".to_string(),
        binary: PathBuf::from("/bin/true"),
        version: Some("9.9.9".to_string()),
        transport: HarnessTransport::ClaudeStreamJson,
        available: true,
    }
}

fn project(folder: PathBuf) -> Project {
    Project {
        id: ProjectId("shop-api".to_string()),
        name: "shop-api".to_string(),
        folder: Some(folder.clone()),
        compose_files: vec![folder.join("compose.yaml")],
        engine: EngineId("colima:default".to_string()),
        source: berth_core::ProjectSource::Compose,
        services: Vec::new(),
        git: None,
        status: berth_core::ProjectStatus::AllRunning,
        pinned: false,
    }
}

fn container(id: &str, name: &str) -> Container {
    let mut world = berth_engine::MockWorld::bundled();
    let mut c = world.containers.remove(0);
    c.id = ContainerId(id.to_string());
    c.name = name.to_string();
    c.project = Some(ProjectId("shop-api".to_string()));
    c.compose_project = Some("shop-api".to_string());
    c
}

#[test]
fn assistant_entry_logs_context_groups_by_container_and_caps_at_400() {
    let a = ContainerId("a".to_string());
    let b = ContainerId("b".to_string());
    let mut lines: Vec<(ContainerId, String)> =
        (0..300).map(|i| (a.clone(), format!("a{i}"))).collect();
    lines.extend((0..300).map(|i| (b.clone(), format!("b{i}"))));
    let items = logs_context(&lines, &[container("a", "web-1"), container("b", "db-1")]);
    assert_eq!(items.len(), 2);
    let total: usize = items
        .iter()
        .map(|item| match item {
            ContextItem::Logs { lines, .. } => lines.len(),
            _ => 0,
        })
        .sum();
    assert_eq!(total, LOG_LINES_CAP);
    match &items[0] {
        ContextItem::Logs { name, lines, .. } => {
            assert_eq!(name, "web-1");
            assert_eq!(lines.len(), 300);
        }
        other => panic!("first item is logs of a, got {other:?}"),
    }
}

#[test]
fn assistant_entry_logs_entry_names_origin_and_rules() {
    let folder = PathBuf::from("/tmp/shop-api");
    let project = project(folder);
    let lines = vec![
        (
            ContainerId("a".to_string()),
            "Error: connect ECONNREFUSED".to_string(),
        ),
        (
            ContainerId("a".to_string()),
            "at main (index.js:4)".to_string(),
        ),
    ];
    let entry = logs_entry(
        &project,
        &harness(),
        &lines,
        &[container("a", "web-1")],
        &[],
    )
    .expect("entry");
    assert_eq!(entry.origin, "From Logs \u{b7} Ask about 2 lines");
    // The selection becomes a context chip the panel can draw.
    let (icon, label) = crate::screens::assistant::context_chip_model(&entry.context[0], &[], &[]);
    assert_eq!(icon, "logs");
    assert_eq!(label, "web-1 \u{b7} 2 lines");

    let explained = logs_entry(
        &project,
        &harness(),
        &lines,
        &[container("a", "web-1")],
        &["db is not reachable".to_string()],
    )
    .expect("entry");
    assert!(
        explained
            .prompt
            .contains("rules already matched: db is not reachable"),
        "{}",
        explained.prompt
    );
    assert!(logs_entry(&project, &harness(), &[], &[], &[]).is_none());
}

#[test]
fn assistant_entry_problem_entry_attaches_problem_logs_inspect_compose_and_tree() {
    let folder = PathBuf::from("/tmp/shop-api");
    let project = project(folder.clone());
    let web = container("a", "web-1");
    let problem = Problem {
        id: ProblemId("p1".to_string()),
        kind: ProblemKind::PortAllocated { port: 56080 },
        severity: Severity::Error,
        subject: Subject::Container(web.id.clone()),
        title: "Port 56080 is in use".to_string(),
        explanation: "Another process holds it.".to_string(),
        evidence: vec!["bind: address already in use".to_string()],
        fixes: Vec::new(),
        first_seen: chrono::Utc::now(),
    };
    let entry = problem_entry(&project, &harness(), &problem, Some(&web)).expect("entry");
    assert!(matches!(entry.context[0], ContextItem::Problem(_)));
    assert!(entry
        .context
        .iter()
        .any(|i| matches!(i, ContextItem::Logs { .. })));
    assert!(entry
        .context
        .iter()
        .any(|i| matches!(i, ContextItem::Inspect(_))));
    assert!(entry
        .context
        .iter()
        .any(|i| matches!(i, ContextItem::ComposeFile(p) if p == &folder.join("compose.yaml"))));
    assert!(entry
        .context
        .iter()
        .any(|i| matches!(i, ContextItem::FolderTree(p) if p == &folder)));
    assert_eq!(entry.origin, "From Problems \u{b7} Port 56080 is in use");
    assert!(
        entry.prompt.contains("files.propose_edit"),
        "{}",
        entry.prompt
    );
}

#[test]
fn assistant_entry_generate_entry_attaches_manifests_never_env() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::write(dir.path().join("package.json"), "{}").expect("pkg");
    std::fs::write(dir.path().join(".env"), "SECRET=1\n").expect("env");
    let detection = Detection {
        folder: dir.path().to_path_buf(),
        found: vec![FoundFile {
            kind: FoundKind::Env { keys: 1 },
            path: dir.path().join(".env"),
        }],
        stack: vec![StackGuess::Node {
            pm: "pnpm".to_string(),
            framework: Some("Next.js".to_string()),
            framework_version: Some("15".to_string()),
            version: None,
        }],
        services_needed: vec![ServiceGuess {
            kind: ServiceKind::Postgres,
            reason: "from DATABASE_URL".to_string(),
        }],
        confidence: Confidence::High,
        git_branch: None,
    };
    let engine = EngineId("colima:default".to_string());
    let entry = generate_entry(dir.path(), &engine, &harness(), &detection);
    assert_eq!(
        entry.request.project,
        ProjectId::for_folder(dir.path(), &engine)
    );
    assert!(entry
        .context
        .iter()
        .any(|i| matches!(i, ContextItem::File(p) if p == &dir.path().join("package.json"))));
    assert!(!entry.context.iter().any(|i| matches!(i,
        ContextItem::File(p) | ContextItem::ComposeFile(p) | ContextItem::Dockerfile(p)
        if p == &dir.path().join(".env"))));
    assert!(entry.prompt.contains("Next.js 15"), "{}", entry.prompt);
    assert!(
        entry.prompt.contains("files.propose_edit"),
        "{}",
        entry.prompt
    );
}

fn bare_service() -> (
    AssistantService,
    tokio::sync::mpsc::UnboundedSender<ProposalNotice>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (service, _ui) = AssistantService::new(AssistantDeps {
        store: None,
        core: None,
        mcp: None,
        approvals: None,
        proposals: Some(rx),
        cancel: CancellationToken::new(),
    });
    (service, tx)
}

#[tokio::test]
async fn assistant_entry_run_attaches_context_and_sends_the_first_turn() {
    let (service, _tx) = bare_service();
    let folder = PathBuf::from("/tmp/shop-api");
    let project = project(folder);
    let lines = vec![(ContainerId("a".to_string()), "boom".to_string())];
    let entry = logs_entry(
        &project,
        &harness(),
        &lines,
        &[container("a", "web-1")],
        &[],
    )
    .expect("entry");
    // No bridge in this rig: the turn is recorded, then send refuses.
    let _ = run(&service, entry).await;
    // The session opened for the folder: reopen and read it back.
    let id = service
        .open(OpenRequest {
            harness: harness(),
            project: project.id.clone(),
            project_name: "shop-api".to_string(),
            folder: project.folder.clone().expect("folder"),
            model: None,
        })
        .await
        .expect("open");
    let session = service.session_snapshot(&id).expect("session");
    assert_eq!(session.context.len(), 1);
    let turn = session.turns.last().expect("turn");
    assert_eq!(turn.role, TurnRole::User);
    assert_eq!(
        turn.origin.as_deref(),
        Some("From Logs \u{b7} Ask about 1 line")
    );
}

#[tokio::test]
async fn assistant_entry_watch_proposes_only_and_never_writes() {
    let dir = tempfile::tempdir().expect("dir");
    let folder = dir.path().join("shop-api");
    std::fs::create_dir_all(&folder).expect("folder");
    let compose = folder.join("compose.yaml");
    std::fs::write(&compose, "services:\n  web:\n    image: nginx\n").expect("compose");
    let (service, notices) = bare_service();
    let id = service
        .open(OpenRequest {
            harness: harness(),
            project: ProjectId("shop-api".to_string()),
            project_name: "shop-api".to_string(),
            folder: folder.clone(),
            model: None,
        })
        .await
        .expect("open");
    let plan_id = uuid::Uuid::now_v7();
    notices
        .send(ProposalNotice {
            session: id,
            plan_id,
            title: "Move web to port 56081".to_string(),
            files: vec![(compose.clone(), 1, 1)],
        })
        .expect("notice");
    // The notice lands on the session through the service's own task.
    let mut seen = false;
    for _ in 0..100 {
        if service.session_snapshot(&id).is_some_and(|s| {
            s.turns.iter().flat_map(|t| &t.parts).any(|p| {
                matches!(
                    p,
                    TurnPart::ProposedChange {
                        state: ProposalState::Pending,
                        ..
                    }
                )
            })
        }) {
            seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(seen, "the proposal reached the session");

    // Not watching: writing the plan adds no turn.
    service.plan_written(plan_id);
    let session = service.session_snapshot(&id).expect("session");
    assert!(
        session.turns.iter().all(|t| t.origin.is_none()),
        "no watch turn while watch is off: {:?}",
        session.turns.iter().map(|t| &t.origin).collect::<Vec<_>>()
    );

    // Watching: writing the (next) plan adds exactly one follow-up
    // question — and nothing is approved or written by the watch.
    service.set_watch(id, true);
    let plan2 = uuid::Uuid::now_v7();
    notices
        .send(ProposalNotice {
            session: id,
            plan_id: plan2,
            title: "Raise the memory limit".to_string(),
            files: vec![(compose.clone(), 1, 0)],
        })
        .expect("notice 2");
    for _ in 0..100 {
        if service.session_snapshot(&id).is_some_and(|s| {
            s.turns
                .iter()
                .flat_map(|t| &t.parts)
                .filter(|p| matches!(p, TurnPart::ProposedChange { .. }))
                .count()
                == 2
        }) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let before = std::fs::read_to_string(&compose).expect("read");
    service.plan_written(plan2);
    let session = service.session_snapshot(&id).expect("session");
    let watch_turns: Vec<_> = session
        .turns
        .iter()
        .filter(|t| t.origin.as_deref().is_some_and(|o| o.starts_with("Watch")))
        .collect();
    assert_eq!(watch_turns.len(), 1);
    match &watch_turns[0].parts[0] {
        TurnPart::Text(text) => {
            assert!(text.contains("Propose only"), "{text}");
            assert!(text.contains("files.propose_edit"), "{text}");
        }
        other => panic!("watch turn is text, got {other:?}"),
    }
    assert!(service.approvals_snapshot().is_empty());
    assert_eq!(
        std::fs::read_to_string(&compose).expect("read"),
        before,
        "watch wrote the file"
    );

    // The pure seam agrees: no follow-up for a busy session.
    let mut busy = session.clone();
    busy.state = SessionState::Thinking;
    assert!(watch_follow_up(&busy, &plan2).is_none());
}
