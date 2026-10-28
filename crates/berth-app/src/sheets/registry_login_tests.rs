use super::*;

#[test]
fn registry_login_url_assumes_https_and_keeps_local_http() {
    assert_eq!(
        parse_registry_url("ghcr.io").expect("url").as_str(),
        "https://ghcr.io/"
    );
    assert_eq!(
        parse_registry_url("http://127.0.0.1:56500/")
            .expect("url")
            .as_str(),
        "http://127.0.0.1:56500/"
    );
    assert!(parse_registry_url("ftp://x").is_err());
    assert!(parse_registry_url("").is_err());
}
