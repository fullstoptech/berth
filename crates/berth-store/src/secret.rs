use secrecy::ExposeSecret;

use crate::SecretError;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<secrecy::SecretString>, SecretError>;
    fn set(&self, key: &str, value: &secrecy::SecretString) -> Result<(), SecretError>;
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

#[cfg(not(test))]
const KEYRING_SERVICE: &str = "dev.berth";

pub struct KeyringStore;

#[cfg(not(test))]
fn entry(key: &str) -> Result<keyring::Entry, SecretError> {
    keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| SecretError::Backend(e.to_string()))
}

#[cfg(not(test))]
impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<secrecy::SecretString>, SecretError> {
        match entry(key)?.get_password() {
            Ok(value) => Ok(Some(secrecy::SecretString::from(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Backend(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &secrecy::SecretString) -> Result<(), SecretError> {
        entry(key)?
            .set_password(value.expose_secret())
            .map_err(|e| SecretError::Backend(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Backend(e.to_string())),
        }
    }
}

// Unit tests must never touch the real Keychain; the live round trip is the
// ignored integration test in tests/keyring.rs (which links this crate
// without cfg(test)).
#[cfg(test)]
impl SecretStore for KeyringStore {
    fn get(&self, _key: &str) -> Result<Option<secrecy::SecretString>, SecretError> {
        Err(SecretError::Backend(
            "keychain disabled in unit tests".to_string(),
        ))
    }

    fn set(&self, _key: &str, _value: &secrecy::SecretString) -> Result<(), SecretError> {
        Err(SecretError::Backend(
            "keychain disabled in unit tests".to_string(),
        ))
    }

    fn delete(&self, _key: &str) -> Result<(), SecretError> {
        Err(SecretError::Backend(
            "keychain disabled in unit tests".to_string(),
        ))
    }
}

#[derive(Default)]
pub struct InMemorySecretStore {
    values: std::sync::Mutex<std::collections::BTreeMap<String, secrecy::SecretString>>,
}

impl InMemorySecretStore {
    fn lock(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, std::collections::BTreeMap<String, secrecy::SecretString>>,
        SecretError,
    > {
        self.values
            .lock()
            .map_err(|_| SecretError::Backend("in-memory secret store lock poisoned".to_string()))
    }
}

impl SecretStore for InMemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<secrecy::SecretString>, SecretError> {
        Ok(self.lock()?.get(key).cloned())
    }

    fn set(&self, key: &str, value: &secrecy::SecretString) -> Result<(), SecretError> {
        self.lock()?.insert(key.to_string(), value.clone());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.lock()?.remove(key);
        Ok(())
    }
}

/// The app's secret store: Keychain, or memory when BERTH_KEYRING=memory or
/// the Keychain does not answer (with a warning; values then last one run).
pub fn secret_store() -> std::sync::Arc<dyn SecretStore> {
    if std::env::var("BERTH_KEYRING").is_ok_and(|v| v == "memory") {
        return std::sync::Arc::new(InMemorySecretStore::default());
    }
    let keyring = KeyringStore;
    match keyring.get("__berth_probe__") {
        Ok(_) => std::sync::Arc::new(keyring),
        Err(err) => {
            tracing::warn!(%err, "keychain unavailable; secrets will not persist between runs");
            std::sync::Arc::new(InMemorySecretStore::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(value: &str) -> secrecy::SecretString {
        secrecy::SecretString::from(value.to_string())
    }

    #[test]
    fn in_memory_round_trip_and_delete() {
        let store = InMemorySecretStore::default();
        assert!(store.get("registry/hub/password").expect("get").is_none());
        store
            .set("registry/hub/password", &secret("s3cret"))
            .expect("set");
        let got = store
            .get("registry/hub/password")
            .expect("get")
            .expect("present");
        assert_eq!(got.expose_secret(), "s3cret");
        store.delete("registry/hub/password").expect("delete");
        assert!(store.get("registry/hub/password").expect("get").is_none());
        store
            .delete("registry/hub/password")
            .expect("deleting a missing key is fine");
    }

    #[test]
    fn in_memory_set_overwrites() {
        let store = InMemorySecretStore::default();
        store.set("k", &secret("old")).expect("set");
        store.set("k", &secret("new")).expect("set");
        assert_eq!(
            store
                .get("k")
                .expect("get")
                .expect("present")
                .expose_secret(),
            "new"
        );
    }

    #[test]
    fn secret_store_falls_back_to_memory_in_unit_tests() {
        let store = secret_store();
        store.set("k", &secret("v")).expect("fallback store works");
        assert_eq!(
            store
                .get("k")
                .expect("get")
                .expect("present")
                .expose_secret(),
            "v"
        );
    }
}
