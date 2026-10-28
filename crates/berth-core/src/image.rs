use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::engine::Arch;
use crate::ids::{ContainerId, EngineId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub id: String,
    pub repo_tags: Vec<String>,
    pub digests: Vec<String>,
    pub size: ByteSize,
    pub created: DateTime<Utc>,
    pub arch: Arch,
    pub os: String,
    pub used_by: Vec<ContainerId>,
    pub dangling: bool,
    /// The engine this image is on (AD27); the mapper fills it as for `Container`.
    pub engine: EngineId,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayer {
    pub created_by: String,
    pub size: ByteSize,
    pub created: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageDetail {
    pub image: Image,
    pub layers: Vec<ImageLayer>,
    pub env_keys: Vec<String>,
    pub exposed_ports: Vec<u16>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSearchHit {
    pub name: String,
    pub description: String,
    pub stars: u32,
    pub official: bool,
}
/// One image build (AD43): a folder context or a Dockerfile written
/// in the editor, built through the engine API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildContext {
    Folder(std::path::PathBuf),
    /// Only the Dockerfile's text; the context is the file alone.
    Inline(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildSpec {
    pub context: BuildContext,
    /// Dockerfile path inside a folder context; `None` = `Dockerfile`.
    pub dockerfile: Option<String>,
    pub tags: Vec<String>,
    pub build_args: Vec<(String, String)>,
    pub platform: Option<String>,
}

/// One line of build output, classified by the step parser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildEvent {
    /// `Step n/m : INSTRUCTION …`
    Step {
        index: u32,
        total: u32,
        text: String,
    },
    /// The failing step's error line.
    Failed { text: String },
    /// Anything else, verbatim.
    Log(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullProgress {
    pub layer: Option<String>,
    pub status: String,
    pub current: Option<u64>,
    pub total: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_detail_serde_round_trip() {
        let d = ImageDetail {
            image: Image {
                id: "sha256:abc".to_string(),
                repo_tags: vec!["postgres:17".to_string()],
                digests: vec![],
                size: ByteSize(100),
                created: DateTime::from_timestamp(10, 0).expect("ts"),
                arch: Arch::Aarch64,
                os: "linux".to_string(),
                used_by: vec![],
                dangling: false,
                engine: EngineId("colima:default".to_string()),
            },
            layers: vec![ImageLayer {
                created_by: "RUN true".to_string(),
                size: ByteSize(0),
                created: None,
            }],
            env_keys: vec!["PATH".to_string()],
            exposed_ports: vec![5432],
        };
        let json = serde_json::to_string(&d).expect("serialize");
        assert_eq!(
            serde_json::from_str::<ImageDetail>(&json).expect("deserialize"),
            d
        );
    }
}
