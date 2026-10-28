//! Hand-written `reqwest` client for the Coolify read endpoints (D46).
//!
//! The API credential is supplied by the caller as a [`SecretString`] — in
//! the running plugin it comes from `host/secret.get` (wired with the
//! plugin binary in P9-T2/T5). It is only ever exposed into the
//! `Authorization` header of a single request; `Debug` and every error
//! message are token-free.

use std::fmt;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

use crate::types::{
    ActionAck, AppLogs, Application, DeployResponse, Deployment, EnvVar, EnvVarWithValue, Resource,
    ResourceKind, RevealedEnvVar, Server, VersionInfo,
};

/// The Coolify major this client is built against (spec tag v4.5.0).
pub const SUPPORTED_MAJOR: u64 = 4;
/// Shown when the instance runs a newer major (TASKS P9-T3 wording).
pub const NEWER_MAJOR_WARNING: &str = "Built for Coolify 4.5; some actions may fail";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The instance could not be built a URL for, or TLS setup failed.
    #[error("could not use that Coolify address: {0}")]
    BadBase(String),
    /// Nothing answered: connect failure or a timeout.
    #[error("Coolify is unreachable: {0}")]
    Unreachable(String),
    /// 401/403 — the stored token was rejected.
    #[error("token invalid")]
    TokenInvalid,
    #[error("not found on this Coolify instance")]
    NotFound,
    #[error("Coolify answered {status}: {message}")]
    Http { status: u16, message: String },
    #[error("could not read Coolify's answer: {0}")]
    Decode(String),
    #[error("cancelled")]
    Cancelled,
}

pub struct CoolifyClient {
    http: reqwest::Client,
    /// Normalized API root, e.g. `https://coolify.example.test/api/v1`.
    base: String,
    token: SecretString,
}

impl fmt::Debug for CoolifyClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoolifyClient")
            .field("base", &self.base)
            .field("token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl CoolifyClient {
    /// `base_url` is the instance address (`https://coolify.example.test`,
    /// with or without the `/api/v1` suffix); `token` is the API bearer
    /// credential from `host/secret.get`.
    pub fn new(base_url: &str, token: SecretString) -> Result<Self, ClientError> {
        // reqwest is built with `rustls-no-provider` (D48); the plugin
        // binary installs ring at startup, tests do it here on first use.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let base = normalize_base(base_url)?;
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("berth-coolify/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ClientError::BadBase(e.to_string()))?;
        Ok(Self { http, base, token })
    }

    /// The normalized API root (`…/api/v1`); the MCP payload builder
    /// derives the instance root from it.
    #[must_use]
    pub fn api_base(&self) -> &str {
        &self.base
    }

    /// `GET /health` — plain-text `OK`.
    pub async fn health(&self, cancel: &CancellationToken) -> Result<bool, ClientError> {
        let text = self.get_text("health", &[], cancel).await?;
        Ok(text.trim() == "OK")
    }

    /// `GET /version` — plain text such as `v4.5.0`. A newer major sets
    /// the warning (D46: reads `/version` on connect and warns).
    pub async fn version(&self, cancel: &CancellationToken) -> Result<VersionInfo, ClientError> {
        let raw = self
            .get_text("version", &[], cancel)
            .await?
            .trim()
            .to_owned();
        Ok(parse_version(&raw))
    }

    /// `GET /resources` — every application, service and database.
    pub async fn resources(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Vec<Resource>, ClientError> {
        self.get_json("resources", &[], cancel).await
    }

    /// `GET /servers`.
    pub async fn servers(&self, cancel: &CancellationToken) -> Result<Vec<Server>, ClientError> {
        self.get_json("servers", &[], cancel).await
    }

    /// `GET /deployments` — currently running deployments.
    pub async fn deployments(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Vec<Deployment>, ClientError> {
        self.get_json("deployments", &[], cancel).await
    }

    /// `GET /applications/{uuid}`.
    pub async fn application(
        &self,
        uuid: &str,
        cancel: &CancellationToken,
    ) -> Result<Application, ClientError> {
        self.get_json(&format!("applications/{uuid}"), &[], cancel)
            .await
    }

    /// `GET /deployments/applications/{uuid}` — an application's
    /// deployment history, optionally paged with `skip`/`take`.
    pub async fn application_deployments(
        &self,
        uuid: &str,
        skip: Option<u32>,
        take: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<Vec<Deployment>, ClientError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(skip) = skip {
            query.push(("skip", skip.to_string()));
        }
        if let Some(take) = take {
            query.push(("take", take.to_string()));
        }
        self.get_json(&format!("deployments/applications/{uuid}"), &query, cancel)
            .await
    }

    /// `GET /applications` — every application with its git repository,
    /// the list `project/badges` matches local remotes against.
    pub async fn applications(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Vec<Application>, ClientError> {
        self.get_json("applications", &[], cancel).await
    }

    /// `GET /applications/{uuid}/logs?lines=N`.
    pub async fn application_logs(
        &self,
        uuid: &str,
        lines: u32,
        cancel: &CancellationToken,
    ) -> Result<AppLogs, ClientError> {
        let query = [("lines", lines.to_string())];
        self.get_json(&format!("applications/{uuid}/logs"), &query, cancel)
            .await
    }

    /// `GET /applications/{uuid}/envs` — keys and flags only; the values
    /// are dropped during decode (see [`EnvVar`]).
    pub async fn envs(
        &self,
        uuid: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<EnvVar>, ClientError> {
        self.get_json(&format!("applications/{uuid}/envs"), &[], cancel)
            .await
    }

    /// The one-click reveal: same endpoint, values kept in
    /// [`SecretString`]s. Nothing else in this client returns a value.
    pub async fn envs_revealed(
        &self,
        uuid: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<RevealedEnvVar>, ClientError> {
        let rows: Vec<EnvVarWithValue> = self
            .get_json(&format!("applications/{uuid}/envs"), &[], cancel)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| RevealedEnvVar {
                key: row.key,
                value: row.value,
            })
            .collect())
    }

    /// `GET /deployments/{uuid}` — one deployment, for log polling.
    pub async fn deployment(
        &self,
        uuid: &str,
        cancel: &CancellationToken,
    ) -> Result<Deployment, ClientError> {
        self.get_json(&format!("deployments/{uuid}"), &[], cancel)
            .await
    }

    /// `POST /deploy?uuid=…` — queue a deployment; `force=true` rebuilds
    /// without the build cache (skipped from the query when false).
    pub async fn deploy(
        &self,
        uuid: &str,
        force: bool,
        cancel: &CancellationToken,
    ) -> Result<DeployResponse, ClientError> {
        let mut query = vec![("uuid", uuid.to_owned())];
        if force {
            query.push(("force", "true".to_owned()));
        }
        self.post_json("deploy", &query, cancel).await
    }

    /// `POST /{applications|services|databases}/{uuid}/start`.
    pub async fn start(
        &self,
        kind: ResourceKind,
        uuid: &str,
        cancel: &CancellationToken,
    ) -> Result<ActionAck, ClientError> {
        self.lifecycle(kind, uuid, "start", cancel).await
    }

    /// `POST /{applications|services|databases}/{uuid}/stop`.
    pub async fn stop(
        &self,
        kind: ResourceKind,
        uuid: &str,
        cancel: &CancellationToken,
    ) -> Result<ActionAck, ClientError> {
        self.lifecycle(kind, uuid, "stop", cancel).await
    }

    /// `POST /{applications|services|databases}/{uuid}/restart`.
    pub async fn restart(
        &self,
        kind: ResourceKind,
        uuid: &str,
        cancel: &CancellationToken,
    ) -> Result<ActionAck, ClientError> {
        self.lifecycle(kind, uuid, "restart", cancel).await
    }

    /// `POST /deployments/{uuid}/cancel`.
    pub async fn cancel_deployment(
        &self,
        uuid: &str,
        cancel: &CancellationToken,
    ) -> Result<ActionAck, ClientError> {
        self.post_json(&format!("deployments/{uuid}/cancel"), &[], cancel)
            .await
    }

    async fn lifecycle(
        &self,
        kind: ResourceKind,
        uuid: &str,
        verb: &str,
        cancel: &CancellationToken,
    ) -> Result<ActionAck, ClientError> {
        self.post_json(
            &format!("{}/{uuid}/{verb}", kind.path_segment()),
            &[],
            cancel,
        )
        .await
    }

    async fn post_json<T: DeserializeOwned + Default>(
        &self,
        path: &str,
        query: &[(&str, String)],
        cancel: &CancellationToken,
    ) -> Result<T, ClientError> {
        let bytes = self
            .request_bytes(reqwest::Method::POST, path, query, cancel)
            .await?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(T::default());
        }
        serde_json::from_slice(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        cancel: &CancellationToken,
    ) -> Result<T, ClientError> {
        let bytes = self
            .request_bytes(reqwest::Method::GET, path, query, cancel)
            .await?;
        serde_json::from_slice(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
    }

    async fn get_text(
        &self,
        path: &str,
        query: &[(&str, String)],
        cancel: &CancellationToken,
    ) -> Result<String, ClientError> {
        let bytes = self
            .request_bytes(reqwest::Method::GET, path, query, cancel)
            .await?;
        String::from_utf8(bytes).map_err(|e| ClientError::Decode(e.to_string()))
    }

    async fn request_bytes(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ClientError> {
        let url = format!("{}/{path}", self.base);
        let send = self
            .http
            .request(method, &url)
            .query(query)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.token.expose_secret()),
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .send();
        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ClientError::Cancelled),
            result = send => result.map_err(map_send_error)?,
        };
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ClientError::TokenInvalid);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ClientError::NotFound);
        }
        if !status.is_success() {
            // Coolify errors are small JSON or text bodies; keep the text
            // (never a request echo, so no token can ride along).
            let message = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(ClientError::Cancelled),
                text = response.text() => text.unwrap_or_default(),
            };
            let message: String = message.trim().chars().take(200).collect();
            return Err(ClientError::Http {
                status: status.as_u16(),
                message,
            });
        }
        let body = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ClientError::Cancelled),
            bytes = response.bytes() => bytes.map_err(map_send_error)?,
        };
        Ok(body.to_vec())
    }
}

fn normalize_base(base_url: &str) -> Result<String, ClientError> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(ClientError::BadBase("empty address".to_owned()));
    }
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| ClientError::BadBase(e.to_string()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(ClientError::BadBase(format!(
            "unsupported scheme {}",
            parsed.scheme()
        )));
    }
    if trimmed.ends_with("/api/v1") {
        Ok(trimmed.to_owned())
    } else {
        Ok(format!("{trimmed}/api/v1"))
    }
}

fn parse_version(raw: &str) -> VersionInfo {
    let major = semver::Version::parse(raw.trim().trim_start_matches('v'))
        .ok()
        .map(|v| v.major)
        .or_else(|| {
            raw.trim()
                .trim_start_matches('v')
                .split('.')
                .next()
                .and_then(|head| head.parse::<u64>().ok())
        });
    let warning = match major {
        Some(major) if major > SUPPORTED_MAJOR => Some(NEWER_MAJOR_WARNING.to_owned()),
        _ => None,
    };
    VersionInfo {
        raw: raw.to_owned(),
        major,
        warning,
    }
}

fn map_send_error(error: reqwest::Error) -> ClientError {
    if error.is_connect() || error.is_timeout() {
        ClientError::Unreachable(error.to_string())
    } else if error.is_decode() {
        ClientError::Decode(error.to_string())
    } else {
        ClientError::Unreachable(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{bearer_token, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TOKEN: &str = "test-token-not-real";
    const TIMEOUT: Duration = Duration::from_secs(5);

    fn fixture(name: &str) -> &'static str {
        match name {
            "servers" => include_str!("../../../fixtures/coolify/servers.json"),
            "resources" => include_str!("../../../fixtures/coolify/resources.json"),
            "deployments" => include_str!("../../../fixtures/coolify/deployments.json"),
            "application" => include_str!("../../../fixtures/coolify/application.json"),
            "app-deployments" => {
                include_str!("../../../fixtures/coolify/app-deployments.json")
            }
            "logs" => include_str!("../../../fixtures/coolify/logs.json"),
            "envs" => include_str!("../../../fixtures/coolify/envs.json"),
            _ => "",
        }
    }

    fn json(name: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .insert_header("content-type", "application/json")
            .set_body_string(fixture(name))
    }

    async fn server() -> MockServer {
        MockServer::start().await
    }

    fn client(server: &MockServer) -> CoolifyClient {
        CoolifyClient::new(&server.uri(), SecretString::from(TOKEN.to_owned()))
            .expect("wiremock base builds")
    }

    async fn mount(server: &MockServer, mock: Mock) {
        mock.mount(server).await;
    }

    #[tokio::test]
    async fn client_health_ok_maps_to_true() {
        let server = server().await;
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path("/api/v1/health"))
                .and(bearer_token(TOKEN))
                .respond_with(ResponseTemplate::new(200).set_body_string("OK")),
        )
        .await;
        let got = tokio::time::timeout(TIMEOUT, client(&server).health(&CancellationToken::new()))
            .await
            .expect("within timeout")
            .expect("health ok");
        assert!(got);
    }

    #[tokio::test]
    async fn client_health_other_body_maps_to_false() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/health"))
                .respond_with(ResponseTemplate::new(200).set_body_string("degraded")),
        )
        .await;
        let got = client(&server)
            .health(&CancellationToken::new())
            .await
            .expect("health call ok");
        assert!(!got);
    }

    #[tokio::test]
    async fn client_version_4x_has_no_warning() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/version"))
                .respond_with(ResponseTemplate::new(200).set_body_string("v4.5.0")),
        )
        .await;
        let got = client(&server)
            .version(&CancellationToken::new())
            .await
            .expect("version ok");
        assert_eq!(got.raw, "v4.5.0");
        assert_eq!(got.major, Some(4));
        assert_eq!(got.warning, None);
    }

    #[tokio::test]
    async fn client_version_5x_warns_built_for_4_5() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/version"))
                .respond_with(ResponseTemplate::new(200).set_body_string("5.0.0")),
        )
        .await;
        let got = client(&server)
            .version(&CancellationToken::new())
            .await
            .expect("version ok");
        assert_eq!(got.major, Some(5));
        assert_eq!(
            got.warning.as_deref(),
            Some("Built for Coolify 4.5; some actions may fail")
        );
    }

    #[tokio::test]
    async fn client_applications_map_with_git_repository() {
        let server = server().await;
        let body = format!("[{}]", fixture("application"));
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path("/api/v1/applications"))
                .and(bearer_token(TOKEN))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .set_body_string(body),
                ),
        )
        .await;
        let apps = client(&server)
            .applications(&CancellationToken::new())
            .await
            .expect("applications ok");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].git_repository.as_deref(), Some("example/landing"));
    }

    #[tokio::test]
    async fn client_version_unparseable_has_no_major_or_warning() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/version"))
                .respond_with(ResponseTemplate::new(200).set_body_string("nightly")),
        )
        .await;
        let got = client(&server)
            .version(&CancellationToken::new())
            .await
            .expect("version ok");
        assert_eq!(got.major, None);
        assert_eq!(got.warning, None);
    }

    #[tokio::test]
    async fn client_resources_map() {
        let server = server().await;
        mount(
            &server,
            Mock::given(method("GET"))
                .and(path("/api/v1/resources"))
                .and(bearer_token(TOKEN))
                .respond_with(json("resources")),
        )
        .await;
        let got = tokio::time::timeout(
            TIMEOUT,
            client(&server).resources(&CancellationToken::new()),
        )
        .await
        .expect("within timeout")
        .expect("resources ok");
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].name.as_deref(), Some("landing"));
        assert_eq!(got[0].kind.as_deref(), Some("application"));
        assert_eq!(got[2].status.as_deref(), Some("exited"));
    }

    #[tokio::test]
    async fn client_servers_map_settings() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/servers")).respond_with(json("servers")),
        )
        .await;
        let got = client(&server)
            .servers(&CancellationToken::new())
            .await
            .expect("servers ok");
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0].settings.as_ref().and_then(|s| s.is_reachable),
            Some(true)
        );
        assert_eq!(got[1].name.as_deref(), Some("hetzner-1"));
    }

    #[tokio::test]
    async fn client_deployments_map() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/deployments")).respond_with(json("deployments")),
        )
        .await;
        let got = client(&server)
            .deployments(&CancellationToken::new())
            .await
            .expect("deployments ok");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].deployment_uuid.as_deref(), Some("dep-uuid-0001"));
        assert_eq!(got[0].application_name.as_deref(), Some("landing"));
        assert_eq!(got[0].server_name.as_deref(), Some("hetzner-1"));
    }

    #[tokio::test]
    async fn client_application_maps_git_fields() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/applications/app-uuid-0001"))
                .respond_with(json("application")),
        )
        .await;
        let got = client(&server)
            .application("app-uuid-0001", &CancellationToken::new())
            .await
            .expect("application ok");
        assert_eq!(got.name.as_deref(), Some("landing"));
        assert_eq!(got.git_repository.as_deref(), Some("example/landing"));
        assert_eq!(got.git_branch.as_deref(), Some("main"));
        assert_eq!(got.status.as_deref(), Some("running:healthy"));
    }

    #[tokio::test]
    async fn client_application_deployments_map_and_page() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/deployments/applications/app-uuid-0001"))
                .and(query_param("take", "10"))
                .respond_with(json("app-deployments")),
        )
        .await;
        let got = client(&server)
            .application_deployments("app-uuid-0001", None, Some(10), &CancellationToken::new())
            .await
            .expect("app deployments ok");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].status.as_deref(), Some("finished"));
        assert_eq!(got[1].status.as_deref(), Some("failed"));
    }

    #[tokio::test]
    async fn client_application_logs_send_lines() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/applications/app-uuid-0001/logs"))
                .and(query_param("lines", "50"))
                .respond_with(json("logs")),
        )
        .await;
        let got = client(&server)
            .application_logs("app-uuid-0001", 50, &CancellationToken::new())
            .await
            .expect("logs ok");
        assert!(got.logs.contains("listening on :3000"));
    }

    #[tokio::test]
    async fn client_envs_never_carry_values() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/applications/app-uuid-0001/envs")).respond_with(json("envs")),
        )
        .await;
        let got = client(&server)
            .envs("app-uuid-0001", &CancellationToken::new())
            .await
            .expect("envs ok");
        assert_eq!(
            got.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
            ["DATABASE_URL", "NODE_ENV"]
        );
        let debug = format!("{got:?}");
        assert!(!debug.contains("fake-value-not-a-secret"), "{debug}");
        assert!(!debug.contains("production"), "{debug}");
    }

    #[tokio::test]
    async fn client_envs_revealed_returns_secret_values() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/applications/app-uuid-0001/envs")).respond_with(json("envs")),
        )
        .await;
        let got = client(&server)
            .envs_revealed("app-uuid-0001", &CancellationToken::new())
            .await
            .expect("revealed envs ok");
        assert_eq!(got[0].key, "DATABASE_URL");
        assert_eq!(got[0].value.expose_secret(), "fake-value-not-a-secret");
        assert!(!format!("{got:?}").contains("fake-value-not-a-secret"));
    }

    #[tokio::test]
    async fn client_401_is_token_invalid() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/resources")).respond_with(ResponseTemplate::new(401)),
        )
        .await;
        let err = client(&server)
            .resources(&CancellationToken::new())
            .await
            .expect_err("401 fails");
        assert!(matches!(err, ClientError::TokenInvalid), "{err:?}");
        assert_eq!(err.to_string(), "token invalid");
    }

    #[tokio::test]
    async fn client_404_is_not_found() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/applications/nope")).respond_with(ResponseTemplate::new(404)),
        )
        .await;
        let err = client(&server)
            .application("nope", &CancellationToken::new())
            .await
            .expect_err("404 fails");
        assert!(matches!(err, ClientError::NotFound), "{err:?}");
    }

    #[tokio::test]
    async fn client_unreachable_is_reported() {
        // Nothing listens on this port; connect fails fast.
        let client = CoolifyClient::new("http://127.0.0.1:9", SecretString::from(TOKEN.to_owned()))
            .expect("base builds");
        let err = tokio::time::timeout(TIMEOUT, client.resources(&CancellationToken::new()))
            .await
            .expect("within timeout")
            .expect_err("unreachable fails");
        assert!(matches!(err, ClientError::Unreachable(_)), "{err:?}");
    }

    #[tokio::test]
    async fn client_cancelled_token_stops_the_call() {
        let server = server().await;
        mount(
            &server,
            Mock::given(path("/api/v1/resources")).respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(2))
                    .set_body_string("[]"),
            ),
        )
        .await;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = tokio::time::timeout(TIMEOUT, client(&server).resources(&cancel))
            .await
            .expect("within timeout")
            .expect_err("cancelled fails");
        assert!(matches!(err, ClientError::Cancelled), "{err:?}");
    }

    #[tokio::test]
    async fn client_debug_never_shows_the_token() {
        let server = server().await;
        let debug = format!("{:?}", client(&server));
        assert!(!debug.contains(TOKEN), "{debug}");
        assert!(debug.contains("REDACTED"), "{debug}");
    }

    #[test]
    fn client_base_normalization() {
        let with = CoolifyClient::new(
            "https://coolify.example.test/api/v1/",
            SecretString::from(TOKEN.to_owned()),
        )
        .expect("base builds");
        assert_eq!(with.base, "https://coolify.example.test/api/v1");
        let without = CoolifyClient::new(
            "https://coolify.example.test/",
            SecretString::from(TOKEN.to_owned()),
        )
        .expect("base builds");
        assert_eq!(without.base, "https://coolify.example.test/api/v1");
        let bad = CoolifyClient::new("ftp://nope", SecretString::from(TOKEN.to_owned()));
        assert!(matches!(bad, Err(ClientError::BadBase(_))));
    }
}
