//! Where engines come from (D29): Colima profiles, docker contexts whose
//! endpoint is a unix socket not already covered by a profile, and engines
//! the user added. Socket and context lookup lives only here.

use crate::colima::ColimaControl;

#[derive(Debug, Clone, PartialEq)]
pub struct DockerContext {
    pub name: String,
    pub host: String,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserEngineRef {
    pub id: berth_core::EngineId,
    pub name: String,
    pub kind: berth_core::EngineKind,
}

#[derive(serde::Deserialize)]
struct DockerConfig {
    #[serde(rename = "currentContext", default)]
    current_context: Option<String>,
}

#[derive(serde::Deserialize)]
struct ContextMeta {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Endpoints", default)]
    endpoints: std::collections::BTreeMap<String, ContextEndpoint>,
}

#[derive(serde::Deserialize)]
struct ContextEndpoint {
    #[serde(rename = "Host", default)]
    host: String,
}

/// $DOCKER_HOST, then ~/.docker/config.json currentContext, then
/// ~/.docker/contexts/meta/*/meta.json. Pure over env + home.
pub fn contexts_with(
    env: &dyn Fn(&str) -> Option<String>,
    home: &std::path::Path,
) -> Vec<DockerContext> {
    let docker_dir = home.join(".docker");
    let current_name = std::fs::read_to_string(docker_dir.join("config.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<DockerConfig>(&text).ok())
        .and_then(|config| config.current_context);

    let mut metas: Vec<ContextMeta> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(docker_dir.join("contexts/meta")) {
        let mut paths: Vec<std::path::PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("meta.json"))
            .collect();
        paths.sort();
        for path in paths {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(meta) = serde_json::from_str::<ContextMeta>(&text) {
                    metas.push(meta);
                }
            }
        }
    }
    metas.sort_by(|a, b| a.name.cmp(&b.name));

    let mut contexts: Vec<DockerContext> = metas
        .into_iter()
        .filter_map(|meta| {
            let host = meta.endpoints.get("docker")?.host.clone();
            if host.is_empty() {
                return None;
            }
            Some(DockerContext {
                current: current_name.as_deref() == Some(meta.name.as_str()),
                name: meta.name,
                host,
            })
        })
        .collect();

    // $DOCKER_HOST wins over every configured context (Ferry ADR 0003).
    if let Some(host) = env("DOCKER_HOST").filter(|h| !h.is_empty()) {
        for ctx in &mut contexts {
            ctx.current = false;
        }
        contexts.insert(
            0,
            DockerContext {
                name: "env".to_string(),
                host,
                current: true,
            },
        );
    }
    contexts
}

pub fn contexts() -> Vec<DockerContext> {
    contexts_with(&|key| std::env::var(key).ok(), &berth_sys::paths::home())
}

fn host_of(urlish: &str) -> String {
    // "ssh://user@host:22" -> "host"; "tcp://10.0.0.5:2376" -> "10.0.0.5:2376"
    let after_scheme = urlish.split("://").nth(1).unwrap_or(urlish);
    let after_user = after_scheme.rsplit('@').next().unwrap_or(after_scheme);
    after_user.trim_end_matches('/').to_string()
}

pub fn engine_id_for_context(ctx: &DockerContext) -> berth_core::EngineId {
    if ctx.name == "env" {
        if let Some(path) = ctx.host.strip_prefix("unix://") {
            return berth_core::EngineId(format!("socket:{path}"));
        }
        if ctx.host.starts_with("ssh://") {
            let host_port = host_of(&ctx.host);
            let host = host_port
                .rsplit_once(':')
                .map_or(host_port.as_str(), |(h, _)| h);
            return berth_core::EngineId(format!("ssh:{host}"));
        }
        if ctx.host.starts_with("tcp://") {
            return berth_core::EngineId(format!("tcp:{}", host_of(&ctx.host)));
        }
        return berth_core::EngineId("context:env".to_string());
    }
    berth_core::EngineId(format!("context:{}", ctx.name))
}

fn engine_for_profile(profile: &berth_core::ColimaProfile) -> berth_core::Engine {
    let state = if profile.runtime != berth_core::Runtime::Docker {
        berth_core::EngineState::Unsupported {
            reason: "Not supported in this version".to_string(),
        }
    } else if profile.running {
        berth_core::EngineState::Running
    } else {
        berth_core::EngineState::Stopped
    };
    berth_core::Engine {
        id: berth_core::EngineId(format!("colima:{}", profile.name)),
        name: profile.name.clone(),
        kind: berth_core::EngineKind::Colima {
            profile: profile.name.clone(),
        },
        state,
        docker_host: profile
            .docker_socket
            .as_ref()
            .map(|path| format!("unix://{}", path.display())),
        docker_version: None,
        resources: profile
            .vm_type
            .clone()
            .map(|vm_type| berth_core::VmResources {
                cpus: profile.cpus,
                memory: profile.memory,
                disk: profile.disk,
                arch: profile.arch.clone(),
                vm_type,
                rosetta: profile.rosetta.unwrap_or(false),
                runtime: profile.runtime,
                mount_type: profile.mount_type.clone(),
            }),
        counts: berth_core::ResourceCounts::default(),
        owner: berth_core::OwnerId::Local,
    }
}

fn engine_for_context(ctx: &DockerContext) -> berth_core::Engine {
    let socket = std::path::PathBuf::from(ctx.host.strip_prefix("unix://").unwrap_or(&ctx.host));
    let state = if socket.exists() {
        berth_core::EngineState::Running
    } else {
        berth_core::EngineState::Unreachable {
            reason: format!("socket {} not found", socket.display()),
            last_seen: None,
        }
    };
    berth_core::Engine {
        id: engine_id_for_context(ctx),
        name: ctx.name.clone(),
        kind: berth_core::EngineKind::Socket {
            path: socket,
            context: (ctx.name != "env").then(|| ctx.name.clone()),
        },
        state,
        docker_host: Some(ctx.host.clone()),
        docker_version: None,
        resources: None,
        counts: berth_core::ResourceCounts::default(),
        owner: berth_core::OwnerId::Local,
    }
}

fn engine_for_user(user: &UserEngineRef) -> berth_core::Engine {
    let (state, docker_host) = match &user.kind {
        berth_core::EngineKind::Socket { path, .. } => (
            if path.exists() {
                berth_core::EngineState::Running
            } else {
                berth_core::EngineState::Unreachable {
                    reason: format!("socket {} not found", path.display()),
                    last_seen: None,
                }
            },
            Some(format!("unix://{}", path.display())),
        ),
        berth_core::EngineKind::Remote { transport } => (
            // Optimistic, like an existing socket (AD41): the engines
            // service connects it and EngineFailed flips a real failure
            // to Unreachable with its last_seen.
            berth_core::EngineState::Running,
            Some(crate::remote::docker_host(transport)),
        ),
        berth_core::EngineKind::Colima { .. } => (berth_core::EngineState::Stopped, None),
    };
    berth_core::Engine {
        id: user.id.clone(),
        name: user.name.clone(),
        kind: user.kind.clone(),
        state,
        docker_host,
        docker_version: None,
        resources: None,
        counts: berth_core::ResourceCounts::default(),
        owner: berth_core::OwnerId::Local,
    }
}

/// `colima list` carries no socket and `colima status` can fail (Linux without
/// lima), so a running profile may arrive without one. Fall back to the
/// profile directory's socket, then to the profile's docker context
/// (`colima`, or `colima-<profile>`), whichever exists on disk.
fn socket_fallback(
    profile: &berth_core::ColimaProfile,
    contexts: &[DockerContext],
    colima_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let in_dir = colima_dir.join(&profile.name).join("docker.sock");
    if in_dir.exists() {
        return Some(in_dir);
    }
    let context_name = if profile.name == "default" {
        "colima".to_string()
    } else {
        format!("colima-{}", profile.name)
    };
    contexts
        .iter()
        .filter(|ctx| ctx.name == context_name)
        .filter_map(|ctx| ctx.host.strip_prefix("unix://"))
        .map(std::path::PathBuf::from)
        .find(|socket| socket.exists())
}

async fn discover_with(
    colima: &dyn ColimaControl,
    user: &[UserEngineRef],
    contexts: &[DockerContext],
    colima_dir: &std::path::Path,
) -> Vec<berth_core::Engine> {
    let mut engines = Vec::new();
    let mut covered_sockets: Vec<std::path::PathBuf> = Vec::new();

    if colima.binary().is_some() {
        match colima.profiles().await {
            Ok(profiles) => {
                for profile in &profiles {
                    let mut profile = profile.clone();
                    if profile.running && profile.docker_socket.is_none() {
                        profile.docker_socket = socket_fallback(&profile, contexts, colima_dir);
                    }
                    covered_sockets.push(
                        profile
                            .docker_socket
                            .clone()
                            .unwrap_or_else(|| colima_dir.join(&profile.name).join("docker.sock")),
                    );
                    engines.push(engine_for_profile(&profile));
                }
            }
            Err(err) => tracing::warn!(%err, "colima profiles unavailable during discovery"),
        }
    }

    for ctx in contexts {
        let Some(socket) = ctx.host.strip_prefix("unix://") else {
            continue;
        };
        if covered_sockets.contains(&std::path::PathBuf::from(socket)) {
            continue;
        }
        engines.push(engine_for_context(ctx));
    }

    for user_engine in user {
        engines.push(engine_for_user(user_engine));
    }
    engines
}

/// Colima profiles + unix-socket contexts not covered by a profile + user engines (D29).
pub async fn discover(
    colima: &dyn ColimaControl,
    user: &[UserEngineRef],
) -> Vec<berth_core::Engine> {
    let ctxs = contexts();
    discover_with(colima, user, &ctxs, &berth_sys::paths::colima_dir()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colima::MockColima;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn write_context_home(dir: &std::path::Path) {
        let meta_dir = dir.join(".docker/contexts/meta/deadbeef");
        std::fs::create_dir_all(&meta_dir).expect("mkdir");
        std::fs::write(
            dir.join(".docker/config.json"),
            "{\"currentContext\":\"colima\"}",
        )
        .expect("config");
        std::fs::write(
            meta_dir.join("meta.json"),
            "{\"Name\":\"colima\",\"Metadata\":{},\"Endpoints\":{\"docker\":{\"Host\":\"unix:///tmp/colima.sock\"}}}",
        )
        .expect("meta");
    }

    #[test]
    fn discovery_contexts_from_temp_home_marks_current() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_context_home(dir.path());
        let ctxs = contexts_with(&no_env, dir.path());
        assert_eq!(
            ctxs,
            [DockerContext {
                name: "colima".to_string(),
                host: "unix:///tmp/colima.sock".to_string(),
                current: true,
            }]
        );
    }

    #[test]
    fn discovery_docker_host_wins_over_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_context_home(dir.path());
        let env =
            |key: &str| (key == "DOCKER_HOST").then(|| "unix:///tmp/from-env.sock".to_string());
        let ctxs = contexts_with(&env, dir.path());
        assert_eq!(ctxs.len(), 2);
        assert_eq!(ctxs[0].name, "env");
        assert!(ctxs[0].current);
        assert!(!ctxs[1].current, "config context loses to DOCKER_HOST");
    }

    #[test]
    fn discovery_engine_ids_per_context_kind() {
        let named = DockerContext {
            name: "colima".to_string(),
            host: "unix:///tmp/c.sock".to_string(),
            current: true,
        };
        assert_eq!(
            engine_id_for_context(&named),
            berth_core::EngineId("context:colima".into())
        );
        let env_unix = DockerContext {
            name: "env".to_string(),
            host: "unix:///tmp/e.sock".to_string(),
            current: true,
        };
        assert_eq!(
            engine_id_for_context(&env_unix),
            berth_core::EngineId("socket:/tmp/e.sock".into())
        );
        let env_ssh = DockerContext {
            name: "env".to_string(),
            host: "ssh://me@prod-1:2222".to_string(),
            current: true,
        };
        assert_eq!(
            engine_id_for_context(&env_ssh),
            berth_core::EngineId("ssh:prod-1".into())
        );
        let env_tcp = DockerContext {
            name: "env".to_string(),
            host: "tcp://10.0.0.5:2376".to_string(),
            current: true,
        };
        assert_eq!(
            engine_id_for_context(&env_tcp),
            berth_core::EngineId("tcp:10.0.0.5:2376".into())
        );
    }

    fn profile(
        name: &str,
        running: bool,
        runtime: berth_core::Runtime,
    ) -> berth_core::ColimaProfile {
        berth_core::ColimaProfile {
            name: name.to_string(),
            running,
            arch: berth_core::Arch::Aarch64,
            cpus: 2,
            memory: bytesize::ByteSize(1024),
            disk: bytesize::ByteSize(2048),
            runtime,
            vm_type: Some(berth_core::VmType::Vz),
            rosetta: Some(false),
            mount_type: None,
            docker_socket: Some(std::path::PathBuf::from("/tmp/colima.sock")),
        }
    }

    #[tokio::test]
    async fn discovery_merges_profiles_contexts_and_user_engines() {
        let colima = MockColima::new(vec![
            profile("default", true, berth_core::Runtime::Docker),
            profile("work", false, berth_core::Runtime::Docker),
            profile("lab", true, berth_core::Runtime::Containerd),
        ]);
        let ctxs = vec![
            DockerContext {
                name: "colima".to_string(),
                host: "unix:///tmp/colima.sock".to_string(),
                current: true,
            },
            DockerContext {
                name: "extra".to_string(),
                host: "unix:///tmp/extra.sock".to_string(),
                current: false,
            },
            DockerContext {
                name: "remote-only".to_string(),
                host: "ssh://prod".to_string(),
                current: false,
            },
        ];
        let user = [UserEngineRef {
            id: berth_core::EngineId("ssh:hetzner-1".to_string()),
            name: "hetzner-1".to_string(),
            kind: berth_core::EngineKind::Remote {
                transport: berth_core::RemoteTransport::Ssh {
                    host: "hetzner-1".to_string(),
                    user: None,
                    port: None,
                },
            },
        }];
        let engines = discover_with(
            &colima,
            &user,
            &ctxs,
            std::path::Path::new("/tmp/colima-home"),
        )
        .await;
        let ids: Vec<&str> = engines.iter().map(|e| e.id.0.as_str()).collect();
        assert_eq!(
            ids,
            [
                "colima:default",
                "colima:work",
                "colima:lab",
                "context:extra",
                "ssh:hetzner-1"
            ],
            "covered context skipped, ssh context skipped, user engine kept"
        );
        assert_eq!(engines[0].state, berth_core::EngineState::Running);
        assert_eq!(engines[1].state, berth_core::EngineState::Stopped);
        assert!(matches!(
            engines[2].state,
            berth_core::EngineState::Unsupported { .. }
        ));
        assert_eq!(
            engines[0].docker_host.as_deref(),
            Some("unix:///tmp/colima.sock")
        );
        assert!(engines[0].resources.is_some());
    }

    #[tokio::test]
    async fn discovery_running_profile_without_status_socket_uses_context_socket() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("ctx.sock");
        std::fs::write(&socket, "").expect("socket stand-in");
        let mut default = profile("default", true, berth_core::Runtime::Docker);
        default.docker_socket = None;
        let mut work = profile("work", true, berth_core::Runtime::Docker);
        work.docker_socket = None;
        let colima = MockColima::new(vec![default, work]);
        let ctxs = [
            DockerContext {
                name: "colima".to_string(),
                host: format!("unix://{}", socket.display()),
                current: true,
            },
            DockerContext {
                name: "colima-work".to_string(),
                host: "unix:///tmp/berth-no-such.sock".to_string(),
                current: false,
            },
        ];
        let engines = discover_with(&colima, &[], &ctxs, &dir.path().join("colima-home")).await;
        assert_eq!(engines[0].id.0, "colima:default");
        assert_eq!(engines[0].state, berth_core::EngineState::Running);
        assert_eq!(
            engines[0].docker_host,
            Some(format!("unix://{}", socket.display()))
        );
        assert_eq!(engines[1].id.0, "colima:work");
        assert_eq!(engines[1].docker_host, None, "no socket on disk, no guess");
    }

    #[tokio::test]
    async fn discovery_running_profile_without_status_socket_uses_profile_dir_socket() {
        let dir = tempfile::tempdir().expect("tempdir");
        let profile_dir = dir.path().join("default");
        std::fs::create_dir_all(&profile_dir).expect("mkdir");
        std::fs::write(profile_dir.join("docker.sock"), "").expect("socket stand-in");
        let mut default = profile("default", true, berth_core::Runtime::Docker);
        default.docker_socket = None;
        let engines = discover_with(&MockColima::new(vec![default]), &[], &[], dir.path()).await;
        assert_eq!(
            engines[0].docker_host,
            Some(format!(
                "unix://{}",
                profile_dir.join("docker.sock").display()
            ))
        );
    }

    #[tokio::test]
    async fn discovery_missing_context_socket_is_unreachable() {
        let colima = MockColima::new(vec![]);
        let ctxs = [DockerContext {
            name: "gone".to_string(),
            host: "unix:///tmp/berth-no-such.sock".to_string(),
            current: false,
        }];
        let engines = discover_with(&colima, &[], &ctxs, std::path::Path::new("/tmp/x")).await;
        assert_eq!(engines.len(), 1);
        assert!(matches!(
            engines[0].state,
            berth_core::EngineState::Unreachable { .. }
        ));
    }
}
