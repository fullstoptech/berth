//! P9-T2 tests for the plugins service. A real child process stands in for
//! a plugin: a small `sh` script that answers the protocol's lines, so these
//! tests need no binary built by another package. (The host's own
//! conformance suite drives the Rust fixture plugin.)

use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use berth_plugin_protocol::{ConfigField, FieldKind, SecretDecl};
use berth_store::InMemorySecretStore;

use super::*;

const TOKEN_KEY: &str = "demo/token";
const FAST: Duration = Duration::from_millis(10);

fn manifest() -> Manifest {
    Manifest {
        id: "demo".into(),
        name: "Demo".into(),
        version: "0.1.0".into(),
        protocol: berth_plugin_protocol::PROTOCOL_VERSION,
        executable: "demo-plugin".into(),
        permissions: vec![
            Permission::Network {
                hosts: vec!["demo.example".into()],
            },
            Permission::Secrets {
                keys: vec![TOKEN_KEY.into()],
            },
        ],
        contributes: Vec::new(),
        config: vec![
            ConfigField {
                key: "base_url".into(),
                label: "Demo address".into(),
                kind: FieldKind::Url,
                required: true,
            },
            ConfigField {
                key: "nickname".into(),
                label: "Nickname".into(),
                kind: FieldKind::Text,
                required: false,
            },
            ConfigField {
                key: "verbose".into(),
                label: "Verbose".into(),
                kind: FieldKind::Bool,
                required: false,
            },
        ],
        secrets: vec![SecretDecl {
            key: TOKEN_KEY.into(),
            label: "API token".into(),
        }],
    }
}

/// How the sh plugin behaves.
#[derive(Clone, Copy)]
enum Mode {
    /// Answers every request.
    Healthy,
    /// Exits before answering `initialize`.
    FailInit,
    /// Answers `initialize`, prints a line on stderr and exits.
    CrashAfterInit,
}

fn script(mode: Mode) -> String {
    let (before, after) = match mode {
        Mode::Healthy => ("", ""),
        Mode::FailInit => ("echo 'cannot open its database' >&2; exit 3", ""),
        Mode::CrashAfterInit => ("", "echo 'boom: database is gone' >&2; exit 7"),
    };
    format!(
        r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed 's/^{{"jsonrpc":"2.0","id":\([0-9]*\),.*/\1/')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "$line" >> init.log
      {before}
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"capabilities":{{}}}}}}\n' "$id"
      {after}
      ;;
    *'"method":"view/load"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"kind":"message","tone":"info","text":"hello"}}}}\n' "$id"
      ;;
    *'"method":"shutdown"'*) exit 0 ;;
  esac
done
"#
    )
}

struct World {
    root: tempfile::TempDir,
    store: Arc<Store>,
    secrets: Arc<InMemorySecretStore>,
}

impl World {
    fn new(mode: Mode) -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let world = Self {
            root,
            store: Arc::new(Store::open_in_memory().expect("store")),
            secrets: Arc::new(InMemorySecretStore::default()),
        };
        world.install(&manifest(), mode);
        world
    }

    fn plugin_dir(&self) -> PathBuf {
        self.root.path().join("demo")
    }

    fn install(&self, manifest: &Manifest, mode: Mode) {
        let dir = self.plugin_dir();
        std::fs::create_dir_all(&dir).expect("plugin dir");
        std::fs::write(
            dir.join("berth-plugin.json"),
            serde_json::to_string(manifest).expect("manifest json"),
        )
        .expect("write manifest");
        let exe = dir.join("demo-plugin");
        std::fs::write(&exe, script(mode)).expect("write plugin");
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    /// A service over this world, like a fresh app launch.
    fn service(&self) -> PluginsService {
        PluginsService::new(PluginsDeps {
            store: Arc::clone(&self.store),
            secrets: self.secrets.clone(),
            dirs: vec![self.root.path().to_path_buf()],
            path: Some(std::env::var_os("PATH").unwrap_or_default()),
            cancel: CancellationToken::new(),
            backoff: Backoff {
                delays: vec![FAST, FAST, FAST],
            },
        })
    }

    fn init_lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.plugin_dir().join("init.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

fn form(url: &str, token: Option<&str>) -> ConfigUpdate {
    let mut update = ConfigUpdate::default();
    update.values.insert("base_url".into(), json!(url));
    if let Some(token) = token {
        update
            .secrets
            .insert(TOKEN_KEY.into(), SecretString::from(token.to_string()));
    }
    update
}

async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !ready() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn state_of(service: &PluginsService) -> PluginState {
    service.entry("demo").expect("demo entry").plugin.state
}

// -- pure rules -----------------------------------------------------------

#[test]
fn plugins_dirs_put_the_override_first_then_the_app_then_the_data_dir() {
    let dirs = plugin_dirs_from(
        Some(OsStr::new("/dev/a:/dev/b")),
        Path::new("/data"),
        Some(Path::new("/app/bin")),
    );
    assert_eq!(
        dirs,
        [
            "/dev/a",
            "/dev/b",
            "/app/bin",
            "/app/bin/plugins",
            "/data/plugins"
        ]
        .map(PathBuf::from)
    );
    let bare = plugin_dirs_from(None, Path::new("/data"), None);
    assert_eq!(bare, [PathBuf::from("/data/plugins")]);
}

#[test]
fn plugins_dirs_add_the_bundle_resources_root_only_inside_a_bundle() {
    let in_bundle = plugin_dirs_from(
        None,
        Path::new("/data"),
        Some(Path::new("/Apps/Berth.app/Contents/MacOS")),
    );
    assert_eq!(
        in_bundle,
        [
            "/Apps/Berth.app/Contents/MacOS",
            "/Apps/Berth.app/Contents/MacOS/plugins",
            "/Apps/Berth.app/Contents/Resources/plugins",
            "/data/plugins"
        ]
        .map(PathBuf::from)
    );
}

#[test]
fn plugins_permission_sentences_name_hosts_and_keys() {
    let any = Permission::Network {
        hosts: vec!["*".into()],
    };
    assert_eq!(
        permission_sentence(&any),
        "Connect to any address on the network"
    );
    let named = Permission::Network {
        hosts: vec!["a.example".into(), "b.example".into()],
    };
    assert_eq!(
        permission_sentence(&named),
        "Connect to a.example, b.example"
    );
    let secrets = Permission::Secrets {
        keys: vec!["coolify/token".into()],
    };
    assert!(permission_sentence(&secrets).contains("coolify/token"));
    assert_eq!(
        network_hosts(&[named.clone(), Permission::Mcp, named]),
        ["a.example", "b.example"]
    );
}

// -- discovery and persistence -------------------------------------------

#[tokio::test]
async fn plugins_scan_lists_an_installed_plugin_disabled_with_no_grants() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    service.scan().await;
    let entry = service.entry("demo").expect("found");
    assert_eq!(entry.plugin.state, PluginState::Disabled);
    assert!(entry.plugin.granted.is_empty());
    assert_eq!(entry.missing_required(), ["Demo address"]);
    assert_eq!(entry.network_hosts(), ["demo.example"]);
}

#[tokio::test]
async fn plugins_enable_grants_the_manifest_permissions_saves_them_and_runs_the_plugin() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    service.scan().await;
    service.enable("demo").await.expect("enable");

    let entry = service.entry("demo").expect("entry");
    assert_eq!(entry.plugin.state, PluginState::Enabled);
    assert_eq!(entry.plugin.granted, manifest().permissions);
    let saved = world
        .store
        .plugin_config("demo")
        .expect("read")
        .expect("saved");
    assert_eq!(saved["enabled"], json!(true));
    assert_eq!(
        serde_json::from_value::<Vec<Permission>>(saved["granted"].clone()).expect("granted"),
        manifest().permissions
    );
    let view = service
        .request(
            "demo",
            "view/load",
            json!({"view": "x"}),
            Duration::from_secs(5),
        )
        .await
        .expect("view");
    assert_eq!(view["text"], json!("hello"));

    // A new launch finds the plugin enabled and starts it again.
    service.shutdown().await;
    let relaunched = world.service();
    relaunched.start().await;
    assert_eq!(state_of(&relaunched), PluginState::Enabled);
    assert!(relaunched
        .request("demo", "view/load", json!({}), Duration::from_secs(5))
        .await
        .is_ok());
    relaunched.shutdown().await;
}

#[tokio::test]
async fn plugins_disable_stops_the_process_and_revokes_the_grants() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    service.scan().await;
    service.enable("demo").await.expect("enable");
    service.disable("demo").await.expect("disable");

    let entry = service.entry("demo").expect("entry");
    assert_eq!(entry.plugin.state, PluginState::Disabled);
    assert!(entry.plugin.granted.is_empty());
    let refused = service
        .request("demo", "view/load", json!({}), Duration::from_secs(2))
        .await;
    assert!(
        matches!(refused, Err(PluginError::Exited(_))),
        "{refused:?}"
    );
    let saved = world
        .store
        .plugin_config("demo")
        .expect("read")
        .expect("saved");
    assert_eq!(saved["enabled"], json!(false));
    assert_eq!(saved["granted"], json!([]));

    let relaunched = world.service();
    relaunched.start().await;
    assert_eq!(state_of(&relaunched), PluginState::Disabled);
}

#[tokio::test]
async fn plugins_a_manifest_that_asks_for_more_needs_approval_again() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    service.scan().await;
    service.enable("demo").await.expect("enable");
    service.shutdown().await;

    let mut wider = manifest();
    wider.permissions.push(Permission::Mcp);
    world.install(&wider, Mode::Healthy);
    let relaunched = world.service();
    relaunched.start().await;
    let entry = relaunched.entry("demo").expect("entry");
    assert_eq!(entry.plugin.state, PluginState::Disabled);
    assert!(entry.plugin.granted.is_empty());
    assert!(!entry.permissions_granted());
}

// -- configure and secrets -----------------------------------------------

#[tokio::test]
async fn plugins_configure_keeps_the_secret_in_the_secret_store_and_out_of_the_database() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    service.scan().await;
    let mut update = form("https://coolify.example.com", Some("tok-123456"));
    update.values.insert("nickname".into(), json!("  prod  "));
    update.values.insert("verbose".into(), json!(true));
    service.configure("demo", update).await.expect("configure");

    let entry = service.entry("demo").expect("entry");
    assert_eq!(
        entry.config["base_url"],
        json!("https://coolify.example.com")
    );
    assert_eq!(entry.config["nickname"], json!("prod"));
    assert_eq!(entry.config["verbose"], json!(true));
    assert!(entry.secrets_set.contains(TOKEN_KEY));
    assert!(entry.missing_required().is_empty());

    let stored = world
        .secrets
        .get(&format!("plugin/demo/{TOKEN_KEY}"))
        .expect("secret store")
        .expect("token stored");
    assert_eq!(stored.expose_secret(), "tok-123456");
    let saved = world
        .store
        .plugin_config("demo")
        .expect("read")
        .expect("saved");
    assert!(
        !saved.to_string().contains("tok-123456"),
        "the token reached the database: {saved}"
    );
    assert!(!format!("{entry:?}").contains("tok-123456"));
    // Settings only; the plugin stays disabled until Enable.
    assert_eq!(entry.plugin.state, PluginState::Disabled);
    assert_eq!(saved["enabled"], json!(false));
}

#[tokio::test]
async fn plugins_configure_refuses_a_missing_or_malformed_setting_and_writes_nothing() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    service.scan().await;

    let missing = service.configure("demo", form("  ", Some("tok-1"))).await;
    assert_eq!(missing, Err("Demo address is required".to_string()));
    let ftp = service
        .configure("demo", form("ftp://x.example", None))
        .await;
    assert_eq!(
        ftp,
        Err("Demo address must start with http:// or https://".to_string())
    );
    let junk = service.configure("demo", form("not a url", None)).await;
    assert_eq!(junk, Err("Demo address is not a web address".to_string()));

    assert_eq!(world.store.plugin_config("demo").expect("read"), None);
    assert!(world
        .secrets
        .get(&format!("plugin/demo/{TOKEN_KEY}"))
        .expect("secret store")
        .is_none());
}

#[tokio::test]
async fn plugins_configure_keeps_a_stored_secret_when_the_field_is_left_empty() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    service.scan().await;
    service
        .configure("demo", form("https://a.example", Some("first")))
        .await
        .expect("first");
    // The field was typed in and cleared again: an empty value, not a key left out.
    service
        .configure("demo", form("https://b.example", Some("")))
        .await
        .expect("second");
    let stored = world
        .secrets
        .get(&format!("plugin/demo/{TOKEN_KEY}"))
        .expect("secret store")
        .expect("token still stored");
    assert_eq!(stored.expose_secret(), "first");
    assert_eq!(
        service.entry("demo").expect("entry").config["base_url"],
        json!("https://b.example")
    );
}

#[tokio::test]
async fn plugins_clear_secret_removes_it_and_refuses_an_unknown_key() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    service.scan().await;
    service
        .configure("demo", form("https://a.example", Some("first")))
        .await
        .expect("configure");
    service
        .clear_secret("demo", TOKEN_KEY)
        .await
        .expect("clear");
    assert!(world
        .secrets
        .get(&format!("plugin/demo/{TOKEN_KEY}"))
        .expect("secret store")
        .is_none());
    assert!(!service
        .entry("demo")
        .expect("entry")
        .secrets_set
        .contains(TOKEN_KEY));
    let unknown = service.clear_secret("demo", "other/key").await;
    assert!(unknown.is_err(), "{unknown:?}");
}

#[tokio::test]
async fn plugins_configure_restarts_a_running_plugin_with_the_new_settings() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    service.scan().await;
    service
        .configure("demo", form("https://a.example", None))
        .await
        .expect("configure");
    service.enable("demo").await.expect("enable");
    service
        .configure("demo", form("https://b.example", None))
        .await
        .expect("reconfigure");

    let inits = world.init_lines();
    assert_eq!(inits.len(), 2, "{inits:?}");
    assert!(inits[0].contains("https://a.example"), "{}", inits[0]);
    assert!(inits[1].contains("https://b.example"), "{}", inits[1]);
    assert_eq!(state_of(&service), PluginState::Enabled);
    service.shutdown().await;
}

// -- failures ------------------------------------------------------------

#[tokio::test]
async fn plugins_a_plugin_that_cannot_start_shows_error_with_the_reason() {
    let world = World::new(Mode::FailInit);
    let service = world.service();
    service.scan().await;
    service
        .enable("demo")
        .await
        .expect("enable records the grant");

    let entry = service.entry("demo").expect("entry");
    let PluginState::Error { reason } = &entry.plugin.state else {
        panic!("expected Error, got {:?}", entry.plugin.state);
    };
    assert!(
        reason.contains("initialize failed") || reason.contains("exited"),
        "{reason}"
    );
    assert_eq!(entry.crash_log.as_slice(), std::slice::from_ref(reason));
    // It stays enabled in the store: Try again restarts it.
    let saved = world
        .store
        .plugin_config("demo")
        .expect("read")
        .expect("saved");
    assert_eq!(saved["enabled"], json!(true));
    let refused = service
        .request("demo", "view/load", json!({}), Duration::from_secs(1))
        .await;
    assert!(refused.is_err());
}

#[tokio::test]
async fn plugins_a_plugin_that_keeps_crashing_ends_in_error_with_its_last_output() {
    let world = World::new(Mode::CrashAfterInit);
    let service = world.service();
    service.scan().await;
    service.enable("demo").await.expect("enable");

    until("the restart budget runs out", || {
        matches!(state_of(&service), PluginState::Error { .. })
    })
    .await;
    let entry = service.entry("demo").expect("entry");
    assert!(
        entry
            .crash_log
            .iter()
            .any(|l| l.contains("boom: database is gone")),
        "{:?}",
        entry.crash_log
    );
    assert!(entry.restarts >= 1, "{}", entry.restarts);
    let PluginState::Error { reason } = &entry.plugin.state else {
        panic!("expected Error");
    };
    assert!(reason.contains("restarts exhausted"), "{reason}");
}

#[tokio::test]
async fn plugins_restart_runs_a_failed_plugin_again() {
    let world = World::new(Mode::FailInit);
    let service = world.service();
    service.scan().await;
    service.enable("demo").await.expect("enable");
    assert!(matches!(state_of(&service), PluginState::Error { .. }));

    world.install(&manifest(), Mode::Healthy);
    service.restart("demo").await.expect("restart");
    assert_eq!(state_of(&service), PluginState::Enabled);
    assert!(service.entry("demo").expect("entry").crash_log.is_empty());
    service.shutdown().await;

    let disabled = world.service();
    disabled.scan().await;
    disabled.disable("demo").await.expect("disable");
    assert!(disabled.restart("demo").await.is_err());
}

#[tokio::test]
async fn plugins_every_change_is_published_to_subscribers() {
    let world = World::new(Mode::Healthy);
    let service = world.service();
    let mut rx = service.subscribe();
    assert!(rx.borrow_and_update().is_empty());
    service.scan().await;
    assert!(rx.has_changed().expect("open"));
    assert_eq!(rx.borrow_and_update().len(), 1);
    service.enable("demo").await.expect("enable");
    assert!(rx.has_changed().expect("open"));
    assert_eq!(rx.borrow_and_update()[0].plugin.state, PluginState::Enabled);
    service.shutdown().await;
}
