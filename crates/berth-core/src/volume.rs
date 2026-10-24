use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ContainerId, ProjectId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub size: Option<ByteSize>,
    pub mountpoint: String,
    pub used_by: Vec<ContainerId>,
    pub project: Option<ProjectId>,
    pub created: Option<DateTime<Utc>>,
    pub labels: std::collections::BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeSpec {
    pub name: String,
    pub driver: String,
    pub driver_opts: std::collections::BTreeMap<String, String>,
    pub labels: std::collections::BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileTarget {
    Container(ContainerId),
    Volume(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    File,
    Dir,
    Link,
    Other,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub kind: FileKind,
    pub size: Option<ByteSize>,
    pub modified: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_and_file_target_round_trip() {
        let v = Volume {
            name: "data".to_string(),
            driver: "local".to_string(),
            size: Some(ByteSize(5)),
            mountpoint: "/var/lib/docker/volumes/data".to_string(),
            used_by: vec![],
            project: None,
            created: None,
            labels: std::collections::BTreeMap::new(),
        };
        let json = serde_json::to_string(&v).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Volume>(&json).expect("deserialize"),
            v
        );
        let target = FileTarget::Volume("data".to_string());
        let json = serde_json::to_string(&target).expect("serialize");
        assert_eq!(
            serde_json::from_str::<FileTarget>(&json).expect("deserialize"),
            target
        );
    }
}
