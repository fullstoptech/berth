//! Docker Hub search and tags, a registry v2 client with login check, and the
//! release check for the app itself (PLAN §6.8, D50).
//!
//! This crate never stores credentials: callers pass a `SecretString` read from
//! `berth_store::SecretStore` and nothing here logs or keeps it.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

mod hub;
mod registry;
mod release;

pub use hub::{has_arm64, HubClient};
pub use registry::RegistryClient;
pub use release::{latest_release, Release};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("Docker Hub limits anonymous searches")]
    RateLimited { reset_in: Option<Duration> },
    #[error("offline")]
    Offline,
    #[error("login required")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Http(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Publisher {
    Official,
    Verified,
    SponsoredOss,
    Community,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageHit {
    pub name: String,
    pub publisher: Publisher,
    pub stars: u32,
    /// Docker Hub shows pull counts as "1B+", so this is a lower bound.
    pub pulls: u64,
    pub updated: Option<DateTime<Utc>>,
    pub description: String,
    /// `None` when the endpoint does not say which platforms the image has.
    pub arm64: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagInfo {
    pub name: String,
    pub digest: Option<String>,
    pub pushed: Option<DateTime<Utc>>,
    pub per_arch: Vec<(berth_core::Arch, bytesize::ByteSize)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// URL of the next page as the server gave it (or built from the same query).
    pub next: Option<String>,
}

/// Builds the one HTTP client shape this crate uses. `None` only if TLS setup fails.
pub(crate) fn build_client() -> Option<reqwest::Client> {
    static PROVIDER: std::sync::Once = std::sync::Once::new();
    // reqwest is built with `rustls-no-provider` (D48); the app installs ring at startup,
    // this keeps library users and tests working without that.
    PROVIDER.call_once(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
    });
    reqwest::Client::builder()
        .user_agent(concat!("Berth/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .ok()
}

pub(crate) fn client_or_err(
    client: Option<&reqwest::Client>,
) -> Result<&reqwest::Client, RegistryError> {
    client.ok_or_else(|| RegistryError::Http("could not set up the HTTP client".to_string()))
}

/// Connection failures and timeouts are "offline"; the rest keep their message.
pub(crate) fn map_send_error(e: reqwest::Error) -> RegistryError {
    if e.is_connect() || e.is_timeout() {
        RegistryError::Offline
    } else {
        RegistryError::Http(e.without_url().to_string())
    }
}

pub(crate) fn decode_error(e: reqwest::Error) -> RegistryError {
    if e.is_decode() {
        RegistryError::Http(format!("unexpected response: {}", e.without_url()))
    } else {
        map_send_error(e)
    }
}

fn header_u64(h: &HeaderMap, name: &str) -> Option<u64> {
    h.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// `Retry-After` (seconds) wins; otherwise `X-RateLimit-Reset`, which Docker Hub and
/// GitHub send as a Unix timestamp in seconds.
pub(crate) fn rate_limit_reset(h: &HeaderMap, now_epoch: u64) -> Option<Duration> {
    if let Some(secs) = header_u64(h, "retry-after") {
        return Some(Duration::from_secs(secs));
    }
    let reset = header_u64(h, "x-ratelimit-reset")?;
    if reset < 1_000_000_000 {
        return Some(Duration::from_secs(reset));
    }
    Some(Duration::from_secs(reset.saturating_sub(now_epoch)))
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Success passes through; everything else becomes a `RegistryError`.
pub(crate) fn check(resp: reqwest::Response) -> Result<reqwest::Response, RegistryError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let h = resp.headers();
    let exhausted = h
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim() == "0");
    Err(match status.as_u16() {
        429 => RegistryError::RateLimited {
            reset_in: rate_limit_reset(h, now_epoch()),
        },
        403 if exhausted => RegistryError::RateLimited {
            reset_in: rate_limit_reset(h, now_epoch()),
        },
        401 | 403 => RegistryError::Unauthorized,
        404 => RegistryError::NotFound,
        code => RegistryError::Http(format!("HTTP {code}")),
    })
}

pub(crate) fn parse_time(s: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Appends path segments to `base`, keeping any path prefix it already has.
pub(crate) fn endpoint(base: &url::Url, segments: &[&str]) -> Result<url::Url, RegistryError> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|()| RegistryError::Http("not a base URL".to_string()))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn reset_prefers_retry_after() {
        let h = headers(&[("retry-after", "37"), ("x-ratelimit-reset", "2000000000")]);
        assert_eq!(
            rate_limit_reset(&h, 1_999_999_000),
            Some(Duration::from_secs(37))
        );
    }

    #[test]
    fn reset_from_epoch_timestamp() {
        let h = headers(&[("x-ratelimit-reset", "1792935657")]);
        assert_eq!(
            rate_limit_reset(&h, 1_792_935_600),
            Some(Duration::from_secs(57))
        );
        // already past: zero, not a panic or a huge wrap-around
        assert_eq!(
            rate_limit_reset(&h, 1_792_935_700),
            Some(Duration::from_secs(0))
        );
    }

    #[test]
    fn reset_none_without_headers() {
        assert_eq!(rate_limit_reset(&HeaderMap::new(), 5), None);
    }

    #[test]
    fn endpoint_keeps_prefix_and_ignores_trailing_slash() {
        let base = url::Url::parse("http://h:1/prefix/").unwrap();
        assert_eq!(
            endpoint(&base, &["v2", "x"]).unwrap().as_str(),
            "http://h:1/prefix/v2/x"
        );
        let bare = url::Url::parse("http://h:1").unwrap();
        assert_eq!(endpoint(&bare, &["v2"]).unwrap().as_str(), "http://h:1/v2");
    }
}
