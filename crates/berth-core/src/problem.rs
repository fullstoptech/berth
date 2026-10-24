use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::compose::{ComposeCommand, ComposeTarget};
use crate::container::ContainerAction;
use crate::effect::{EffectSummary, PrunePlan};
use crate::engine::{Arch, ColimaStartOptions};
use crate::generation::GenerationPlan;
use crate::ids::{ContainerId, EngineId, HarnessId, ProblemId, ProjectId};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProblemKind {
    PortAllocated { port: u16 },
    ImageNotFound { reference: String },
    ExecFormat { image_arch: Option<Arch> },
    OutOfMemory,
    ExitedNoLogs { code: i32 },
    Unhealthy,
    BindPathMissing { path: String },
    BindPermission { path: String },
    DiskFull,
    EngineSocketMissing,
    NetworkNotFound { name: String },
    NameInUse { name: String },
    EnvVarMissing { var: String },
    DependencyFailed { service: String },
    BuildFailed { step: Option<String> },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Subject {
    Container(ContainerId),
    Project(ProjectId),
    Engine(EngineId),
    Image(String),
    Volume(String),
    Network(String),
    Plugin(String),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    pub id: ProblemId,
    pub kind: ProblemKind,
    pub severity: Severity,
    pub subject: Subject,
    pub title: String,
    pub explanation: String,
    pub evidence: Vec<String>,
    pub fixes: Vec<Fix>,
    pub first_seen: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixSource {
    Rule,
    Harness(HarnessId),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    pub label: String,
    pub source: FixSource,
    pub effect: FixEffect,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FixEffect {
    EditFiles(GenerationPlan),
    RunAction {
        action: EngineAction,
        effect: EffectSummary,
    },
    Explain,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ColimaOp {
    Start,
    Stop,
    Restart,
    Apply(ColimaStartOptions),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EngineAction {
    Container {
        engine: EngineId,
        id: ContainerId,
        action: ContainerAction,
    },
    Compose {
        target: ComposeTarget,
        command: ComposeCommand,
    },
    Colima {
        profile: String,
        op: ColimaOp,
    },
    PullImage {
        engine: EngineId,
        reference: String,
        platform: Option<String>,
    },
    Prune {
        plan: PrunePlan,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_serde_round_trip() {
        let kind = ProblemKind::PortAllocated { port: 5432 };
        let subject = Subject::Container(ContainerId("abc".to_string()));
        let p = Problem {
            id: ProblemId::new(&kind, &subject),
            kind,
            severity: Severity::Error,
            subject,
            title: "Port 5432 is already in use".to_string(),
            explanation: "Another process holds it.".to_string(),
            evidence: vec!["bind: address already in use".to_string()],
            fixes: vec![Fix {
                label: "Use a free port".to_string(),
                source: FixSource::Rule,
                effect: FixEffect::Explain,
            }],
            first_seen: DateTime::from_timestamp(1_700_000_000, 0).expect("ts"),
        };
        let json = serde_json::to_string(&p).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Problem>(&json).expect("deserialize"),
            p
        );
    }

    #[test]
    fn problem_kind_has_15_variants_representable() {
        let kinds = [
            ProblemKind::PortAllocated { port: 1 },
            ProblemKind::ImageNotFound {
                reference: "x".to_string(),
            },
            ProblemKind::ExecFormat { image_arch: None },
            ProblemKind::OutOfMemory,
            ProblemKind::ExitedNoLogs { code: 1 },
            ProblemKind::Unhealthy,
            ProblemKind::BindPathMissing {
                path: "x".to_string(),
            },
            ProblemKind::BindPermission {
                path: "x".to_string(),
            },
            ProblemKind::DiskFull,
            ProblemKind::EngineSocketMissing,
            ProblemKind::NetworkNotFound {
                name: "x".to_string(),
            },
            ProblemKind::NameInUse {
                name: "x".to_string(),
            },
            ProblemKind::EnvVarMissing {
                var: "X".to_string(),
            },
            ProblemKind::DependencyFailed {
                service: "db".to_string(),
            },
            ProblemKind::BuildFailed { step: None },
        ];
        assert_eq!(kinds.len(), 15);
    }
}
