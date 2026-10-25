//! Update check against the GitHub releases API.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use url::Url;

use crate::{
    build_client, check, client_or_err, decode_error, endpoint, map_send_error, RegistryError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub url: Url,
    pub published: DateTime<Utc>,
}

#[derive(Deserialize)]
struct ReleaseBody {
    tag_name: String,
    html_url: Url,
    published_at: Option<DateTime<Utc>>,
    created_at: Option<DateTime<Utc>>,
}

/// `GET <api_base>/repos/<slug>/releases/latest` (GitHub skips drafts and pre-releases).
/// `Ok(None)` when the repository has no release yet.
///
/// # Errors
/// `RateLimited` when GitHub's limit is used up, `Offline`, or `Http`.
pub async fn latest_release(api_base: &Url, slug: &str) -> Result<Option<Release>, RegistryError> {
    let (owner, repo) = slug
        .split_once('/')
        .filter(|(o, r)| !o.is_empty() && !r.is_empty() && !r.contains('/'))
        .ok_or(RegistryError::NotFound)?;
    let url = endpoint(api_base, &["repos", owner, repo, "releases", "latest"])?;
    let client = build_client();
    let resp = client_or_err(client.as_ref())?
        .get(url)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(map_send_error)?;
    let resp = match check(resp) {
        Ok(r) => r,
        Err(RegistryError::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    let body: ReleaseBody = resp.json().await.map_err(decode_error)?;
    let published = body
        .published_at
        .or(body.created_at)
        .ok_or_else(|| RegistryError::Http("the release has no date".into()))?;
    Ok(Some(Release {
        tag: body.tag_name,
        url: body.html_url,
        published,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn api(s: &MockServer) -> Url {
        Url::parse(&s.uri()).unwrap()
    }

    #[tokio::test]
    async fn latest_release_maps_tag_url_and_date() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/berth/releases/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tag_name": "v0.4.1",
                "html_url": "https://github.com/owner/berth/releases/tag/v0.4.1",
                "published_at": "2026-10-20T10:00:00Z",
                "draft": false
            })))
            .mount(&s)
            .await;
        let r = latest_release(&api(&s), "owner/berth")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.tag, "v0.4.1");
        assert_eq!(
            r.url.as_str(),
            "https://github.com/owner/berth/releases/tag/v0.4.1"
        );
        assert_eq!(r.published.to_rfc3339(), "2026-10-20T10:00:00+00:00");
    }

    #[tokio::test]
    async fn a_repository_without_releases_gives_none() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&s)
            .await;
        assert_eq!(latest_release(&api(&s), "owner/berth").await.unwrap(), None);
    }

    #[tokio::test]
    async fn github_rate_limit_is_rate_limited_with_reset() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("retry-after", "90"),
            )
            .mount(&s)
            .await;
        assert_eq!(
            latest_release(&api(&s), "owner/berth").await.unwrap_err(),
            RegistryError::RateLimited {
                reset_in: Some(Duration::from_secs(90))
            }
        );
    }

    #[tokio::test]
    async fn a_slug_that_is_not_owner_slash_repo_makes_no_request() {
        let s = MockServer::start().await;
        for bad in ["", "owner", "owner/", "/repo", "a/b/c"] {
            assert_eq!(
                latest_release(&api(&s), bad).await.unwrap_err(),
                RegistryError::NotFound,
                "{bad:?}"
            );
        }
        assert!(s.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_network_is_offline() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let url = Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        assert_eq!(
            latest_release(&url, "o/r").await.unwrap_err(),
            RegistryError::Offline
        );
    }
}
