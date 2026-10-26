//! The app <-> `berth mcp` bridge (PLAN §6.9, ADR 0006): one JSON request
//! per line over a Unix socket in `berth_sys::paths::runtime_dir()`.
//!
//! The app binds the socket and owns tool execution; `berth mcp` only
//! forwards. Every request carries the launch token (`BERTH_BRIDGE_TOKEN`
//! on the MCP side); a request with a wrong token is refused and never
//! reaches the executor. Request arguments are never logged: they can hold
//! a proposed `.env` (CLAUDE.md secrets rule).

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::AssistantError;

/// The socket name in the runtime dir: short on purpose, because
/// `runtime_dir()` is 64 bytes and `sun_path` holds 104 (AD37).
pub const SOCKET_NAME: &str = "bridge.sock";

/// One forwarded tool call. `call_id` is the tool_use id from MCP `_meta`,
/// when the harness sends one; it ties the approval card to the tool row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeRequest {
    pub id: uuid::Uuid,
    pub token: String,
    pub session: berth_core::SessionId,
    pub tool: String,
    pub args: serde_json::Value,
    #[serde(default)]
    pub call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgeResponse {
    Ok {
        id: uuid::Uuid,
        text: String,
        data: Option<serde_json::Value>,
    },
    Denied {
        id: uuid::Uuid,
        reason: String,
    },
    Error {
        id: uuid::Uuid,
        message: String,
    },
}

impl BridgeResponse {
    fn error(id: uuid::Uuid, message: impl Into<String>) -> Self {
        Self::Error {
            id,
            message: message.into(),
        }
    }
}

/// One authenticated request on its way to the executor, with the way back.
pub struct Incoming {
    pub request: BridgeRequest,
    pub reply: oneshot::Sender<BridgeResponse>,
}

pub struct BridgeServer {
    pub socket: std::path::PathBuf,
    pub token: String,
}

/// How long `call` waits for the socket to accept before deciding Berth is
/// not running. Reading the reply has no deadline: an approval can wait as
/// long as the user takes.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// A line cap well above any tool result, so a stuck peer cannot grow a
/// buffer without bound. The socket is same-user and token-checked.
const MAX_LINE: u64 = 8 * 1024 * 1024;

impl BridgeServer {
    /// Binds the one `bridge.sock` in the runtime dir with a fresh token.
    ///
    /// # Errors
    /// [`AssistantError::Bridge`] when the socket cannot be bound.
    pub async fn bind(
        cancel: CancellationToken,
    ) -> Result<(Self, mpsc::Receiver<Incoming>), AssistantError> {
        let socket = berth_sys::paths::runtime_dir().join(SOCKET_NAME);
        Self::bind_at(socket, uuid::Uuid::now_v7().to_string(), cancel).await
    }

    /// Binds `socket` for `token`. The accept loop runs until `cancel`
    /// fires, then removes the socket file. A stale socket file is replaced.
    ///
    /// # Errors
    /// [`AssistantError::Bridge`] when the socket cannot be bound.
    pub async fn bind_at(
        socket: std::path::PathBuf,
        token: String,
        cancel: CancellationToken,
    ) -> Result<(Self, mpsc::Receiver<Incoming>), AssistantError> {
        if let Some(dir) = socket.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| AssistantError::Bridge(format!("runtime dir: {e}")))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        match std::fs::remove_file(&socket) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(AssistantError::Bridge(format!(
                    "clearing the old socket {}: {e}",
                    socket.display()
                )));
            }
        }
        let listener = tokio::net::UnixListener::bind(&socket)
            .map_err(|e| AssistantError::Bridge(format!("binding {}: {e}", socket.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));
        }
        let (tx, rx) = mpsc::channel(32);
        let server = Self { socket, token };
        let loop_socket = server.socket.clone();
        let loop_token = server.token.clone();
        tokio::spawn(async move {
            accept_loop(listener, loop_token, tx, cancel).await;
            let _ = std::fs::remove_file(&loop_socket);
        });
        Ok((server, rx))
    }
}

async fn accept_loop(
    listener: tokio::net::UnixListener,
    token: String,
    tx: mpsc::Sender<Incoming>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let tx = tx.clone();
                        let token = token.clone();
                        tokio::spawn(async move {
                            handle_connection(stream, &token, tx).await;
                        });
                    }
                    Err(err) => {
                        tracing::debug!(%err, "bridge accept failed");
                        return;
                    }
                }
            }
        }
    }
}

/// One connection: a request line in, a response line out, until the peer
/// closes or a wrong token ends it. Arguments are never logged.
async fn handle_connection(
    stream: tokio::net::UnixStream,
    token: &str,
    tx: mpsc::Sender<Incoming>,
) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read.take(MAX_LINE)).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) if !line.trim().is_empty() => line,
            Ok(Some(_)) => continue,
            Ok(None) => return,
            Err(_) => return,
        };
        let request: BridgeRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(_) => {
                let _ = write_line(
                    &mut write,
                    &BridgeResponse::error(uuid::Uuid::nil(), "unreadable bridge request"),
                )
                .await;
                return;
            }
        };
        if request.token != token {
            let _ = write_line(
                &mut write,
                &BridgeResponse::error(request.id, "invalid bridge token"),
            )
            .await;
            return;
        }
        let id = request.id;
        let (reply_tx, reply_rx) = oneshot::channel();
        let incoming = Incoming {
            request,
            reply: reply_tx,
        };
        if tx.send(incoming).await.is_err() {
            let _ = write_line(
                &mut write,
                &BridgeResponse::error(id, "Berth is shutting down"),
            )
            .await;
            return;
        }
        let response = reply_rx
            .await
            .unwrap_or_else(|_| BridgeResponse::error(id, "Berth dropped the request"));
        if write_line(&mut write, &response).await.is_err() {
            return;
        }
    }
}

async fn write_line(
    write: &mut tokio::net::unix::OwnedWriteHalf,
    response: &BridgeResponse,
) -> std::io::Result<()> {
    let mut buf = serde_json::to_vec(response).unwrap_or_default();
    buf.push(b'\n');
    write.write_all(&buf).await?;
    write.flush().await
}

/// Sends one request to the app side and waits for its response. This is
/// the whole client half of the bridge: `berth mcp` calls it per tool call.
///
/// # Errors
/// [`AssistantError::Bridge`] when nothing answers at `socket` (Berth is
/// not running), the reply never comes, or it cannot be read.
pub async fn call(
    socket: &std::path::Path,
    req: &BridgeRequest,
) -> Result<BridgeResponse, AssistantError> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::UnixStream::connect(socket))
        .await
        .map_err(|_| not_running(socket))?
        .map_err(|_| not_running(socket))?;
    let (read, mut write) = stream.into_split();
    let mut buf = serde_json::to_vec(req)
        .map_err(|e| AssistantError::Bridge(format!("encoding the request: {e}")))?;
    buf.push(b'\n');
    write
        .write_all(&buf)
        .await
        .map_err(|e| AssistantError::Bridge(format!("sending the request: {e}")))?;
    write
        .flush()
        .await
        .map_err(|e| AssistantError::Bridge(format!("sending the request: {e}")))?;
    let mut lines = BufReader::new(read.take(MAX_LINE)).lines();
    let line = lines
        .next_line()
        .await
        .map_err(|e| AssistantError::Bridge(format!("reading the reply: {e}")))?
        .ok_or_else(|| AssistantError::Bridge("Berth closed the bridge".into()))?;
    serde_json::from_str(&line)
        .map_err(|e| AssistantError::Bridge(format!("reading the reply: {e}")))
}

fn not_running(socket: &std::path::Path) -> AssistantError {
    AssistantError::Bridge(format!(
        "Berth is not running (no answer at {}). Open Berth and try again.",
        socket.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(token: &str) -> BridgeRequest {
        BridgeRequest {
            id: uuid::Uuid::now_v7(),
            token: token.to_string(),
            session: berth_core::SessionId(uuid::Uuid::nil()),
            tool: "containers.list".to_string(),
            args: serde_json::json!({}),
            call_id: None,
        }
    }

    #[tokio::test]
    async fn bridge_round_trips_a_request_and_refuses_a_wrong_token() {
        let dir = tempfile::tempdir().expect("dir");
        let socket = dir.path().join("b.sock");
        let cancel = CancellationToken::new();
        let (server, mut rx) = BridgeServer::bind_at(socket.clone(), "tok".into(), cancel.clone())
            .await
            .expect("bind");
        assert_eq!(server.socket, socket);

        let echo = tokio::spawn(async move {
            let incoming = rx.recv().await.expect("incoming");
            assert_eq!(incoming.request.tool, "containers.list");
            let _ = incoming.reply.send(BridgeResponse::Ok {
                id: incoming.request.id,
                text: "hello".to_string(),
                data: None,
            });
            rx
        });
        let response = call(&socket, &request("tok")).await.expect("call");
        match response {
            BridgeResponse::Ok { text, .. } => assert_eq!(text, "hello"),
            other => panic!("expected Ok, got {other:?}"),
        }
        let mut rx = echo.await.expect("echo task");

        let refused = call(&socket, &request("wrong"))
            .await
            .expect("refused call");
        match refused {
            BridgeResponse::Error { message, .. } => {
                assert!(message.contains("token"), "{message}");
            }
            other => panic!("expected a token refusal, got {other:?}"),
        }
        // The refused request never reached the executor.
        assert!(rx.try_recv().is_err(), "wrong token reached the executor");

        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while socket.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cancel removes the socket");
    }

    #[tokio::test]
    async fn bridge_call_says_berth_is_not_running_when_nothing_listens() {
        let dir = tempfile::tempdir().expect("dir");
        let socket = dir.path().join("missing.sock");
        let err = call(&socket, &request("tok"))
            .await
            .expect_err("no listener");
        assert!(err.to_string().contains("not running"), "{err}");
    }

    #[test]
    fn bridge_wire_shapes_round_trip_with_call_id_defaulted() {
        let json = r#"{"id":"00000000-0000-0000-0000-000000000000","token":"t","session":"00000000-0000-0000-0000-000000000000","tool":"containers.list","args":{}}"#;
        let req: BridgeRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.call_id, None);
        let denied = BridgeResponse::Denied {
            id: req.id,
            reason: "no".to_string(),
        };
        let text = serde_json::to_string(&denied).expect("serialize");
        let back: BridgeResponse = serde_json::from_str(&text).expect("parse back");
        assert!(matches!(back, BridgeResponse::Denied { .. }));
    }
}
