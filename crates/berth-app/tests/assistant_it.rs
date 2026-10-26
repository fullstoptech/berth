#![allow(clippy::expect_used)]
//! `mcp_it`: the whole assistant path end to end (P7-T2, ADR 0006, AD37).
//! An rmcp client spawns the real `berth mcp` binary, which forwards over
//! a test bridge to the gated executor backed by a MockEngine and a temp
//! folder. No live Docker engine is touched, so these tests are not
//! ignored; `scripts/it.sh -p berth-app mcp_it` runs them too.

use std::sync::Arc;
use std::time::Duration;

use berth_app::services::backing::Backing;
use berth_app::services::engines;
use berth_app::services::plans::PlanStore;
use berth_app::services::tools::{ApprovalHub, ApprovalRequest, Decision, ToolCore, ToolDeps};
use berth_assistant::bridge::BridgeServer;
use berth_core::EngineId;
use berth_engine::{EngineClient, MockCompose, MockEngine, MockWorld};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::ServiceExt;
use serde_json::json;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;

const SECRET: &str = "s3cr3t-mcp-it-value";
const TOKEN: &str = "mcp-it-token";

struct TestBridge {
    socket: std::path::PathBuf,
    approvals: Receiver<ApprovalRequest>,
    plans: PlanStore,
    folder: std::path::PathBuf,
    cancel: CancellationToken,
    _dir: tempfile::TempDir,
}

async fn test_bridge() -> TestBridge {
    let dir = tempfile::tempdir().expect("dir");
    let folder = dir.path().join("proj");
    std::fs::create_dir_all(&folder).expect("folder");
    std::fs::write(folder.join(".env"), format!("SECRET={SECRET}\n")).expect("env");
    std::fs::write(
        folder.join("compose.yaml"),
        "services:\n  web:\n    image: nginx\n",
    )
    .expect("compose");

    let world = MockWorld::bundled();
    let engine = EngineId("colima:default".to_string());
    let mock = MockEngine::from_world(world.clone(), &engine).expect("mock engine");
    let clients = engines::client_map();
    clients
        .lock()
        .expect("lock")
        .insert(engine, Arc::new(mock) as Arc<dyn EngineClient>);
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let plans = PlanStore::default();
    let (hub, approvals) = ApprovalHub::channel();
    let core = Arc::new(ToolCore::new(
        ToolDeps {
            clients,
            store,
            compose: Arc::new(MockCompose::from_world(&world)),
            plans: plans.clone(),
            backing: Backing::Live,
            session_folder: Some(folder.clone()),
        },
        hub,
    ));
    let socket = dir.path().join("b.sock");
    let cancel = CancellationToken::new();
    let (_server, incoming) =
        BridgeServer::bind_at(socket.clone(), TOKEN.to_string(), cancel.clone())
            .await
            .expect("bind bridge");
    tokio::spawn(berth_app::services::tools::serve(
        core,
        incoming,
        cancel.clone(),
    ));
    TestBridge {
        socket,
        approvals,
        plans,
        folder,
        cancel,
        _dir: dir,
    }
}

type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn spawn_client(socket: &std::path::Path, token: &str) -> Client {
    let session = uuid::Uuid::now_v7().to_string();
    let socket_arg = socket.to_string_lossy().into_owned();
    let child = TokioChildProcess::builder(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_berth")).configure(|cmd| {
            cmd.arg("mcp")
                .arg("--bridge")
                .arg(socket_arg)
                .arg("--session")
                .arg(session)
                .env("BERTH_BRIDGE_TOKEN", token)
                .kill_on_drop(true);
        }),
    )
    .stderr(std::process::Stdio::null())
    .spawn()
    .expect("spawn berth mcp")
    .0;
    ().serve(child).await.expect("mcp client starts")
}

fn params(name: &str, args: serde_json::Value) -> CallToolRequestParams {
    let map = match args {
        serde_json::Value::Object(map) => map,
        _ => panic!("args must be an object"),
    };
    CallToolRequestParams::new(name.to_string()).with_arguments(map)
}

fn result_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_error(result: &CallToolResult) -> bool {
    result.is_error == Some(true)
}

/// Calls a gated tool and answers its approval with `decision`, proving on
/// the way that the call is still pending while unapproved.
async fn call_gated(
    client: &Client,
    bridge: &mut TestBridge,
    name: &'static str,
    args: serde_json::Value,
    decision: Decision,
) -> CallToolResult {
    let peer = client.peer().clone();
    let call = tokio::spawn(async move { peer.call_tool(params(name, args)).await });
    let approval = tokio::time::timeout(Duration::from_secs(10), bridge.approvals.recv())
        .await
        .expect("an approval is asked")
        .expect("approval");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!call.is_finished(), "{name} ran before it was approved");
    approval.answer(decision);
    call.await
        .expect("call task")
        .unwrap_or_else(|e| panic!("{name} call failed at the protocol level: {e}"))
}

#[tokio::test]
async fn mcp_it_lists_25_tools_reads_run_and_changes_wait_for_approval() {
    let mut bridge = test_bridge().await;
    let client = spawn_client(&bridge.socket, TOKEN).await;

    let tools = client.peer().list_all_tools().await.expect("list tools");
    assert_eq!(
        tools.len(),
        25,
        "{:?}",
        tools.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for wire in [
        "containers_list",
        "containers_restart",
        "files_propose_edit",
        "system_prune",
    ] {
        assert!(names.contains(&wire), "missing {wire}: {names:?}");
    }
    assert!(
        names.iter().all(|n| !n.contains('.')),
        "wire names: {names:?}"
    );

    // A Read returns at once, naming the mock containers.
    let listed = client
        .peer()
        .call_tool(params("containers_list", json!({})))
        .await
        .expect("containers_list");
    assert!(!is_error(&listed), "{}", result_text(&listed));
    assert!(
        result_text(&listed).contains("shop-api-api-1"),
        "{}",
        result_text(&listed)
    );
    assert!(bridge.approvals.try_recv().is_err(), "a read asked");

    // A Change blocks until the test approves, then runs.
    let restarted = call_gated(
        &client,
        &mut bridge,
        "containers_restart",
        json!({"container": "shop-api-api-1"}),
        Decision::Approved { typed_name: None },
    )
    .await;
    assert!(!is_error(&restarted), "{}", result_text(&restarted));
    assert!(
        result_text(&restarted).contains("Restarted shop-api-api-1"),
        "{}",
        result_text(&restarted)
    );

    // Denied is denied, and the reason reaches the caller.
    let denied = call_gated(
        &client,
        &mut bridge,
        "containers_restart",
        json!({"container": "shop-api-api-1"}),
        Decision::Denied {
            reason: "the test said no".to_string(),
        },
    )
    .await;
    assert!(is_error(&denied), "{}", result_text(&denied));
    assert!(
        result_text(&denied).contains("the test said no"),
        "{}",
        result_text(&denied)
    );

    client.cancel().await.expect("close client");
    bridge.cancel.cancel();
}

#[tokio::test]
async fn mcp_it_destructive_needs_the_typed_name_and_files_stay_masked() {
    let mut bridge = test_bridge().await;
    let client = spawn_client(&bridge.socket, TOKEN).await;

    // A wrong name is refused outright: no approval is even asked.
    let refused = client
        .peer()
        .call_tool(params(
            "containers_remove",
            json!({"name": "no-such-container"}),
        ))
        .await
        .expect("refused call returns");
    assert!(is_error(&refused), "{}", result_text(&refused));
    assert!(
        result_text(&refused).contains("no container named"),
        "{}",
        result_text(&refused)
    );
    assert!(bridge.approvals.try_recv().is_err(), "a refusal asked");

    // The right item but the wrong typed name removes nothing.
    let mistyped = call_gated(
        &client,
        &mut bridge,
        "containers_remove",
        json!({"name": "shop-api-db-1"}),
        Decision::Approved {
            typed_name: Some("shop-api-db-9".to_string()),
        },
    )
    .await;
    assert!(is_error(&mistyped), "{}", result_text(&mistyped));
    let still_there = client
        .peer()
        .call_tool(params("containers_list", json!({})))
        .await
        .expect("list after mistype");
    assert!(
        result_text(&still_there).contains("shop-api-db-1"),
        "a mistyped confirm removed the container"
    );

    // files.read of a .env is masked.
    let env = client
        .peer()
        .call_tool(params("files_read", json!({"path": ".env"})))
        .await
        .expect("read .env");
    assert!(!is_error(&env), "{}", result_text(&env));
    assert!(
        result_text(&env).contains("SECRET=••••"),
        "{}",
        result_text(&env)
    );
    assert!(
        !result_text(&env).contains(SECRET),
        "the .env value reached the harness"
    );

    // files.propose_edit parks a pending plan and writes nothing.
    let before = std::fs::read_to_string(bridge.folder.join("compose.yaml")).expect("before");
    let proposed = client
        .peer()
        .call_tool(params(
            "files_propose_edit",
            json!({
                "path": "compose.yaml",
                "content": "services:\n  web:\n    image: nginx:1.27\n",
                "summary": "pin nginx",
            }),
        ))
        .await
        .expect("propose");
    assert!(!is_error(&proposed), "{}", result_text(&proposed));
    assert!(
        result_text(&proposed).contains("Nothing was written"),
        "{}",
        result_text(&proposed)
    );
    assert_eq!(
        std::fs::read_to_string(bridge.folder.join("compose.yaml")).expect("after"),
        before,
        "propose_edit wrote the file"
    );
    assert_eq!(bridge.plans.len(), 1, "no pending plan was parked");
    let plan_id = proposed
        .structured_content
        .as_ref()
        .and_then(|d| d.get("plan_id"))
        .and_then(serde_json::Value::as_str)
        .expect("plan id in the result")
        .parse::<uuid::Uuid>()
        .expect("plan id is a uuid");
    let pending = bridge.plans.get(&plan_id).expect("the parked plan");
    assert!(matches!(
        pending.plan.source,
        berth_core::PlanSource::Harness(_)
    ));

    client.cancel().await.expect("close client");
    bridge.cancel.cancel();
}

#[tokio::test]
async fn mcp_it_wrong_token_is_refused_and_no_bridge_exits_nonzero_fast() {
    let bridge = test_bridge().await;
    let impostor = spawn_client(&bridge.socket, "not-the-token").await;
    let refused = impostor
        .peer()
        .call_tool(params("containers_list", json!({})))
        .await
        .expect("the refusal comes back as a tool error");
    assert!(is_error(&refused), "{}", result_text(&refused));
    assert!(
        result_text(&refused).contains("token"),
        "{}",
        result_text(&refused)
    );
    impostor.cancel().await.expect("close impostor");
    bridge.cancel.cancel();

    // No Berth listening: `berth mcp` fails at once (ADR 0006).
    let missing = bridge.socket.with_file_name("nobody.sock");
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_berth"))
            .arg("mcp")
            .arg("--bridge")
            .arg(&missing)
            .env("BERTH_BRIDGE_TOKEN", TOKEN)
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .expect("exits fast")
    .expect("run berth mcp");
    assert!(
        !output.status.success(),
        "exited 0 with no bridge: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not running"), "{stderr}");
}
