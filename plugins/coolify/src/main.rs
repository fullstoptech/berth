//! `berth-coolify`: the Coolify plugin binary. Berth starts it, speaks the
//! plugin protocol to it over stdio and never gives it anything but its
//! declared config and the secrets the user granted. Stdout carries
//! protocol lines only.

use berth_coolify::plugin::{serve, Timing};
use tokio::io::BufReader;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // reqwest is built without a TLS provider (PLAN D48).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let reader = BufReader::new(tokio::io::stdin());
    match serve(reader, tokio::io::stdout(), Timing::default()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("berth-coolify: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
