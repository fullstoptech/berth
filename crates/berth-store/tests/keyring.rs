//! Live Keychain round trip. Run with:
//!   BERTH_KEYRING_TEST=1 cargo nextest run -p berth-store --run-ignored only keyring
//! The test deletes its own item; nothing is left in the Keychain.

use berth_store::{KeyringStore, SecretStore};
use secrecy::ExposeSecret;

#[test]
#[ignore = "touches the real Keychain; needs BERTH_KEYRING_TEST=1"]
fn keyring_round_trip() {
    if std::env::var("BERTH_KEYRING_TEST").is_err() {
        eprintln!("BERTH_KEYRING_TEST not set, skipping");
        return;
    }
    let store = KeyringStore;
    let key = "test/roundtrip";
    store.delete(key).expect("clean slate");
    assert!(store.get(key).expect("get missing").is_none());
    let value = secrecy::SecretString::from("round-trip-value".to_string());
    store.set(key, &value).expect("set");
    let got = store.get(key).expect("get").expect("value present");
    assert_eq!(got.expose_secret(), "round-trip-value");
    store.delete(key).expect("delete");
    assert!(store.get(key).expect("get after delete").is_none());
}
