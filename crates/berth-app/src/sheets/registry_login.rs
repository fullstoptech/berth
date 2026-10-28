//! The registry log-in sheet (P8-T2; the Add engine card). Name, URL,
//! username and password; "Log in" checks the credentials against
//! the registry and stores the password in the Keychain only.
//! Re-opened for a stored registry (`existing`), its fields start
//! from the row and the password field empty.

use dioxus::prelude::*;

use crate::screens::registries::REGISTRIES_CSS;
use crate::services::registries::RegistriesService;
use crate::state::AppState;
use crate::ui::{Button, ButtonVariant, ControlSize, TextField};

/// Whether a bare `host[:port]` is a loopback address (`localhost`,
/// `127.0.0.0/8`, `::1`). Docker talks plain HTTP to those registries.
fn is_loopback_host(bare: &str) -> bool {
    let host_port = bare.split('/').next().unwrap_or(bare);
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if host_port.matches(':').count() > 1 {
        host_port
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Parses the sheet's registry URL: a bare loopback host is `http://`
/// (Docker allows plain HTTP there), every other bare host is `https://`;
/// a scheme typed by hand stays as typed.
///
/// # Errors
/// A sentence for the field's inline error.
pub fn parse_registry_url(text: &str) -> Result<url::Url, String> {
    let text = text.trim().trim_end_matches('/');
    if text.is_empty() {
        return Err("Give the registry's URL".to_string());
    }
    let with_scheme = if text.contains("://") {
        text.to_string()
    } else if is_loopback_host(text) {
        if text.parse::<std::net::Ipv6Addr>().is_ok() {
            format!("http://[{text}]")
        } else {
            format!("http://{text}")
        }
    } else {
        format!("https://{text}")
    };
    let url = url::Url::parse(&with_scheme).map_err(|_| format!("{text} is not a registry URL"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("{other}:// is not a registry address")),
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("The registry URL needs a host".to_string());
    }
    Ok(url)
}

#[component]
pub fn RegistryLoginSheet(
    existing: Option<String>,
    /// A refusal already showing (the sheet starts clean; tests start here).
    #[props(default)]
    refusal: Option<String>,
) -> Element {
    let mut state = use_context::<AppState>();
    let service = try_consume_context::<Option<RegistriesService>>().flatten();
    let row = existing.as_ref().and_then(|id| {
        state
            .registries
            .peek()
            .iter()
            .find(|r| r.id == *id)
            .cloned()
    });
    let mut name = use_signal(|| row.as_ref().map_or_else(String::new, |r| r.name.clone()));
    let mut url = use_signal(|| {
        row.as_ref().map_or_else(String::new, |r| {
            r.url.to_string().trim_end_matches('/').to_string()
        })
    });
    let mut username = use_signal(|| {
        row.as_ref()
            .and_then(|r| r.username.clone())
            .unwrap_or_default()
    });
    let mut password = use_signal(String::new);
    let mut working = use_signal(|| false);
    let mut refused = use_signal(|| refusal.clone());

    let parsed = parse_registry_url(&url.read());
    let url_problem = if url.read().trim().is_empty() {
        None
    } else {
        parsed.as_ref().err().cloned()
    };
    let blocker = url_problem
        .clone()
        .or_else(|| (username.read().trim().is_empty()).then(|| "Give the username".to_string()))
        .or_else(|| (password.read().is_empty()).then(|| "Give the password".to_string()))
        .or_else(|| (service.is_none()).then(|| "Registries are not connected".to_string()));

    let initial_name = name.read().clone();
    let initial_url = url.read().clone();
    let initial_username = username.read().clone();

    let log_in = {
        let service = service.clone();
        move |_| {
            let (Some(service), Ok(url)) = (service.clone(), parse_registry_url(&url.peek()))
            else {
                return;
            };
            refused.set(None);
            working.set(true);
            let name = name.peek().clone();
            let username = username.peek().clone();
            let password = secrecy::SecretString::from(password.peek().clone());
            spawn(async move {
                match service
                    .log_in(&mut state, &name, &url, &username, &password)
                    .await
                {
                    Ok(_) => {
                        state.overlays.write().pop();
                    }
                    Err(reason) => refused.set(Some(reason)),
                }
                working.set(false);
            });
        }
    };

    rsx! {
        style { "{REGISTRIES_CSS}" }
        div {
            class: "rl-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Registry log in",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "rl-title",
                if row.is_some() { "Log in again" } else { "Add registry" }
            }
            p { class: "rl-lede",
                "Berth checks the login against the registry, then keeps the password in your Keychain \u{2014} never in its database or logs."
            }
            div { class: "rl-form",
                TextField {
                    label: "Name".to_string(),
                    initial_value: initial_name,
                    placeholder: "Docker Hub".to_string(),
                    aria_label: "Registry name".to_string(),
                    oninput: move |e: FormEvent| name.set(e.value()),
                }
                TextField {
                    label: "URL".to_string(),
                    initial_value: initial_url,
                    placeholder: "https://index.docker.io".to_string(),
                    help: "A bare localhost or 127.0.0.1 address uses plain http.".to_string(),
                    aria_label: "Registry URL".to_string(),
                    error: url_problem,
                    oninput: move |e: FormEvent| url.set(e.value()),
                }
                TextField {
                    label: "Username".to_string(),
                    initial_value: initial_username,
                    aria_label: "Registry username".to_string(),
                    oninput: move |e: FormEvent| username.set(e.value()),
                }
                TextField {
                    label: "Password".to_string(),
                    input_type: "password".to_string(),
                    aria_label: "Registry password".to_string(),
                    error: refused.read().clone(),
                    oninput: move |e: FormEvent| password.set(e.value()),
                }            }
            div { class: "rl-actions",
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| {
                        state.overlays.write().pop();
                    },
                }
                Button {
                    label: if working() { "Logging in\u{2026}".to_string() } else { "Log in".to_string() },
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    loading: working(),
                    disabled: blocker.is_some() || working(),
                    disabled_reason: blocker,
                    onclick: log_in,
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "registry_login_tests.rs"]
mod tests;
