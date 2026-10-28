//! The mock's image workflows (P8-T3): build replays the Dockerfile's
//! steps, push replays layer lines, export/import round-trip a tar with a
//! `manifest.json`, commit snapshots a container into a new image.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use tokio_util::sync::CancellationToken as Cancel;

use super::{find_container, lock_err, State};
use crate::client::Bs;
use crate::EngineError;

type Shared = Arc<Mutex<State>>;

const EXPORT_CHUNK: usize = 1024;

/// What the engine replays, in order. `Add` and `Fail` are side effects
/// applied when the replay reaches them, so a cancelled build leaves no
/// image behind.
enum Step<T> {
    Emit(T),
    Add(Box<berth_core::Image>),
    Fail(EngineError),
}

/// Replays `steps` until done or cancelled; `Add` puts the image into
/// `state` (tags move off any other image that carried them).
fn replay<T: Send + 'static>(state: Shared, steps: Vec<Step<T>>, cancel: Cancel) -> Bs<T> {
    futures::stream::unfold(
        (VecDeque::from(steps), state, cancel),
        |(mut steps, state, cancel)| async move {
            loop {
                if cancel.is_cancelled() {
                    return None;
                }
                match steps.pop_front()? {
                    Step::Emit(item) => return Some((Ok(item), (steps, state, cancel))),
                    Step::Fail(e) => return Some((Err(e), (VecDeque::new(), state, cancel))),
                    Step::Add(image) => {
                        if let Ok(mut state) = state.lock() {
                            add_image(&mut state, *image);
                        }
                    }
                }
            }
        },
    )
    .boxed()
}

/// Adds `image`, or merges its tags into the image that already has the
/// same id. A tag moves off any other image that carried it; an image
/// left without tags becomes dangling.
fn add_image(state: &mut State, mut image: berth_core::Image) {
    for other in &mut state.images {
        if other.id == image.id {
            continue;
        }
        other.repo_tags.retain(|t| !image.repo_tags.contains(t));
        other.dangling = other.repo_tags.is_empty() && other.digests.is_empty();
    }
    image.dangling = image.repo_tags.is_empty();
    if let Some(existing) = state.images.iter_mut().find(|i| i.id == image.id) {
        for tag in image.repo_tags {
            if !existing.repo_tags.contains(&tag) {
                existing.repo_tags.push(tag);
            }
        }
        existing.dangling = existing.repo_tags.is_empty();
    } else {
        state.images.push(image);
    }
}

fn with_default_tag(name: &str) -> String {
    let last = name.rsplit('/').next().unwrap_or(name);
    if last.contains(':') || name.contains('@') {
        name.to_string()
    } else {
        format!("{name}:latest")
    }
}

fn hash_id(parts: &[&str]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    let a = hasher.finish();
    a.hash(&mut hasher);
    format!("sha256:mock{a:016x}{:016x}", hasher.finish())
}

fn find_image<'a>(images: &'a [berth_core::Image], id: &str) -> Option<&'a berth_core::Image> {
    let wanted = with_default_tag(id);
    images.iter().find(|i| {
        i.id == id
            || i.id
                .trim_start_matches("sha256:")
                .starts_with(id.trim_start_matches("sha256:"))
            || i.repo_tags.iter().any(|t| *t == wanted || t == id)
    })
}

fn one_error<T: Send + 'static>(e: EngineError) -> Bs<T> {
    futures::stream::once(async move { Err(e) }).boxed()
}

// ---- build ---------------------------------------------------------------

fn dockerfile_text(spec: &berth_core::BuildSpec) -> Result<String, EngineError> {
    match &spec.context {
        berth_core::BuildContext::Inline(text) => Ok(text.clone()),
        berth_core::BuildContext::Folder(folder) => {
            let name = spec.dockerfile.as_deref().unwrap_or("Dockerfile");
            std::fs::read_to_string(folder.join(name)).map_err(|_| {
                EngineError::Docker(format!("Cannot locate specified Dockerfile: {name}"))
            })
        }
    }
}

/// The replay of a build. A `RUN` containing `exit 1` fails the way a
/// legacy-builder step does; nothing else in the Dockerfile is run.
pub(super) fn build(
    state: &Shared,
    engine: &berth_core::EngineId,
    spec: &berth_core::BuildSpec,
    cancel: Cancel,
) -> Bs<berth_core::BuildEvent> {
    use berth_core::BuildEvent;
    let text = match dockerfile_text(spec) {
        Ok(text) => text,
        Err(e) => return one_error(e),
    };
    let instructions: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let total = u32::try_from(instructions.len()).unwrap_or(u32::MAX);
    let mut steps = Vec::new();
    for (n, line) in instructions.iter().enumerate() {
        let index = u32::try_from(n + 1).unwrap_or(u32::MAX);
        steps.push(Step::Emit(BuildEvent::Step {
            index,
            total,
            text: format!("Step {index}/{total} : {line}"),
        }));
        if let Some(command) = line.strip_prefix("RUN ") {
            if command.contains("exit 1") {
                let message =
                    format!("The command '/bin/sh -c {command}' returned a non-zero code: 1");
                steps.push(Step::Emit(BuildEvent::Failed {
                    text: message.clone(),
                }));
                steps.push(Step::Fail(EngineError::Docker(message)));
                return replay(state.clone(), steps, cancel);
            }
        }
        steps.push(Step::Emit(BuildEvent::Log(format!(" ---> mock{index:04}"))));
    }
    let tags: Vec<String> = spec.tags.iter().map(|t| with_default_tag(t)).collect();
    let id = hash_id(&[&text, &tags.join(",")]);
    let short = id
        .trim_start_matches("sha256:")
        .chars()
        .take(12)
        .collect::<String>();
    steps.push(Step::Emit(BuildEvent::Log(format!(
        "Successfully built {short}"
    ))));
    steps.push(Step::Add(Box::new(berth_core::Image {
        id,
        repo_tags: tags.clone(),
        digests: Vec::new(),
        size: bytesize::ByteSize::mb(5) + bytesize::ByteSize::kb(u64::from(total) * 64),
        created: chrono::Utc::now(),
        arch: spec
            .platform
            .as_deref()
            .map_or(berth_core::Arch::Aarch64, |p| {
                berth_core::Arch::parse(p.rsplit('/').next().unwrap_or(p))
            }),
        os: "linux".to_string(),
        used_by: Vec::new(),
        dangling: tags.is_empty(),
        engine: engine.clone(),
    })));
    for tag in tags {
        steps.push(Step::Emit(BuildEvent::Log(format!(
            "Successfully tagged {tag}"
        ))));
    }
    replay(state.clone(), steps, cancel)
}

// ---- push ----------------------------------------------------------------

pub(super) fn push(
    state: &Shared,
    reference: &str,
    cancel: Cancel,
) -> Bs<berth_core::PullProgress> {
    let wanted = with_default_tag(reference);
    let image = match state.lock() {
        Ok(state) => find_image(&state.images, &wanted).cloned(),
        Err(_) => return one_error(lock_err()),
    };
    let Some(image) = image else {
        return one_error(EngineError::NotFound(format!(
            "An image does not exist locally with the tag: {reference}"
        )));
    };
    let repo = wanted.rsplit_once(':').map_or(wanted.as_str(), |(r, _)| r);
    let tag = wanted.rsplit_once(':').map_or("latest", |(_, t)| t);
    let line = |layer: Option<&str>, status: &str, current: Option<u64>, total: Option<u64>| {
        Step::Emit(berth_core::PullProgress {
            layer: layer.map(str::to_string),
            status: status.to_string(),
            current,
            total,
        })
    };
    let layer_size = image.size.as_u64() / 2;
    let mut steps = vec![line(
        None,
        &format!("The push refers to repository [{repo}]"),
        None,
        None,
    )];
    for layer in ["mock-layer-1", "mock-layer-2"] {
        steps.push(line(Some(layer), "Preparing", None, None));
    }
    for layer in ["mock-layer-1", "mock-layer-2"] {
        steps.push(line(
            Some(layer),
            "Pushing",
            Some(layer_size / 2),
            Some(layer_size),
        ));
        steps.push(line(
            Some(layer),
            "Pushed",
            Some(layer_size),
            Some(layer_size),
        ));
    }
    steps.push(line(
        None,
        &format!(
            "{tag}: digest: {} size: {}",
            hash_id(&[&image.id, "digest"]),
            image.size.as_u64()
        ),
        None,
        None,
    ));
    replay(state.clone(), steps, cancel)
}

// ---- export / import -----------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize)]
struct ManifestEntry {
    #[serde(rename = "Config")]
    config: String,
    #[serde(rename = "RepoTags", default)]
    repo_tags: Vec<String>,
    #[serde(rename = "Layers", default)]
    layers: Vec<String>,
}

/// The slice of an image config the mock keeps through a round trip.
#[derive(serde::Serialize, serde::Deserialize)]
struct ImageConfig {
    architecture: String,
    os: String,
    created: chrono::DateTime<chrono::Utc>,
    size: u64,
}

fn tar_err(e: &std::io::Error) -> EngineError {
    EngineError::Docker(format!("image tar: {e}"))
}

fn export_tar(image: &berth_core::Image) -> Result<Vec<u8>, EngineError> {
    let config_name = format!("{}.json", image.id.trim_start_matches("sha256:"));
    let config = serde_json::to_vec(&ImageConfig {
        architecture: image.arch.docker_name().to_string(),
        os: image.os.clone(),
        created: image.created,
        size: image.size.as_u64(),
    })
    .map_err(|e| EngineError::Docker(e.to_string()))?;
    let manifest = serde_json::to_vec(&[ManifestEntry {
        config: config_name.clone(),
        repo_tags: image.repo_tags.clone(),
        layers: Vec::new(),
    }])
    .map_err(|e| EngineError::Docker(e.to_string()))?;
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in [
        (config_name.as_str(), &config),
        ("manifest.json", &manifest),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, bytes.as_slice())
            .map_err(|e| tar_err(&e))?;
    }
    builder.into_inner().map_err(|e| tar_err(&e))
}

pub(super) fn export(state: &Shared, id: &str, cancel: Cancel) -> Bs<bytes::Bytes> {
    let image = match state.lock() {
        Ok(state) => find_image(&state.images, id).cloned(),
        Err(_) => return one_error(lock_err()),
    };
    let Some(image) = image else {
        return one_error(EngineError::NotFound(format!("image {id}")));
    };
    let tar = match export_tar(&image) {
        Ok(tar) => tar,
        Err(e) => return one_error(e),
    };
    let chunks: Vec<Result<bytes::Bytes, EngineError>> = tar
        .chunks(EXPORT_CHUNK)
        .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
        .collect();
    futures::stream::iter(chunks)
        .take_while(move |_| futures::future::ready(!cancel.is_cancelled()))
        .boxed()
}

pub(super) fn import(
    state: &Shared,
    engine: &berth_core::EngineId,
    tar: &[u8],
) -> Result<Vec<String>, EngineError> {
    let mut files = std::collections::BTreeMap::new();
    let mut archive = tar::Archive::new(tar);
    let entries = archive.entries().map_err(|e| tar_err(&e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| tar_err(&e))?;
        let name = entry
            .path()
            .map_err(|e| tar_err(&e))?
            .to_string_lossy()
            .to_string();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).map_err(|e| tar_err(&e))?;
        files.insert(name, bytes);
    }
    let manifest = files
        .get("manifest.json")
        .ok_or_else(|| EngineError::Docker("invalid tar: manifest.json not found".to_string()))?;
    let entries: Vec<ManifestEntry> = serde_json::from_slice(manifest)
        .map_err(|e| EngineError::Docker(format!("invalid manifest.json: {e}")))?;
    let mut names = Vec::new();
    let mut state = state.lock().map_err(|_| lock_err())?;
    for entry in entries {
        let config: ImageConfig = files
            .get(&entry.config)
            .and_then(|bytes| serde_json::from_slice(bytes).ok())
            .unwrap_or(ImageConfig {
                architecture: "arm64".to_string(),
                os: "linux".to_string(),
                created: chrono::Utc::now(),
                size: 0,
            });
        let id = format!("sha256:{}", entry.config.trim_end_matches(".json"));
        if entry.repo_tags.is_empty() {
            names.push(id.clone());
        } else {
            names.extend(entry.repo_tags.iter().cloned());
        }
        add_image(
            &mut state,
            berth_core::Image {
                id,
                repo_tags: entry.repo_tags,
                digests: Vec::new(),
                size: bytesize::ByteSize(config.size),
                created: config.created,
                arch: berth_core::Arch::parse(&config.architecture),
                os: config.os,
                used_by: Vec::new(),
                dangling: false,
                engine: engine.clone(),
            },
        );
    }
    Ok(names)
}

// ---- commit ----------------------------------------------------------------

pub(super) fn commit(
    state: &Shared,
    engine: &berth_core::EngineId,
    id: &berth_core::ContainerId,
    repo: &str,
    tag: &str,
) -> Result<String, EngineError> {
    let mut state = state.lock().map_err(|_| lock_err())?;
    let container = find_container(&state.containers, id)
        .ok_or_else(|| EngineError::NotFound(format!("container {}", id.0)))?
        .clone();
    let base = container
        .image
        .id
        .as_deref()
        .and_then(|i| find_image(&state.images, i))
        .or_else(|| find_image(&state.images, &container.image.reference))
        .cloned();
    let name = format!("{repo}:{tag}");
    let nanos = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_default()
        .to_string();
    let image_id = hash_id(&[&container.id.0, &container.name, &name, &nanos]);
    add_image(
        &mut state,
        berth_core::Image {
            id: image_id.clone(),
            repo_tags: vec![name],
            digests: Vec::new(),
            size: base.as_ref().map_or(bytesize::ByteSize::mb(1), |b| {
                b.size + bytesize::ByteSize::kb(4)
            }),
            created: chrono::Utc::now(),
            arch: base
                .as_ref()
                .map_or(berth_core::Arch::Aarch64, |b| b.arch.clone()),
            os: base.map_or_else(|| "linux".to_string(), |b| b.os),
            used_by: Vec::new(),
            dangling: false,
            engine: engine.clone(),
        },
    );
    Ok(image_id)
}

#[cfg(test)]
mod tests {
    use crate::client::EngineClient;
    use crate::mock::MockEngine;
    use crate::EngineError;
    use berth_core::{BuildContext, BuildEvent, BuildSpec};
    use futures::StreamExt;
    use tokio_util::sync::CancellationToken as Cancel;

    fn engine() -> MockEngine {
        MockEngine::bundled(&berth_core::EngineId("colima:default".to_string()))
    }

    fn spec(dockerfile: &str, tags: &[&str]) -> BuildSpec {
        BuildSpec {
            context: BuildContext::Inline(dockerfile.to_string()),
            dockerfile: None,
            tags: tags.iter().map(ToString::to_string).collect(),
            build_args: Vec::new(),
            platform: None,
        }
    }

    async fn run_build(
        client: &MockEngine,
        spec: &BuildSpec,
    ) -> Vec<Result<BuildEvent, EngineError>> {
        client
            .build_image(spec, None, Cancel::new())
            .collect::<Vec<_>>()
            .await
    }

    #[tokio::test]
    async fn mock_build_replays_steps_and_adds_the_tagged_image() {
        let client = engine();
        let before = client.images().await.expect("images").len();
        let items = run_build(
            &client,
            &spec(
                "FROM alpine\nRUN echo hi\n",
                &["berthfx/app:1", "berthfx/app"],
            ),
        )
        .await;
        let events: Vec<_> = items.into_iter().map(|e| e.expect("event")).collect();
        let steps: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                BuildEvent::Step { index, total, text } => Some((*index, *total, text.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            steps,
            [
                (1, 2, "Step 1/2 : FROM alpine"),
                (2, 2, "Step 2/2 : RUN echo hi")
            ]
        );
        let images = client.images().await.expect("images");
        assert_eq!(images.len(), before + 1);
        let built = client.image("berthfx/app:1").await.expect("by first tag");
        assert!(built
            .image
            .repo_tags
            .contains(&"berthfx/app:latest".to_string()));
        assert!(!built.image.dangling);
    }

    #[tokio::test]
    async fn mock_build_failing_run_ends_with_failed_event_and_adds_nothing() {
        let client = engine();
        let before = client.images().await.expect("images").len();
        let items = run_build(
            &client,
            &spec(
                "FROM alpine\nRUN sh -c 'exit 1'\nCMD true\n",
                &["berthfx/broken:1"],
            ),
        )
        .await;
        assert!(matches!(
            items.get(items.len() - 2),
            Some(Ok(BuildEvent::Failed { text })) if text.contains("non-zero code: 1")
        ));
        assert!(matches!(items.last(), Some(Err(EngineError::Docker(_)))));
        assert!(
            !items
                .iter()
                .any(|i| matches!(i, Ok(BuildEvent::Step { index: 3, .. }))),
            "steps after the failure do not run"
        );
        assert_eq!(client.images().await.expect("images").len(), before);
    }

    #[tokio::test]
    async fn mock_build_cancelled_midway_leaves_no_image() {
        let client = engine();
        let before = client.images().await.expect("images").len();
        let cancel = Cancel::new();
        let mut stream = client.build_image(
            &spec("FROM alpine\nRUN echo hi\n", &["berthfx/app:1"]),
            None,
            cancel.clone(),
        );
        assert!(matches!(
            stream.next().await,
            Some(Ok(BuildEvent::Step { index: 1, .. }))
        ));
        cancel.cancel();
        assert!(stream.next().await.is_none());
        assert_eq!(client.images().await.expect("images").len(), before);
    }

    #[tokio::test]
    async fn mock_build_again_moves_the_tag_and_leaves_the_old_image_dangling() {
        let client = engine();
        run_build(
            &client,
            &spec("FROM alpine\nRUN echo one\n", &["berthfx/app:1"]),
        )
        .await;
        let first = client.image("berthfx/app:1").await.expect("first").image.id;
        run_build(
            &client,
            &spec("FROM alpine\nRUN echo two\n", &["berthfx/app:1"]),
        )
        .await;
        let second = client
            .image("berthfx/app:1")
            .await
            .expect("second")
            .image
            .id;
        assert_ne!(first, second);
        let old = client.image(&first).await.expect("old image stays");
        assert!(old.image.repo_tags.is_empty());
        assert!(old.image.dangling);
    }

    #[tokio::test]
    async fn mock_build_folder_context_reads_the_named_dockerfile() {
        let client = engine();
        let folder = tempfile::tempdir().expect("tempdir");
        std::fs::write(folder.path().join("Dockerfile.dev"), "FROM scratch\n").expect("write");
        let mut spec = BuildSpec {
            context: BuildContext::Folder(folder.path().to_path_buf()),
            dockerfile: Some("Dockerfile.dev".to_string()),
            tags: vec!["berthfx/dev:1".to_string()],
            build_args: Vec::new(),
            platform: Some("linux/amd64".to_string()),
        };
        let items = run_build(&client, &spec).await;
        assert!(items.iter().all(Result::is_ok), "{items:?}");
        let built = client.image("berthfx/dev:1").await.expect("built");
        assert_eq!(built.image.arch, berth_core::Arch::X86_64);
        // A Dockerfile that is not there is an error, not an empty image.
        spec.dockerfile = Some("Missing".to_string());
        let items = run_build(&client, &spec).await;
        assert!(
            matches!(items.as_slice(), [Err(EngineError::Docker(m))] if m.contains("Missing")),
            "{items:?}"
        );
    }

    #[tokio::test]
    async fn mock_push_replays_layers_then_digest_and_rejects_unknown_images() {
        let client = engine();
        let lines: Vec<_> = client
            .push("postgres:17-alpine", None, Cancel::new())
            .map(|l| l.expect("line"))
            .collect()
            .await;
        assert!(lines[0]
            .status
            .starts_with("The push refers to repository [postgres]"));
        let pushed: Vec<_> = lines.iter().filter(|l| l.status == "Pushed").collect();
        assert_eq!(pushed.len(), 2);
        assert!(pushed
            .iter()
            .all(|l| l.layer.is_some() && l.current == l.total));
        assert!(lines
            .last()
            .expect("last")
            .status
            .starts_with("17-alpine: digest: sha256:"));

        let missing: Vec<_> = client
            .push("127.0.0.1:56500/nope/none:1", None, Cancel::new())
            .collect()
            .await;
        assert!(
            matches!(missing.as_slice(), [Err(EngineError::NotFound(_))]),
            "{missing:?}"
        );

        let cancel = Cancel::new();
        let mut stream = client.push("postgres:17-alpine", None, cancel.clone());
        assert!(stream.next().await.is_some());
        cancel.cancel();
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn mock_export_then_import_restores_the_removed_image() {
        let client = engine();
        let original = client
            .image("postgres:17-alpine")
            .await
            .expect("image")
            .image;
        let tar: Vec<u8> = client
            .export_image("postgres:17-alpine", Cancel::new())
            .map(|c| c.expect("chunk"))
            .collect::<Vec<_>>()
            .await
            .concat();
        // It is a real tar with a manifest.json that names the image.
        let mut archive = tar::Archive::new(tar.as_slice());
        let mut manifest = String::new();
        for entry in archive.entries().expect("entries") {
            let mut entry = entry.expect("entry");
            if entry.path().expect("path").to_string_lossy() == "manifest.json" {
                std::io::Read::read_to_string(&mut entry, &mut manifest).expect("read");
            }
        }
        assert!(manifest.contains("postgres:17-alpine"), "{manifest}");

        client
            .remove_image(&original.id, true)
            .await
            .expect("remove");
        assert!(client.image("postgres:17-alpine").await.is_err());
        let names = client
            .import_image(bytes::Bytes::from(tar))
            .await
            .expect("import");
        assert!(
            names.contains(&"postgres:17-alpine".to_string()),
            "{names:?}"
        );
        let back = client
            .image("postgres:17-alpine")
            .await
            .expect("restored")
            .image;
        assert_eq!(back.id, original.id);
        assert_eq!(back.size, original.size);
        assert_eq!(back.arch, original.arch);
    }

    #[tokio::test]
    async fn mock_import_rejects_a_tar_without_manifest_and_non_tars() {
        let client = engine();
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(2);
        header.set_cksum();
        builder
            .append_data(&mut header, "other.txt", &b"hi"[..])
            .expect("append");
        let no_manifest = builder.into_inner().expect("tar");
        let result = client.import_image(bytes::Bytes::from(no_manifest)).await;
        assert!(
            matches!(&result, Err(EngineError::Docker(m)) if m.contains("manifest.json")),
            "{result:?}"
        );
        let junk = client
            .import_image(bytes::Bytes::from_static(b"definitely not a tar archive"))
            .await;
        assert!(junk.is_err(), "{junk:?}");
    }

    #[tokio::test]
    async fn mock_export_of_an_unknown_image_is_not_found() {
        let items: Vec<_> = engine()
            .export_image("nope:1", Cancel::new())
            .collect()
            .await;
        assert!(
            matches!(items.as_slice(), [Err(EngineError::NotFound(_))]),
            "{items:?}"
        );
    }

    #[tokio::test]
    async fn mock_commit_snapshots_a_container_into_a_tagged_image() {
        let client = engine();
        let container = client
            .containers()
            .await
            .expect("containers")
            .into_iter()
            .next()
            .expect("a container");
        let before = client.images().await.expect("images").len();
        let id = client
            .commit(&container.id, "berthfx/web-snap", "1")
            .await
            .expect("commit");
        let images = client.images().await.expect("images");
        assert_eq!(images.len(), before + 1);
        let snap = client
            .image("berthfx/web-snap:1")
            .await
            .expect("by name")
            .image;
        assert_eq!(snap.id, id);
        assert!(snap.size.as_u64() > 0);
        let missing = client
            .commit(
                &berth_core::ContainerId("no-such-container".to_string()),
                "x",
                "y",
            )
            .await;
        assert!(
            matches!(missing, Err(EngineError::NotFound(_))),
            "{missing:?}"
        );
    }
}
