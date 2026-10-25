//! The mock world (P1-T8, PLAN D58): `fixtures/mock/berth.json` loaded by
//! [`MockEngine`], [`MockColima`] and [`MockCompose`] for `BERTH_MOCK=1`
//! runs and UI scenes.
//!
//! Schema (all payloads reuse the `berth_core` serde shapes, byte sizes as
//! plain byte counts which `bytesize` accepts): `engines` (with state),
//! `engine_infos`, `colima_profiles` and `pins` by value; `projects` as the
//! frames show them (status, folder, git branch, service summaries);
//! `containers` with labels/ports/`stats`; `details` overrides per container
//! name; `images_by_engine`, `volumes_by_engine`, `networks_by_engine` and
//! `disk_usage_by_engine` keyed by engine id; scripted `logs` and
//! `stats_series` per container name; `compose_services` per project for
//! [`MockCompose::config`].

pub mod colima;
pub mod compose;
pub mod engine;

pub use colima::MockColima;
pub use compose::MockCompose;
pub use engine::MockEngine;

use crate::EngineError;

/// The bundled mock-world fixture, compiled in (PLAN section 6.5).
pub const BUNDLED_JSON: &str = include_str!("../../../../fixtures/mock/berth.json");

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ScriptedLog {
    pub stream: berth_core::Stream,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct DetailOverride {
    pub entrypoint: Option<Vec<String>>,
    pub cmd: Option<Vec<String>>,
    #[serde(default)]
    pub env: Vec<berth_core::EnvVar>,
    pub working_dir: Option<String>,
    pub user: Option<String>,
    pub restart: berth_core::RestartPolicy,
    #[serde(default)]
    pub limits: berth_core::Limits,
}

/// One parsed `fixtures/mock/berth.json`.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct MockWorld {
    #[serde(default)]
    pub engines: Vec<berth_core::Engine>,
    #[serde(default)]
    pub engine_infos: std::collections::BTreeMap<String, berth_core::EngineInfo>,
    #[serde(default)]
    pub colima_profiles: Vec<berth_core::ColimaProfile>,
    #[serde(default)]
    pub pins: Vec<berth_core::PinnedFolder>,
    #[serde(default)]
    pub projects: Vec<berth_core::Project>,
    #[serde(default)]
    pub containers: Vec<berth_core::Container>,
    #[serde(default)]
    pub details: std::collections::BTreeMap<String, DetailOverride>,
    #[serde(default)]
    pub images_by_engine: std::collections::BTreeMap<String, Vec<berth_core::Image>>,
    #[serde(default)]
    pub volumes_by_engine: std::collections::BTreeMap<String, Vec<berth_core::Volume>>,
    #[serde(default)]
    pub networks_by_engine: std::collections::BTreeMap<String, Vec<berth_core::Network>>,
    #[serde(default)]
    pub disk_usage_by_engine: std::collections::BTreeMap<String, berth_core::DiskUsage>,
    #[serde(default)]
    pub logs: std::collections::BTreeMap<String, Vec<ScriptedLog>>,
    #[serde(default)]
    pub stats_series: std::collections::BTreeMap<String, Vec<berth_core::LiveStats>>,
    /// Project name -> the `services` object of `compose config --format json`.
    #[serde(default)]
    pub compose_services: std::collections::BTreeMap<String, serde_json::Value>,
}

impl MockWorld {
    pub fn from_json(json: &str) -> Result<Self, EngineError> {
        serde_json::from_str(json).map_err(|err| EngineError::Parse {
            what: "mock world fixture".to_string(),
            message: err.to_string(),
        })
    }

    /// The bundled world. A world that fails to parse degrades to empty
    /// rather than panicking (PLAN types `bundled` as infallible); the
    /// `mock_world_matches_brief` test guards the shipped fixture.
    #[must_use]
    pub fn bundled() -> Self {
        Self::from_json(BUNDLED_JSON).unwrap_or_default()
    }

    #[must_use]
    pub fn engine(&self, id: &berth_core::EngineId) -> Option<&berth_core::Engine> {
        self.engines.iter().find(|e| e.id == *id)
    }

    #[must_use]
    pub fn containers_for(&self, engine: &berth_core::EngineId) -> Vec<berth_core::Container> {
        self.containers
            .iter()
            .filter(|c| c.engine == *engine)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::EngineClient;
    use futures::StreamExt;

    fn engine_id(s: &str) -> berth_core::EngineId {
        berth_core::EngineId(s.to_string())
    }

    /// The frames' sidebar badges are canonical: the default engine has 23
    /// containers (15 running), 31 images and 12 volumes, and every
    /// engine's stored counts equal the lists the engine serves.
    #[tokio::test]
    async fn mock_engine_counts_match_the_frames_and_their_lists() {
        let world = MockWorld::bundled();
        let default = world.engine(&engine_id("colima:default")).expect("default");
        assert_eq!(default.counts.containers_total, 23);
        assert_eq!(default.counts.containers_running, 15);
        assert_eq!(default.counts.images, 31);
        assert_eq!(default.counts.volumes, 12);
        for engine in &world.engines {
            let client = MockEngine::bundled(&engine.id);
            let containers = client.containers().await.expect("containers");
            assert_eq!(
                containers.len(),
                engine.counts.containers_total as usize,
                "{} containers",
                engine.id.0
            );
            assert_eq!(
                containers
                    .iter()
                    .filter(|c| c.state == berth_core::ContainerState::Running)
                    .count(),
                engine.counts.containers_running as usize,
                "{} running",
                engine.id.0
            );
            assert_eq!(
                client.images().await.expect("images").len(),
                engine.counts.images as usize,
                "{} images",
                engine.id.0
            );
            assert_eq!(
                client.volumes().await.expect("volumes").len(),
                engine.counts.volumes as usize,
                "{} volumes",
                engine.id.0
            );
        }
        // Unassigned on the default engine: the frame's loose containers
        // are 6 running, 1 paused and eager_hopper exited.
        let loose: Vec<_> = world
            .containers_for(&engine_id("colima:default"))
            .into_iter()
            .filter(|c| c.compose_project.is_none())
            .collect();
        assert_eq!(loose.len(), 8, "{loose:?}");
    }

    #[tokio::test]
    async fn mock_engine_tag_image_adds_a_name_that_resolves_to_the_same_image() {
        let client = MockEngine::bundled(&engine_id("colima:default"));
        let before = client.images().await.expect("images");
        let source = before
            .iter()
            .find(|i| i.repo_tags.iter().any(|t| t == "postgres:17-alpine"))
            .expect("postgres image")
            .clone();
        client
            .tag_image(&source.id, "berthfx/pg", "mine")
            .await
            .expect("tag");
        let after = client.images().await.expect("images");
        assert_eq!(after.len(), before.len(), "a tag adds no image");
        let tagged = after.iter().find(|i| i.id == source.id).expect("image");
        assert!(tagged.repo_tags.contains(&"berthfx/pg:mine".to_string()));
        assert!(tagged.repo_tags.contains(&"postgres:17-alpine".to_string()));
        let by_new_name = client.image("berthfx/pg:mine").await.expect("by name");
        assert_eq!(by_new_name.image.id, source.id);
        // Tagging an image that is not there says so.
        let missing = client.tag_image("sha256:nope", "x", "y").await;
        assert!(
            matches!(missing, Err(crate::EngineError::NotFound(_))),
            "{missing:?}"
        );
    }

    #[tokio::test]
    async fn mock_world_matches_brief() {
        let world = MockWorld::bundled();
        // 3 engines in the three states.
        assert_eq!(world.engines.len(), 3, "{:?}", world.engines);
        let by_id = |id: &str| world.engine(&engine_id(id)).expect(id);
        let default = by_id("colima:default");
        assert_eq!(default.state, berth_core::EngineState::Running);
        assert_eq!(default.docker_version.as_deref(), Some("28.4.0"));
        let res = default.resources.as_ref().expect("resources");
        assert_eq!(res.cpus, 4);
        assert_eq!(berth_core::fmt::bytes(res.memory), "8 GB");
        assert_eq!(by_id("colima:work").state, berth_core::EngineState::Stopped);
        assert!(matches!(
            by_id("ssh:hetzner-1").state,
            berth_core::EngineState::Unreachable { .. }
        ));
        // 7 projects on the default engine with the brief's statuses.
        let local: Vec<&berth_core::Project> = world
            .projects
            .iter()
            .filter(|p| p.engine == engine_id("colima:default"))
            .collect();
        assert_eq!(local.len(), 7, "{local:?}");
        let status = |name: &str| local.iter().find(|p| p.name == name).expect(name).status;
        assert_eq!(status("shop-api"), berth_core::ProjectStatus::Failing);
        assert_eq!(
            status("analytics"),
            berth_core::ProjectStatus::Partial {
                running: 3,
                total: 4
            }
        );
        assert_eq!(status("berth-site"), berth_core::ProjectStatus::AllRunning);
        assert_eq!(
            status("payments-mock"),
            berth_core::ProjectStatus::AllRunning
        );
        assert_eq!(status("ml-notebooks"), berth_core::ProjectStatus::Stopped);
        assert_eq!(status("docs-site"), berth_core::ProjectStatus::Stopped);
        assert_eq!(
            status("docker-manger"),
            berth_core::ProjectStatus::NotCreated
        );
        let manger = local
            .iter()
            .find(|p| p.name == "docker-manger")
            .expect("docker-manger");
        assert!(manger.pinned && manger.services.is_empty());
        // >= 40 containers across the three mock engines.
        let mut total = 0;
        for id in ["colima:default", "colima:work", "ssh:hetzner-1"] {
            let engine = MockEngine::bundled(&engine_id(id));
            total += engine.containers().await.expect("containers").len();
        }
        assert!(total >= 40, "only {total} containers");
        assert_eq!(world.containers.len(), total);
        // Exactly one amd64 image, and it sits on the arm64 default engine.
        let engine = MockEngine::bundled(&engine_id("colima:default"));
        assert_eq!(
            engine.info().await.expect("info").arch,
            berth_core::Arch::Aarch64
        );
        let all_images: Vec<&berth_core::Image> =
            world.images_by_engine.values().flatten().collect();
        let amd64: Vec<_> = all_images
            .iter()
            .filter(|i| i.arch == berth_core::Arch::X86_64)
            .collect();
        assert_eq!(amd64.len(), 1, "{amd64:?}");
        let on_default = engine.images().await.expect("images");
        assert!(on_default
            .iter()
            .any(|i| i.arch == berth_core::Arch::X86_64));
        // A 1.2 GB volume, and the restart loop at 14.
        let sizes = engine.volume_sizes().await.expect("volume sizes");
        let pgdata = sizes.get("shop-api_pgdata").expect("shop-api_pgdata");
        assert_eq!(berth_core::fmt::bytes(*pgdata), "1.2 GB");
        let worker = world
            .containers
            .iter()
            .find(|c| c.name == "shop-api-worker-1")
            .expect("worker");
        assert_eq!(worker.restart_count, 14);
        assert_eq!(worker.state, berth_core::ContainerState::Restarting);
        let db = world
            .containers
            .iter()
            .find(|c| c.name == "shop-api-db-1")
            .expect("db");
        assert_eq!(db.state, berth_core::ContainerState::Exited);
        assert_eq!(db.exit_code, Some(1));
        // The frames' unassigned trio.
        for name in ["pg-scratch", "eager_hopper", "api-worker-7f9c4d8b6-xk2lp"] {
            assert!(
                world.containers.iter().any(|c| c.name == name),
                "missing {name}"
            );
        }
        let paused = world
            .containers
            .iter()
            .find(|c| c.name == "api-worker-7f9c4d8b6-xk2lp")
            .expect("paused");
        assert_eq!(paused.state, berth_core::ContainerState::Paused);
    }

    #[tokio::test]
    async fn mock_stop_running_container_yields_exited_plus_one_event() {
        let engine = MockEngine::bundled(&engine_id("colima:default"));
        let containers = engine.containers().await.expect("containers");
        let api = containers
            .iter()
            .find(|c| c.name == "shop-api-api-1")
            .expect("api")
            .clone();
        assert_eq!(api.state, berth_core::ContainerState::Running);
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut events = engine.events(cancel.clone());
        engine
            .act(
                &api.id,
                berth_core::ContainerAction::Stop { timeout_s: None },
            )
            .await
            .expect("stop");
        let after = engine.container(&api.id).await.expect("detail");
        assert_eq!(
            berth_core::status_of(
                after.container.state,
                after.container.health,
                after.container.exit_code
            ),
            berth_core::StatusKind::Exited(0)
        );
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.next())
            .await
            .expect("event within 2s")
            .expect("stream open")
            .expect("event ok");
        assert_eq!(
            event.kind,
            berth_core::EventKind::Container {
                id: api.id.clone(),
                action: "die".to_string()
            }
        );
        // Exactly one event for the stop: nothing more arrives promptly.
        let extra =
            tokio::time::timeout(std::time::Duration::from_millis(400), events.next()).await;
        assert!(extra.is_err(), "unexpected second event: {extra:?}");
        cancel.cancel();
    }

    #[tokio::test]
    async fn mock_start_exited_container_yields_running_plus_event() {
        let engine = MockEngine::bundled(&engine_id("colima:default"));
        let containers = engine.containers().await.expect("containers");
        let db = containers
            .iter()
            .find(|c| c.name == "shop-api-db-1")
            .expect("db")
            .clone();
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut events = engine.events(cancel.clone());
        engine
            .act(&db.id, berth_core::ContainerAction::Start)
            .await
            .expect("start");
        assert_eq!(
            engine
                .container(&db.id)
                .await
                .expect("detail")
                .container
                .state,
            berth_core::ContainerState::Running
        );
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.next())
            .await
            .expect("event")
            .expect("open")
            .expect("ok");
        assert!(matches!(
            event.kind,
            berth_core::EventKind::Container { ref action, .. } if action == "start"
        ));
        cancel.cancel();
    }

    #[tokio::test]
    async fn mock_logs_replay_scripted_shop_api_lines() {
        let engine = MockEngine::bundled(&engine_id("colima:default"));
        let containers = engine.containers().await.expect("containers");
        let worker = containers
            .iter()
            .find(|c| c.name == "shop-api-worker-1")
            .expect("worker");
        let lines: Vec<_> = engine
            .logs(
                &worker.id,
                berth_core::LogQuery::default(),
                tokio_util::sync::CancellationToken::new(),
            )
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("logs ok");
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("ECONNREFUSED 172.19.0.3:5432")),
            "{texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|t| t.contains("TCPConnectWrap.afterConnect")),
            "stack trace line: {texts:?}"
        );
        assert!(lines.iter().any(|l| l.stream == berth_core::Stream::Stderr));
        let api = containers
            .iter()
            .find(|c| c.name == "shop-api-api-1")
            .expect("api");
        let api_lines: Vec<_> = engine
            .logs(
                &api.id,
                berth_core::LogQuery {
                    tail: Some(2),
                    ..Default::default()
                },
                tokio_util::sync::CancellationToken::new(),
            )
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("api logs");
        assert_eq!(api_lines.len(), 2, "tail keeps the last two lines");
        let api_all: Vec<_> = engine
            .logs(
                &api.id,
                berth_core::LogQuery::default(),
                tokio_util::sync::CancellationToken::new(),
            )
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("api logs all");
        assert!(
            api_all
                .iter()
                .any(|l| l.text.contains(r#""level":"info""#) && l.text.contains("/health")),
            "JSON log line: {api_all:?}"
        );
    }

    #[tokio::test]
    async fn mock_logs_and_stats_honour_cancel() {
        let engine = MockEngine::bundled(&engine_id("colima:default"));
        let containers = engine.containers().await.expect("containers");
        let api = containers
            .iter()
            .find(|c| c.name == "shop-api-api-1")
            .expect("api")
            .clone();
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut follow = engine.logs(
            &api.id,
            berth_core::LogQuery {
                follow: true,
                ..Default::default()
            },
            cancel.clone(),
        );
        let first = follow.next().await.expect("first line").expect("ok");
        assert!(!first.text.is_empty());
        cancel.cancel();
        let after = tokio::time::timeout(std::time::Duration::from_secs(1), follow.next())
            .await
            .expect("stream ends after cancel");
        assert!(
            after.is_none(),
            "follow stream must end on cancel: {after:?}"
        );
        let cancel2 = tokio_util::sync::CancellationToken::new();
        let mut stats = engine.stats(&api.id, cancel2.clone());
        let sample = stats.next().await.expect("sample").expect("ok");
        assert!(sample.mem_used.as_u64() > 0);
        cancel2.cancel();
        let after = tokio::time::timeout(std::time::Duration::from_secs(1), stats.next())
            .await
            .expect("stats ends after cancel");
        assert!(after.is_none(), "stats stream must end on cancel");
    }

    #[tokio::test]
    async fn mock_stats_replays_series_and_reads_match_frames() {
        let engine = MockEngine::bundled(&engine_id("colima:default"));
        assert_eq!(engine.id().0, "colima:default");
        assert!(engine.docker_host().contains("mock"));
        let info = engine.info().await.expect("info");
        assert_eq!(info.docker_version, "28.4.0");
        assert_eq!(info.ncpu, 4);
        assert_eq!(berth_core::fmt::bytes(info.mem_total), "8 GB");
        let containers = engine.containers().await.expect("containers");
        let api = containers
            .iter()
            .find(|c| c.name == "shop-api-api-1")
            .expect("api")
            .clone();
        let frame_stats = api.stats.as_ref().expect("frame stats");
        assert_eq!(berth_core::fmt::bytes(frame_stats.mem_used), "184 MB");
        assert!((frame_stats.cpu_pct - 3.2).abs() < 0.01);
        assert_eq!(berth_core::fmt::port(&api.ports[0]), ":3000→3000/tcp");
        let cancel = tokio_util::sync::CancellationToken::new();
        let samples: Vec<_> = engine
            .stats(&api.id, cancel.clone())
            .take(3)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("stats ok");
        assert_eq!(samples.len(), 3);
        assert!(samples.iter().all(|s| s.mem_used.as_u64() > 0));
        cancel.cancel();
        // Images / volumes / networks / disk usage reads.
        let images = engine.images().await.expect("images");
        assert!(images.len() >= 20, "{}", images.len());
        let pg = engine
            .image("postgres:17-alpine")
            .await
            .expect("image detail");
        assert_eq!(pg.image.repo_tags, ["postgres:17-alpine"]);
        assert!(!pg.layers.is_empty());
        let volumes = engine.volumes().await.expect("volumes");
        assert!(volumes.iter().any(|v| v.name == "shop-api_pgdata"));
        let networks = engine.networks().await.expect("networks");
        let shop_net = networks
            .iter()
            .find(|n| n.name == "shop-api_default")
            .expect("shop network");
        assert_eq!(shop_net.containers.len(), 5);
        let usage = engine.disk_usage().await.expect("disk usage");
        assert_eq!(
            berth_core::fmt::bytes(usage.disk_total.expect("disk total")),
            "60 GB"
        );
        assert!(usage.images.size.as_u64() > 0);
    }

    #[tokio::test]
    async fn mock_container_detail_uses_fixture_override() {
        let engine = MockEngine::bundled(&engine_id("colima:default"));
        let containers = engine.containers().await.expect("containers");
        let worker = containers
            .iter()
            .find(|c| c.name == "shop-api-worker-1")
            .expect("worker");
        let detail = engine.container(&worker.id).await.expect("detail");
        assert_eq!(
            detail.cmd.as_deref(),
            Some(["node".to_string(), "dist/worker.js".to_string()].as_slice())
        );
        assert_eq!(detail.restart, berth_core::RestartPolicy::UnlessStopped);
        assert!(detail.env.iter().any(|e| e.key == "DATABASE_URL"));
        assert!(detail.inspect.is_object());
        let missing = engine
            .container(&berth_core::ContainerId("nope".to_string()))
            .await
            .expect_err("unknown container");
        assert!(matches!(missing, EngineError::NotFound(_)), "{missing}");
    }

    #[tokio::test]
    async fn mock_remove_container_and_unknown_act() {
        let engine = MockEngine::bundled(&engine_id("colima:default"));
        let containers = engine.containers().await.expect("containers");
        let eager = containers
            .iter()
            .find(|c| c.name == "eager_hopper")
            .expect("eager")
            .clone();
        engine
            .act(
                &eager.id,
                berth_core::ContainerAction::Remove {
                    force: false,
                    volumes: false,
                },
            )
            .await
            .expect("remove");
        assert!(engine
            .containers()
            .await
            .expect("containers")
            .iter()
            .all(|c| c.name != "eager_hopper"));
        let err = engine
            .act(
                &berth_core::ContainerId("gone".to_string()),
                berth_core::ContainerAction::Start,
            )
            .await
            .expect_err("unknown act");
        assert!(matches!(err, EngineError::NotFound(_)), "{err}");
        let exec_err = engine
            .exec(
                &eager.id,
                berth_core::ExecRequest {
                    shell: None,
                    user: None,
                    workdir: None,
                    cols: 80,
                    rows: 24,
                },
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .err()
            .expect("exec unsupported in mock");
        assert!(
            matches!(exec_err, EngineError::Unsupported(_)),
            "{exec_err}"
        );
    }

    #[tokio::test]
    async fn mock_engines_are_isolated_per_engine_id() {
        let work = MockEngine::bundled(&engine_id("colima:work"));
        let work_containers = work.containers().await.expect("work containers");
        assert_eq!(work_containers.len(), 11);
        assert!(work_containers
            .iter()
            .all(|c| c.engine == engine_id("colima:work")));
        let hetzner = MockEngine::bundled(&engine_id("ssh:hetzner-1"));
        let remote = hetzner.containers().await.expect("remote containers");
        assert_eq!(remote.len(), 9);
        assert_eq!(
            remote
                .iter()
                .filter(|c| c.state == berth_core::ContainerState::Running)
                .count(),
            7
        );
        let bad = MockEngine::from_fixture(BUNDLED_JSON, &engine_id("colima:nope"))
            .err()
            .expect("unknown engine in fixture");
        assert!(matches!(bad, EngineError::NotFound(_)), "{bad}");
        let parse = MockWorld::from_json("{not json").expect_err("bad json");
        assert!(matches!(parse, EngineError::Parse { .. }), "{parse}");
    }

    #[tokio::test]
    async fn mock_update_limits_is_visible_in_detail() {
        let engine = MockEngine::bundled(&engine_id("colima:default"));
        let containers = engine.containers().await.expect("containers");
        let api = containers
            .iter()
            .find(|c| c.name == "berth-site-web-1")
            .expect("web")
            .clone();
        let limits = berth_core::Limits {
            cpus: Some(0.5),
            memory: Some(bytesize::ByteSize::mb(256)),
            memory_swap: None,
        };
        engine
            .update_limits(&api.id, &limits, &berth_core::RestartPolicy::Always)
            .await
            .expect("update limits");
        let detail = engine.container(&api.id).await.expect("detail");
        assert_eq!(detail.limits.cpus, Some(0.5));
        assert_eq!(detail.restart, berth_core::RestartPolicy::Always);
    }
}
