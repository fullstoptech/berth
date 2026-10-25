//! Registry v2 client: login check and tag listing, with the Bearer token flow
//! (`WWW-Authenticate: Bearer realm=…,service=…,scope=…`) or Basic auth.

use reqwest::header::{HeaderMap, LINK, WWW_AUTHENTICATE};
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;

use crate::{
    build_client, check, client_or_err, decode_error, endpoint, map_send_error, RegistryError,
};

/// Registries rarely have more than a few hundred tags; this bounds a hostile one.
const MAX_TAG_PAGES: usize = 20;

type Auth<'a> = Option<(&'a str, &'a SecretString)>;

pub struct RegistryClient {
    client: Option<reqwest::Client>,
}

impl Default for RegistryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RegistryClient {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: build_client(),
        }
    }

    /// `GET <url>/v2/`, answering a Bearer or Basic challenge with these credentials.
    ///
    /// # Errors
    /// `Unauthorized` when the registry or its token service rejects the credentials,
    /// `Offline` when it cannot be reached, `Http` for anything else.
    pub async fn check_login(
        &self,
        url: &Url,
        username: &str,
        password: &SecretString,
    ) -> Result<(), RegistryError> {
        let v2 = endpoint(url, &["v2", ""])?;
        self.get(&v2, Some((username, password)), None)
            .await
            .map(|_| ())
    }

    /// All tags of `repo`, following `Link: rel="next"` pages.
    ///
    /// # Errors
    /// `NotFound` when the repository does not exist, `Unauthorized` when it needs a
    /// login, `Offline` or `Http`.
    pub async fn tags(
        &self,
        url: &Url,
        repo: &str,
        auth: Auth<'_>,
    ) -> Result<Vec<String>, RegistryError> {
        if repo.is_empty() || repo.split('/').any(str::is_empty) {
            return Err(RegistryError::NotFound);
        }
        let mut segments = vec!["v2"];
        segments.extend(repo.split('/'));
        segments.extend(["tags", "list"]);
        let mut next = endpoint(url, &segments)?;
        next.query_pairs_mut().append_pair("n", "1000");
        let scope = format!("repository:{repo}:pull");

        let mut tags = Vec::new();
        for _ in 0..MAX_TAG_PAGES {
            let resp = self.get(&next, auth, Some(&scope)).await?;
            let link = next_link(resp.headers(), &next);
            let body: TagList = resp.json().await.map_err(decode_error)?;
            tags.extend(body.tags.unwrap_or_default());
            match link {
                Some(n) => next = n,
                None => break,
            }
        }
        Ok(tags)
    }

    /// GET with the challenge/response dance. Sends no credentials until challenged.
    async fn get(
        &self,
        url: &Url,
        auth: Auth<'_>,
        default_scope: Option<&str>,
    ) -> Result<reqwest::Response, RegistryError> {
        let client = client_or_err(self.client.as_ref())?;
        let first = client
            .get(url.clone())
            .send()
            .await
            .map_err(map_send_error)?;
        if first.status() != StatusCode::UNAUTHORIZED {
            return check(first);
        }
        let challenge = first
            .headers()
            .get_all(WWW_AUTHENTICATE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(Challenge::parse)
            .ok_or(RegistryError::Unauthorized)?;
        let req = client.get(url.clone());
        let req = match challenge {
            Challenge::Basic => {
                let (user, pass) = auth.ok_or(RegistryError::Unauthorized)?;
                req.basic_auth(user, Some(pass.expose_secret()))
            }
            Challenge::Bearer {
                realm,
                service,
                scope,
            } => {
                let scope = scope.or_else(|| default_scope.map(str::to_string));
                let token = self
                    .fetch_token(url, &realm, service.as_deref(), scope.as_deref(), auth)
                    .await?;
                req.bearer_auth(token.expose_secret())
            }
        };
        check(req.send().await.map_err(map_send_error)?)
    }

    async fn fetch_token(
        &self,
        registry: &Url,
        realm: &str,
        service: Option<&str>,
        scope: Option<&str>,
        auth: Auth<'_>,
    ) -> Result<SecretString, RegistryError> {
        let mut realm = Url::parse(realm)
            .map_err(|_| RegistryError::Http("the registry sent an invalid token URL".into()))?;
        // Never carry a password to a plain-http token service of an https registry.
        let scheme_ok =
            realm.scheme() == "https" || (realm.scheme() == "http" && registry.scheme() == "http");
        if !scheme_ok {
            return Err(RegistryError::Http(
                "the registry's token service is not https".into(),
            ));
        }
        {
            let mut q = realm.query_pairs_mut();
            if let Some(service) = service {
                q.append_pair("service", service);
            }
            if let Some(scope) = scope {
                q.append_pair("scope", scope);
            }
        }
        let mut req = client_or_err(self.client.as_ref())?.get(realm);
        if let Some((user, pass)) = auth {
            req = req.basic_auth(user, Some(pass.expose_secret()));
        }
        let resp = req.send().await.map_err(map_send_error)?;
        let body: TokenBody = check(resp)?.json().await.map_err(decode_error)?;
        body.token
            .or(body.access_token)
            .filter(|t| !t.is_empty())
            .map(SecretString::from)
            .ok_or_else(|| RegistryError::Http("the token service sent no token".into()))
    }
}

#[derive(Deserialize)]
struct TagList {
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct TokenBody {
    token: Option<String>,
    access_token: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Challenge {
    Basic,
    Bearer {
        realm: String,
        service: Option<String>,
        scope: Option<String>,
    },
}

impl Challenge {
    /// Parses one `WWW-Authenticate` value such as
    /// `Bearer realm="https://auth.docker.io/token",service="registry.docker.io"`.
    fn parse(header: &str) -> Option<Self> {
        let header = header.trim();
        let (scheme, rest) = header
            .split_once(char::is_whitespace)
            .unwrap_or((header, ""));
        if scheme.eq_ignore_ascii_case("basic") {
            return Some(Self::Basic);
        }
        if !scheme.eq_ignore_ascii_case("bearer") {
            return None;
        }
        let params = parse_params(rest);
        let get = |k: &str| {
            params
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(k))
                .map(|(_, v)| v.clone())
        };
        Some(Self::Bearer {
            realm: get("realm")?,
            service: get("service"),
            scope: get("scope"),
        })
    }
}

/// `k="v with, comma",k2=v2` -> pairs. Quoted values may contain commas.
fn parse_params(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == ',').is_some() {}
        let mut key = String::new();
        while let Some(c) = chars.next_if(|c| *c != '=' && *c != ',') {
            key.push(c);
        }
        if chars.next_if_eq(&'=').is_none() {
            if key.trim().is_empty() {
                break;
            }
            continue;
        }
        let mut value = String::new();
        if chars.next_if_eq(&'"').is_some() {
            while let Some(c) = chars.next() {
                match c {
                    '\\' => value.extend(chars.next()),
                    '"' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(c) = chars.next_if(|c| *c != ',') {
                value.push(c);
            }
        }
        out.push((key.trim().to_string(), value.trim().to_string()));
    }
    out
}

/// `Link: </v2/x/tags/list?last=b&n=100>; rel="next"`, relative to the request URL.
fn next_link(headers: &HeaderMap, current: &Url) -> Option<Url> {
    headers
        .get_all(LINK)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .find(|part| part.contains("rel=\"next\"") || part.contains("rel=next"))
        .and_then(|part| {
            let start = part.find('<')? + 1;
            let end = part.find('>')?;
            current.join(part.get(start..end)?).ok()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{basic_auth, bearer_token, header_exists, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    fn base(server: &MockServer) -> Url {
        Url::parse(&server.uri()).unwrap()
    }

    /// `/v2/…` answers 401 with a Bearer challenge pointing at `/token`.
    async fn bearer_registry(server: &MockServer, scope: Option<&str>) {
        let scope = scope.map_or(String::new(), |s| format!(",scope=\"{s}\""));
        let challenge = format!(
            "Bearer realm=\"{}/token\",service=\"test-registry\"{scope}",
            server.uri()
        );
        Mock::given(method("GET"))
            .and(path_regex_v2())
            .respond_with(ResponseTemplate::new(401).insert_header("www-authenticate", challenge))
            .with_priority(5)
            .mount(server)
            .await;
    }

    fn path_regex_v2() -> wiremock::matchers::PathRegexMatcher {
        wiremock::matchers::path_regex(r"^/v2/.*")
    }

    #[tokio::test]
    async fn login_succeeds_when_the_token_service_accepts_the_password() {
        let s = MockServer::start().await;
        bearer_registry(&s, None).await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .and(query_param("service", "test-registry"))
            .and(basic_auth("emma", "hunter2-not-real"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "T1"})),
            )
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/"))
            .and(bearer_token("T1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .with_priority(1)
            .mount(&s)
            .await;
        RegistryClient::new()
            .check_login(&base(&s), "emma", &secret("hunter2-not-real"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn login_with_a_wrong_password_is_unauthorized() {
        let s = MockServer::start().await;
        bearer_registry(&s, None).await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&s)
            .await;
        let err = RegistryClient::new()
            .check_login(&base(&s), "emma", &secret("wrong"))
            .await
            .unwrap_err();
        assert_eq!(err, RegistryError::Unauthorized);
    }

    #[tokio::test]
    async fn login_to_an_open_registry_is_fine_and_sends_no_credentials() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&s)
            .await;
        RegistryClient::new()
            .check_login(&base(&s), "u", &secret("p"))
            .await
            .unwrap();
        let reqs = s.received_requests().await.unwrap();
        assert!(reqs
            .iter()
            .all(|r| !r.headers.contains_key("authorization")));
    }

    #[tokio::test]
    async fn login_answers_a_basic_challenge() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/"))
            .respond_with(
                ResponseTemplate::new(401).insert_header("www-authenticate", "Basic realm=\"r\""),
            )
            .with_priority(5)
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/"))
            .and(basic_auth("u", "p"))
            .and(header_exists("authorization"))
            .respond_with(ResponseTemplate::new(200))
            .with_priority(1)
            .mount(&s)
            .await;
        let c = RegistryClient::new();
        c.check_login(&base(&s), "u", &secret("p")).await.unwrap();
        assert_eq!(
            c.check_login(&base(&s), "u", &secret("nope"))
                .await
                .unwrap_err(),
            RegistryError::Unauthorized
        );
    }

    #[tokio::test]
    async fn tags_use_the_challenge_scope_and_follow_link_headers() {
        let s = MockServer::start().await;
        bearer_registry(&s, Some("repository:team/app:pull")).await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .and(query_param("scope", "repository:team/app:pull"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "T2"})),
            )
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/team/app/tags/list"))
            .and(bearer_token("T2"))
            .and(query_param("last", "b"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"name": "team/app", "tags": ["c"]})),
            )
            .with_priority(1)
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/team/app/tags/list"))
            .and(bearer_token("T2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(
                        "link",
                        "</v2/team/app/tags/list?last=b&n=1000>; rel=\"next\"",
                    )
                    .set_body_json(serde_json::json!({"name": "team/app", "tags": ["a", "b"]})),
            )
            .with_priority(2)
            .mount(&s)
            .await;
        let tags = RegistryClient::new()
            .tags(&base(&s), "team/app", Some(("emma", &secret("pw"))))
            .await
            .unwrap();
        assert_eq!(tags, ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn tags_without_a_scope_in_the_challenge_ask_for_pull_on_the_repo() {
        let s = MockServer::start().await;
        bearer_registry(&s, None).await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .and(query_param("scope", "repository:app:pull"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "T3"})),
            )
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/app/tags/list"))
            .and(bearer_token("T3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"name": "app", "tags": null})),
            )
            .with_priority(1)
            .mount(&s)
            .await;
        // anonymous pull: no credentials, token service still issues a token
        let tags = RegistryClient::new()
            .tags(&base(&s), "app", None)
            .await
            .unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn tags_of_a_missing_repo_are_not_found() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&s)
            .await;
        let err = RegistryClient::new()
            .tags(&base(&s), "nope", None)
            .await
            .unwrap_err();
        assert_eq!(err, RegistryError::NotFound);
    }

    #[tokio::test]
    async fn a_bearer_realm_on_plain_http_is_refused_for_an_https_registry() {
        let c = RegistryClient::new();
        let err = c
            .fetch_token(
                &Url::parse("https://registry.example").unwrap(),
                "http://auth.example/token",
                None,
                None,
                Some(("u", &secret("p"))),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, RegistryError::Http(m) if m.contains("https")));
    }

    #[tokio::test]
    async fn an_unreachable_registry_is_offline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let url = Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let err = RegistryClient::new()
            .check_login(&url, "u", &secret("p"))
            .await
            .unwrap_err();
        assert_eq!(err, RegistryError::Offline);
    }

    #[test]
    fn challenge_parser_reads_docker_hub_and_quoted_commas() {
        assert_eq!(
            Challenge::parse(
                r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/x:pull,push""#
            ),
            Some(Challenge::Bearer {
                realm: "https://auth.docker.io/token".into(),
                service: Some("registry.docker.io".into()),
                scope: Some("repository:library/x:pull,push".into()),
            })
        );
        assert_eq!(
            Challenge::parse("basic realm=\"x\""),
            Some(Challenge::Basic)
        );
        assert_eq!(Challenge::parse("Negotiate"), None);
        assert_eq!(Challenge::parse("Bearer service=\"no-realm\""), None);
    }
}
