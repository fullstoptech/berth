//! Scenes of the Assistant tab (P7-T5). The frame scene seeds the
//! shop-api conversation of "Project / assistant / 1440" — origin line,
//! three read rows, the answer, chips, the proposed change and its
//! "Then:" approval — and [`seed_plans`] parks the plan the card names
//! so Review changes really opens. The rest are dev scenes for the
//! states the frame cannot show.

use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::screens::project_detail::{ProjectDetailSeed, Tab, Validity};
use crate::services::plans::{PendingPlan, PlanRun, PlanStore};
use crate::services::project_files::{Editor, ProjectFsOverride};
use crate::state::{Approval, BottomTab};

use berth_core::{
    AssistantSession, ContextItem, EffectKind, EffectLine, EffectSummary, ProposalState, Risk,
    SessionId, SessionState, ToolCall, ToolStatus, Turn, TurnPart, TurnRole,
};

fn at(secs: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_792_850_000 + secs, 0).unwrap_or_default()
}

fn open_shop_api(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Dark);
    let id = state
        .projects
        .peek()
        .iter()
        .find(|p| p.name == "shop-api")
        .map(|p| p.id.clone());
    if let Some(id) = id {
        state.selection.set(Selection::Project(id));
        state.section.set(Section::Project);
    }
    provide_context(ProjectFsOverride(Arc::new(
        super::project_detail::shop_api_folder(super::project_detail::SHOP_API_COMPOSE),
    )));
    provide_context(ProjectDetailSeed {
        tab: Tab::Services,
        validity: Some(Validity::Valid),
        editor: Some(Editor {
            program: Some(PathBuf::from("/usr/local/bin/code")),
            label: "Visual Studio Code (detected)".to_string(),
        }),
        activity: None,
    });
    state.harnesses.set(super::first_run::frame_harnesses());
    let mut bottom = state.bottom.write();
    bottom.open = true;
    bottom.tab = BottomTab::Assistant;
    bottom.height_px = 380;
    bottom.maximized = false;
}

fn shop_api_id(state: &AppState) -> berth_core::ProjectId {
    state
        .projects
        .peek()
        .iter()
        .find(|p| p.name == "shop-api")
        .map(|p| p.id.clone())
        .unwrap_or_else(|| berth_core::ProjectId("shop-api".to_string()))
}

fn container_id(state: &AppState, name: &str) -> berth_core::ContainerId {
    state
        .containers
        .peek()
        .iter()
        .find(|c| c.name == name)
        .map(|c| c.id.clone())
        .unwrap_or_else(|| berth_core::ContainerId(name.to_string()))
}

fn frame_context(state: &AppState) -> Vec<ContextItem> {
    let folder = state
        .projects
        .peek()
        .iter()
        .find(|p| p.name == "shop-api")
        .and_then(|p| p.folder.clone())
        .unwrap_or_else(|| PathBuf::from("/Users/berth/Dev/clients/acme/shop-api"));
    let problem = state
        .problems
        .peek()
        .iter()
        .find(|p| p.title.contains("5432"))
        .map(|p| p.id.clone())
        .unwrap_or_else(|| berth_core::ProblemId("port".to_string()));
    vec![
        ContextItem::Logs {
            container: container_id(state, "shop-api-worker-1"),
            name: "shop-api-worker-1".to_string(),
            lines: vec![
                "Error: connect ECONNREFUSED 127.0.0.1:5432".to_string(),
                "worker exited, restarting".to_string(),
            ],
        },
        ContextItem::Problem(problem),
        ContextItem::ComposeFile(folder.join("compose.yaml")),
    ]
}

fn read_row(call_id: &str, tool: &str, args: serde_json::Value, ms: u64) -> TurnPart {
    let call = ToolCall {
        call_id: call_id.to_string(),
        tool: tool.to_string(),
        args,
        risk: Risk::Read,
    };
    let summary = crate::services::assistant::tool_summary(&call.tool, &call.args);
    TurnPart::Tool {
        call,
        status: ToolStatus::Done { ms, summary },
    }
}

fn frame_session(state: &AppState, change_id: uuid::Uuid) -> AssistantSession {
    AssistantSession {
        id: SessionId(uuid::Uuid::now_v7()),
        harness: berth_core::HarnessId("claude-code".to_string()),
        harness_session: Some("frame-session".to_string()),
        project: Some(shop_api_id(state)),
        context: frame_context(state),
        turns: vec![
            Turn {
                role: TurnRole::User,
                parts: vec![TurnPart::Text(
                    "Why does the worker keep restarting? Fix it.".to_string(),
                )],
                at: at(0),
                origin: Some("From Logs \u{b7} Ask about 2 lines".to_string()),
            },
            Turn {
                role: TurnRole::Assistant,
                parts: vec![
                    read_row(
                        "call-1",
                        "containers.logs",
                        serde_json::json!({"container": "shop-api-worker-1", "tail": 200}),
                        140,
                    ),
                    read_row(
                        "call-2",
                        "containers.inspect",
                        serde_json::json!({"container": "shop-api-db-1"}),
                        38,
                    ),
                    read_row(
                        "call-3",
                        "files.read",
                        serde_json::json!({"path": "compose.yaml"}),
                        12,
                    ),
                    TurnPart::Text(
                        "The worker restarts because Postgres never started: port 5432 on your \
                         Mac is taken by pg-scratch, so shop-api-db-1 exits and the worker's \
                         connection is refused. Your .env sets DATABASE_URL to port 5432 (Berth \
                         read the port locally; the value is not sent). I'll move it to 5433, \
                         move shop-api's Postgres to host port 5433 and make the worker wait \
                         for a healthy database."
                            .to_string(),
                    ),
                    TurnPart::ProposedChange {
                        change_id,
                        title: "Move db to 5433 and update DATABASE_URL".to_string(),
                        files: vec![
                            (PathBuf::from("compose.yaml"), 9, 2),
                            (PathBuf::from(".env"), 1, 1),
                        ],
                        state: ProposalState::Pending,
                    },
                ],
                at: at(20),
                origin: None,
            },
        ],
        state: SessionState::WaitingForApproval(ToolCall {
            call_id: "call-4".to_string(),
            tool: "compose.up".to_string(),
            args: serde_json::json!({"project": "shop-api", "services": ["db", "worker"]}),
            risk: Risk::Change,
        }),
        watch: false,
    }
}

fn frame_approval(session: SessionId, change_id: uuid::Uuid) -> Approval {
    Approval {
        id: uuid::Uuid::now_v7(),
        session,
        call: ToolCall {
            call_id: "call-4".to_string(),
            tool: "compose.up".to_string(),
            args: serde_json::json!({"project": "shop-api", "services": ["db", "worker"]}),
            risk: Risk::Change,
        },
        effect: EffectSummary {
            verb: "Recreate".to_string(),
            noun_one: "service".to_string(),
            noun_many: "services".to_string(),
            count: 2,
            bytes: None,
            lines: vec![
                EffectLine {
                    kind: EffectKind::Changes,
                    text: "recreate db and worker".to_string(),
                },
                EffectLine {
                    kind: EffectKind::Changes,
                    text: "Runs automatically with 'Write and recreate', or approve it here \
                           after 'Write files only'. Volume shop-api_pgdata is kept."
                        .to_string(),
                },
            ],
            typed_name: None,
        },
        typed_name: None,
        after_plan: Some(change_id),
    }
}

/// "Project / assistant / 1440": the frame's conversation, cards and all.
fn project_assistant(state: &mut AppState) {
    open_shop_api(state);
    let change_id = uuid::Uuid::now_v7();
    let session = frame_session(state, change_id);
    let approval = frame_approval(session.id, change_id);
    state.assistant.write().insert(session.id, session);
    state.approvals.set(vec![approval]);
}

fn base_session(state: &AppState, session_state: SessionState, turns: Vec<Turn>) -> AssistantSession {
    AssistantSession {
        id: SessionId(uuid::Uuid::now_v7()),
        harness: berth_core::HarnessId("claude-code".to_string()),
        harness_session: Some("dev-session".to_string()),
        project: Some(shop_api_id(state)),
        context: frame_context(state),
        turns,
        state: session_state,
        watch: false,
    }
}

fn user_turn(text: &str) -> Turn {
    Turn {
        role: TurnRole::User,
        parts: vec![TurnPart::Text(text.to_string())],
        at: at(0),
        origin: None,
    }
}

fn thinking(state: &mut AppState) {
    open_shop_api(state);
    let session = base_session(
        state,
        SessionState::Thinking,
        vec![
            user_turn("What is using the most memory?"),
            Turn {
                role: TurnRole::Assistant,
                parts: vec![
                    read_row(
                        "call-1",
                        "containers.list",
                        serde_json::json!({"project": "shop-api"}),
                        41,
                    ),
                    TurnPart::Tool {
                        call: ToolCall {
                            call_id: "call-2".to_string(),
                            tool: "containers.stats".to_string(),
                            args: serde_json::json!({"container": "shop-api-api-1"}),
                            risk: Risk::Read,
                        },
                        status: ToolStatus::Running,
                    },
                    TurnPart::Text("Checking each container".to_string()),
                ],
                at: at(5),
                origin: None,
            },
        ],
    );
    state.assistant.write().insert(session.id, session);
}

fn waiting(state: &mut AppState) {
    open_shop_api(state);
    let call = ToolCall {
        call_id: "call-9".to_string(),
        tool: "containers.restart".to_string(),
        args: serde_json::json!({"container": "shop-api-worker-1"}),
        risk: Risk::Change,
    };
    let mut session = base_session(
        state,
        SessionState::WaitingForApproval(call.clone()),
        vec![
            user_turn("Restart the worker, please."),
            Turn {
                role: TurnRole::Assistant,
                parts: vec![TurnPart::Tool {
                    call: call.clone(),
                    status: ToolStatus::WaitingForApproval,
                }],
                at: at(5),
                origin: None,
            },
        ],
    );
    session.context = Vec::new();
    let approval = Approval {
        id: uuid::Uuid::now_v7(),
        session: session.id,
        call,
        effect: EffectSummary {
            verb: "Restart".to_string(),
            noun_one: "container".to_string(),
            noun_many: "containers".to_string(),
            count: 1,
            bytes: None,
            lines: vec![EffectLine {
                kind: EffectKind::Changes,
                text: "Restart container shop-api-worker-1".to_string(),
            }],
            typed_name: None,
        },
        typed_name: None,
        after_plan: None,
    };
    state.assistant.write().insert(session.id, session);
    state.approvals.set(vec![approval]);
}

fn errored(state: &mut AppState) {
    open_shop_api(state);
    let session = base_session(
        state,
        SessionState::Error {
            stderr: "claude: stream closed before the turn finished\n(exit 1)".to_string(),
        },
        vec![user_turn("Why does the worker keep restarting?")],
    );
    state.assistant.write().insert(session.id, session);
}

fn no_harness(state: &mut AppState) {
    open_shop_api(state);
    state.harnesses.set(Vec::new());
}

fn watch(state: &mut AppState) {
    open_shop_api(state);
    let mut session = base_session(
        state,
        SessionState::Idle,
        vec![
            user_turn("Keep an eye on the worker while I work."),
            Turn {
                role: TurnRole::Assistant,
                parts: vec![TurnPart::Text(
                    "Watching shop-api. When a fix looks clear, it appears here for your click."
                        .to_string(),
                )],
                at: at(5),
                origin: None,
            },
        ],
    );
    session.watch = true;
    state.assistant.write().insert(session.id, session);
}

fn empty(state: &mut AppState) {
    open_shop_api(state);
}

fn no_project(state: &mut AppState) {
    open_shop_api(state);
    state.selection.set(Selection::None);
    state.section.set(Section::Projects);
}

fn maximized(state: &mut AppState) {
    project_assistant(state);
    state.bottom.write().maximized = true;
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Project / assistant / 1440", project_assistant),
    Scene::dev("Assistant / thinking", thinking),
    Scene::dev("Assistant / waiting for approval", waiting),
    Scene::dev("Assistant / error", errored),
    Scene::dev("Assistant / no harness", no_harness),
    Scene::dev("Assistant / watch", watch),
    Scene::dev("Assistant / empty", empty),
    Scene::dev("Assistant / no project", no_project),
    Scene::dev("Assistant / maximized", maximized),
];

/// Park the plan each seeded pending proposal names, so its Review
/// changes button opens a real sheet in the shot app.
pub(super) fn seed_plans(state: &AppState, plans: PlanStore) {
    let sessions = state.assistant.peek().clone();
    for session in sessions.values() {
        for proposal in crate::screens::assistant::proposals_of(session) {
            if proposal.state != ProposalState::Pending || plans.contains(&proposal.change_id) {
                continue;
            }
            let project = state
                .projects
                .peek()
                .iter()
                .find(|p| Some(&p.id) == session.project.as_ref())
                .cloned();
            let root = project
                .as_ref()
                .and_then(|p| p.folder.clone())
                .unwrap_or_else(|| PathBuf::from("/Users/berth/Dev/clients/acme/shop-api"));
            let plan = berth_core::GenerationPlan {
                root,
                files: proposal
                    .files
                    .iter()
                    .map(|(path, added, removed)| berth_core::FileDiff {
                        path: path.clone(),
                        change: berth_core::FileChange::Modify,
                        old_sha256: None,
                        new_text: Some(String::new()),
                        hunks: Vec::new(),
                        added: *added,
                        removed: *removed,
                    })
                    .collect(),
                source: berth_core::PlanSource::Harness(session.harness.clone()),
                notes: vec![proposal.title.clone()],
                after_write: None,
            };
            let mut pending = PendingPlan::new(plan, "shop-api");
            pending.run = project.as_ref().and_then(PlanRun::for_project);
            plans.insert_at(proposal.change_id, pending);
        }
    }
}
