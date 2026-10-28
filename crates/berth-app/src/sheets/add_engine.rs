//! The Add engine sheet (P8-T1, D47): SSH (hosts offered from
//! `~/.ssh/config`) or TCP with optional TLS files, a Test connection
//! that really asks the engine, a name, and the colour marker the
//! engine will carry (derived from its id, AD41 — shown, not picked).
//! Adding stores a `UserEngine` and connects it through the engines
//! service like a Colima profile that just started.

use dioxus::prelude::*;

use berth_core::{Engine, EngineId, RemoteTransport, TlsFiles};

use crate::services::engines::EngineLink;
use crate::services::Services;
use crate::state::AppState;
use crate::ui::{
    Button, ButtonVariant, ControlSize, InlineMessage, InlineTone, Select, SelectOption, Switch,
    TextField,
};

pub const ADD_ENGINE_CSS: &str = include_str!("../../assets/css/add_engine.css");

/// The series colour (1..=5) an engine id maps to, stable across runs
/// (AD41: derived, never stored — §6 has no colour field).
#[must_use]
pub fn marker_series(id: &EngineId) -> u8 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in id.0.as_bytes() {
        hash = (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
    }
    // 1..=5; the cast cannot truncate after the modulo.
    #[allow(clippy::cast_possible_truncation)]
    let series = (hash % 5) as u8;
    series + 1
}

/// Builds the stored row for a name and transport, refusing an empty
/// or duplicate name and an engine id that already exists.
///
/// # Errors
/// A sentence for the sheet's inline error.
pub fn build_engine(
    name: &str,
    transport: &RemoteTransport,
    existing: &[Engine],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<berth_store::UserEngine, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the engine a name".to_string());
    }
    if existing.iter().any(|e| e.name == name) {
        return Err(format!("An engine named {name} already exists"));
    }
    let id = berth_engine::remote::engine_id(transport);
    if existing.iter().any(|e| e.id == id) {
        return Err(format!("{} is already an engine", id.0));
    }
    Ok(berth_store::UserEngine {
        id,
        name: name.to_string(),
        kind: berth_core::EngineKind::Remote {
            transport: transport.clone(),
        },
        added: now,
    })
}

/// The engine a probe or the first connect runs against.
#[must_use]
pub fn probe_engine(row: &berth_store::UserEngine) -> Engine {
    Engine {
        id: row.id.clone(),
        name: row.name.clone(),
        kind: row.kind.clone(),
        state: berth_core::EngineState::Running,
        docker_host: None,
        docker_version: None,
        resources: None,
        counts: berth_core::ResourceCounts::default(),
        owner: berth_core::OwnerId::Local,
    }
}

/// Parses the sheet's TCP address: `tcp://`, `http://` or `https://`
/// with a host. A bare `host:port` gains `tcp://`.
///
/// # Errors
/// A sentence for the field's inline error.
pub fn parse_tcp_url(text: &str) -> Result<url::Url, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Give the engine's TCP address".to_string());
    }
    let with_scheme = if text.contains("://") {
        text.to_string()
    } else {
        format!("tcp://{text}")
    };
    let url = url::Url::parse(&with_scheme)
        .map_err(|_| format!("{text} is not a TCP address Berth can use"))?;
    match url.scheme() {
        "tcp" | "http" | "https" => {}
        other => return Err(format!("{other}:// is not a Docker TCP address")),
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("The TCP address needs a host".to_string());
    }
    Ok(url)
}

/// The transport the sheet's fields describe.
///
/// # Errors
/// A sentence for the sheet's inline error.
#[allow(clippy::too_many_arguments)]
pub fn draft_transport(
    mode: &str,
    host: &str,
    user: &str,
    port: &str,
    url: &str,
    use_tls: bool,
    ca: &str,
    cert: &str,
    key: &str,
) -> Result<RemoteTransport, String> {
    if mode == "tcp" {
        let url = parse_tcp_url(url)?;
        let tls = if use_tls {
            let (ca, cert, key) = (ca.trim(), cert.trim(), key.trim());
            if ca.is_empty() || cert.is_empty() || key.is_empty() {
                return Err("TLS needs the CA, certificate and key files".to_string());
            }
            Some(TlsFiles {
                ca: ca.into(),
                cert: cert.into(),
                key: key.into(),
            })
        } else {
            None
        };
        return Ok(RemoteTransport::Tcp { url, tls });
    }
    let host = host.trim();
    if host.is_empty() {
        return Err("Give the SSH host or its ~/.ssh/config alias".to_string());
    }
    let port = port.trim();
    let port = if port.is_empty() {
        None
    } else {
        Some(
            port.parse::<u16>()
                .map_err(|_| format!("{port} is not a port number"))?,
        )
    };
    let user = user.trim();
    Ok(RemoteTransport::Ssh {
        host: host.to_string(),
        user: (!user.is_empty()).then(|| user.to_string()),
        port,
    })
}

/// Asks the engine who it is, bounded so an unreachable host answers
/// within seconds, not whenever SSH gives up.
async fn probe(engine: Engine) -> Result<String, String> {
    use berth_engine::EngineClient as _;
    let query = async {
        let client = berth_engine::BollardEngine::connect(&engine).map_err(|e| e.to_string())?;
        let info = client.info().await.map_err(|e| e.to_string())?;
        Ok::<_, String>(format!(
            "Connected \u{b7} Docker {} on {}",
            info.docker_version, info.os
        ))
    };
    match tokio::time::timeout(std::time::Duration::from_secs(15), query).await {
        Ok(result) => result,
        Err(_elapsed) => Err("No answer within 15 s".to_string()),
    }
}

#[component]
pub fn AddEngineSheet() -> Element {
    let mut state = use_context::<AppState>();
    let services = try_consume_context::<Services>();
    let link = try_consume_context::<Option<EngineLink>>().flatten();
    let ssh_hosts = use_hook(berth_engine::remote::ssh_hosts);

    let mut name = use_signal(String::new);
    let mut mode = use_signal(|| "ssh".to_string());
    let mut host = use_signal(String::new);
    let mut user = use_signal(String::new);
    let mut port = use_signal(String::new);
    let mut url = use_signal(String::new);
    let mut use_tls = use_signal(|| false);
    let mut ca = use_signal(String::new);
    let mut cert = use_signal(String::new);
    let mut key = use_signal(String::new);
    // Bumped when a config host is picked so the uncontrolled fields
    // remount with its values.
    let mut epoch = use_signal(|| 0_u32);
    let mut testing = use_signal(|| false);
    let mut tested = use_signal(|| None::<Result<String, String>>);
    let mut refused = use_signal(|| None::<String>);

    let draft = draft_transport(
        &mode.read(),
        &host.read(),
        &user.read(),
        &port.read(),
        &url.read(),
        use_tls(),
        &ca.read(),
        &cert.read(),
        &key.read(),
    );
    let draft_problem = draft.as_ref().err().cloned();
    let built = draft.as_ref().ok().and_then(|transport| {
        build_engine(
            &name.read(),
            transport,
            &state.engines.peek(),
            chrono::Utc::now(),
        )
        .ok()
    });
    let name_problem = if name.read().trim().is_empty() {
        None
    } else {
        draft.as_ref().ok().and_then(|transport| {
            build_engine(
                &name.read(),
                transport,
                &state.engines.peek(),
                chrono::Utc::now(),
            )
            .err()
        })
    };
    let blocker = name_problem
        .clone()
        .or_else(|| draft_problem.clone())
        .or_else(|| (name.read().trim().is_empty()).then(|| "Give the engine a name".to_string()))
        .or_else(|| (services.is_none()).then(|| "The store is not connected".to_string()));
    let marker = built.as_ref().map(|row| marker_series(&row.id));
    let tested_now = tested.read().clone();

    let run_test = {
        let built = built.clone();
        move |_| {
            let Some(row) = built.clone() else {
                return;
            };
            testing.set(true);
            tested.set(None);
            spawn(async move {
                let result = probe(probe_engine(&row)).await;
                tested.set(Some(result));
                testing.set(false);
            });
        }
    };
    let add = {
        let built = built.clone();
        let services = services.clone();
        let link = link.clone();
        move |_| {
            let (Some(row), Some(services)) = (built.clone(), services.clone()) else {
                refused.set(Some("The store is not connected".to_string()));
                return;
            };
            match services.store.upsert_engine(&row) {
                Ok(()) => {
                    if let Some(link) = &link {
                        link.connect(probe_engine(&row), None);
                    }
                    state.overlays.write().pop();
                }
                Err(err) => refused.set(Some(err.to_string())),
            }
        }
    };

    let host_options: Vec<SelectOption> =
        std::iter::once(SelectOption::new("", "Type a host\u{2026}"))
            .chain(
                ssh_hosts
                    .iter()
                    .map(|h| SelectOption::new(h.name.clone(), h.name.clone())),
            )
            .collect();
    let epoch_now = epoch();

    rsx! {
        style { "{ADD_ENGINE_CSS}" }
        div {
            class: "ae-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Add engine",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "ae-title", "Add engine" }
            p { class: "ae-lede",
                "A Docker engine on another machine, over SSH or TCP. Berth connects with your SSH config and agent; nothing is installed there."
            }
            div { class: "ae-form",
                TextField {
                    label: "Name".to_string(),
                    placeholder: "prod-1".to_string(),
                    aria_label: "Engine name".to_string(),
                    error: name_problem,
                    oninput: move |e: FormEvent| name.set(e.value()),
                }
                if let Some(series) = marker {
                    div { class: "ae-marker-row",
                        span {
                            class: "ae-marker",
                            style: "background: var(--color-series-{series});",
                            "aria-hidden": "true",
                        }
                        "This engine's colour marker, from its address."
                    }
                }
                Select {
                    label: "Connection".to_string(),
                    options: vec![
                        SelectOption::new("ssh", "SSH"),
                        SelectOption::new("tcp", "TCP"),
                    ],
                    value: mode.read().clone(),
                    aria_label: "Connection type".to_string(),
                    onchange: move |e: FormEvent| mode.set(e.value()),
                }
                if mode.read().as_str() == "ssh" {
                    if !ssh_hosts.is_empty() {
                        Select {
                            label: "From ~/.ssh/config".to_string(),
                            options: host_options,
                            value: String::new(),
                            aria_label: "SSH config host".to_string(),
                            onchange: move |e: FormEvent| {
                                if let Some(found) = ssh_hosts.iter().find(|h| h.name == e.value()) {
                                    host.set(found.name.clone());
                                    user.set(found.user.clone().unwrap_or_default());
                                    port.set(found.port.map_or_else(String::new, |p| p.to_string()));
                                    if name.peek().trim().is_empty() {
                                        name.set(found.name.clone());
                                    }
                                    let next = epoch.peek().wrapping_add(1);
                                    epoch.set(next);
                                }
                            },
                        }
                    }
                    for at in [epoch_now] {
                        div { key: "{at}", class: "ae-form",
                            TextField {
                                label: "Host".to_string(),
                                initial_value: host.read().clone(),
                                placeholder: "prod-1 or 10.0.0.5".to_string(),
                                aria_label: "SSH host".to_string(),
                                oninput: move |e: FormEvent| host.set(e.value()),
                            }
                            TextField {
                                label: "User".to_string(),
                                initial_value: user.read().clone(),
                                placeholder: "deploy (optional)".to_string(),
                                aria_label: "SSH user".to_string(),
                                oninput: move |e: FormEvent| user.set(e.value()),
                            }
                            TextField {
                                label: "Port".to_string(),
                                initial_value: port.read().clone(),
                                placeholder: "22 (optional)".to_string(),
                                aria_label: "SSH port".to_string(),
                                error: draft_problem.clone().filter(|p| p.contains("port")),
                                oninput: move |e: FormEvent| port.set(e.value()),
                            }
                        }
                    }
                } else {
                    TextField {
                        label: "TCP address".to_string(),
                        placeholder: "tcp://10.0.0.5:2376".to_string(),
                        aria_label: "TCP address".to_string(),
                        error: draft_problem.clone().filter(|p| !p.starts_with("TLS")),
                        oninput: move |e: FormEvent| url.set(e.value()),
                    }
                    Switch {
                        on: use_tls(),
                        label: "TLS with client certificates".to_string(),
                        aria_label: Some("TLS with client certificates".to_string()),
                        on_toggle: move |next: bool| use_tls.set(next),
                    }
                    if use_tls() {
                        TextField {
                            label: "CA file".to_string(),
                            placeholder: "~/.docker/ca.pem".to_string(),
                            aria_label: "CA file".to_string(),
                            oninput: move |e: FormEvent| ca.set(e.value()),
                        }
                        TextField {
                            label: "Certificate file".to_string(),
                            placeholder: "~/.docker/cert.pem".to_string(),
                            aria_label: "Certificate file".to_string(),
                            oninput: move |e: FormEvent| cert.set(e.value()),
                        }
                        TextField {
                            label: "Key file".to_string(),
                            placeholder: "~/.docker/key.pem".to_string(),
                            aria_label: "Key file".to_string(),
                            error: draft_problem.clone().filter(|p| p.starts_with("TLS")),
                            oninput: move |e: FormEvent| key.set(e.value()),
                        }
                    }
                }
                if let Some(Ok(line)) = &tested_now {
                    p { class: "ae-test-ok", role: "status", "{line}" }
                }
                if let Some(Err(reason)) = &tested_now {
                    InlineMessage { message: reason.clone(), tone: InlineTone::Danger }
                }
                if let Some(reason) = refused.read().clone() {
                    InlineMessage { message: reason, tone: InlineTone::Danger }
                }
            }
            div { class: "ae-actions",
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| {
                        state.overlays.write().pop();
                    },
                }
                Button {
                    label: if testing() { "Testing\u{2026}".to_string() } else { "Test connection".to_string() },
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    loading: testing(),
                    disabled: blocker.is_some() || testing(),
                    disabled_reason: blocker.clone(),
                    onclick: run_test,
                }
                Button {
                    label: "Add engine".to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    disabled: blocker.is_some(),
                    disabled_reason: blocker,
                    onclick: add,
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "add_engine_tests.rs"]
mod tests;
