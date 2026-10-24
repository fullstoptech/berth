//! Live Colima read (no start/stop). Run with:
//!   BERTH_IT=1 cargo nextest run -p berth-engine --run-ignored only colima_it

use berth_engine::colima::ColimaControl;
use berth_engine::ColimaCli;

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn colima_it_default_profile() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let cli = ColimaCli::new(search);
    assert!(cli.binary().is_some(), "colima binary found");
    let profiles = cli.profiles().await.expect("profiles");
    assert!(profiles.iter().any(|p| p.name == "default"), "{profiles:?}");
    let status = cli.status("default").await.expect("status");
    println!(
        "profile={} vm_type={:?} rosetta={:?} socket={:?}",
        status.name, status.vm_type, status.rosetta, status.docker_socket
    );
    assert_eq!(status.name, "default");
    assert_eq!(status.vm_type, Some(berth_core::VmType::Vz));
    assert_eq!(status.rosetta, Some(false));
    let socket = status.docker_socket.expect("socket");
    assert!(
        socket
            .to_string_lossy()
            .ends_with(".colima/default/docker.sock"),
        "{}",
        socket.display()
    );
}
