fn main() {
    berth_app::services::timing::init();
    // `berth mcp --bridge <socket> --session <uuid>` serves Berth's tools to
    // a harness over stdio (PLAN §6.11, ADR 0006) and never starts Dioxus.
    // The token comes from BERTH_BRIDGE_TOKEN. When Berth is not running
    // this fails at once with a clear stderr line and a nonzero exit.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("mcp") {
        std::process::exit(run_mcp(&argv[1..]));
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

/// The `mcp` mode: parse `--bridge`/`--session` (both `--flag value` and
/// `--flag=value`), defaulting to the app's one `bridge.sock` and a fresh
/// session, then serve until stdio closes. Exit code 0 on a clean close.
fn run_mcp(args: &[String]) -> i32 {
    init_tracing();
    let mut bridge: Option<std::path::PathBuf> = None;
    let mut session: Option<uuid::Uuid> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        let value = |iter: &mut std::slice::Iter<'_, String>| {
            inline.clone().or_else(|| iter.next().cloned())
        };
        match flag {
            "--bridge" => match value(&mut iter) {
                Some(path) => bridge = Some(path.into()),
                None => {
                    eprintln!("berth mcp: --bridge needs a socket path");
                    return 2;
                }
            },
            "--session" => match value(&mut iter) {
                Some(text) => match text.parse::<uuid::Uuid>() {
                    Ok(id) => session = Some(id),
                    Err(_) => {
                        eprintln!("berth mcp: --session needs a uuid, not {text}");
                        return 2;
                    }
                },
                None => {
                    eprintln!("berth mcp: --session needs a uuid");
                    return 2;
                }
            },
            other => {
                eprintln!("berth mcp: unknown argument {other}");
                return 2;
            }
        }
    }
    let bridge = bridge.unwrap_or_else(|| {
        berth_sys::paths::runtime_dir().join(berth_assistant::bridge::SOCKET_NAME)
    });
    let session = berth_core::SessionId(session.unwrap_or_else(uuid::Uuid::now_v7));
    let token = std::env::var("BERTH_BRIDGE_TOKEN").unwrap_or_default();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("berth mcp: could not start: {err}");
            return 1;
        }
    };
    match runtime.block_on(berth_assistant::serve_mcp_stdio(bridge, session, token)) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("berth mcp: {err}");
            1
        }
    }
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
