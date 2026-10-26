fn main() {
    berth_app::services::timing::init();
    // `berth mcp` serves the MCP bridge (D44); P7-T2 fills it in.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        eprintln!("not implemented yet");
        std::process::exit(2);
    }

    // D48: rustls ring provider, installed once before anything opens TLS.
    let _ = rustls::crypto::ring::default_provider().install_default();

    init_tracing();
    berth_app::services::timing::mark("main");

    // The login shell's PATH takes 0.1-3 s; ask for it while the window is being built. A
    // scene or the mock world never runs a real binary.
    let mock = std::env::var("BERTH_MOCK").is_ok_and(|v| v == "1");
    if !mock && std::env::var_os("BERTH_SCENE").is_none() {
        berth_app::services::login_path::prewarm();
    }

    let config = berth_app::platform::config();
    berth_app::services::timing::mark("config");
    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(berth_app::app::App);
}

/// D15, minus the file sink until berth_sys::paths lands in P1-T1: stderr,
/// BERTH_LOG overrides, noisy engine/http crates pinned at warn.
fn init_tracing() {
    let filter = std::env::var("BERTH_LOG")
        .unwrap_or_else(|_| "info,bollard=warn,hyper=warn,reqwest=warn,rmcp=warn".to_string());
    let env_filter = tracing_subscriber::EnvFilter::try_new(filter)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiles() {}
}
