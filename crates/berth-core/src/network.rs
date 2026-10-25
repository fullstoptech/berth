use serde::{Deserialize, Serialize};

use crate::ids::{ContainerId, EngineId, ProjectId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub subnet: Option<String>,
    pub gateway: Option<String>,
    pub internal: bool,
    pub containers: Vec<ContainerId>,
    pub project: Option<ProjectId>,
    pub built_in: bool,
    /// The engine this network is on (D27).
    pub engine: EngineId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSpec {
    pub name: String,
    pub driver: String,
    pub subnet: Option<String>,
    pub gateway: Option<String>,
    pub internal: bool,
    pub attach: Vec<ContainerId>,
    /// Labels on the new network (D25b), e.g. `dev.berth.fixture=1`.
    pub labels: std::collections::BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_serde_round_trip() {
        let n = Network {
            id: "abc".to_string(),
            name: "bridge".to_string(),
            driver: "bridge".to_string(),
            subnet: Some("172.17.0.0/16".to_string()),
            gateway: Some("172.17.0.1".to_string()),
            internal: false,
            containers: vec![],
            project: None,
            built_in: true,
            engine: EngineId("colima:default".to_string()),
        };
        let json = serde_json::to_string(&n).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Network>(&json).expect("deserialize"),
            n
        );
    }
}
