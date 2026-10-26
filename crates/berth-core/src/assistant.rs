use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ContainerId, HarnessId, ProblemId, ProjectId, SessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessTransport {
    ClaudeStreamJson,
    MuseMsp,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Harness {
    pub id: HarnessId,
    pub name: String,
    pub binary: std::path::PathBuf,
    pub version: Option<String>,
    pub transport: HarnessTransport,
    pub available: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Risk {
    Read,
    Change,
    Destructive,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextItem {
    Logs {
        container: ContainerId,
        name: String,
        lines: Vec<String>,
    },
    Problem(ProblemId),
    ComposeFile(std::path::PathBuf),
    Dockerfile(std::path::PathBuf),
    Inspect(ContainerId),
    FolderTree(std::path::PathBuf),
    /// A plain project file (AD37): `package.json` for Generate with a harness.
    File(std::path::PathBuf),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub risk: Risk,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    Running,
    Done { ms: u64, summary: String },
    Failed { message: String },
    WaitingForApproval,
    Denied,
}
/// Where a proposed change stands (AD37). The plan itself lives in the
/// app's `PlanStore`; `Gone` when it no longer exists (e.g. after a restart).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalState {
    Pending,
    Written,
    Discarded,
    Gone,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TurnPart {
    Text(String),
    Tool {
        call: ToolCall,
        status: ToolStatus,
    },
    /// AD37: `save_session` writes JSON to berth.db and a `GenerationPlan`
    /// carries `.env` text, so a turn stores only a summary of the plan
    /// (per file: path, lines added, lines removed), never the plan.
    ProposedChange {
        change_id: uuid::Uuid,
        title: String,
        files: Vec<(std::path::PathBuf, u32, u32)>,
        state: ProposalState,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnRole {
    User,
    Assistant,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub role: TurnRole,
    pub parts: Vec<TurnPart>,
    pub at: DateTime<Utc>,
    /// AD37: the entry point shown above the turn, e.g.
    /// "From Logs · Ask about 2 lines".
    #[serde(default)]
    pub origin: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionState {
    Idle,
    Thinking,
    WaitingForApproval(ToolCall),
    Error { stderr: String },
    Stopped,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantSession {
    pub id: SessionId,
    pub harness: HarnessId,
    pub harness_session: Option<String>,
    pub project: Option<ProjectId>,
    pub context: Vec<ContextItem>,
    pub turns: Vec<Turn>,
    pub state: SessionState,
    pub watch: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trip_and_risk_distinct() {
        let s = AssistantSession {
            id: SessionId(uuid::Uuid::nil()),
            harness: HarnessId("muse".to_string()),
            harness_session: None,
            project: None,
            context: vec![],
            turns: vec![Turn {
                role: TurnRole::User,
                parts: vec![TurnPart::Text("hi".to_string())],
                at: DateTime::from_timestamp(40, 0).expect("ts"),
                origin: Some("From Logs · Ask about 2 lines".to_string()),
            }],
            state: SessionState::Idle,
            watch: false,
        };
        let json = serde_json::to_string(&s).expect("serialize");
        assert_eq!(
            serde_json::from_str::<AssistantSession>(&json).expect("deserialize"),
            s
        );
        assert_ne!(Risk::Read, Risk::Destructive);
    }

    #[test]
    fn tool_call_round_trip() {
        let c = ToolCall {
            call_id: "1".to_string(),
            tool: "container.list".to_string(),
            args: serde_json::json!({"all": true}),
            risk: Risk::Read,
        };
        let json = serde_json::to_string(&c).expect("serialize");
        assert_eq!(
            serde_json::from_str::<ToolCall>(&json).expect("deserialize"),
            c
        );
    }
}
