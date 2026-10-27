//! Registries (P8-T2): the stored rows, their Keychain passwords and
//! the credentials a pull or push uses. Passwords live only in the
//! secret store under `registry/<id>/password` — never in SQLite,
//! never in a log line, never in `AppState`.

use std::sync::Arc;

use berth_core::{Registry, RegistryStatus};
use berth_store::{SecretStore, Store};
use dioxus::prelude::WritableExt as _;

use crate::state::AppState;

/// The secret-store key of one registry's password.
#[must_use]
pub fn secret_key(registry_id: &str) -> String {
    format!("registry/{registry_id}/password")
}

/// The id of a registry from its URL: the host (with port), so the
/// same registry always lands on the same row.
#[must_use]
pub fn registry_id(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

/// The registry host an image reference names: its first segment when
/// it looks like a host (contains `.` or `:` or is `localhost`), else
/// Docker Hub.
#[must_use]
pub fn reference_host(reference: &str) -> String {
    let bare = reference.split('@').next().unwrap_or(reference);
    // A first segment is a host only when a path follows it: bare
    // `postgres:17` is a Hub name with a tag, not a host with a port.
    let Some(first) = bare.split_once('/').map(|(first, _)| first) else {
        return "docker.io".to_string();
    };
    if first.contains('.') || first.contains(':') || first == "localhost" {
        first.to_string()
    } else {
        "docker.io".to_string()
    }
}

/// Whether a registry row serves an image reference. Docker Hub's
/// spellings (`docker.io`, `index.docker.io`, `registry-1.docker.io`)
/// are one registry.
#[must_use]
pub fn matches(registry: &Registry, reference: &str) -> bool {
    fn normalize(host: &str) -> &str {
        match host {
            "docker.io" | "index.docker.io" | "registry-1.docker.io" => "docker.io",
            other => other,
        }
    }
    let mine = registry_id(&registry.url);
    normalize(&mine) == normalize(&reference_host(reference))
}

/// Owns registry writes: the store for rows, the secret store for
/// passwords, the v2 client for the login check.
#[derive(Clone)]
pub struct RegistriesService {
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for RegistriesService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistriesService").finish_non_exhaustive()
    }
}

impl RegistriesService {
    #[must_use]
    pub fn new(store: Arc<Store>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { store, secrets }
    }

    /// Loads the stored rows into state and returns the service.
    pub fn start(mut state: AppState, store: Arc<Store>) -> Self {
        let service = Self::new(Arc::clone(&store), berth_store::secret_store());
        if let Ok(rows) = store.registries() {
            state.registries.set(rows);
        }
        service
    }

    fn refresh(&self, state: &mut AppState) {
        if let Ok(rows) = self.store.registries() {
            state.registries.set(rows);
        }
    }

    /// Checks the login against the registry, then stores the row and
    /// the password (Keychain only). A registry that needs no login
    /// can be added without credentials by passing an empty username.
    ///
    /// # Errors
    /// A sentence: the registry refused, is unreachable, or the
    /// stores failed.
    pub async fn log_in(
        &self,
        state: &mut AppState,
        name: &str,
        url: &url::Url,
        username: &str,
        password: &secrecy::SecretString,
    ) -> Result<Registry, String> {
        let username = username.trim();
        if !username.is_empty() {
            berth_registry::RegistryClient::new()
                .check_login(url, username, password)
                .await
                .map_err(|e| e.to_string())?;
        }
        let registry = Registry {
            id: registry_id(url),
            name: if name.trim().is_empty() {
                registry_id(url)
            } else {
                name.trim().to_string()
            },
            url: url.clone(),
            username: (!username.is_empty()).then(|| username.to_string()),
            status: if username.is_empty() {
                RegistryStatus::LoggedOut
            } else {
                RegistryStatus::LoggedIn
            },
        };
        self.store
            .upsert_registry(&registry)
            .map_err(|e| e.to_string())?;
        if !username.is_empty() {
            self.secrets
                .set(&secret_key(&registry.id), password)
                .map_err(|e| e.to_string())?;
        }
        self.refresh(state);
        Ok(registry)
    }

    /// Forgets the password; the row stays, logged out.
    ///
    /// # Errors
    /// A store failure as a sentence.
    pub fn log_out(&self, state: &mut AppState, id: &str) -> Result<(), String> {
        let mut row = self
            .store
            .registries()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| "That registry is not stored".to_string())?;
        self.secrets
            .delete(&secret_key(id))
            .map_err(|e| e.to_string())?;
        row.status = RegistryStatus::LoggedOut;
        self.store
            .upsert_registry(&row)
            .map_err(|e| e.to_string())?;
        self.refresh(state);
        Ok(())
    }

    /// Removes the row and its password.
    ///
    /// # Errors
    /// A store failure as a sentence.
    pub fn remove(&self, state: &mut AppState, id: &str) -> Result<(), String> {
        self.secrets
            .delete(&secret_key(id))
            .map_err(|e| e.to_string())?;
        self.store.remove_registry(id).map_err(|e| e.to_string())?;
        self.refresh(state);
        Ok(())
    }

    /// The credentials for one reference: the logged-in registry that
    /// serves it and still has a password stored.
    #[must_use]
    pub fn credentials_for(
        &self,
        registries: &[Registry],
        reference: &str,
    ) -> Option<berth_engine::client::PullCredentials> {
        let registry = registries
            .iter()
            .find(|r| r.status == RegistryStatus::LoggedIn && matches(r, reference))?;
        let username = registry.username.clone()?;
        let password = self.secrets.get(&secret_key(&registry.id)).ok().flatten()?;
        Some(berth_engine::client::PullCredentials {
            username,
            password,
            server: registry_id(&registry.url),
        })
    }
}

#[cfg(test)]
#[path = "registries_tests.rs"]
mod tests;
