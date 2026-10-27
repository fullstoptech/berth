//! Live SSH engine read (P8-T1, Q7). Run with:
//!   BERTH_SSH_TEST=<user@host> BERTH_IT=1 \
//!     cargo nextest run -p berth-engine --run-ignored only ssh_it
//! Without `BERTH_SSH_TEST` the test reports skipped (passes with a note).

use berth_engine::EngineClient;

#[tokio::test]
#[ignore = "needs BERTH_SSH_TEST=<user@host> with Docker; run with BERTH_IT=1"]
async fn ssh_it_lists_containers_on_the_named_host() {
    let Ok(target) = std::env::var("BERTH_SSH_TEST") else {
        eprintln!("BERTH_SSH_TEST not set, skipping (Q7)");
        return;
    };
    let (user, host) = match target.rsplit_once('@') {
        Some((user, host)) => (Some(user.to_string()), host.to_string()),
        None => (None, target.clone()),
    };
    let transport = berth_core::RemoteTransport::Ssh {
        host,
        user,
        port: None,
    };
    let engine = berth_core::Engine {
        id: berth_engine::remote::engine_id(&transport),
        name: target,
        kind: berth_core::EngineKind::Remote { transport },
        state: berth_core::EngineState::Running,
        docker_host: None,
        docker_version: None,
        resources: None,
        counts: berth_core::ResourceCounts::default(),
        owner: berth_core::OwnerId::Local,
    };
    let client = berth_engine::BollardEngine::connect(&engine).expect("connect over ssh");
    let containers = client.containers().await.expect("list containers");
    println!("{} containers on {}", containers.len(), engine.name);
}
