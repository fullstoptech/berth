use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{Actor, EngineId, ProjectId};
use crate::problem::Subject;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityOrigin {
    Engine,
    Berth,
    Plugin(String),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Ok,
    Failed { reason: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub id: uuid::Uuid,
    pub ts: DateTime<Utc>,
    pub engine: EngineId,
    pub actor: Actor,
    pub origin: ActivityOrigin,
    pub action: String,
    pub subject: Subject,
    pub subject_name: String,
    pub project: Option<ProjectId>,
    pub outcome: Outcome,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_entry_round_trip() {
        let e = ActivityEntry {
            id: uuid::Uuid::nil(),
            ts: DateTime::from_timestamp(30, 0).expect("ts"),
            engine: EngineId("colima:default".to_string()),
            actor: Actor::Local,
            origin: ActivityOrigin::Berth,
            action: "container.restart".to_string(),
            subject: Subject::Volume("data".to_string()),
            subject_name: "data".to_string(),
            project: None,
            outcome: Outcome::Failed {
                reason: "no".to_string(),
            },
        };
        let json = serde_json::to_string(&e).expect("serialize");
        assert_eq!(
            serde_json::from_str::<ActivityEntry>(&json).expect("deserialize"),
            e
        );
    }
}
