//! Docker Hub search and tags.
//!
//! Search uses `GET /api/search/v3/catalog/search?query=&size=&from=`, the endpoint the
//! Hub website itself calls. Its `source` field tells Official (`store`), Verified
//! Publisher (`verified_publisher`) and Docker-Sponsored Open Source (`open_source`)
//! from Community (`community`), and `rate_plans[0].architectures` lists the platforms.
//! It is not a documented API, so when it fails with anything but rate limiting or
//! offline the client falls back to the documented `GET /v2/search/repositories/`,
//! which only knows Official and Community and says nothing about platforms.

use berth_core::Arch;
use bytesize::ByteSize;
use serde::Deserialize;
use url::Url;

use crate::{
    build_client, check, client_or_err, decode_error, endpoint, map_send_error, parse_time,
    ImageHit, Page, Publisher, RegistryError, TagInfo,
};

const MAX_PAGE: u32 = 100;

pub struct HubClient {
    client: Option<reqwest::Client>,
    base: Url,
}

impl HubClient {
    /// `base` is `https://hub.docker.com`; tests pass a wiremock URL.
    #[must_use]
    pub fn new(base: Url) -> Self {
        Self {
            client: build_client(),
            base,
        }
    }

    /// # Errors
    /// `RateLimited` on HTTP 429, `Offline` when Docker Hub cannot be reached,
    /// `Http` for any other failure of both search endpoints.
    pub async fn search(
        &self,
        query: &str,
        page_size: u32,
    ) -> Result<Page<ImageHit>, RegistryError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Page {
                items: Vec::new(),
                next: None,
            });
        }
        let size = page_size.clamp(1, MAX_PAGE);
        match self.search_v3(query, size).await {
            Err(RegistryError::Http(_) | RegistryError::NotFound) => {
                self.search_v2(query, size).await
            }
            other => other,
        }
    }

    /// `repo` is `postgres` (meaning `library/postgres`) or `namespace/name`.
    ///
    /// # Errors
    /// `NotFound` when the repository does not exist, `RateLimited`, `Offline` or `Http`.
    pub async fn tags(&self, repo: &str, page_size: u32) -> Result<Page<TagInfo>, RegistryError> {
        let segments: Vec<&str> = repo.split('/').collect();
        let (ns, name) = match segments.as_slice() {
            [name] => ("library", *name),
            [ns, name] => (*ns, *name),
            _ => return Err(RegistryError::NotFound),
        };
        if ns.is_empty() || name.is_empty() {
            return Err(RegistryError::NotFound);
        }
        let mut url = endpoint(&self.base, &["v2", "repositories", ns, name, "tags"])?;
        url.query_pairs_mut()
            .append_pair("page_size", &page_size.clamp(1, MAX_PAGE).to_string());
        let body: TagsBody = self.get_json(url).await?;
        Ok(Page {
            items: body.results.into_iter().map(TagDto::into_info).collect(),
            next: body.next.filter(|n| !n.is_empty()),
        })
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: Url) -> Result<T, RegistryError> {
        let resp = client_or_err(self.client.as_ref())?
            .get(url)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(map_send_error)?;
        check(resp)?.json::<T>().await.map_err(decode_error)
    }

    async fn search_v3(&self, query: &str, size: u32) -> Result<Page<ImageHit>, RegistryError> {
        let mut url = endpoint(&self.base, &["api", "search", "v3", "catalog", "search"])?;
        url.query_pairs_mut()
            .append_pair("query", query)
            .append_pair("size", &size.to_string());
        let body: V3Body = self.get_json(url.clone()).await?;
        let returned = body.results.len() as u64;
        let items: Vec<ImageHit> = body
            .results
            .into_iter()
            .filter_map(V3Hit::into_hit)
            .collect();
        let next = (returned >= u64::from(size) && returned < body.total).then(|| {
            let mut n = url;
            n.query_pairs_mut()
                .append_pair("from", &returned.to_string());
            n.to_string()
        });
        Ok(Page { items, next })
    }

    async fn search_v2(&self, query: &str, size: u32) -> Result<Page<ImageHit>, RegistryError> {
        let mut url = endpoint(&self.base, &["v2", "search", "repositories", ""])?;
        url.query_pairs_mut()
            .append_pair("query", query)
            .append_pair("page_size", &size.to_string());
        let body: V2Body = self.get_json(url).await?;
        Ok(Page {
            items: body.results.into_iter().map(V2Hit::into_hit).collect(),
            next: body.next.filter(|n| !n.is_empty()),
        })
    }
}

/// True when any of these tags has an arm64 image. Callers use it on the first page of
/// `HubClient::tags` to settle `ImageHit::arm64` when search could not say.
#[must_use]
pub fn has_arm64(tags: &[TagInfo]) -> bool {
    tags.iter()
        .any(|t| t.per_arch.iter().any(|(a, _)| *a == Arch::Aarch64))
}

// ---------- v3 search ----------

#[derive(Deserialize)]
struct V3Body {
    #[serde(default)]
    total: u64,
    #[serde(default)]
    results: Vec<V3Hit>,
}

#[derive(Deserialize)]
struct V3Hit {
    id: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    short_description: Option<String>,
    #[serde(default)]
    star_count: u32,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    rate_plans: Vec<V3Plan>,
}

#[derive(Deserialize, Default)]
struct V3Plan {
    #[serde(default)]
    repositories: Vec<V3Repo>,
    #[serde(default)]
    architectures: Vec<V3Arch>,
}

#[derive(Deserialize)]
struct V3Repo {
    #[serde(default)]
    pull_count: Option<PullCount>,
    #[serde(default)]
    is_official: bool,
}

#[derive(Deserialize)]
struct V3Arch {
    name: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PullCount {
    Number(u64),
    Text(String),
}

impl PullCount {
    fn value(&self) -> u64 {
        match self {
            Self::Number(n) => *n,
            Self::Text(s) => parse_pull_count(s),
        }
    }
}

/// "1B+" -> 1_000_000_000, "500K+" -> 500_000, "<1K" -> 0, "12" -> 12; anything else 0.
fn parse_pull_count(s: &str) -> u64 {
    let s = s.trim().trim_end_matches('+').replace(',', "");
    if s.starts_with('<') {
        return 0;
    }
    let (digits, factor) = match s.chars().last() {
        Some('K' | 'k') => (&s[..s.len() - 1], 1_000),
        Some('M' | 'm') => (&s[..s.len() - 1], 1_000_000),
        Some('B' | 'b') => (&s[..s.len() - 1], 1_000_000_000),
        _ => (s.as_str(), 1),
    };
    digits
        .parse::<u64>()
        .map_or(0, |n| n.saturating_mul(factor))
}

impl V3Hit {
    /// `None` for results that are not plain images (Docker Hardened Images need a
    /// different registry and login; MCP servers and models are not images).
    fn into_hit(self) -> Option<ImageHit> {
        if self.kind != "image" {
            return None;
        }
        let plan = self.rate_plans.first();
        let repo = plan.and_then(|p| p.repositories.first());
        let publisher = if self.source == "store" || repo.is_some_and(|r| r.is_official) {
            Publisher::Official
        } else if self.source == "verified_publisher" {
            Publisher::Verified
        } else if self.source == "open_source" {
            Publisher::SponsoredOss
        } else {
            Publisher::Community
        };
        let arm64 = plan
            .map(|p| &p.architectures)
            .filter(|a| !a.is_empty())
            .map(|a| a.iter().any(|x| x.name == "arm64"));
        Some(ImageHit {
            name: self
                .id
                .strip_prefix("library/")
                .unwrap_or(&self.id)
                .to_string(),
            publisher,
            stars: self.star_count,
            pulls: repo
                .and_then(|r| r.pull_count.as_ref())
                .map_or(0, PullCount::value),
            updated: parse_time(self.updated_at.as_deref()),
            description: self.short_description.unwrap_or_default(),
            arm64,
        })
    }
}

// ---------- v2 search ----------

#[derive(Deserialize)]
struct V2Body {
    #[serde(default)]
    next: Option<String>,
    #[serde(default)]
    results: Vec<V2Hit>,
}

#[derive(Deserialize)]
struct V2Hit {
    repo_name: String,
    #[serde(default)]
    short_description: Option<String>,
    #[serde(default)]
    star_count: u32,
    #[serde(default)]
    pull_count: u64,
    #[serde(default)]
    is_official: bool,
}

impl V2Hit {
    fn into_hit(self) -> ImageHit {
        ImageHit {
            name: self.repo_name,
            publisher: if self.is_official {
                Publisher::Official
            } else {
                Publisher::Community
            },
            stars: self.star_count,
            pulls: self.pull_count,
            updated: None,
            description: self.short_description.unwrap_or_default(),
            arm64: None,
        }
    }
}

// ---------- tags ----------

#[derive(Deserialize)]
struct TagsBody {
    #[serde(default)]
    next: Option<String>,
    #[serde(default)]
    results: Vec<TagDto>,
}

#[derive(Deserialize)]
struct TagDto {
    name: String,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    tag_last_pushed: Option<String>,
    #[serde(default)]
    last_updated: Option<String>,
    #[serde(default)]
    images: Vec<TagImage>,
}

#[derive(Deserialize)]
struct TagImage {
    #[serde(default)]
    architecture: String,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    size: u64,
}

impl TagDto {
    fn into_info(self) -> TagInfo {
        let mut per_arch: Vec<(Arch, ByteSize)> = Vec::new();
        for img in &self.images {
            // "unknown" entries are attestations, not runnable platforms.
            if img.architecture.is_empty()
                || img.architecture == "unknown"
                || img.os.as_deref().is_some_and(|os| os != "linux")
            {
                continue;
            }
            let arch = match (img.architecture.as_str(), img.variant.as_deref()) {
                ("arm", Some(v)) => Arch::Other(format!("arm/{v}")),
                (a, _) => Arch::parse(a),
            };
            if per_arch.iter().all(|(a, _)| *a != arch) {
                per_arch.push((arch, ByteSize::b(img.size)));
            }
        }
        TagInfo {
            name: self.name,
            digest: self.digest,
            pushed: parse_time(
                self.tag_last_pushed
                    .as_deref()
                    .or(self.last_updated.as_deref()),
            ),
            per_arch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fixture(name: &str) -> String {
        let p = format!("{}/../../fixtures/hub/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
    }

    fn json(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json")
    }

    async fn client(server: &MockServer) -> HubClient {
        HubClient::new(Url::parse(&server.uri()).unwrap())
    }

    const V3: &str = "/api/search/v3/catalog/search";

    #[tokio::test]
    async fn search_maps_official_and_verified_from_the_recorded_page() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(V3))
            .and(query_param("query", "postgres"))
            .and(query_param("size", "10"))
            .respond_with(json(&fixture("search-postgres.json")))
            .mount(&s)
            .await;
        let page = client(&s).await.search("postgres", 10).await.unwrap();

        let first = &page.items[0];
        assert_eq!(first.name, "postgres");
        assert_eq!(first.publisher, Publisher::Official);
        assert_eq!(first.stars, 15038);
        assert_eq!(first.pulls, 1_000_000_000);
        assert_eq!(first.arm64, Some(true));
        assert!(first.updated.is_some());
        assert!(first.description.starts_with("The PostgreSQL"));

        let by_name = |n: &str| page.items.iter().find(|h| h.name == n).unwrap();
        assert_eq!(by_name("circleci/postgres").publisher, Publisher::Verified);
        assert_eq!(by_name("circleci/postgres").pulls, 500_000_000);
        assert_eq!(
            by_name("artifacthub/postgres").publisher,
            Publisher::SponsoredOss
        );
        // Docker Hardened Images (type "dhi") are not pullable as plain Hub images.
        assert!(page.items.iter().all(|h| h.name != "dhi/postgres"));
        // 10 asked, 10 returned, 53k exist: there is a next page at offset 10.
        assert!(page.next.unwrap().contains("from=10"));
    }

    #[tokio::test]
    async fn search_maps_every_publisher_source() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(V3))
            .respond_with(json(&fixture("search-sources.json")))
            .mount(&s)
            .await;
        let page = client(&s).await.search("redis", 100).await.unwrap();
        let got: Vec<_> = page
            .items
            .iter()
            .map(|h| (h.name.as_str(), h.publisher))
            .collect();
        assert_eq!(
            got,
            vec![
                ("redis", Publisher::Official),
                ("redis/redis-stack-server", Publisher::Verified),
                ("dtagdevsec/redis", Publisher::SponsoredOss),
                ("bitnami/redis", Publisher::Community),
            ]
        );
    }

    #[tokio::test]
    async fn search_arm64_is_false_when_platforms_are_listed_without_it() {
        let s = MockServer::start().await;
        let body = r#"{"total":1,"results":[{"id":"someone/old","type":"image","source":"community",
          "rate_plans":[{"repositories":[{"pull_count":"10K+","is_official":false}],
          "architectures":[{"name":"amd64"},{"name":"arm"}]}]},
          {"id":"someone/unknown","type":"image","source":"community",
          "rate_plans":[{"repositories":[],"architectures":[]}]}]}"#;
        Mock::given(method("GET"))
            .and(path(V3))
            .respond_with(json(body))
            .mount(&s)
            .await;
        let page = client(&s).await.search("old", 10).await.unwrap();
        assert_eq!(page.items[0].arm64, Some(false));
        assert_eq!(page.items[0].pulls, 10_000);
        assert_eq!(page.items[1].arm64, None);
        assert_eq!(page.next, None);
    }

    #[tokio::test]
    async fn search_with_no_matches_is_an_empty_page() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(V3))
            .respond_with(json(&fixture("search-empty.json")))
            .mount(&s)
            .await;
        let page = client(&s).await.search("zzzzqqxx9", 10).await.unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.next, None);
    }

    #[tokio::test]
    async fn blank_query_makes_no_request() {
        let s = MockServer::start().await;
        let page = client(&s).await.search("   ", 10).await.unwrap();
        assert!(page.items.is_empty());
        assert!(s.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn search_falls_back_to_the_documented_endpoint() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(V3))
            .respond_with(ResponseTemplate::new(404))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/search/repositories/"))
            .and(query_param("query", "postgres"))
            .respond_with(json(&fixture("search-v2-postgres.json")))
            .mount(&s)
            .await;
        let page = client(&s).await.search("postgres", 5).await.unwrap();
        assert_eq!(page.items[0].name, "postgres");
        assert_eq!(page.items[0].publisher, Publisher::Official);
        assert_eq!(page.items[0].pulls, 12_026_714_878);
        assert_eq!(page.items[1].publisher, Publisher::Community);
        assert!(page
            .items
            .iter()
            .all(|h| h.arm64.is_none() && h.updated.is_none()));
    }

    #[tokio::test]
    async fn rate_limit_on_search_does_not_fall_back() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(V3))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "12"))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/search/repositories/"))
            .respond_with(json("{}"))
            .expect(0)
            .mount(&s)
            .await;
        let err = client(&s).await.search("x", 5).await.unwrap_err();
        assert_eq!(
            err,
            RegistryError::RateLimited {
                reset_in: Some(Duration::from_secs(12))
            }
        );
    }

    #[tokio::test]
    async fn tags_report_size_per_architecture() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/repositories/library/postgres/tags"))
            .and(query_param("page_size", "5"))
            .respond_with(json(&fixture("tags-postgres.json")))
            .mount(&s)
            .await;
        let page = client(&s).await.tags("postgres", 5).await.unwrap();
        assert_eq!(page.items.len(), 5);
        assert!(page.next.is_some());

        let t = &page.items[0];
        assert_eq!(t.name, "19rc1-trixie");
        assert!(t.digest.as_deref().unwrap().starts_with("sha256:7c70"));
        assert_eq!(
            t.pushed.unwrap().to_rfc3339(),
            "2026-10-23T07:08:39.790009+00:00"
        );
        let size = |a: &Arch| {
            t.per_arch
                .iter()
                .find(|(x, _)| x == a)
                .map(|(_, s)| s.as_u64())
        };
        assert_eq!(size(&Arch::X86_64), Some(164_233_533));
        assert_eq!(size(&Arch::Aarch64), Some(162_828_373));
        // arm v5 and v7 are different platforms; attestation entries are dropped.
        assert_eq!(size(&Arch::Other("arm/v7".into())), Some(88_971_783));
        assert_eq!(size(&Arch::Other("arm/v5".into())), Some(92_702_505));
        assert!(t
            .per_arch
            .iter()
            .all(|(a, _)| *a != Arch::Other("unknown".into())));
        assert!(has_arm64(&page.items));
    }

    #[tokio::test]
    async fn tags_use_the_namespace_when_given() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/repositories/bitnami/redis/tags"))
            .respond_with(json(
                r#"{"next":null,"results":[{"name":"7","images":[]}]}"#,
            ))
            .mount(&s)
            .await;
        let page = client(&s).await.tags("bitnami/redis", 20).await.unwrap();
        assert_eq!(page.items[0].name, "7");
        assert!(page.items[0].per_arch.is_empty());
        assert_eq!(page.next, None);
    }

    #[tokio::test]
    async fn tags_of_a_missing_repository_are_not_found() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_raw(fixture("tags-notfound.json"), "application/json"),
            )
            .mount(&s)
            .await;
        let err = client(&s).await.tags("zzzznotthere", 3).await.unwrap_err();
        assert_eq!(err, RegistryError::NotFound);
    }

    #[tokio::test]
    async fn tags_reject_paths_that_are_not_repositories() {
        let s = MockServer::start().await;
        let c = client(&s).await;
        for bad in ["", "a/b/c", "/x", "x/"] {
            assert_eq!(
                c.tags(bad, 5).await.unwrap_err(),
                RegistryError::NotFound,
                "{bad:?}"
            );
        }
        assert!(s.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn http_429_is_rate_limited_with_the_reset_from_the_recorded_case() {
        let case: serde_json::Value =
            serde_json::from_str(&fixture("ratelimited-429.json")).unwrap();
        let mut resp = ResponseTemplate::new(case["status"].as_u64().unwrap() as u16);
        for (k, v) in case["headers"].as_object().unwrap() {
            resp = resp.insert_header(k.as_str(), v.as_str().unwrap());
        }
        resp = resp.set_body_json(&case["body"]);
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(resp)
            .mount(&s)
            .await;
        let c = client(&s).await;
        let expected = RegistryError::RateLimited {
            reset_in: Some(Duration::from_secs(37)),
        };
        assert_eq!(c.search("postgres", 10).await.unwrap_err(), expected);
        assert_eq!(c.tags("postgres", 10).await.unwrap_err(), expected);
    }

    #[tokio::test]
    async fn http_429_without_headers_has_no_reset() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&s)
            .await;
        let err = client(&s).await.tags("postgres", 3).await.unwrap_err();
        assert_eq!(err, RegistryError::RateLimited { reset_in: None });
    }

    #[tokio::test]
    async fn unreachable_host_is_offline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let c = HubClient::new(Url::parse(&format!("http://127.0.0.1:{port}")).unwrap());
        assert_eq!(
            c.search("postgres", 5).await.unwrap_err(),
            RegistryError::Offline
        );
        assert_eq!(
            c.tags("postgres", 5).await.unwrap_err(),
            RegistryError::Offline
        );
    }

    #[tokio::test]
    async fn server_error_on_both_endpoints_is_http() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&s)
            .await;
        let err = client(&s).await.search("postgres", 5).await.unwrap_err();
        assert_eq!(err, RegistryError::Http("HTTP 503".to_string()));
    }

    #[test]
    fn pull_counts_are_lower_bounds() {
        for (text, want) in [
            ("1B+", 1_000_000_000),
            ("500M+", 500_000_000),
            ("100K+", 100_000),
            ("<1K", 0),
            ("12", 12),
            ("", 0),
            ("lots", 0),
        ] {
            assert_eq!(parse_pull_count(text), want, "{text:?}");
        }
    }

    #[test]
    fn has_arm64_needs_an_arm64_image() {
        let tag = |arches: Vec<Arch>| TagInfo {
            name: "t".into(),
            digest: None,
            pushed: None,
            per_arch: arches.into_iter().map(|a| (a, ByteSize::b(1))).collect(),
        };
        assert!(!has_arm64(&[tag(vec![Arch::X86_64]), tag(vec![])]));
        assert!(has_arm64(&[
            tag(vec![Arch::X86_64]),
            tag(vec![Arch::Aarch64])
        ]));
    }

    /// `BERTH_NET=1 cargo nextest run -p berth-registry --run-ignored only live`
    #[tokio::test]
    #[ignore = "needs network: BERTH_NET=1"]
    async fn live_search_finds_postgres_official_with_arm64() {
        if std::env::var("BERTH_NET").as_deref() != Ok("1") {
            eprintln!("skipped: set BERTH_NET=1");
            return;
        }
        let c = HubClient::new(Url::parse("https://hub.docker.com").unwrap());
        let page = c.search("postgres", 10).await.unwrap();
        let hit = page
            .items
            .iter()
            .find(|h| h.name == "postgres")
            .expect("postgres in results");
        assert_eq!(hit.publisher, Publisher::Official);
        assert_eq!(hit.arm64, Some(true));

        let tags = c.tags("postgres", 10).await.unwrap();
        assert!(!tags.items.is_empty());
        assert!(has_arm64(&tags.items));
    }
}
