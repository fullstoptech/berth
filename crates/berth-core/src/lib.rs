#![deny(clippy::pedantic)]

pub mod activity;
pub mod assistant;
pub mod compose;
pub mod container;
pub mod effect;
pub mod engine;
pub mod events;
pub mod fmt;
pub mod generation;
pub mod ids;
pub mod image;
pub mod network;
pub mod problem;
pub mod project;
pub mod registry;
pub mod status;
pub mod template;
pub mod volume;

#[cfg(test)]
mod run_settings_tests;

pub use activity::{ActivityEntry, ActivityOrigin, Outcome};
pub use assistant::{
    AssistantSession, ContextItem, Harness, HarnessTransport, Risk, SessionState, ToolCall,
    ToolStatus, Turn, TurnPart, TurnRole,
};
pub use compose::{ComposeCommand, ComposeEvent, ComposeTarget};
pub use container::{
    needs_recreate, published_ports, spec_diff, Container, ContainerAction, ContainerDetail,
    ContainerSpec, ContainerState, EnvVar, ExecRequest, ExecSession, Health, HealthCheck,
    HealthCheckResult, ImageRef, Limits, LiveStats, Mount, MountKind, PortMapping, Proto,
    RestartPolicy, SpecChange,
};
pub use effect::{
    EffectKind, EffectLine, EffectSummary, PruneItem, PruneKind, PrunePlan, PruneReport,
};
pub use engine::{
    Arch, ColimaProfile, ColimaStartOptions, DiskUsage, Engine, EngineInfo, EngineKind,
    EngineState, RemoteTransport, ResourceCounts, Runtime, TlsFiles, UsageGroup, VmResources,
    VmType,
};
pub use events::{EngineEvent, EventKind, LogLine, LogQuery, Stream};
pub use generation::{
    Confidence, Detection, DiffHunk, DiffLine, DiffLineKind, FileChange, FileDiff, FoundFile,
    FoundKind, GenerationPlan, PlanSource, ServiceGuess, ServiceKind, StackGuess,
};
pub use ids::{Actor, ContainerId, EngineId, HarnessId, OwnerId, ProblemId, ProjectId, SessionId};
pub use image::{Image, ImageDetail, ImageLayer, ImageSearchHit, PullProgress};
pub use network::{Network, NetworkSpec};
pub use problem::{
    ColimaOp, EngineAction, Fix, FixEffect, FixSource, Problem, ProblemKind, Severity, Subject,
};
pub use project::{GitInfo, PinnedFolder, Project, ProjectSource, ProjectStatus, ServiceSummary};
pub use registry::{Registry, RegistryStatus};
pub use status::{status_of, StatusKind, Tone};
pub use template::{Template, TemplateInput, TemplateInputKind, TemplateKind};
pub use volume::{FileEntry, FileKind, FileTarget, Volume, VolumeSpec};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("could not parse {what}: {value}")]
    Parse { what: String, value: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles() {}

    #[test]
    fn core_error_display_is_a_sentence() {
        let e = CoreError::Parse {
            what: "port".to_string(),
            value: "x".to_string(),
        };
        assert_eq!(e.to_string(), "could not parse port: x");
    }

    #[test]
    fn session_id_serde_round_trip() {
        let id = SessionId(uuid::Uuid::now_v7());
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(
            serde_json::from_str::<SessionId>(&json).expect("deserialize"),
            id
        );
    }

    #[test]
    fn engine_serde_round_trip() {
        let e = Engine {
            id: EngineId("colima:default".to_string()),
            name: "default".to_string(),
            kind: EngineKind::Colima {
                profile: "default".to_string(),
            },
            state: EngineState::Running,
            docker_host: Some("unix:///tmp/docker.sock".to_string()),
            docker_version: Some("29.5.2".to_string()),
            resources: None,
            counts: ResourceCounts::default(),
            owner: OwnerId::Local,
        };
        let json = serde_json::to_string(&e).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Engine>(&json).expect("deserialize"),
            e
        );
    }
}
