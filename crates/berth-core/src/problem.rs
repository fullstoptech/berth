use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::compose::{ComposeCommand, ComposeTarget};
use crate::container::{ContainerAction, ContainerSpec};
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
// RunAction carries an EngineAction, which can carry a whole spec (D18).
#[allow(clippy::large_enum_variant)]
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
// Boxing the spec would change the PLAN §6 type (D18).
#[allow(clippy::large_enum_variant)]
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
    /// Edit and recreate, and "Recreate with latest image" (`pull`) (D18).
    Recreate {
        engine: EngineId,
        id: ContainerId,
        spec: ContainerSpec,
        pull: bool,
    },
    /// Delete one image, volume or network (D25a). Containers use
    /// `ContainerAction::Remove`; build cache stays under `Prune`.
    Remove {
        engine: EngineId,
        target: ResourceRef,
        force: bool,
    },
}

/// What `EngineAction::Remove` deletes (D25a). The action id, the nouns, the
/// past tense and the pending key's subject all come from here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceRef {
    Image(String),
    Volume(String),
    Network(String),
}

impl ResourceRef {
    /// The image id, volume name or network id.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Image(id) | Self::Volume(id) | Self::Network(id) => id,
        }
    }

    /// The `ActionDef` id of deleting this: also its pending key's action
    /// and its activity entry's action.
    #[must_use]
    pub fn action_id(&self) -> &'static str {
        match self {
            Self::Image(_) => "image.delete",
            Self::Volume(_) => "volume.delete",
            Self::Network(_) => "network.delete",
        }
    }

    /// "image", "volume" or "network".
    #[must_use]
    pub fn noun_one(&self) -> &'static str {
        match self {
            Self::Image(_) => "image",
            Self::Volume(_) => "volume",
            Self::Network(_) => "network",
        }
    }

    /// "images", "volumes" or "networks".
    #[must_use]
    pub fn noun_many(&self) -> &'static str {
        match self {
            Self::Image(_) => "images",
            Self::Volume(_) => "volumes",
            Self::Network(_) => "networks",
        }
    }

    /// The subject its pending state and activity entry are about.
    #[must_use]
    pub fn subject(&self) -> Subject {
        match self {
            Self::Image(id) => Subject::Image(id.clone()),
            Self::Volume(name) => Subject::Volume(name.clone()),
            Self::Network(id) => Subject::Network(id.clone()),
        }
    }

    /// The resource a subject stands for; `None` for the subjects that are
    /// not deleted this way (containers, projects, engines, plugins).
    #[must_use]
    pub fn from_subject(subject: &Subject) -> Option<Self> {
        match subject {
            Subject::Image(id) => Some(Self::Image(id.clone())),
            Subject::Volume(name) => Some(Self::Volume(name.clone())),
            Subject::Network(id) => Some(Self::Network(id.clone())),
            _ => None,
        }
    }

    /// The summary of deleting one of these: the button reads "Delete
    /// network" and the result "Deleted 1 network" from it.
    #[must_use]
    pub fn delete_effect(&self, lines: Vec<crate::effect::EffectLine>) -> EffectSummary {
        EffectSummary {
            verb: "Delete".to_string(),
            noun_one: self.noun_one().to_string(),
            noun_many: self.noun_many().to_string(),
            count: 1,
            bytes: None,
            lines,
            typed_name: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_ref_derives_action_noun_past_tense_and_subject_per_kind() {
        let cases = [
            (
                ResourceRef::Image("sha256:abc".to_string()),
                "image.delete",
                "image",
                "images",
                Subject::Image("sha256:abc".to_string()),
            ),
            (
                ResourceRef::Volume("pgdata".to_string()),
                "volume.delete",
                "volume",
                "volumes",
                Subject::Volume("pgdata".to_string()),
            ),
            (
                ResourceRef::Network("net1".to_string()),
                "network.delete",
                "network",
                "networks",
                Subject::Network("net1".to_string()),
            ),
        ];
        for (target, action, one, many, subject) in cases {
            assert_eq!(target.action_id(), action);
            assert_eq!((target.noun_one(), target.noun_many()), (one, many));
            assert_eq!(target.subject(), subject);
            assert_eq!(ResourceRef::from_subject(&subject), Some(target.clone()));
            let effect = target.delete_effect(Vec::new());
            assert_eq!(effect.button_label(), format!("Delete 1 {one}"));
            assert_eq!(effect.result_label(), format!("Deleted 1 {one}"));
        }
        assert_eq!(ResourceRef::Volume("v".to_string()).id(), "v");
    }

    #[test]
    fn resource_ref_is_not_a_container_project_engine_or_plugin() {
        for subject in [
            Subject::Container(ContainerId("c".to_string())),
            Subject::Project(ProjectId("p".to_string())),
            Subject::Engine(EngineId("e".to_string())),
            Subject::Plugin("x".to_string()),
        ] {
            assert_eq!(ResourceRef::from_subject(&subject), None);
        }
    }

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
