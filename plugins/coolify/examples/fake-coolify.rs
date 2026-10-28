//! A fake Coolify for trying the plugin by hand (P9-T5 acceptance) without a
//! real instance:
//!
//! ```text
//! cargo run -p berth-coolify --example fake-coolify -- [--port 18080] [--version v4.5.0]
//! ```
//!
//! It serves `fixtures/coolify/*.json` on `/api/v1`, wiremock-backed. Any
//! bearer token is accepted; a request with no `Authorization` header gets
//! 401, so the "token invalid" path can be tried by clearing the token.
//!
//! What it knows: one application (`app-uuid-0001`, "landing") with a
//! finished, a failed (`dep-uuid-00ff`, log ends at the failing build step)
//! and a running deployment (`dep-uuid-0001`, finishes after two polls). A
//! deploy request queues `dep-uuid-0002`, which also finishes after two
//! polls. Start, stop, restart and cancel answer with a message and change
//! nothing. Pass `--version 5.0.0` to see the "Built for Coolify 4.5"
//! warning.

use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use wiremock::matchers::{any, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const RESOURCES: &str = include_str!("../../../fixtures/coolify/resources.json");
const SERVERS: &str = include_str!("../../../fixtures/coolify/servers.json");
const APPLICATION: &str = include_str!("../../../fixtures/coolify/application.json");
const DEPLOYMENTS: &str = include_str!("../../../fixtures/coolify/deployments.json");
const APP_DEPLOYMENTS: &str = include_str!("../../../fixtures/coolify/app-deployments.json");
const DEPLOYMENT_FAILED: &str = include_str!("../../../fixtures/coolify/deployment-failed.json");
const DEPLOYMENT_FINISHED: &str =
    include_str!("../../../fixtures/coolify/deployment-finished.json");
const ENVS: &str = include_str!("../../../fixtures/coolify/envs.json");
const LOGS: &str = include_str!("../../../fixtures/coolify/logs.json");

const API: &str = "/api/v1";

fn json_body(body: impl Into<String>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_string(body)
}

fn authed(request: &Request) -> bool {
    request.headers.contains_key("authorization")
}

/// A deployment that reports `in_progress` for two polls, then `finished`.
struct Settling {
    polls: AtomicUsize,
    running: String,
    done: String,
}

impl Respond for Settling {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        if self.polls.fetch_add(1, Ordering::SeqCst) < 2 {
            json_body(self.running.clone())
        } else {
            json_body(self.done.clone())
        }
    }
}

/// `dep-uuid-0001` / `dep-uuid-0002` bodies built from the fixtures: the
/// running listing entry plus a growing step log.
fn settling(uuid: &str) -> Settling {
    let mut base: Value =
        serde_json::from_str(DEPLOYMENTS).map_or(Value::Null, |list: Value| list[0].clone());
    base["deployment_uuid"] = json!(uuid);
    let step = |text: &str, order: u32| json!({ "output": text, "type": "stdout", "order": order });
    let mut running = base.clone();
    running["status"] = json!("in_progress");
    running["logs"] = json!(json!([
        step("Starting deployment of landing to hetzner-1.", 1),
        step("Building the image with nixpacks.", 2),
    ])
    .to_string());
    let mut done = base;
    done["status"] = json!("finished");
    done["logs"] = json!(json!([
        step("Starting deployment of landing to hetzner-1.", 1),
        step("Building the image with nixpacks.", 2),
        step("New container started. Deployment finished.", 3),
    ])
    .to_string());
    Settling {
        polls: AtomicUsize::new(0),
        running: running.to_string(),
        done: done.to_string(),
    }
}

async fn get(server: &MockServer, route: &str, body: &'static str) {
    Mock::given(method("GET"))
        .and(path(format!("{API}/{route}")))
        .and(authed)
        .respond_with(json_body(body))
        .mount(server)
        .await;
}

async fn serve(server: &MockServer, version: &str) {
    // No Authorization header: 401, whatever the route.
    Mock::given(any())
        .and(|request: &Request| !authed(request))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthenticated."))
        .with_priority(1)
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("{API}/health")))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{API}/version")))
        .respond_with(ResponseTemplate::new(200).set_body_string(version.to_owned()))
        .mount(server)
        .await;

    get(server, "resources", RESOURCES).await;
    get(server, "servers", SERVERS).await;
    get(server, "deployments", DEPLOYMENTS).await;
    get(server, "applications/app-uuid-0001", APPLICATION).await;
    get(
        server,
        "deployments/applications/app-uuid-0001",
        APP_DEPLOYMENTS,
    )
    .await;
    get(server, "applications/app-uuid-0001/logs", LOGS).await;
    get(server, "applications/app-uuid-0001/envs", ENVS).await;
    get(server, "deployments/dep-uuid-00ff", DEPLOYMENT_FAILED).await;
    get(server, "deployments/dep-uuid-0000", DEPLOYMENT_FINISHED).await;
    Mock::given(method("GET"))
        .and(path(format!("{API}/applications")))
        .and(authed)
        .respond_with(json_body(format!("[{APPLICATION}]")))
        .mount(server)
        .await;
    for uuid in ["dep-uuid-0001", "dep-uuid-0002"] {
        Mock::given(method("GET"))
            .and(path(format!("{API}/deployments/{uuid}")))
            .and(authed)
            .respond_with(settling(uuid))
            .mount(server)
            .await;
    }

    Mock::given(method("POST"))
        .and(path(format!("{API}/deploy")))
        .and(authed)
        .respond_with(json_body(
            json!({ "deployments": [{
                "message": "Application landing deployment queued.",
                "resource_uuid": "app-uuid-0001",
                "deployment_uuid": "dep-uuid-0002",
            }] })
            .to_string(),
        ))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(format!(
            r"^{API}/(applications|services|databases)/[^/]+/(start|stop|restart)$"
        )))
        .and(authed)
        .respond_with(json_body(r#"{"message":"Request queued."}"#))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(format!(r"^{API}/deployments/[^/]+/cancel$")))
        .and(authed)
        .respond_with(json_body(r#"{"message":"Deployment cancelled."}"#))
        .mount(server)
        .await;
}

fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == name {
            return args.next();
        }
    }
    None
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = arg("--port").unwrap_or_else(|| "18080".to_owned());
    let version = arg("--version").unwrap_or_else(|| "v4.5.0".to_owned());
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))?;
    let server = MockServer::builder().listener(listener).start().await;
    serve(&server, &version).await;
    println!("fake Coolify {version} on {}", server.uri());
    println!("set the plugin's Coolify address to that URL; any API token works");
    println!("press Ctrl-C to stop");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
