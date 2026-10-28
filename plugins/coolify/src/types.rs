//! Response types for the Coolify endpoints Berth reads (P9-T3).
//!
//! Field names come from the component schemas in
//! `spec/openapi-v4.5.0.json` (`Server`, `Application`,
//! `ApplicationDeploymentQueue`, `EnvironmentVariable`). Every field is
//! optional and unknown fields are ignored, so a Coolify point release
//! adding or dropping a field cannot break decoding. Only the fields the
//! plugin's views and actions need are carried.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

/// What `GET /version` reports: the raw text plus the parsed shape.
/// `warning` is set on a newer major than the one this client is built for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub raw: String,
    pub major: Option<u64>,
    pub warning: Option<String>,
}

/// One entry of `GET /resources`. The spec leaves this response
/// undocumented ("Content is very complex"), so decoding is deliberately
/// tolerant: only the identity/status fields Coolify sends for every
/// resource kind (application, service, database) are read.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Resource {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub fqdn: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// One entry of `GET /servers` (spec schema `Server`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Server {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub port: Option<i64>,
    #[serde(default)]
    pub proxy_type: Option<String>,
    #[serde(default)]
    pub settings: Option<ServerSettings>,
}

/// The `settings` object nested in a `Server` (spec schema `ServerSetting`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServerSettings {
    #[serde(default)]
    pub is_reachable: Option<bool>,
    #[serde(default)]
    pub is_usable: Option<bool>,
}

/// `GET /applications/{uuid}` (spec schema `Application`, the subset the
/// plugin shows and links by; `git_repository` feeds project badges in
/// P9-T4). Secret-shaped fields the spec carries (webhook secrets, basic
/// auth password) are deliberately not decoded.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Application {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub fqdn: Option<String>,
    #[serde(default)]
    pub git_repository: Option<String>,
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub git_commit_sha: Option<String>,
    #[serde(default)]
    pub git_full_url: Option<String>,
    #[serde(default)]
    pub build_pack: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub ports_exposes: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// One entry of `GET /deployments` and
/// `GET /deployments/applications/{uuid}` (spec schema
/// `ApplicationDeploymentQueue`). The spec's schema for the per-application
/// listing points at `Application` by mistake; Coolify returns deployment
/// queue entries from both endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Deployment {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub application_id: Option<i64>,
    #[serde(default)]
    pub deployment_uuid: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub commit_message: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub is_webhook: Option<bool>,
    #[serde(default)]
    pub is_api: Option<bool>,
    #[serde(default)]
    pub force_rebuild: Option<bool>,
    #[serde(default)]
    pub application_name: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub server_id: Option<i64>,
    #[serde(default)]
    pub deployment_url: Option<String>,
    /// The deployment log so far: a JSON array of entries with an
    /// `output` field, carried as a string.
    #[serde(default)]
    pub logs: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// `POST /deploy` body: one entry per resource accepted for deployment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct DeployResponse {
    #[serde(default)]
    pub deployments: Vec<DeployEntry>,
}

/// One accepted deployment inside [`DeployResponse`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DeployEntry {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub resource_uuid: Option<String>,
    #[serde(default)]
    pub deployment_uuid: Option<String>,
}

/// The small acknowledgement body the start/stop/restart/cancel endpoints
/// return (`{"message": "…"}`); an empty body decodes as all-`None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ActionAck {
    #[serde(default)]
    pub message: Option<String>,
}

/// Which resource family a start/stop/restart acts on; each maps to its
/// own URL family in the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Application,
    Service,
    Database,
}

impl ResourceKind {
    /// The URL segment: `applications`, `services` or `databases`.
    #[must_use]
    pub fn path_segment(self) -> &'static str {
        match self {
            ResourceKind::Application => "applications",
            ResourceKind::Service => "services",
            ResourceKind::Database => "databases",
        }
    }

    /// Parses the `kind` an action's params carry ("application",
    /// "service", "database"; Coolify's `standalone-*` resource types
    /// count as databases).
    #[must_use]
    pub fn parse(kind: &str) -> Option<Self> {
        match kind {
            "application" => Some(ResourceKind::Application),
            "service" => Some(ResourceKind::Service),
            "database" => Some(ResourceKind::Database),
            other if other.starts_with("standalone-") => Some(ResourceKind::Database),
            _ => None,
        }
    }
}

/// `GET /applications/{uuid}/logs` body: `{"logs": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AppLogs {
    #[serde(default)]
    pub logs: String,
}

/// One entry of `GET /applications/{uuid}/envs` with the value dropped at
/// decode time. The spec's `EnvironmentVariable` carries `value` and
/// `real_value`; neither is read here, so an `EnvVar` can be logged,
/// printed and sent to the host without leaking a secret. Values are only
/// available through [`RevealedEnvVar`] (the client's explicit reveal API).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EnvVar {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub is_buildtime: Option<bool>,
    #[serde(default)]
    pub is_runtime: Option<bool>,
    #[serde(default)]
    pub is_preview: Option<bool>,
    #[serde(default)]
    pub is_shared: Option<bool>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// An env var with its value, only produced by the explicit reveal API.
/// The value stays in a [`SecretString`]: `Debug` prints `[REDACTED]` and
/// reading it requires an explicit `expose_secret()`.
#[derive(Debug, Clone)]
pub struct RevealedEnvVar {
    pub key: String,
    pub value: SecretString,
}

impl PartialEq for RevealedEnvVar {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.value.expose_secret() == other.value.expose_secret()
    }
}

/// Wire shape behind [`RevealedEnvVar`]: decodes `value` once, into the
/// secret, and is never stored or printed itself.
#[derive(Deserialize)]
pub(crate) struct EnvVarWithValue {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub value: SecretString,
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESOURCES: &str = include_str!("../../../fixtures/coolify/resources.json");
    const SERVERS: &str = include_str!("../../../fixtures/coolify/servers.json");
    const APPLICATION: &str = include_str!("../../../fixtures/coolify/application.json");
    const DEPLOYMENTS: &str = include_str!("../../../fixtures/coolify/deployments.json");
    const APP_DEPLOYMENTS: &str = include_str!("../../../fixtures/coolify/app-deployments.json");
    const ENVS: &str = include_str!("../../../fixtures/coolify/envs.json");
    const LOGS: &str = include_str!("../../../fixtures/coolify/logs.json");

    #[test]
    fn types_resources_fixture_decodes_with_kinds_and_statuses() {
        let resources: Vec<Resource> = serde_json::from_str(RESOURCES).expect("decodes");
        assert_eq!(resources.len(), 3);
        assert_eq!(resources[0].kind.as_deref(), Some("application"));
        assert_eq!(resources[0].uuid.as_deref(), Some("app-uuid-0001"));
        assert_eq!(resources[1].kind.as_deref(), Some("standalone-postgresql"));
        assert_eq!(resources[2].status.as_deref(), Some("exited"));
        assert_eq!(resources[2].fqdn, None);
    }

    #[test]
    fn types_servers_fixture_reads_nested_settings() {
        let servers: Vec<Server> = serde_json::from_str(SERVERS).expect("decodes");
        assert_eq!(servers[0].name.as_deref(), Some("localhost"));
        assert_eq!(servers[0].port, Some(22));
        assert_eq!(
            servers[0].description.as_deref(),
            Some("This is the Coolify host.")
        );
        assert_eq!(servers[1].description, None, "null becomes None");
        let reachable = |s: &Server| s.settings.as_ref().and_then(|x| x.is_reachable);
        assert_eq!(reachable(&servers[0]), Some(true));
        assert_eq!(reachable(&servers[1]), Some(false));
    }

    #[test]
    fn types_application_fixture_carries_the_link_fields() {
        let app: Application = serde_json::from_str(APPLICATION).expect("decodes");
        assert_eq!(app.git_repository.as_deref(), Some("example/landing"));
        assert_eq!(app.git_branch.as_deref(), Some("main"));
        assert_eq!(app.status.as_deref(), Some("running:healthy"));
        assert_eq!(app.ports_exposes.as_deref(), Some("3000"));
    }

    #[test]
    fn types_deployment_fixtures_decode_both_listings() {
        let running: Vec<Deployment> = serde_json::from_str(DEPLOYMENTS).expect("decodes");
        assert_eq!(running[0].status.as_deref(), Some("in_progress"));
        assert_eq!(running[0].server_name.as_deref(), Some("hetzner-1"));
        let history: Vec<Deployment> = serde_json::from_str(APP_DEPLOYMENTS).expect("decodes");
        let statuses: Vec<_> = history.iter().filter_map(|d| d.status.as_deref()).collect();
        assert!(statuses.contains(&"finished") && statuses.contains(&"failed"));
        let failed = history
            .iter()
            .find(|d| d.status.as_deref() == Some("failed"))
            .expect("a failed deployment");
        assert_eq!(failed.force_rebuild, Some(true));
        assert_eq!(failed.is_webhook, Some(true));
    }

    #[test]
    fn types_unknown_fields_are_ignored_and_missing_ones_default() {
        let app: Application =
            serde_json::from_str(r#"{"uuid":"u","brand_new_field":{"nested":[1,2]},"name":"x"}"#)
                .expect("a newer Coolify's extra fields do not break decoding");
        assert_eq!(app.uuid.as_deref(), Some("u"));
        assert_eq!(app.git_branch, None);
        let empty: Deployment = serde_json::from_str("{}").expect("all fields optional");
        assert_eq!(empty.status, None);
    }

    #[test]
    fn types_env_var_drops_the_value_at_decode_time() {
        let envs: Vec<EnvVar> = serde_json::from_str(ENVS).expect("decodes");
        assert_eq!(envs[0].key, "DATABASE_URL");
        assert_eq!(envs[0].is_runtime, Some(true));
        assert_eq!(envs[1].is_buildtime, Some(true));
        let printed = format!("{envs:?}");
        assert!(!printed.contains("fake-value-not-a-secret"), "{printed}");
        assert!(!printed.contains("production"), "{printed}");
    }

    #[test]
    fn types_revealed_env_keeps_the_value_secret_in_debug() {
        let rows: Vec<EnvVarWithValue> = serde_json::from_str(ENVS).expect("decodes");
        let revealed = RevealedEnvVar {
            key: rows[0].key.clone(),
            value: rows[0].value.clone(),
        };
        assert_eq!(revealed.value.expose_secret(), "fake-value-not-a-secret");
        assert!(!format!("{revealed:?}").contains("fake-value-not-a-secret"));
    }

    #[test]
    fn types_deploy_response_and_ack_decode() {
        let deploy: DeployResponse = serde_json::from_str(
            r#"{"deployments":[{"message":"queued.","resource_uuid":"app-uuid-0001","deployment_uuid":"dep-uuid-0002"}]}"#,
        )
        .expect("decodes");
        assert_eq!(
            deploy.deployments[0].deployment_uuid.as_deref(),
            Some("dep-uuid-0002")
        );
        let ack: ActionAck =
            serde_json::from_str(r#"{"message":"Restart request queued."}"#).expect("decodes");
        assert_eq!(ack.message.as_deref(), Some("Restart request queued."));
        assert_eq!(ActionAck::default().message, None);
        assert!(DeployResponse::default().deployments.is_empty());
    }

    #[test]
    fn types_app_logs_fixture_decodes_and_missing_logs_is_empty() {
        let logs: AppLogs = serde_json::from_str(LOGS).expect("decodes");
        assert_eq!(logs.logs.lines().count(), 3);
        let none: AppLogs = serde_json::from_str("{}").expect("decodes");
        assert_eq!(none.logs, "");
    }

    #[test]
    fn types_resource_kind_parse_and_path_segments() {
        assert_eq!(
            ResourceKind::parse("application"),
            Some(ResourceKind::Application)
        );
        assert_eq!(ResourceKind::parse("service"), Some(ResourceKind::Service));
        assert_eq!(
            ResourceKind::parse("database"),
            Some(ResourceKind::Database)
        );
        assert_eq!(
            ResourceKind::parse("standalone-postgresql"),
            Some(ResourceKind::Database)
        );
        assert_eq!(ResourceKind::parse("server"), None);
        assert_eq!(ResourceKind::Application.path_segment(), "applications");
        assert_eq!(ResourceKind::Service.path_segment(), "services");
        assert_eq!(ResourceKind::Database.path_segment(), "databases");
    }
}
