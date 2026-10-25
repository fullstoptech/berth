use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::problem::{ProblemKind, Subject};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EngineId(pub String);
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProjectId(pub String);
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContainerId(pub String);
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProblemId(pub String);
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HarnessId(pub String);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SessionId(pub uuid::Uuid);

fn first16hex(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update([0_u8]);
        }
        hasher.update(part);
    }
    hex::encode(hasher.finalize())[..16].to_string()
}

impl ContainerId {
    #[must_use]
    pub fn short(&self) -> &str {
        self.0.get(..12).unwrap_or(&self.0)
    }
}

impl ProjectId {
    #[must_use]
    pub fn for_folder(canonical_folder: &std::path::Path, engine: &EngineId) -> Self {
        Self(first16hex(&[
            canonical_folder.to_string_lossy().as_bytes(),
            engine.0.as_bytes(),
        ]))
    }

    #[must_use]
    pub fn for_remote(compose_project: &str, engine: &EngineId) -> Self {
        Self(first16hex(&[
            compose_project.as_bytes(),
            engine.0.as_bytes(),
        ]))
    }
}

impl ProblemId {
    #[must_use]
    pub fn new(kind: &ProblemKind, subject: &Subject) -> Self {
        Self(first16hex(&[
            format!("{kind:?}").as_bytes(),
            format!("{subject:?}").as_bytes(),
        ]))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Actor {
    Local,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OwnerId {
    Local,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn container_short_is_first_12() {
        let id = ContainerId("abcdef0123456789".to_string());
        assert_eq!(id.short(), "abcdef012345");
    }

    #[test]
    fn container_short_of_short_id_is_whole() {
        assert_eq!(ContainerId("abc".to_string()).short(), "abc");
    }

    #[test]
    fn project_for_folder_is_stable_and_16_hex() {
        let e = EngineId("colima:default".to_string());
        let a = ProjectId::for_folder(Path::new("/Users/me/Dev/app"), &e);
        let b = ProjectId::for_folder(Path::new("/Users/me/Dev/app"), &e);
        assert_eq!(a, b);
        assert_eq!(a.0.len(), 16);
        assert!(a.0.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn project_for_folder_differs_per_engine_and_folder() {
        let a = ProjectId::for_folder(Path::new("/x/app"), &EngineId("colima:default".to_string()));
        let b = ProjectId::for_folder(Path::new("/x/app"), &EngineId("colima:work".to_string()));
        let c = ProjectId::for_folder(
            Path::new("/x/other"),
            &EngineId("colima:default".to_string()),
        );
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn project_for_remote_is_stable_and_differs_from_folder() {
        let e = EngineId("ssh:prod".to_string());
        assert_eq!(
            ProjectId::for_remote("shop", &e),
            ProjectId::for_remote("shop", &e)
        );
        assert_ne!(
            ProjectId::for_remote("shop", &e),
            ProjectId::for_folder(Path::new("/Users/me/Dev/shop"), &e)
        );
    }

    #[test]
    fn problem_id_is_stable_per_kind_and_subject() {
        let kind = ProblemKind::PortAllocated { port: 5432 };
        let subject = Subject::Container(ContainerId("abc".to_string()));
        assert_eq!(
            ProblemId::new(&kind, &subject),
            ProblemId::new(&kind, &subject)
        );
        assert_ne!(
            ProblemId::new(&kind, &subject),
            ProblemId::new(&ProblemKind::DiskFull, &subject)
        );
    }
}
