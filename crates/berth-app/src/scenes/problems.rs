//! Scenes of the Problems tab (P5-T8). The brief gives the tab no frame ("built from the components
//! page and the nearest frame's layout"), so these are dev scenes: the Containers screen of
//! "Containers / selected / 1440" with the bottom panel open on Problems.

use super::*;

use berth_core::{
    ContainerAction, EffectKind, EffectLine, EffectSummary, EngineAction, Fix, FixEffect,
    FixSource, GenerationPlan, HarnessId, PlanSource, Problem, ProblemId, ProblemKind, Severity,
    Subject,
};

use crate::state::{BottomTab, PendingKey, PendingState};

fn effect(verb: &str, noun: &str, lines: &[(EffectKind, &str)]) -> EffectSummary {
    EffectSummary {
        verb: verb.to_string(),
        noun_one: noun.to_string(),
        noun_many: format!("{noun}s"),
        count: 1,
        bytes: None,
        lines: lines
            .iter()
            .map(|(kind, text)| EffectLine {
                kind: *kind,
                text: (*text).to_string(),
            })
            .collect(),
        typed_name: None,
    }
}

fn claude() -> Fix {
    Fix {
        label: "Fix with Claude Code".to_string(),
        source: FixSource::Harness(HarnessId("claude".to_string())),
        effect: FixEffect::Explain,
    }
}

fn restart(engine: &berth_core::EngineId, c: &berth_core::Container) -> Fix {
    Fix {
        label: format!("Restart {}", c.name),
        source: FixSource::Rule,
        effect: FixEffect::RunAction {
            action: EngineAction::Container {
                engine: engine.clone(),
                id: c.id.clone(),
                action: ContainerAction::Restart { timeout_s: None },
            },
            effect: effect(
                "Restart",
                "container",
                &[
                    (EffectKind::Changes, "It stops and starts again"),
                    (EffectKind::Stays, "Its data and settings"),
                ],
            ),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn problem(
    kind: ProblemKind,
    severity: Severity,
    subject: Subject,
    title: &str,
    explanation: &str,
    evidence: &[&str],
    fixes: Vec<Fix>,
    seconds_ago: i64,
) -> Problem {
    Problem {
        id: ProblemId::new(&kind, &subject),
        kind,
        severity,
        subject,
        title: title.to_string(),
        explanation: explanation.to_string(),
        evidence: evidence.iter().map(|l| (*l).to_string()).collect(),
        fixes,
        first_seen: chrono::Utc::now() - chrono::Duration::seconds(seconds_ago),
    }
}

/// Seven problems across the shop-api containers, one of each kind the broken fixtures raise.
fn seven(state: &AppState) -> Vec<Problem> {
    let containers = state.containers.peek().clone();
    let find = |name: &str| {
        containers
            .iter()
            .find(|c| c.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("mock container {name}"))
    };
    let (db, api, worker, redis, mailpit) = (
        find("shop-api-db-1"),
        find("shop-api-api-1"),
        find("shop-api-worker-1"),
        find("shop-api-redis-1"),
        find("shop-api-mailpit-1"),
    );
    let engine = db.engine.clone();
    let folder = state
        .projects
        .peek()
        .iter()
        .find(|p| p.name == "shop-api")
        .and_then(|p| p.folder.clone())
        .unwrap_or_default();
    let plan = GenerationPlan {
        root: folder,
        files: vec![berth_rules::diff::file_diff(
            std::path::Path::new("compose.yaml"),
            Some("services:\n  db:\n    ports:\n      - \"5432:5432\"\n"),
            Some("services:\n  db:\n    ports:\n      - \"5433:5432\"\n"),
        )],
        source: PlanSource::Rule(ProblemKind::PortAllocated { port: 5432 }),
        notes: Vec::new(),
        after_write: None,
    };
    vec![
        problem(
            ProblemKind::PortAllocated { port: 5432 },
            Severity::Error,
            Subject::Container(db.id.clone()),
            "Port 5432 is already in use",
            "pg-scratch publishes 5432 on this Mac, so shop-api-db-1 could not start. Map it to a free port instead.",
            &["Bind for 0.0.0.0:5432 failed: port is already allocated"],
            vec![
                Fix {
                    label: "Map db to port 5433".to_string(),
                    source: FixSource::Rule,
                    effect: FixEffect::EditFiles(plan),
                },
                claude(),
            ],
            310,
        ),
        problem(
            ProblemKind::ExitedNoLogs { code: 1 },
            Severity::Error,
            Subject::Container(worker.id.clone()),
            "shop-api-worker-1 exits at once",
            "It stops with code 1 and prints nothing, so there is no message to read. Look at its command and its environment.",
            &[],
            vec![claude()],
            240,
        ),
        problem(
            ProblemKind::OutOfMemory,
            Severity::Error,
            Subject::Container(api.id.clone()),
            "shop-api-api-1 ran out of memory",
            "The kernel killed it at its 256 MB limit (exit 137). Give it more room.",
            &["Exited (137), OOMKilled"],
            vec![claude()],
            200,
        ),
        problem(
            ProblemKind::BindPathMissing {
                path: "/Users/you/Dev/shop-api/uploads".to_string(),
            },
            Severity::Error,
            Subject::Container(mailpit.id.clone()),
            "A folder shop-api-mailpit-1 needs is missing",
            "The bind mount points at /Users/you/Dev/shop-api/uploads, which does not exist on this Mac.",
            &["invalid mount config: bind source path does not exist"],
            vec![claude()],
            150,
        ),
        problem(
            ProblemKind::EnvVarMissing {
                var: "SESSION_SECRET".to_string(),
            },
            Severity::Warning,
            Subject::Project(
                state
                    .projects
                    .peek()
                    .iter()
                    .find(|p| p.name == "shop-api")
                    .map(|p| p.id.clone())
                    .unwrap_or_else(|| berth_core::ProjectId("shop-api".to_string())),
            ),
            "SESSION_SECRET is not set",
            "compose.yaml uses ${SESSION_SECRET}, and .env has no line for it, so Compose fills in an empty value.",
            &["The \"SESSION_SECRET\" variable is not set. Defaulting to a blank string."],
            vec![claude()],
            120,
        ),
        problem(
            ProblemKind::Unhealthy,
            Severity::Warning,
            Subject::Container(redis.id.clone()),
            "shop-api-redis-1 is unhealthy",
            "Its health check failed 3 times in a row.",
            &["redis-cli ping: Could not connect to Redis at 127.0.0.1:6379"],
            vec![restart(&engine, &redis), claude()],
            60,
        ),
        problem(
            ProblemKind::ImageNotFound {
                reference: "shop/mailer:2".to_string(),
            },
            Severity::Error,
            Subject::Project(
                state
                    .projects
                    .peek()
                    .iter()
                    .find(|p| p.name == "shop-api")
                    .map(|p| p.id.clone())
                    .unwrap_or_else(|| berth_core::ProjectId("shop-api".to_string())),
            ),
            "The image shop/mailer:2 was not found",
            "Docker could not pull it: the repository does not exist, or it needs a login.",
            &["pull access denied for shop/mailer, repository does not exist or may require 'docker login'"],
            vec![claude()],
            20,
        ),
    ]
}

fn base(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Dark;
    state.section.set(Section::Containers);
    let mut bottom = state.bottom.write();
    bottom.open = true;
    bottom.height_px = 408;
    bottom.tab = BottomTab::Problems;
}

fn problems_tab(state: &mut AppState) {
    base(state);
    let list = seven(state);
    state.problems.set(list);
}

/// Two problems, both open: the restart of the unhealthy one was clicked and the engine refused it.
fn problems_failed(state: &mut AppState) {
    base(state);
    let mut list = seven(state);
    list.retain(|p| matches!(p.kind, ProblemKind::Unhealthy | ProblemKind::PortAllocated { .. }));
    list.reverse();
    let redis = state
        .containers
        .peek()
        .iter()
        .find(|c| c.name == "shop-api-redis-1")
        .map(|c| c.id.clone());
    state.problems.set(list);
    if let Some(id) = redis {
        state.pending.write().insert(
            PendingKey {
                subject: Subject::Container(id),
                action: "container.restart",
            },
            PendingState::Failed {
                message: "Cannot restart shop-api-redis-1: the container is being removed"
                    .to_string(),
            },
        );
    }
}

fn problems_empty(state: &mut AppState) {
    base(state);
    state.problems.set(Vec::new());
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Problems / empty / 1440", problems_empty),
    Scene::dev("Problems / fix failed / 1440", problems_failed),
    Scene::dev("Problems / tab / 1440", problems_tab),
];
