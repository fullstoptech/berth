//! Live remote engine over SSH (P8-T1 done-when). No default host exists
//! (QUESTIONS Q7), so this only runs when the owner names one:
//!   BERTH_SSH_TEST=<user@host[:port]> scripts/it.sh -p berth-engine ssh_it
//! Without the variable the test reports skipped.

use std::time::Duration;

use berth_engine::{BollardEngine, EngineClient};

#[tokio::test]
#[ignore = "needs BERTH_SSH_TEST=<user@host> with a reachable Docker engine"]
async fn ssh_it_lists_containers_on_the_given_host() {
    let Ok(target) = std::env::var("BERTH_SSH_TEST") else {
        eprintln!("BERTH_SSH_TEST not set, skipping");
        return;
    };
    let transport = berth_engine::remote::parse_ssh_target(&target)
        .unwrap_or_else(|| panic!("BERTH_SSH_TEST={target:?} is not [user@]host[:port]"));
    let engine = berth_core::Engine {
        id: berth_core::EngineId("ssh-it".to_string()),
        name: "ssh-it".to_string(),
        docker_host: Some(berth_engine::remote::docker_host_for_transport(&transport)),
        kind: berth_core::EngineKind::Remote { transport },
        state: berth_core::EngineState::Running,
        docker_version: None,
        resources: None,
        counts: berth_core::ResourceCounts::default(),
        owner: berth_core::OwnerId::Local,
    };
    let client = BollardEngine::connect(&engine).expect("connect over ssh");
    assert!(client.docker_host().starts_with("ssh://"));

    let info = tokio::time::timeout(Duration::from_secs(30), client.info())
        .await
        .expect("info timed out after 30 s")
        .expect("info over ssh");
    assert!(!info.docker_version.is_empty(), "{info:?}");

    let containers = tokio::time::timeout(Duration::from_secs(30), client.containers())
        .await
        .expect("containers timed out after 30 s")
        .expect("list containers over ssh");
    eprintln!("ssh_it: {} containers on {target}", containers.len());
}
