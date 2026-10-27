use std::sync::Arc;

use berth_store::{InMemorySecretStore, SecretStore};
use secrecy::ExposeSecret as _;
use wiremock::matchers::{basic_auth, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use dioxus::prelude::*;

use super::*;
use crate::state::{use_app_state, AppState};

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { div {} }
}

fn app_state() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    (dom, state)
}

fn registry(id: &str, url: &str) -> Registry {
    Registry {
        id: id.to_string(),
        name: id.to_string(),
        url: url::Url::parse(url).expect("url"),
        username: Some("emma".to_string()),
        status: RegistryStatus::LoggedIn,
    }
}

#[test]
fn registries_reference_host_and_matching_cover_hub_and_private() {
    assert_eq!(reference_host("postgres:17"), "docker.io");
    assert_eq!(reference_host("library/postgres"), "docker.io");
    assert_eq!(reference_host("ghcr.io/acme/app:1"), "ghcr.io");
    assert_eq!(
        reference_host("127.0.0.1:56500/team/app@sha256:x"),
        "127.0.0.1:56500"
    );
    assert_eq!(reference_host("localhost:5000/app"), "localhost:5000");

    let hub = registry("docker.io", "https://index.docker.io");
    assert!(matches(&hub, "postgres:17"));
    assert!(matches(&hub, "docker.io/library/postgres"));
    assert!(!matches(&hub, "ghcr.io/acme/app"));
    let local = registry("127.0.0.1:56500", "http://127.0.0.1:56500");
    assert!(matches(&local, "127.0.0.1:56500/team/app:1"));
    assert!(!matches(&local, "postgres:17"));
}

#[test]
fn registries_credentials_come_from_the_secret_store_only_when_logged_in() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Arc::new(berth_store::Store::open(&dir.path().join("berth.db")).expect("store"));
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::default());
    secrets
        .set(
            &secret_key("ghcr.io"),
            &secrecy::SecretString::from("pw-not-real".to_string()),
        )
        .expect("set");
    let service = RegistriesService::new(store, secrets);
    let rows = [registry("ghcr.io", "https://ghcr.io")];
    let creds = service
        .credentials_for(&rows, "ghcr.io/acme/app:1")
        .expect("credentials");
    assert_eq!(creds.username, "emma");
    assert_eq!(creds.password.expose_secret(), "pw-not-real");
    assert_eq!(creds.server, "ghcr.io");
    assert!(service.credentials_for(&rows, "postgres:17").is_none());
    let mut logged_out = rows[0].clone();
    logged_out.status = RegistryStatus::LoggedOut;
    assert!(service
        .credentials_for(&[logged_out], "ghcr.io/acme/app:1")
        .is_none());
}

/// A registry that answers 401 with a Basic challenge, then 200 for
/// the right password.
async fn basic_registry(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v2/"))
        .respond_with(
            ResponseTemplate::new(401).insert_header("www-authenticate", "Basic realm=\"r\""),
        )
        .with_priority(5)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/"))
        .and(basic_auth("berthfx", "hunter2-not-real"))
        .respond_with(ResponseTemplate::new(200))
        .with_priority(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn registries_login_wrong_password_fails_and_right_one_is_stored_in_the_keychain() {
    let server = MockServer::start().await;
    basic_registry(&server).await;
    let url = url::Url::parse(&server.uri()).expect("url");
    let host_port = url.port().map(|p| format!("127.0.0.1:{p}")).expect("port");
    let dir = tempfile::tempdir().expect("dir");
    let db = dir.path().join("berth.db");
    let store = Arc::new(berth_store::Store::open(&db).expect("store"));
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::default());
    let service = RegistriesService::new(Arc::clone(&store), Arc::clone(&secrets));
    let (_dom, mut state) = app_state();

    let wrong = service
        .log_in(
            &mut state,
            "Fixture",
            &url,
            "berthfx",
            &secrecy::SecretString::from("wrong".to_string()),
        )
        .await;
    assert!(wrong.is_err(), "{wrong:?}");
    assert!(store.registries().expect("rows").is_empty());

    let row = service
        .log_in(
            &mut state,
            "Fixture",
            &url,
            "berthfx",
            &secrecy::SecretString::from("hunter2-not-real".to_string()),
        )
        .await
        .expect("login");
    assert_eq!(row.status, RegistryStatus::LoggedIn);
    assert_eq!(row.id, host_port);
    // The password is in the secret store and nowhere in the database.
    assert_eq!(
        secrets
            .get(&secret_key(&row.id))
            .expect("get")
            .expect("stored")
            .expose_secret(),
        "hunter2-not-real"
    );
    let bytes = std::fs::read(&db).expect("db bytes");
    assert!(
        !bytes.windows(16).any(|w| w == b"hunter2-not-real"),
        "the password reached SQLite"
    );
    assert_eq!(state.registries.peek().len(), 1);

    service.log_out(&mut state, &row.id).expect("logout");
    assert!(secrets.get(&secret_key(&row.id)).expect("get").is_none());
    assert_eq!(
        store.registries().expect("rows")[0].status,
        RegistryStatus::LoggedOut
    );
    service.remove(&mut state, &row.id).expect("remove");
    assert!(store.registries().expect("rows").is_empty());
}
