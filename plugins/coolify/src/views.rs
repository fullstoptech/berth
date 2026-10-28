//! Pure mappings from Coolify API types to `berth-plugin-protocol` views.
//!
//! These are the shapes `view/load` returns (routed in `plugin.rs`);
//! keeping them pure and free of I/O keeps them easy to test. Env values never reach a view: the env table shows
//! keys with a masked value cell.

use std::collections::BTreeMap;

use berth_plugin_protocol::{Badge, Cell, Column, Row, Tone, View};

use crate::actions::{
    application_actions, deployment_actions, redeploy_actions, resource_actions, Redeploy,
};
use crate::types::{Application, Deployment, EnvVar, Resource, ResourceKind, Server, VersionInfo};

fn column(key: &str, label: &str) -> Column {
    Column {
        key: key.to_owned(),
        label: label.to_owned(),
        mono: false,
        align_right: false,
    }
}

fn mono_column(key: &str, label: &str) -> Column {
    Column {
        mono: true,
        ..column(key, label)
    }
}

fn row(id: String, cells: Vec<(&str, Cell)>, open: Option<String>) -> Row {
    Row {
        id,
        cells: cells
            .into_iter()
            .map(|(key, cell)| (key.to_owned(), cell))
            .collect::<BTreeMap<_, _>>(),
        open,
        actions: Vec::new(),
        group: None,
    }
}

/// Maps a Coolify status string (`running:healthy`, `exited`,
/// `in_progress`, `failed`, …) onto a badge tone.
fn status_badge(status: Option<&str>) -> Cell {
    let status = status.unwrap_or("unknown");
    let tone = if status.starts_with("running") || status == "finished" {
        Tone::Success
    } else if status.contains("fail") || status == "exited" || status == "cancelled" {
        Tone::Danger
    } else if status == "in_progress" || status == "queued" {
        Tone::Warning
    } else {
        Tone::Neutral
    };
    Cell::Badge(Badge {
        text: humanize_status(status),
        tone,
    })
}

/// `in_progress` reads "In progress", `running:healthy` "Running (healthy)",
/// `cancelled-by-user` "Cancelled by user".
fn humanize_status(status: &str) -> String {
    let (head, qualifier) = match status.split_once(':') {
        Some((head, qualifier)) => (head, Some(qualifier)),
        None => (status, None),
    };
    let spaced = head.replace(['_', '-'], " ");
    let mut chars = spaced.chars();
    let capitalised = chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    });
    match qualifier {
        Some(qualifier) if !qualifier.is_empty() => format!("{capitalised} ({qualifier})"),
        _ => capitalised,
    }
}

/// `38 s` between two Coolify timestamps as `0 m 38 s`; empty when either
/// one is missing or unreadable.
fn duration_text(start: Option<&String>, end: Option<&String>) -> String {
    let parse =
        |value: Option<&String>| value.and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok());
    match (parse(start), parse(end)) {
        (Some(start), Some(end)) => {
            let seconds = (end - start).num_seconds().max(0);
            format!("{} m {:02} s", seconds / 60, seconds % 60)
        }
        _ => String::new(),
    }
}

/// How a deployment started: a git webhook, the API, or by hand.
fn trigger_text(deployment: &Deployment) -> &'static str {
    if deployment.is_webhook == Some(true) {
        "webhook"
    } else if deployment.is_api == Some(true) {
        "API"
    } else {
        "manual"
    }
}

fn text(value: Option<&String>) -> Cell {
    Cell::Text(value.cloned().unwrap_or_else(|| "—".to_owned()))
}

fn time(value: Option<&String>) -> Cell {
    Cell::Time(value.cloned().unwrap_or_default())
}

/// Resources table (applications, services, databases).
#[must_use]
pub fn resources_table(resources: &[Resource]) -> View {
    View::Table {
        title: "Resources".to_owned(),
        columns: vec![
            column("name", "Name"),
            column("kind", "Type"),
            column("status", "Status"),
            mono_column("fqdn", "Address"),
            column("updated", "Updated"),
        ],
        rows: resources
            .iter()
            .map(|resource| {
                let id = resource
                    .uuid
                    .clone()
                    .or_else(|| resource.id.map(|id| id.to_string()))
                    .unwrap_or_default();
                let open = resource
                    .uuid
                    .as_ref()
                    .map(|uuid| format!("application/{uuid}"));
                let mut row = row(
                    id,
                    vec![
                        ("name", text(resource.name.as_ref())),
                        ("kind", text(resource.kind.as_ref())),
                        ("status", status_badge(resource.status.as_deref())),
                        (
                            "fqdn",
                            Cell::Mono(resource.fqdn.clone().unwrap_or_default()),
                        ),
                        ("updated", time(resource.updated_at.as_ref())),
                    ],
                    open,
                );
                if let (Some(kind), Some(uuid)) = (
                    resource.kind.as_deref().and_then(ResourceKind::parse),
                    resource.uuid.as_deref(),
                ) {
                    row.actions = resource_actions(
                        kind,
                        resource.name.as_deref().unwrap_or("resource"),
                        uuid,
                        None,
                    );
                }
                row
            })
            .collect(),
        empty: "No resources on this Coolify instance.".to_owned(),
        groups: Vec::new(),
    }
}

/// Servers table; reachability comes from the nested settings.
#[must_use]
pub fn servers_table(servers: &[Server]) -> View {
    View::Table {
        title: "Servers".to_owned(),
        columns: vec![
            column("name", "Name"),
            mono_column("ip", "IP"),
            column("status", "Status"),
        ],
        rows: servers
            .iter()
            .map(|server| {
                let id = server
                    .uuid
                    .clone()
                    .or_else(|| server.id.map(|id| id.to_string()))
                    .unwrap_or_default();
                let reachable = server
                    .settings
                    .as_ref()
                    .and_then(|settings| settings.is_reachable);
                let (label, tone) = match reachable {
                    Some(true) => ("reachable", Tone::Success),
                    Some(false) => ("unreachable", Tone::Danger),
                    None => ("unknown", Tone::Neutral),
                };
                row(
                    id,
                    vec![
                        ("name", text(server.name.as_ref())),
                        ("ip", Cell::Mono(server.ip.clone().unwrap_or_default())),
                        (
                            "status",
                            Cell::Badge(Badge {
                                text: label.to_owned(),
                                tone,
                            }),
                        ),
                    ],
                    None,
                )
            })
            .collect(),
        empty: "No servers on this Coolify instance.".to_owned(),
        groups: Vec::new(),
    }
}

/// Deployments table (running now, or one application's history).
/// `applications` resolves each row's application (its uuid and branch, for
/// Redeploy, and its repository, which lets Berth link a local folder);
/// `failures` maps a failed deployment's uuid to why it failed.
#[must_use]
pub fn deployments_table(
    deployments: &[Deployment],
    applications: &[Application],
    failures: &BTreeMap<String, String>,
) -> View {
    View::Table {
        title: "Deployments".to_owned(),
        columns: vec![
            column("application", "App"),
            column("status", "Status"),
            mono_column("commit", "Commit"),
            column("message", "Message"),
            Column {
                align_right: true,
                ..column("duration", "Duration")
            },
            column("trigger", "Trigger"),
        ],
        rows: deployments
            .iter()
            .map(|deployment| {
                let id = deployment
                    .deployment_uuid
                    .clone()
                    .or_else(|| deployment.id.map(|id| id.to_string()))
                    .unwrap_or_default();
                let commit = deployment
                    .commit
                    .as_deref()
                    .map(|sha| sha.chars().take(7).collect::<String>())
                    .unwrap_or_default();
                let mut row = row(
                    id,
                    vec![
                        ("application", text(deployment.application_name.as_ref())),
                        ("status", status_badge(deployment.status.as_deref())),
                        ("commit", Cell::Mono(commit)),
                        ("message", text(deployment.commit_message.as_ref())),
                        (
                            "duration",
                            Cell::Text(duration_text(
                                deployment.created_at.as_ref(),
                                deployment.updated_at.as_ref(),
                            )),
                        ),
                        ("trigger", Cell::Text(trigger_text(deployment).to_owned())),
                        // Not a column: the pane beside the table names the server.
                        ("server", text(deployment.server_name.as_ref())),
                    ],
                    deployment
                        .deployment_uuid
                        .as_ref()
                        .map(|uuid| format!("deployment/{uuid}/logs")),
                );
                let application = applications.iter().find(|a| {
                    a.id.is_some() && a.id == deployment.application_id
                        || a.name.is_some() && a.name == deployment.application_name
                });
                if let Some(repository) = application.and_then(|a| a.git_repository.as_ref()) {
                    // Not a column: Berth links a local project by it.
                    row.cells
                        .insert("repository".to_owned(), Cell::Text(repository.clone()));
                }
                row.actions = if crate::logs::is_final_status(deployment.status.as_deref()) {
                    let failure = deployment
                        .deployment_uuid
                        .as_ref()
                        .and_then(|uuid| failures.get(uuid))
                        .map(String::as_str);
                    redeploy_actions(
                        deployment,
                        &Redeploy {
                            application,
                            failure,
                        },
                    )
                } else {
                    deployment_actions(deployment)
                };
                row
            })
            .collect(),
        empty: "No deployments.".to_owned(),
        groups: Vec::new(),
    }
}

/// Application detail: identity, git coordinates and status. Logs and
/// envs ride as tabs the host resolves through `view/load`.
#[must_use]
pub fn application_detail(application: &Application, server_name: Option<&str>) -> View {
    let name = application
        .name
        .clone()
        .unwrap_or_else(|| "Application".to_owned());
    let uuid = application.uuid.clone().unwrap_or_default();
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut push = |label: &str, value: &Option<String>| {
        if let Some(value) = value {
            fields.push((label.to_owned(), value.clone()));
        }
    };
    push("Address", &application.fqdn);
    push("Repository", &application.git_repository);
    push("Branch", &application.git_branch);
    push("Commit", &application.git_commit_sha);
    push("Build pack", &application.build_pack);
    push("Ports", &application.ports_exposes);
    push("Updated", &application.updated_at);
    View::Detail {
        title: name,
        status: match status_badge(application.status.as_deref()) {
            Cell::Badge(badge) => Some(badge),
            _ => None,
        },
        fields,
        tabs: vec![
            berth_plugin_protocol::Tab {
                id: "deployments".to_owned(),
                title: "Deployments".to_owned(),
                view: format!("deployments/{uuid}"),
            },
            berth_plugin_protocol::Tab {
                id: "logs".to_owned(),
                title: "Logs".to_owned(),
                view: format!("application/{uuid}/logs"),
            },
            berth_plugin_protocol::Tab {
                id: "env".to_owned(),
                title: "Env".to_owned(),
                view: format!("application/{uuid}/envs"),
            },
        ],
        actions: application_actions(application, server_name),
    }
}

/// Env table: keys and scope, values always masked. Reveal is a host UI
/// action backed by the client's `envs_revealed`, never a view cell.
#[must_use]
pub fn envs_table(envs: &[EnvVar]) -> View {
    View::Table {
        title: "Environment".to_owned(),
        columns: vec![
            mono_column("key", "Key"),
            mono_column("value", "Value"),
            column("scope", "Scope"),
        ],
        rows: envs
            .iter()
            .map(|env| {
                let mut scope: Vec<&str> = Vec::new();
                if env.is_buildtime == Some(true) {
                    scope.push("build");
                }
                if env.is_runtime == Some(true) {
                    scope.push("runtime");
                }
                if env.is_preview == Some(true) {
                    scope.push("preview");
                }
                row(
                    env.uuid.clone().unwrap_or_else(|| env.key.clone()),
                    vec![
                        ("key", Cell::Mono(env.key.clone())),
                        ("value", Cell::Mono("••••".to_owned())),
                        ("scope", Cell::Text(scope.join(" · "))),
                    ],
                    None,
                )
            })
            .collect(),
        empty: "No environment variables.".to_owned(),
        groups: Vec::new(),
    }
}

/// The version as a message view; the newer-major warning rides along.
#[must_use]
pub fn version_message(version: &VersionInfo) -> View {
    match &version.warning {
        Some(warning) => View::Message {
            tone: Tone::Warning,
            text: format!("Coolify {} — {warning}", version.raw),
            action: None,
        },
        None => View::Message {
            tone: Tone::Info,
            text: format!("Coolify {}", version.raw),
            action: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(name: &str, status: &str) -> Resource {
        Resource {
            id: Some(1),
            uuid: Some(format!("{name}-uuid")),
            name: Some(name.to_owned()),
            kind: Some("application".to_owned()),
            status: Some(status.to_owned()),
            fqdn: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn views_resources_table_badges_running_and_exited() {
        let view = resources_table(&[
            resource("landing", "running:healthy"),
            resource("wiki", "exited"),
        ]);
        let View::Table { rows, .. } = view else {
            panic!("expected a table");
        };
        assert_eq!(rows.len(), 2);
        let Cell::Badge(badge) = &rows[0].cells["status"] else {
            panic!("status is a badge");
        };
        assert_eq!(badge.tone, Tone::Success);
        let Cell::Badge(badge) = &rows[1].cells["status"] else {
            panic!("status is a badge");
        };
        assert_eq!(badge.tone, Tone::Danger);
        assert_eq!(rows[0].open.as_deref(), Some("application/landing-uuid"));
    }

    #[test]
    fn views_servers_table_reads_reachability() {
        let server = Server {
            id: Some(0),
            uuid: Some("srv-1".to_owned()),
            name: Some("localhost".to_owned()),
            description: None,
            ip: Some("127.0.0.1".to_owned()),
            user: None,
            port: None,
            proxy_type: None,
            settings: Some(crate::types::ServerSettings {
                is_reachable: Some(false),
                is_usable: Some(false),
            }),
        };
        let View::Table { rows, .. } = servers_table(&[server]) else {
            panic!("expected a table");
        };
        let Cell::Badge(badge) = &rows[0].cells["status"] else {
            panic!("status is a badge");
        };
        assert_eq!(badge.text, "unreachable");
        assert_eq!(badge.tone, Tone::Danger);
    }

    #[test]
    fn views_deployments_table_shortens_the_commit() {
        let deployment: Deployment = serde_json::from_str(
            r#"{"deployment_uuid":"dep-1","commit":"0123456789abcdef","status":"in_progress","application_name":"landing"}"#,
        )
        .expect("deployment decodes");
        let View::Table { rows, .. } = deployments_table(&[deployment], &[], &BTreeMap::new())
        else {
            panic!("expected a table");
        };
        assert_eq!(rows[0].cells["commit"], Cell::Mono("0123456".to_owned()));
    }

    #[test]
    fn views_deployments_table_has_duration_trigger_message_and_words_for_status() {
        let deployment: Deployment = serde_json::from_str(
            r#"{"deployment_uuid":"d1","commit":"c2a7e94aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","commit_message":"fix caddy upstream","status":"in_progress","is_webhook":true,"application_name":"landing","server_name":"hetzner-1","created_at":"2026-10-24T09:28:00.000000Z","updated_at":"2026-10-24T09:28:54.000000Z"}"#,
        )
        .expect("deployment decodes");
        let View::Table { columns, rows, .. } =
            deployments_table(&[deployment], &[], &BTreeMap::new())
        else {
            panic!("expected a table");
        };
        let keys: Vec<&str> = columns.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "application",
                "status",
                "commit",
                "message",
                "duration",
                "trigger"
            ]
        );
        assert!(
            columns[4].align_right,
            "durations are right-aligned numbers"
        );
        assert_eq!(rows[0].cells["duration"], Cell::Text("0 m 54 s".to_owned()));
        assert_eq!(rows[0].cells["trigger"], Cell::Text("webhook".to_owned()));
        assert_eq!(
            rows[0].cells["message"],
            Cell::Text("fix caddy upstream".to_owned())
        );
        let Cell::Badge(badge) = &rows[0].cells["status"] else {
            panic!("status is a badge");
        };
        assert_eq!(badge.text, "In progress");
        assert_eq!(badge.tone, Tone::Warning);
        // The server stays on the row for the pane's header, not as a column.
        assert_eq!(rows[0].cells["server"], Cell::Text("hetzner-1".to_owned()));
    }

    #[test]
    fn views_status_words_keep_the_health_qualifier() {
        assert_eq!(humanize_status("running:healthy"), "Running (healthy)");
        assert_eq!(humanize_status("cancelled-by-user"), "Cancelled by user");
        assert_eq!(humanize_status("exited"), "Exited");
    }

    #[test]
    fn views_application_detail_has_git_fields_and_tabs() {
        let application: Application =
            serde_json::from_str(include_str!("../../../fixtures/coolify/application.json"))
                .expect("fixture decodes");
        let View::Detail {
            title,
            fields,
            tabs,
            ..
        } = application_detail(&application, Some("hetzner-1"))
        else {
            panic!("expected a detail view");
        };
        assert_eq!(title, "landing");
        assert!(fields.iter().any(|(k, v)| k == "Branch" && v == "main"));
        assert_eq!(tabs.len(), 3);
    }

    #[test]
    fn views_envs_table_masks_every_value() {
        let envs: Vec<EnvVar> =
            serde_json::from_str(include_str!("../../../fixtures/coolify/envs.json"))
                .expect("fixture decodes");
        let view = envs_table(&envs);
        let debug = format!("{view:?}");
        assert!(debug.contains("DATABASE_URL"), "{debug}");
        assert!(debug.contains("••••"), "{debug}");
        assert!(!debug.contains("fake-value-not-a-secret"), "{debug}");
    }

    #[test]
    fn views_resource_rows_carry_start_stop_restart_by_kind() {
        let resources: Vec<Resource> =
            serde_json::from_str(include_str!("../../../fixtures/coolify/resources.json"))
                .expect("fixture decodes");
        let View::Table { rows, .. } = resources_table(&resources) else {
            panic!("expected a table");
        };
        for row in &rows {
            let ids: Vec<&str> = row.actions.iter().map(|a| a.id.as_str()).collect();
            assert_eq!(ids, ["start", "stop", "restart"], "{}", row.id);
        }
        // The postgres database acts on the databases URL family.
        let db = rows
            .iter()
            .find(|r| r.id == "db-uuid-0001")
            .expect("db row");
        assert_eq!(db.actions[0].params["kind"], "database");
    }

    #[test]
    fn views_application_detail_deploy_confirm_names_app_server_and_branch() {
        let application: Application =
            serde_json::from_str(include_str!("../../../fixtures/coolify/application.json"))
                .expect("fixture decodes");
        let View::Detail { actions, .. } = application_detail(&application, Some("hetzner-1"))
        else {
            panic!("expected a detail view");
        };
        let deploy = actions.iter().find(|a| a.id == "deploy").expect("deploy");
        let confirm = deploy.confirm.as_ref().expect("deploy confirms");
        let lines = confirm.lines.join("\n");
        assert!(lines.contains("landing"), "{lines}");
        assert!(lines.contains("hetzner-1"), "{lines}");
        assert!(lines.contains("main"), "{lines}");
        assert_eq!(deploy.options[0].key, "force");
    }

    #[test]
    fn views_deployment_rows_offer_cancel_only_before_a_final_status() {
        let deployments: Vec<Deployment> = serde_json::from_str(include_str!(
            "../../../fixtures/coolify/app-deployments.json"
        ))
        .expect("fixture decodes");
        let mut running = deployments[0].clone();
        running.status = Some("in_progress".to_owned());
        let View::Table { rows, .. } =
            deployments_table(&[running, deployments[0].clone()], &[], &BTreeMap::new())
        else {
            panic!("expected a table");
        };
        assert_eq!(rows[0].actions.len(), 1);
        assert_eq!(rows[0].actions[0].id, "cancel-deployment");
        assert!(
            rows[1].actions.iter().all(|a| a.id != "cancel-deployment"),
            "a finished deployment cannot be cancelled"
        );
    }

    #[test]
    fn views_deployment_rows_carry_the_repository_and_redeploy_when_the_application_is_known() {
        let app: Application =
            serde_json::from_str(include_str!("../../../fixtures/coolify/application.json"))
                .expect("fixture decodes");
        let deployments: Vec<Deployment> = serde_json::from_str(include_str!(
            "../../../fixtures/coolify/app-deployments.json"
        ))
        .expect("fixture decodes");
        let View::Table { rows, .. } = deployments_table(&deployments, &[app], &BTreeMap::new())
        else {
            panic!("expected a table");
        };
        assert_eq!(
            rows[0].cells["repository"],
            Cell::Text("example/landing".to_owned())
        );
        let ids: Vec<&str> = rows[0].actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["deploy", "open-logs"]);
    }

    #[test]
    fn views_version_message_carries_the_warning() {
        let view = version_message(&VersionInfo {
            raw: "5.0.0".to_owned(),
            major: Some(5),
            warning: Some("Built for Coolify 4.5; some actions may fail".to_owned()),
        });
        let View::Message { tone, text, .. } = view else {
            panic!("expected a message");
        };
        assert_eq!(tone, Tone::Warning);
        assert!(text.contains("Built for Coolify 4.5"), "{text}");
    }
}
