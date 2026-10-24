use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ContainerId, EngineId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    Container { id: ContainerId, action: String },
    Image { id: String, action: String },
    Volume { name: String, action: String },
    Network { id: String, action: String },
    Other { ty: String, action: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineEvent {
    pub engine: EngineId,
    pub at: DateTime<Utc>,
    pub kind: EventKind,
    pub attributes: std::collections::BTreeMap<String, String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Stream {
    Stdout,
    Stderr,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub container: ContainerId,
    pub ts: DateTime<Utc>,
    pub stream: Stream,
    pub text: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogQuery {
    pub since: Option<DateTime<Utc>>,
    pub tail: Option<u32>,
    pub follow: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_and_log_line_round_trip() {
        let at = DateTime::from_timestamp(20, 0).expect("ts");
        let e = EngineEvent {
            engine: EngineId("colima:default".to_string()),
            at,
            kind: EventKind::Container {
                id: ContainerId("abc".to_string()),
                action: "die".to_string(),
            },
            attributes: std::collections::BTreeMap::new(),
        };
        let json = serde_json::to_string(&e).expect("serialize");
        assert_eq!(
            serde_json::from_str::<EngineEvent>(&json).expect("deserialize"),
            e
        );
        let line = LogLine {
            container: ContainerId("abc".to_string()),
            ts: at,
            stream: Stream::Stderr,
            text: "boom".to_string(),
        };
        let json = serde_json::to_string(&line).expect("serialize");
        assert_eq!(
            serde_json::from_str::<LogLine>(&json).expect("deserialize"),
            line
        );
    }

    #[test]
    fn log_query_defaults_to_no_follow() {
        let q = LogQuery::default();
        assert_eq!(
            q,
            LogQuery {
                since: None,
                tail: None,
                follow: false
            }
        );
    }
}
