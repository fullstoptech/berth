//! Build, push, export, import and commit against a canned engine: a
//! unix socket that records each request and answers with a fixed body,
//! so the request the engine sends and the events it returns are both
//! checked without a daemon.

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    auth_header: Option<String>,
    body: Vec<u8>,
}

impl Seen {
    fn param(&self, key: &str) -> Vec<&str> {
        self.query
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

/// What the canned engine answers: status line plus body. `hang` keeps
/// the response open without a byte, for cancellation.
#[derive(Clone)]
struct Reply {
    status: &'static str,
    body: String,
    hang: bool,
}

fn ok(body: &str) -> Reply {
    Reply {
        status: "200 OK",
        body: body.to_string(),
        hang: false,
    }
}

struct Fake {
    engine: BollardEngine,
    seen: Arc<Mutex<Vec<Seen>>>,
    _dir: tempfile::TempDir,
}

impl Fake {
    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().expect("lock").clone()
    }
}

async fn read_request(stream: &mut tokio::net::UnixStream) -> Option<Seen> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let mut length = 0_usize;
    let mut auth_header = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "x-registry-auth" | "x-registry-config" => {
                    auth_header = Some(value.trim().to_string());
                }
                _ => {}
            }
        }
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let parsed = url::Url::parse(&format!("http://engine{target}")).ok()?;
    Some(Seen {
        method,
        path: parsed.path().to_string(),
        query: parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect(),
        auth_header,
        body,
    })
}

/// A canned engine answering every request through `reply`.
async fn fake(reply: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Fake {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("docker.sock");
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let reply = Arc::new(reply);
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let log = log.clone();
            let reply = reply.clone();
            tokio::spawn(async move {
                let Some(request) = read_request(&mut stream).await else {
                    return;
                };
                let answer = reply(&request);
                log.lock().expect("lock").push(request);
                if answer.hang {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    return;
                }
                let head = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                    answer.status
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(answer.body.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    let docker = bollard::Docker::connect_with_unix(
        socket.to_str().expect("utf8 path"),
        10,
        bollard::API_DEFAULT_VERSION,
    )
    .expect("connect");
    Fake {
        engine: BollardEngine {
            docker,
            id: berth_core::EngineId("test".to_string()),
            docker_host: "unix://test".to_string(),
            engine_name: "test".to_string(),
        },
        seen,
        _dir: dir,
    }
}

fn inline_spec(tags: &[&str]) -> berth_core::BuildSpec {
    berth_core::BuildSpec {
        context: berth_core::BuildContext::Inline("FROM alpine\nRUN echo hi\n".to_string()),
        dockerfile: None,
        tags: tags.iter().map(ToString::to_string).collect(),
        build_args: vec![("VERSION".to_string(), "9".to_string())],
        platform: Some("linux/arm64".to_string()),
    }
}

fn tar_entries(bytes: &[u8]) -> Vec<(String, String)> {
    let mut archive = tar::Archive::new(bytes);
    archive
        .entries()
        .expect("entries")
        .map(|entry| {
            let mut entry = entry.expect("entry");
            let name = entry.path().expect("path").to_string_lossy().to_string();
            let mut text = String::new();
            std::io::Read::read_to_string(&mut entry, &mut text).expect("read");
            (name, text)
        })
        .collect()
}

const BUILD_OK: &str = concat!(
    "{\"stream\":\"Step 1/2 : FROM alpine\\n\"}\r\n",
    "{\"stream\":\" ---> abc\\n\"}\r\n",
    "{\"stream\":\"Step 2/2 : RUN ec\"}\r\n",
    "{\"stream\":\"ho hi\\n\"}\r\n",
    "{\"stream\":\"Successfully built abc\\n\"}\r\n",
);

fn build_reply(request: &Seen) -> Reply {
    if request.path.ends_with("/tag") {
        Reply {
            status: "201 Created",
            body: String::new(),
            hang: false,
        }
    } else {
        ok(BUILD_OK)
    }
}

#[tokio::test]
async fn build_image_streams_steps_and_tags_extra_names_from_the_first() {
    let fake = fake(build_reply).await;
    let spec = inline_spec(&[
        "berthfx/app:1",
        "berthfx/app:2",
        "127.0.0.1:56500/berthfx/app",
    ]);
    let events: Vec<_> = fake
        .engine
        .build_image(&spec, None, Cancel::new())
        .map(|e| e.expect("event"))
        .collect()
        .await;

    // A step split across two stream messages is one Step event.
    assert_eq!(
        events,
        [
            berth_core::BuildEvent::Step {
                index: 1,
                total: 2,
                text: "Step 1/2 : FROM alpine".to_string()
            },
            berth_core::BuildEvent::Log(" ---> abc".to_string()),
            berth_core::BuildEvent::Step {
                index: 2,
                total: 2,
                text: "Step 2/2 : RUN echo hi".to_string()
            },
            berth_core::BuildEvent::Log("Successfully built abc".to_string()),
        ]
    );

    let requests = fake.requests();
    let build = &requests[0];
    assert_eq!(build.method, "POST");
    assert!(build.path.ends_with("/build"), "{}", build.path);
    // Only the first tag goes in the build; the others are tagged after.
    assert_eq!(build.param("t"), ["berthfx/app:1"]);
    assert_eq!(build.param("dockerfile"), ["Dockerfile"]);
    assert_eq!(build.param("platform"), ["linux/arm64"]);
    assert_eq!(build.param("buildargs").len(), 1);
    assert!(build.param("buildargs")[0].contains("VERSION"));
    assert_eq!(
        tar_entries(&build.body),
        [(
            "Dockerfile".to_string(),
            "FROM alpine\nRUN echo hi\n".to_string()
        )]
    );

    let tags: Vec<_> = requests[1..]
        .iter()
        .map(|r| {
            assert!(r.path.ends_with("/tag"), "{}", r.path);
            assert!(
                r.path.contains("berthfx/app:1"),
                "tags the first: {}",
                r.path
            );
            (
                r.param("repo")[0].to_string(),
                r.param("tag")[0].to_string(),
            )
        })
        .collect();
    assert_eq!(
        tags,
        [
            ("berthfx/app".to_string(), "2".to_string()),
            (
                "127.0.0.1:56500/berthfx/app".to_string(),
                "latest".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn build_image_failure_yields_a_failed_event_then_the_error_and_no_tags() {
    let body = concat!(
        "{\"stream\":\"Step 1/2 : FROM alpine\\n\"}\r\n",
        "{\"stream\":\"Step 2/2 : RUN false\\n\"}\r\n",
        "{\"errorDetail\":{\"code\":1,\"message\":\"The command '/bin/sh -c false' returned a non-zero code: 1\"},",
        "\"error\":\"The command '/bin/sh -c false' returned a non-zero code: 1\"}\r\n",
    );
    let fake = fake(move |_| ok(body)).await;
    let spec = inline_spec(&["berthfx/app:1", "berthfx/app:2"]);
    let items: Vec<_> = fake
        .engine
        .build_image(&spec, None, Cancel::new())
        .collect()
        .await;

    assert_eq!(items.len(), 4, "{items:?}");
    assert!(matches!(
        &items[1],
        Ok(berth_core::BuildEvent::Step { index: 2, .. })
    ));
    assert_eq!(
        items[2].as_ref().expect("failed event"),
        &berth_core::BuildEvent::Failed {
            text: "The command '/bin/sh -c false' returned a non-zero code: 1".to_string()
        }
    );
    assert!(
        matches!(&items[3], Err(EngineError::Docker(m)) if m.contains("non-zero code")),
        "{:?}",
        items[3]
    );
    assert_eq!(fake.requests().len(), 1, "a failed build tags nothing");
}

#[tokio::test]
async fn build_image_ends_quietly_when_cancelled() {
    let fake = fake(|_| Reply {
        status: "200 OK",
        body: String::new(),
        hang: true,
    })
    .await;
    let cancel = Cancel::new();
    let mut stream =
        fake.engine
            .build_image(&inline_spec(&["berthfx/app:1"]), None, cancel.clone());
    let waiting = tokio::spawn(async move { stream.next().await });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    cancel.cancel();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .expect("the stream must end after cancel")
        .expect("join");
    assert!(first.is_none(), "{first:?}");
}

#[tokio::test]
async fn build_image_folder_context_honours_dockerignore_and_dockerfile_name() {
    let folder = tempfile::tempdir().expect("tempdir");
    std::fs::write(folder.path().join("Dockerfile.dev"), "FROM scratch\n").expect("write");
    std::fs::write(folder.path().join("keep.txt"), "keep").expect("write");
    std::fs::write(folder.path().join("secret.env"), "TOKEN=x").expect("write");
    std::fs::write(folder.path().join(".dockerignore"), "secret.env\n").expect("write");
    let fake = fake(build_reply).await;
    let spec = berth_core::BuildSpec {
        context: berth_core::BuildContext::Folder(folder.path().to_path_buf()),
        dockerfile: Some("Dockerfile.dev".to_string()),
        tags: vec!["berthfx/app:1".to_string()],
        build_args: Vec::new(),
        platform: None,
    };
    let items: Vec<_> = fake
        .engine
        .build_image(&spec, None, Cancel::new())
        .collect()
        .await;
    assert!(items.iter().all(Result::is_ok), "{items:?}");
    let request = &fake.requests()[0];
    assert_eq!(request.param("dockerfile"), ["Dockerfile.dev"]);
    assert!(request.param("buildargs").is_empty());
    let names: Vec<String> = tar_entries(&request.body)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(names.contains(&"Dockerfile.dev".to_string()), "{names:?}");
    assert!(names.contains(&"keep.txt".to_string()), "{names:?}");
    assert!(!names.contains(&"secret.env".to_string()), "{names:?}");
}

fn decode_registry_header(value: &str) -> String {
    let mut bits = 0_u32;
    let mut count = 0;
    let mut out = Vec::new();
    for c in value.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(v);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push(u8::try_from((bits >> count) & 0xff).expect("byte"));
        }
    }
    String::from_utf8(out).expect("utf8 header")
}

#[tokio::test]
async fn push_sends_the_tag_and_credentials_and_maps_progress() {
    let body = concat!(
        "{\"status\":\"The push refers to repository [127.0.0.1:56500/berthfx/app]\"}\r\n",
        "{\"status\":\"Pushing\",\"progressDetail\":{\"current\":5,\"total\":10},\"id\":\"abc\"}\r\n",
        "{\"status\":\"Pushed\"}\r\n",
    );
    let fake = fake(move |_| ok(body)).await;
    let auth = crate::client::RegistryAuth {
        server: "127.0.0.1:56500".to_string(),
        username: "fx-user".to_string(),
        password: secrecy::SecretString::from("fx-pass".to_string()),
    };
    let events: Vec<_> = fake
        .engine
        .push("127.0.0.1:56500/berthfx/app:1", Some(&auth), Cancel::new())
        .map(|e| e.expect("progress"))
        .collect()
        .await;
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[1].status, "Pushing");
    assert_eq!((events[1].current, events[1].total), (Some(5), Some(10)));
    assert_eq!(events[2].status, "Pushed");

    let request = &fake.requests()[0];
    assert!(
        request
            .path
            .ends_with("/images/127.0.0.1:56500/berthfx/app/push"),
        "{}",
        request.path
    );
    assert_eq!(request.param("tag"), ["1"]);
    let header = decode_registry_header(request.auth_header.as_deref().expect("auth header"));
    assert!(
        header.contains("fx-user") && header.contains("fx-pass"),
        "{header}"
    );
    assert!(header.contains("127.0.0.1:56500"), "{header}");
}

#[tokio::test]
async fn push_without_a_tag_pushes_latest_and_a_registry_error_ends_the_stream() {
    let body = concat!(
        "{\"status\":\"Preparing\"}\r\n",
        "{\"errorDetail\":{\"message\":\"denied: requested access to the resource is denied\"},",
        "\"error\":\"denied: requested access to the resource is denied\"}\r\n",
    );
    let fake = fake(move |_| ok(body)).await;
    let items: Vec<_> = fake
        .engine
        .push("127.0.0.1:56500/berthfx/app", None, Cancel::new())
        .collect()
        .await;
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(matches!(&items[1], Err(EngineError::Docker(m)) if m.contains("denied")));
    assert_eq!(fake.requests()[0].param("tag"), ["latest"]);
}

#[tokio::test]
async fn export_image_streams_the_tar_bytes_of_the_named_image() {
    let fake = fake(|_| ok("TAR-BYTES-OF-THE-IMAGE")).await;
    let chunks: Vec<_> = fake
        .engine
        .export_image("127.0.0.1:56500/berthfx/app:1", Cancel::new())
        .map(|c| c.expect("chunk"))
        .collect()
        .await;
    assert_eq!(chunks.concat(), b"TAR-BYTES-OF-THE-IMAGE");
    let request = &fake.requests()[0];
    assert_eq!(request.method, "GET");
    assert!(
        request
            .path
            .ends_with("/images/127.0.0.1:56500/berthfx/app:1/get"),
        "{}",
        request.path
    );
}

#[tokio::test]
async fn import_image_returns_the_loaded_names_and_sends_the_tar() {
    let body = concat!(
        "{\"stream\":\"Loaded image: 127.0.0.1:56500/berthfx/app:1\\n\"}\r\n",
        "{\"stream\":\"Loaded image ID: sha256:feed\\n\"}\r\n",
    );
    let fake = fake(move |_| ok(body)).await;
    let names = fake
        .engine
        .import_image(bytes::Bytes::from_static(b"the-tar"))
        .await
        .expect("import");
    assert_eq!(names, ["127.0.0.1:56500/berthfx/app:1", "sha256:feed"]);
    let request = &fake.requests()[0];
    assert_eq!(request.method, "POST");
    assert!(request.path.ends_with("/images/load"), "{}", request.path);
    assert_eq!(request.body, b"the-tar");
}

#[tokio::test]
async fn import_image_reports_an_engine_error() {
    let body = "{\"errorDetail\":{\"message\":\"invalid tar header\"},\"error\":\"invalid tar header\"}\r\n";
    let fake = fake(move |_| ok(body)).await;
    let result = fake
        .engine
        .import_image(bytes::Bytes::from_static(b"junk"))
        .await;
    assert!(
        matches!(&result, Err(EngineError::Docker(m)) if m.contains("invalid tar")),
        "{result:?}"
    );
}

#[tokio::test]
async fn commit_pauses_the_container_and_returns_the_new_image_id() {
    let fake = fake(|_| Reply {
        status: "201 Created",
        body: "{\"Id\":\"sha256:c0ffee\"}".to_string(),
        hang: false,
    })
    .await;
    let id = fake
        .engine
        .commit(
            &berth_core::ContainerId("cafe01".to_string()),
            "berthfx/web-snap",
            "1",
        )
        .await
        .expect("commit");
    assert_eq!(id, "sha256:c0ffee");
    let request = &fake.requests()[0];
    assert!(request.path.ends_with("/commit"), "{}", request.path);
    assert_eq!(request.param("container"), ["cafe01"]);
    assert_eq!(request.param("repo"), ["berthfx/web-snap"]);
    assert_eq!(request.param("tag"), ["1"]);
    assert_eq!(request.param("pause"), ["true"]);
}
