use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::container::{ContainerState, Health, PortMapping};
use crate::ids::{ContainerId, EngineId, ProjectId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub folder: Option<std::path::PathBuf>,
    pub compose_files: Vec<std::path::PathBuf>,
    pub engine: EngineId,
    pub source: ProjectSource,
    pub services: Vec<ServiceSummary>,
    pub git: Option<GitInfo>,
    pub status: ProjectStatus,
    pub pinned: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProjectSource {
    Compose,
    Pinned,
    Git {
        repo: Url,
        branch: String,
        last_commit: Option<String>,
        poll: Option<std::time::Duration>,
        auto_redeploy: bool,
    },
    RemoteOnly,
    Inferred {
        from_bind_mount: std::path::PathBuf,
    },
    Loose,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectStatus {
    AllRunning,
    Partial { running: u16, total: u16 },
    Stopped,
    Failing,
    NotCreated,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitInfo {
    pub branch: Option<String>,
    pub dirty: bool,
    pub remote: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceSummary {
    pub name: String,
    pub image: String,
    pub replicas: u16,
    pub state: ContainerState,
    pub ports: Vec<PortMapping>,
    pub health: Option<Health>,
    pub containers: Vec<ContainerId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedFolder {
    pub folder: std::path::PathBuf,
    pub engine: EngineId,
    pub added: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Project {
        let engine = EngineId("colima:default".to_string());
        Project {
            id: ProjectId::for_folder(std::path::Path::new("/Users/me/Dev/shop"), &engine),
            name: "shop".to_string(),
            folder: Some(std::path::PathBuf::from("/Users/me/Dev/shop")),
            compose_files: vec![std::path::PathBuf::from("compose.yaml")],
            engine,
            source: ProjectSource::Compose,
            services: vec![ServiceSummary {
                name: "db".to_string(),
                image: "postgres:17".to_string(),
                replicas: 1,
                state: ContainerState::Running,
                ports: vec![],
                health: Some(Health::Healthy),
                containers: vec![ContainerId("abc".to_string())],
            }],
            git: Some(GitInfo {
                branch: Some("main".to_string()),
                dirty: false,
                remote: None,
                ahead: 0,
                behind: 0,
            }),
            status: ProjectStatus::AllRunning,
            pinned: false,
        }
    }

    #[test]
    fn project_serde_round_trip() {
        let p = sample();
        let json = serde_json::to_string(&p).expect("serialize");
        let back: Project = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, back);
    }

    #[test]
    fn git_source_serde_round_trip() {
        let mut p = sample();
        p.source = ProjectSource::Git {
            repo: Url::parse("https://github.com/fullstoptech/berth").expect("url"),
            branch: "main".to_string(),
            last_commit: Some("abc123".to_string()),
            poll: Some(std::time::Duration::from_secs(60)),
            auto_redeploy: false,
        };
        let json = serde_json::to_string(&p).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Project>(&json).expect("deserialize"),
            p
        );
    }
}
