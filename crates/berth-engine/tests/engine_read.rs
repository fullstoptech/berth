//! Live read side against the berthfx fixtures (P1-T4 done-when). Run with:
//!   scripts/it.sh -p berth-engine engine_read

use berth_engine::{BollardEngine, ColimaCli, EngineClient};

#[tokio::test]
#[ignore = "needs the live engine and berthfx fixtures; run with BERTH_IT=1"]
async fn engine_read_fixtures() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima = ColimaCli::new(search);
    let engines = berth_engine::discovery::discover(&colima, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default")
        .expect("colima:default discovered");
    let client = BollardEngine::connect(engine).expect("connect");
    assert_eq!(client.id().0, "colima:default");
    assert!(client.docker_host().starts_with("unix://"));

    let info = client.info().await.expect("info");
    assert!(!info.docker_version.is_empty(), "{info:?}");
    assert!(!info.api_version.is_empty());
    assert!(info.ncpu > 0 && info.mem_total.0 > 0);

    let containers = client.containers().await.expect("containers");
    let fx: Vec<_> = containers
        .iter()
        .filter(|c| c.name.starts_with("berthfx-"))
        .collect();
    assert_eq!(fx.len(), 4, "{fx:?}");
    assert!(fx
        .iter()
        .all(|c| c.compose_project.as_deref() == Some("berthfx")));

    let web = fx
        .iter()
        .find(|c| c.name == "berthfx-web-1")
        .expect("berthfx-web-1");
    assert!(
        web.ports.iter().any(|p| {
            p.host_port == Some(56080)
                && p.container_port == 80
                && p.proto == berth_core::Proto::Tcp
        }),
        "{:?}",
        web.ports
    );
    assert_eq!(web.health, Some(berth_core::Health::Healthy));

    let db = fx
        .iter()
        .find(|c| c.name == "berthfx-db-1")
        .expect("berthfx-db-1");
    let detail = client.container(&db.id).await.expect("db detail");
    let inspect = serde_json::to_string(&detail.inspect).expect("inspect serializes");
    let password = detail
        .env
        .iter()
        .find(|e| e.key == "POSTGRES_PASSWORD")
        .expect("env keeps POSTGRES_PASSWORD for the reveal");
    assert!(!password.value.is_empty(), "env value kept");
    assert!(
        !inspect.contains(&password.value),
        "fixture password leaked through inspect"
    );

    let images = client.images().await.expect("images");
    let nginx = images
        .iter()
        .find(|i| i.repo_tags.contains(&"nginx:alpine".to_string()))
        .expect("nginx:alpine image");
    assert!(nginx.used_by.contains(&web.id), "{nginx:?}");

    let image_detail = client.image("nginx:alpine").await.expect("image detail");
    assert!(!image_detail.layers.is_empty());
    assert!(image_detail.exposed_ports.contains(&80));

    let volumes = client.volumes().await.expect("volumes");
    assert!(
        volumes.iter().any(|v| v.name == "berthfx_berthfx_pgdata"),
        "{} volumes",
        volumes.len()
    );

    let networks = client.networks().await.expect("networks");
    let fixture_network = networks
        .iter()
        .find(|n| n.name == "berthfx_default")
        .expect("berthfx_default network");
    assert!(fixture_network.containers.contains(&web.id));

    let usage = client.disk_usage().await.expect("disk usage");
    assert!(
        usage.images.count > 0 && usage.images.size.0 > 0,
        "{usage:?}"
    );

    let sizes = client.volume_sizes().await.expect("volume sizes");
    let pgdata = sizes
        .get("berthfx_berthfx_pgdata")
        .expect("pgdata size present");
    assert!(pgdata.0 > 0);

    println!(
        "engine_read ok: docker={} fx={} pgdata={pgdata}",
        info.docker_version,
        fx.len()
    );
}
