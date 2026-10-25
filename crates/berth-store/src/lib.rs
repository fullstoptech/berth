pub mod secret;
pub mod settings;
pub mod store;

pub use secret::{secret_store, InMemorySecretStore, KeyringStore, SecretStore};
pub use settings::{Density, Settings, Theme};
pub use store::{ActivityFilter, EngineSnapshot, SessionRecord, Store, UserEngine};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Sql(String),
    #[error("migration {0} failed: {1}")]
    Migration(u32, String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("keychain: {0}")]
    Backend(String),
    #[error("no secret stored for {0}")]
    Missing(String),
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiles() {}
}
