use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistryStatus {
    LoggedIn,
    LoggedOut,
    Error { reason: String },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    pub id: String,
    pub name: String,
    pub url: Url,
    pub username: Option<String>,
    pub status: RegistryStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_round_trip() {
        let r = Registry {
            id: "docker-hub".to_string(),
            name: "Docker Hub".to_string(),
            url: Url::parse("https://hub.docker.com").expect("url"),
            username: None,
            status: RegistryStatus::LoggedOut,
        };
        let json = serde_json::to_string(&r).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Registry>(&json).expect("deserialize"),
            r
        );
    }
}
