//! Prints the coding harnesses this Mac has, the way Berth finds them.
//!
//! `cargo run -q -p berth-assistant --example detect`. Only `--version` runs;
//! nothing is sent to a harness.

use std::collections::BTreeMap;

use berth_sys::SearchPath;

#[tokio::main]
async fn main() {
    let path = SearchPath::from_process().with_login_shell().await;
    for harness in berth_assistant::detect(&path, &BTreeMap::new()).await {
        println!(
            "{}: available={} version={} binary={}",
            harness.name,
            harness.available,
            harness.version.as_deref().unwrap_or("unknown"),
            harness.binary.display()
        );
    }
}
