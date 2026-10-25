//! Live mutations against the berthfx fixtures (P1-T6 done-when). Run with:
//!   scripts/it.sh -p berth-engine mutate
//! One sequential test: it creates only `berthfx-scratch*` containers,
//! `berthfx-vol*` volumes and the `berthfx-net` network, and removes
//! everything it created (also on failure, best effort).

// Test helpers sit outside the #[test] fn, so clippy.toml's test allowance
// does not reach them.
#![allow(clippy::expect_used)]

use std::io::Read;
use std::time::Duration;

use berth_engine::{BollardEngine, ColimaCli, EngineClient};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

const SCRATCH: &str = "berthfx-scratch";
const VOL: &str = "berthfx-vol";
const VOL2: &str = "berthfx-vol2";
const NET: &str = "berthfx-net";

async fn connect() -> BollardEngine {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima = ColimaCli::new(search);
    let engines = berth_engine::discovery::discover(&colima, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default")
        .expect("colima:default discovered");
    BollardEngine::connect(engine).expect("connect")
}

async fn fixture_id(client: &BollardEngine, name: &str) -> berth_core::ContainerId {
    let containers = client.containers().await.expect("containers");
    containers
        .iter()
        .find(|c| c.name == name)
        .map(|c| c.id.clone())
        .expect("fixture container present")
}

/// Remove everything this test may create. Best effort; used before and
/// after the checks so a failed run never leaves scratch resources.
async fn cleanup(client: &BollardEngine) {
    if let Ok(containers) = client.containers().await {
        for c in containers.iter().filter(|c| c.name.starts_with(SCRATCH)) {
            let _ = client
                .act(
                    &c.id,
                    berth_core::ContainerAction::Remove {
                        force: true,
                        volumes: false,
                    },
                )
                .await;
        }
        // Disconnect anything still on berthfx-net so the network can go.
        for c in containers
            .iter()
            .filter(|c| c.networks.iter().any(|n| n == NET))
        {
            let _ = client.disconnect_network(NET, &c.id).await;
        }
    }
    let _ = client.remove_network(NET).await;
    let _ = client.remove_volume(VOL).await;
    let _ = client.remove_volume(VOL2).await;
}

fn scratch_spec(host_port: u16, image: &str) -> berth_core::ContainerSpec {
    berth_core::ContainerSpec {
        name: SCRATCH.to_string(),
        image: berth_core::ImageRef {
            reference: image.to_string(),
            id: None,
        },
        command: Some(vec!["sleep".to_string(), "300".to_string()]),
        entrypoint: None,
        env: vec![("BERTH_SCRATCH".to_string(), "1".to_string())],
        ports: vec![berth_core::PortMapping {
            host_ip: Some(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
            host_port: Some(host_port),
            container_port: 8080,
            proto: berth_core::Proto::Tcp,
        }],
        mounts: Vec::new(),
        networks: Vec::new(),
        labels: [("dev.berth.fixture".to_string(), "1".to_string())]
            .into_iter()
            .collect(),
        restart: berth_core::RestartPolicy::No,
        limits: berth_core::Limits::default(),
        platform: None,
    }
}

fn make_tar(name: &str, content: &[u8]) -> bytes::Bytes {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    builder
        .append_data(&mut header, name, content)
        .expect("build tar");
    bytes::Bytes::from(builder.into_inner().expect("tar bytes"))
}

fn tar_link_target(tar_bytes: &[u8], suffix: &str) -> Option<String> {
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    for entry in archive.entries().ok()? {
        let entry = entry.ok()?;
        let path = entry.path().ok()?.to_string_lossy().to_string();
        if path.ends_with(suffix) && entry.header().entry_type() == tar::EntryType::Symlink {
            return entry
                .header()
                .link_name()
                .ok()??
                .to_str()
                .map(str::to_string);
        }
    }
    None
}

fn tar_file_content(tar_bytes: &[u8], suffix: &str) -> Option<Vec<u8>> {
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    for entry in archive.entries().ok()? {
        let mut entry = entry.ok()?;
        let path = entry.path().ok()?.to_string_lossy().to_string();
        if path.ends_with(suffix) && entry.header().entry_type() == tar::EntryType::Regular {
            let mut content = Vec::new();
            entry.read_to_end(&mut content).ok()?;
            return Some(content);
        }
    }
    None
}

async fn download_all(
    client: &BollardEngine,
    target: &berth_core::FileTarget,
    path: &str,
) -> Result<Vec<u8>, String> {
    let chunks = tokio::time::timeout(
        Duration::from_secs(30),
        client
            .download(target, path, CancellationToken::new())
            .collect::<Vec<_>>(),
    )
    .await
    .map_err(|_| format!("download {path} timed out"))?;
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend_from_slice(&chunk.map_err(|e| format!("download {path}: {e}"))?);
    }
    Ok(out)
}

async fn run_checks(client: &BollardEngine) -> Result<(), String> {
    let web = fixture_id(client, "berthfx-web-1").await;

    // Pull of an already-present image streams progress without error.
    let progress = tokio::time::timeout(
        Duration::from_secs(60),
        client
            .pull("alpine:3", None, CancellationToken::new())
            .collect::<Vec<_>>(),
    )
    .await
    .map_err(|_| "pull alpine:3 timed out".to_string())?;
    for item in progress.iter() {
        if let Err(e) = item {
            return Err(format!("pull alpine:3: {e}"));
        }
    }
    if progress.is_empty() {
        return Err("pull alpine:3 produced no progress".to_string());
    }

    // Create from alpine:3 with env/ports/labels; detail matches.
    let scratch = client
        .create(&scratch_spec(56180, "alpine:3"), true)
        .await
        .map_err(|e| format!("create scratch: {e}"))?;
    let detail = client
        .container(&scratch)
        .await
        .map_err(|e| format!("scratch detail: {e}"))?;
    if detail.container.name != SCRATCH
        || detail.container.state != berth_core::ContainerState::Running
    {
        return Err(format!("scratch after create: {:?}", detail.container));
    }
    if !detail
        .env
        .iter()
        .any(|e| e.key == "BERTH_SCRATCH" && e.value == "1")
    {
        return Err(format!(
            "scratch env missing BERTH_SCRATCH=1: {:?}",
            detail.env
        ));
    }
    if !detail
        .container
        .ports
        .iter()
        .any(|p| p.host_port == Some(56180) && p.container_port == 8080)
    {
        return Err(format!("scratch ports: {:?}", detail.container.ports));
    }
    if detail
        .container
        .labels
        .get("dev.berth.fixture")
        .map(String::as_str)
        != Some("1")
    {
        return Err(format!("scratch labels: {:?}", detail.container.labels));
    }

    // Exec: echo berth through an interactive sh returns a "berth" line.
    let exec_cancel = CancellationToken::new();
    let mut handle = client
        .exec(
            &scratch,
            berth_core::ExecRequest {
                shell: Some("sh".to_string()),
                user: None,
                workdir: None,
                cols: 80,
                rows: 24,
            },
            exec_cancel.clone(),
        )
        .await
        .map_err(|e| format!("exec open: {e}"))?;
    if handle.session.shell != "sh" || handle.session.container != scratch {
        return Err(format!("exec session: {:?}", handle.session));
    }
    handle
        .resize(100, 30)
        .await
        .map_err(|e| format!("exec resize: {e}"))?;
    handle
        .input
        .send(bytes::Bytes::from_static(b"echo berth\n"))
        .await
        .map_err(|e| format!("exec input: {e}"))?;
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline
        && !seen.lines().any(|l| l.trim_end_matches('\r') == "berth")
    {
        match tokio::time::timeout(Duration::from_secs(2), handle.output.next()).await {
            Ok(Some(Ok(chunk))) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(Some(Err(e))) => return Err(format!("exec output: {e}")),
            Ok(None) => break,
            Err(_) => {}
        }
    }
    exec_cancel.cancel();
    if !seen.lines().any(|l| l.trim_end_matches('\r') == "berth") {
        return Err(format!("exec echo berth output: {seen:?}"));
    }

    // Files on the scratch container: download os-release, upload round trip.
    let os_release_tar = download_all(
        client,
        &berth_core::FileTarget::Container(scratch.clone()),
        "/etc/os-release",
    )
    .await?;
    // Alpine's /etc/os-release is a symlink to ../usr/lib/os-release and
    // the archive API preserves it as one; follow it inside the container.
    let os_release = match tar_file_content(&os_release_tar, "os-release") {
        Some(content) => content,
        None => {
            let target = tar_link_target(&os_release_tar, "os-release")
                .ok_or("os-release not in download tar")?;
            let resolved = if target.starts_with('/') {
                target
            } else {
                format!("/etc/{target}").replace("/etc/../", "/")
            };
            let target_tar = download_all(
                client,
                &berth_core::FileTarget::Container(scratch.clone()),
                &resolved,
            )
            .await?;
            tar_file_content(&target_tar, "os-release").ok_or("os-release target not in tar")?
        }
    };
    if !String::from_utf8_lossy(&os_release).contains("Alpine") {
        return Err(format!(
            "os-release content: {}",
            String::from_utf8_lossy(&os_release)
        ));
    }
    client
        .upload(
            &berth_core::FileTarget::Container(scratch.clone()),
            "/tmp",
            make_tar("berth-upload.txt", b"berth-upload-roundtrip"),
        )
        .await
        .map_err(|e| format!("upload to scratch: {e}"))?;
    let back_tar = download_all(
        client,
        &berth_core::FileTarget::Container(scratch.clone()),
        "/tmp/berth-upload.txt",
    )
    .await?;
    let back = tar_file_content(&back_tar, "berth-upload.txt").ok_or("uploaded file not in tar")?;
    if back != b"berth-upload-roundtrip" {
        return Err(format!(
            "upload round trip content: {}",
            String::from_utf8_lossy(&back)
        ));
    }

    // list_files of /etc in berthfx-web-1 includes nginx.
    let etc = client
        .list_files(&berth_core::FileTarget::Container(web.clone()), "/etc")
        .await
        .map_err(|e| format!("list_files web /etc: {e}"))?;
    if !etc.iter().any(|e| e.name == "nginx") {
        return Err(format!(
            "web /etc listing lacks nginx: {:?}",
            etc.iter().map(|e| &e.name).collect::<Vec<_>>()
        ));
    }

    // Recreate with a new port succeeds: new id, same name, running.
    let recreated = client
        .recreate(&scratch, &scratch_spec(56181, "alpine:3"))
        .await
        .map_err(|e| format!("recreate scratch: {e}"))?;
    if recreated == scratch {
        return Err("recreate returned the old container id".to_string());
    }
    let detail = client
        .container(&recreated)
        .await
        .map_err(|e| format!("recreated detail: {e}"))?;
    if detail.container.name != SCRATCH
        || detail.container.state != berth_core::ContainerState::Running
        || !detail
            .container
            .ports
            .iter()
            .any(|p| p.host_port == Some(56181) && p.container_port == 8080)
    {
        return Err(format!("recreated container: {:?}", detail.container));
    }
    if client.container(&scratch).await.is_ok() {
        return Err("old scratch id still inspectable after recreate".to_string());
    }

    // Recreate with a missing image fails and restores the old container.
    let bad = client
        .recreate(&recreated, &scratch_spec(56181, "berthfx/does-not-exist:1"))
        .await;
    match bad {
        Ok(id) => return Err(format!("recreate with bad image succeeded: {id:?}")),
        Err(berth_engine::EngineError::Docker(_)) => {}
        Err(other) => return Err(format!("recreate bad image error kind: {other}")),
    }
    let restored = client
        .container(&recreated)
        .await
        .map_err(|e| format!("restored detail: {e}"))?;
    if restored.container.name != SCRATCH
        || restored.container.state != berth_core::ContainerState::Running
    {
        return Err(format!("restored container: {:?}", restored.container));
    }
    let names: Vec<String> = client
        .containers()
        .await
        .map_err(|e| format!("containers after failed recreate: {e}"))?
        .iter()
        .map(|c| c.name.clone())
        .collect();
    if names.iter().any(|n| n.contains("berth-old")) {
        return Err(format!(
            "berth-old leftover after failed recreate: {names:?}"
        ));
    }

    // Volume: create, empty listing through the helper, upload/list/download,
    // then removal works.
    let volume = client
        .create_volume(&berth_core::VolumeSpec {
            name: VOL.to_string(),
            driver: "local".to_string(),
            driver_opts: Default::default(),
            labels: [("dev.berth.fixture".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
        })
        .await
        .map_err(|e| format!("create_volume: {e}"))?;
    if volume.name != VOL {
        return Err(format!("created volume: {volume:?}"));
    }
    let empty = client
        .list_files(&berth_core::FileTarget::Volume(VOL.to_string()), "/")
        .await
        .map_err(|e| format!("list_files empty volume: {e}"))?;
    if !empty.is_empty() {
        return Err(format!("new volume not empty: {empty:?}"));
    }
    client
        .upload(
            &berth_core::FileTarget::Volume(VOL.to_string()),
            "/",
            make_tar("berth-vol.txt", b"in-the-volume"),
        )
        .await
        .map_err(|e| format!("upload to volume: {e}"))?;
    let listed = client
        .list_files(&berth_core::FileTarget::Volume(VOL.to_string()), "/")
        .await
        .map_err(|e| format!("list_files volume after upload: {e}"))?;
    if !listed.iter().any(|e| e.name == "berth-vol.txt") {
        return Err(format!("volume listing after upload: {listed:?}"));
    }
    let vol_tar = download_all(
        client,
        &berth_core::FileTarget::Volume(VOL.to_string()),
        "/berth-vol.txt",
    )
    .await?;
    let vol_content =
        tar_file_content(&vol_tar, "berth-vol.txt").ok_or("volume file not in tar")?;
    if vol_content != b"in-the-volume" {
        return Err("volume download content mismatch".to_string());
    }
    client
        .remove_volume(VOL)
        .await
        .map_err(|e| format!("remove_volume: {e}"))?;
    let volumes = client
        .volumes()
        .await
        .map_err(|e| format!("volumes after remove: {e}"))?;
    if volumes.iter().any(|v| v.name == VOL) {
        return Err("berthfx-vol still listed after remove_volume".to_string());
    }

    // Network: create berthfx-net, connect/disconnect berthfx-web-1, remove.
    let network = client
        .create_network(&berth_core::NetworkSpec {
            name: NET.to_string(),
            driver: "bridge".to_string(),
            subnet: Some("172.31.250.0/24".to_string()),
            gateway: None,
            internal: false,
            attach: Vec::new(),
        })
        .await
        .map_err(|e| format!("create_network: {e}"))?;
    if network.name != NET {
        return Err(format!("created network: {network:?}"));
    }
    client
        .connect_network(NET, &web)
        .await
        .map_err(|e| format!("connect_network web: {e}"))?;
    let connected = client
        .container(&web)
        .await
        .map_err(|e| format!("web detail after connect: {e}"))?;
    if !connected.container.networks.iter().any(|n| n == NET) {
        return Err(format!(
            "web networks after connect: {:?}",
            connected.container.networks
        ));
    }
    client
        .disconnect_network(NET, &web)
        .await
        .map_err(|e| format!("disconnect_network web: {e}"))?;
    let disconnected = client
        .container(&web)
        .await
        .map_err(|e| format!("web detail after disconnect: {e}"))?;
    if disconnected.container.networks.iter().any(|n| n == NET) {
        return Err(format!(
            "web networks after disconnect: {:?}",
            disconnected.container.networks
        ));
    }
    client
        .remove_network(NET)
        .await
        .map_err(|e| format!("remove_network: {e}"))?;

    // Prune: a plan with only berthfx-vol2 removes exactly that volume.
    client
        .create_volume(&berth_core::VolumeSpec {
            name: VOL2.to_string(),
            driver: "local".to_string(),
            driver_opts: Default::default(),
            labels: [("dev.berth.fixture".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
        })
        .await
        .map_err(|e| format!("create_volume vol2: {e}"))?;
    let engine_id = client.id().clone();
    let plan = berth_core::PrunePlan {
        engine: engine_id.clone(),
        items: vec![
            berth_core::PruneItem {
                kind: berth_core::PruneKind::Volume,
                id: VOL2.to_string(),
                name: VOL2.to_string(),
                size: None,
                selected: true,
            },
            berth_core::PruneItem {
                kind: berth_core::PruneKind::Container,
                id: recreated.0.clone(),
                name: SCRATCH.to_string(),
                size: None,
                selected: false,
            },
        ],
    };
    let report = client
        .prune(&plan, CancellationToken::new())
        .await
        .map_err(|e| format!("prune vol2: {e}"))?;
    if report.removed.len() != 1 || !report.failed.is_empty() {
        return Err(format!("prune vol2 report: {report:?}"));
    }
    let volumes = client
        .volumes()
        .await
        .map_err(|e| format!("volumes after prune: {e}"))?;
    if volumes.iter().any(|v| v.name == VOL2) {
        return Err("berthfx-vol2 survived prune".to_string());
    }
    if client.container(&recreated).await.is_err() {
        return Err("unselected scratch container was pruned".to_string());
    }

    // Prune: the scratch container alone; berthfx-web-1 survives running.
    let plan = berth_core::PrunePlan {
        engine: engine_id,
        items: vec![berth_core::PruneItem {
            kind: berth_core::PruneKind::Container,
            id: recreated.0.clone(),
            name: SCRATCH.to_string(),
            size: None,
            selected: true,
        }],
    };
    let report = client
        .prune(&plan, CancellationToken::new())
        .await
        .map_err(|e| format!("prune scratch: {e}"))?;
    if report.removed.len() != 1 || !report.failed.is_empty() {
        return Err(format!("prune scratch report: {report:?}"));
    }
    if client.container(&recreated).await.is_ok() {
        return Err("scratch survived its prune".to_string());
    }
    let web_detail = client
        .container(&web)
        .await
        .map_err(|e| format!("web detail after prune: {e}"))?;
    if web_detail.container.state != berth_core::ContainerState::Running {
        return Err(format!(
            "berthfx-web-1 after prune: {:?}",
            web_detail.container.state
        ));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs the live engine and berthfx fixtures; run with BERTH_IT=1"]
async fn mutate_scratch_lifecycle() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let client = connect().await;
    cleanup(&client).await;

    let result = run_checks(&client).await;

    cleanup(&client).await;
    // Nothing scratch-shaped may remain, and the fixtures stay up.
    let containers = client.containers().await.expect("final containers");
    let leftovers: Vec<&str> = containers
        .iter()
        .filter(|c| c.name.starts_with(SCRATCH))
        .map(|c| c.name.as_str())
        .collect();
    assert!(leftovers.is_empty(), "scratch leftovers: {leftovers:?}");
    let volumes = client.volumes().await.expect("final volumes");
    assert!(
        !volumes.iter().any(|v| v.name == VOL || v.name == VOL2),
        "volume leftovers: {:?}",
        volumes.iter().map(|v| &v.name).collect::<Vec<_>>()
    );
    let networks = client.networks().await.expect("final networks");
    assert!(
        !networks.iter().any(|n| n.name == NET),
        "network leftover berthfx-net"
    );
    let web = fixture_id(&client, "berthfx-web-1").await;
    let web_detail = client.container(&web).await.expect("final web detail");
    assert_eq!(
        web_detail.container.state,
        berth_core::ContainerState::Running,
        "berthfx-web-1 must end running"
    );
    result.expect("mutate checks");
    println!("mutate ok: create, exec, files, recreate(+restore), volume, network, prune");
}
