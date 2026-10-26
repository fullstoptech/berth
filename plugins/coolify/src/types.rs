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
