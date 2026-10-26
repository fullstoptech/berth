//! Coolify API client for the Berth Coolify plugin (P9-T3, PLAN D46/D76).
//!
//! Hand-written `reqwest` client over the Coolify v4.5.0 OpenAPI surface
//! (vendored at `spec/openapi-v4.5.0.json`). The plugin binary's host
//! lifecycle (initialize, view/load, secret.get) lands with P9-T2/T5; this
//! library is the crate-only slice: response types, the HTTP client and the
//! pure mappings from API types to `berth-plugin-protocol` views.

pub mod actions;
pub mod badges;
pub mod client;
pub mod logs;
pub mod mcp;
pub mod types;
pub mod views;

pub use client::{ClientError, CoolifyClient};

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_manifest_parses_against_the_protocol() {
        let manifest: berth_plugin_protocol::Manifest =
            serde_json::from_str(include_str!("../berth-plugin.json"))
                .expect("berth-plugin.json is a valid Manifest");
        assert_eq!(manifest.id, "coolify");
        assert_eq!(manifest.protocol, berth_plugin_protocol::PROTOCOL_VERSION);
        assert!(manifest
            .secrets
            .iter()
            .any(|secret| secret.key == "coolify/token"));
    }
}
