use serde::{Deserialize, Serialize};

use crate::events::Stream;
use crate::ids::EngineId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeTarget {
    pub engine: EngineId,
    pub docker_host: String,
    pub project_name: String,
    pub folder: std::path::PathBuf,
    pub files: Vec<std::path::PathBuf>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComposeCommand {
    Up { services: Vec<String>, build: bool },
    Down { volumes: bool, remove_images: bool },
    Start { services: Vec<String> },
    Stop { services: Vec<String> },
    Restart { services: Vec<String> },
    Build { services: Vec<String>, pull: bool },
    Pull { services: Vec<String> },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComposeEvent {
    Progress {
        id: String,
        status: String,
        text: Option<String>,
    },
    Line {
        stream: Stream,
        text: String,
    },
    Done {
        code: i32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_command_round_trip() {
        let c = ComposeCommand::Up {
            services: vec!["db".to_string()],
            build: true,
        };
        let json = serde_json::to_string(&c).expect("serialize");
        assert_eq!(
            serde_json::from_str::<ComposeCommand>(&json).expect("deserialize"),
            c
        );
    }
}
