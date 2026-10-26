//! Plugin actions: the `ActionSpec`s views carry and the `action/invoke`
//! dispatch onto the client (P9-T4, D76).
//!
//! Every action that changes something on the instance carries a
//! [`Confirm`] naming the application, its server and its branch, so a
//! click can never deploy to the wrong place by accident (PLAN §10 risk
//! row). Action params are plain JSON: `{uuid, kind?, force?}`.

use berth_plugin_protocol::{ActionOption, ActionResult, ActionSpec, Confirm};
use tokio_util::sync::CancellationToken;

use crate::client::{ClientError, CoolifyClient};
use crate::logs::is_final_status;
use crate::types::{Application, Deployment, ResourceKind};

pub const ACTION_DEPLOY: &str = "deploy";
pub const ACTION_START: &str = "start";
pub const ACTION_STOP: &str = "stop";
pub const ACTION_RESTART: &str = "restart";
pub const ACTION_CANCEL_DEPLOYMENT: &str = "cancel-deployment";

/// The `force` option key on the Deploy action ("Force rebuild").
pub const OPTION_FORCE_REBUILD: &str = "force";

fn confirm_lines(app_name: &str, server_name: Option<&str>, branch: Option<&str>) -> Vec<String> {
    let mut lines = vec![format!("Application: {app_name}")];
    if let Some(server) = server_name {
        lines.push(format!("Server: {server}"));
    }
    if let Some(branch) = branch {
        lines.push(format!("Branch: {branch}"));
    }
    lines
}

fn spec(
    id: &str,
    label: &str,
    danger: bool,
    params: serde_json::Value,
    confirm: Option<Confirm>,
) -> ActionSpec {
    ActionSpec {
        id: id.to_owned(),
        label: label.to_owned(),
        danger,
        params,
        confirm,
        options: Vec::new(),
    }
}

/// Deploy / Start / Stop / Restart for one application. `server_name` is
/// resolved by the caller (views know the servers list); the confirm still
/// names app and branch without it.
#[must_use]
pub fn application_actions(
    application: &Application,
    server_name: Option<&str>,
) -> Vec<ActionSpec> {
    let name = application.name.as_deref().unwrap_or("application");
    let uuid = application.uuid.clone().unwrap_or_default();
    let lines = confirm_lines(name, server_name, application.git_branch.as_deref());
    let params = serde_json::json!({ "uuid": uuid, "kind": "application" });

    let mut deploy = spec(
        ACTION_DEPLOY,
        "Deploy",
        false,
        params.clone(),
        Some(Confirm {
            title: format!("Deploy {name}?"),
            lines: lines.clone(),
            button: "Deploy".to_owned(),
        }),
    );
    deploy.options.push(ActionOption {
        key: OPTION_FORCE_REBUILD.to_owned(),
        label: "Force rebuild".to_owned(),
        default: false,
    });

    vec![
        deploy,
        spec(ACTION_START, "Start", false, params.clone(), None),
        spec(
            ACTION_STOP,
            "Stop",
            true,
            params.clone(),
            Some(Confirm {
                title: format!("Stop {name}?"),
                lines: lines.clone(),
                button: "Stop".to_owned(),
            }),
        ),
        spec(
            ACTION_RESTART,
            "Restart",
            false,
            params,
            Some(Confirm {
                title: format!("Restart {name}?"),
                lines,
                button: "Restart".to_owned(),
            }),
        ),
    ]
}

/// Start / Stop / Restart for a service or database row (no Deploy:
/// Coolify deploys applications).
#[must_use]
pub fn resource_actions(
    kind: ResourceKind,
    name: &str,
    uuid: &str,
    server_name: Option<&str>,
) -> Vec<ActionSpec> {
    let kind_word = match kind {
        ResourceKind::Application => "application",
        ResourceKind::Service => "service",
        ResourceKind::Database => "database",
    };
    let params = serde_json::json!({ "uuid": uuid, "kind": kind_word });
    let lines = confirm_lines(name, server_name, None);
    vec![
        spec(ACTION_START, "Start", false, params.clone(), None),
        spec(
            ACTION_STOP,
            "Stop",
            true,
            params.clone(),
            Some(Confirm {
                title: format!("Stop {name}?"),
                lines: lines.clone(),
                button: "Stop".to_owned(),
            }),
        ),
        spec(
            ACTION_RESTART,
            "Restart",
            false,
            params,
            Some(Confirm {
                title: format!("Restart {name}?"),
                lines,
                button: "Restart".to_owned(),
            }),
        ),
    ]
}

/// Cancel for a deployment that has not reached a final status yet.
#[must_use]
pub fn deployment_actions(deployment: &Deployment) -> Vec<ActionSpec> {
    if is_final_status(deployment.status.as_deref()) {
        return Vec::new();
    }
    let uuid = deployment.deployment_uuid.clone().unwrap_or_default();
    let app = deployment
        .application_name
        .as_deref()
        .unwrap_or("application");
    vec![spec(
        ACTION_CANCEL_DEPLOYMENT,
        "Cancel deployment",
        true,
        serde_json::json!({ "uuid": uuid }),
        Some(Confirm {
            title: format!("Cancel the deployment of {app}?"),
            lines: confirm_lines(app, deployment.server_name.as_deref(), None),
            button: "Cancel deployment".to_owned(),
        }),
    )]
}

fn param_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, ClientError> {
    params
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ClientError::Decode(format!("action is missing its {key}")))
}

fn param_kind(params: &serde_json::Value) -> Result<ResourceKind, ClientError> {
    let kind = param_str(params, "kind")?;
    ResourceKind::parse(kind)
        .ok_or_else(|| ClientError::Decode(format!("unknown resource kind {kind}")))
}

/// Runs one `action/invoke` against the client. The host has already
/// shown the action's [`Confirm`]; this trusts the params the spec built.
pub async fn invoke(
    client: &CoolifyClient,
    action: &str,
    params: &serde_json::Value,
    cancel: &CancellationToken,
) -> Result<ActionResult, ClientError> {
    match action {
        ACTION_DEPLOY => {
            let uuid = param_str(params, "uuid")?;
            let force = params
                .get(OPTION_FORCE_REBUILD)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let accepted = client.deploy(uuid, force, cancel).await?;
            let open = accepted
                .deployments
                .first()
                .and_then(|entry| entry.deployment_uuid.as_ref())
                .map(|deployment| format!("deployment/{deployment}/logs"));
            Ok(ActionResult {
                message: "Deployment queued.".to_owned(),
                open,
                invalidate: vec!["deployments".to_owned(), "resources".to_owned()],
            })
        }
        ACTION_START | ACTION_STOP | ACTION_RESTART => {
            let uuid = param_str(params, "uuid")?;
            let kind = param_kind(params)?;
            let ack = match action {
                ACTION_START => client.start(kind, uuid, cancel).await?,
                ACTION_STOP => client.stop(kind, uuid, cancel).await?,
                _ => client.restart(kind, uuid, cancel).await?,
            };
            Ok(ActionResult {
                message: ack.message.unwrap_or_else(|| match action {
                    ACTION_START => "Started.".to_owned(),
                    ACTION_STOP => "Stopped.".to_owned(),
                    _ => "Restarted.".to_owned(),
                }),
                open: None,
                invalidate: vec!["resources".to_owned()],
            })
        }
        ACTION_CANCEL_DEPLOYMENT => {
            let uuid = param_str(params, "uuid")?;
            let ack = client.cancel_deployment(uuid, cancel).await?;
            Ok(ActionResult {
                message: ack
                    .message
                    .unwrap_or_else(|| "Deployment cancelled.".to_owned()),
                open: None,
                invalidate: vec!["deployments".to_owned()],
            })
        }
        other => Err(ClientError::Decode(format!("unknown action {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use secrecy::SecretString;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(5);

    fn client(server: &MockServer) -> CoolifyClient {
        CoolifyClient::new(
            &server.uri(),
            SecretString::from("test-token-not-real".to_owned()),
        )
        .expect("base builds")
    }

    fn landing() -> Application {
        serde_json::from_str(include_str!("../../../fixtures/coolify/application.json"))
            .expect("fixture decodes")
    }

    fn ack(message: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .insert_header("content-type", "application/json")
            .set_body_string(format!(r#"{{"message":"{message}"}}"#))
    }

    #[test]
    fn actions_deploy_confirm_names_app_server_and_branch() {
        let specs = application_actions(&landing(), Some("hetzner-1"));
        let deploy = &specs[0];
        assert_eq!(deploy.id, ACTION_DEPLOY);
        let confirm = deploy.confirm.as_ref().expect("deploy confirms");
        let joined = confirm.lines.join("\n");
        assert!(joined.contains("landing"), "{joined}");
        assert!(joined.contains("hetzner-1"), "{joined}");
        assert!(joined.contains("main"), "{joined}");
        assert_eq!(deploy.options.len(), 1);
        assert_eq!(deploy.options[0].key, OPTION_FORCE_REBUILD);
        assert_eq!(deploy.options[0].label, "Force rebuild");
        assert!(!deploy.options[0].default);
    }

    #[test]
    fn actions_deployment_cancel_only_before_a_final_status() {
        let running: Deployment =
            serde_json::from_str(r#"{"deployment_uuid":"d1","status":"in_progress","application_name":"landing","server_name":"hetzner-1"}"#)
                .expect("decodes");
        let specs = deployment_actions(&running);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].id, ACTION_CANCEL_DEPLOYMENT);

        let done: Deployment =
            serde_json::from_str(r#"{"deployment_uuid":"d2","status":"finished"}"#)
                .expect("decodes");
        assert!(deployment_actions(&done).is_empty());
    }

    #[tokio::test]
    async fn actions_deploy_with_force_sends_force_true() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/deploy"))
            .and(query_param("uuid", "app-uuid-0001"))
            .and(query_param("force", "true"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(
                        r#"{"deployments":[{"message":"queued","resource_uuid":"app-uuid-0001","deployment_uuid":"dep-uuid-0009"}]}"#,
                    ),
            )
            .mount(&server)
            .await;
        let result = tokio::time::timeout(
            TIMEOUT,
            invoke(
                &client(&server),
                ACTION_DEPLOY,
                &serde_json::json!({ "uuid": "app-uuid-0001", "force": true }),
                &CancellationToken::new(),
            ),
        )
        .await
        .expect("within timeout")
        .expect("deploy ok");
        assert_eq!(
            result.open.as_deref(),
            Some("deployment/dep-uuid-0009/logs")
        );
    }

    #[tokio::test]
    async fn actions_deploy_without_force_omits_it() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/deploy"))
            .and(query_param("uuid", "app-uuid-0001"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(r#"{"deployments":[]}"#),
            )
            .mount(&server)
            .await;
        // If force=true were sent, nothing else would match either — so
        // also assert on the recorded request.
        invoke(
            &client(&server),
            ACTION_DEPLOY,
            &serde_json::json!({ "uuid": "app-uuid-0001" }),
            &CancellationToken::new(),
        )
        .await
        .expect("deploy ok");
        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1);
        assert!(
            !requests[0].url.query_pairs().any(|(k, _)| k == "force"),
            "{}",
            requests[0].url
        );
    }

    #[tokio::test]
    async fn actions_lifecycle_paths_per_kind() {
        let server = MockServer::start().await;
        for (segment, uuid) in [
            ("applications", "app-1"),
            ("services", "svc-1"),
            ("databases", "db-1"),
        ] {
            Mock::given(method("POST"))
                .and(path(format!("/api/v1/{segment}/{uuid}/restart")))
                .respond_with(ack("restarting"))
                .mount(&server)
                .await;
        }
        let c = client(&server);
        for (kind, uuid) in [
            ("application", "app-1"),
            ("service", "svc-1"),
            ("database", "db-1"),
        ] {
            let result = tokio::time::timeout(
                TIMEOUT,
                invoke(
                    &c,
                    ACTION_RESTART,
                    &serde_json::json!({ "uuid": uuid, "kind": kind }),
                    &CancellationToken::new(),
                ),
            )
            .await
            .expect("within timeout")
            .expect("restart ok");
            assert_eq!(result.message, "restarting");
        }
    }

    #[tokio::test]
    async fn actions_start_and_stop_hit_their_verbs() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/applications/app-1/start"))
            .respond_with(ack("starting"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/applications/app-1/stop"))
            .respond_with(ack("stopping"))
            .mount(&server)
            .await;
        let c = client(&server);
        let params = serde_json::json!({ "uuid": "app-1", "kind": "application" });
        let started = invoke(&c, ACTION_START, &params, &CancellationToken::new())
            .await
            .expect("start ok");
        assert_eq!(started.message, "starting");
        let stopped = invoke(&c, ACTION_STOP, &params, &CancellationToken::new())
            .await
            .expect("stop ok");
        assert_eq!(stopped.message, "stopping");
    }

    #[tokio::test]
    async fn actions_cancel_deployment_posts_cancel() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/deployments/dep-1/cancel"))
            .respond_with(ack("cancelled"))
            .mount(&server)
            .await;
        let result = invoke(
            &client(&server),
            ACTION_CANCEL_DEPLOYMENT,
            &serde_json::json!({ "uuid": "dep-1" }),
            &CancellationToken::new(),
        )
        .await
        .expect("cancel ok");
        assert_eq!(result.message, "cancelled");
    }

    #[tokio::test]
    async fn actions_unknown_action_and_missing_uuid_fail() {
        let server = MockServer::start().await;
        let c = client(&server);
        let unknown = invoke(
            &c,
            "explode",
            &serde_json::json!({ "uuid": "app-1" }),
            &CancellationToken::new(),
        )
        .await
        .expect_err("unknown action fails");
        assert!(unknown.to_string().contains("unknown action"), "{unknown}");
        let missing = invoke(
            &c,
            ACTION_DEPLOY,
            &serde_json::json!({}),
            &CancellationToken::new(),
        )
        .await
        .expect_err("missing uuid fails");
        assert!(missing.to_string().contains("uuid"), "{missing}");
    }
}
