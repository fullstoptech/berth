use super::*;

fn sheet_html(refusal: Option<&str>) -> String {
    let refusal = refusal.map(str::to_string);
    let mut dom = VirtualDom::new_with_props(
        |refusal: Option<String>| {
            let state = crate::state::use_app_state();
            use_context_provider(|| state);
            rsx! { RegistryLoginSheet { existing: None, refusal } }
        },
        refusal,
    );
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

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

#[test]
fn registry_login_bare_loopback_hosts_are_plain_http() {
    for (typed, want) in [
        ("127.0.0.1:56500", "http://127.0.0.1:56500/"),
        ("127.0.0.1", "http://127.0.0.1/"),
        ("127.1.2.3:5000", "http://127.1.2.3:5000/"),
        ("localhost", "http://localhost/"),
        ("localhost:5000", "http://localhost:5000/"),
        ("LocalHost:5000/", "http://localhost:5000/"),
        ("[::1]:5000", "http://[::1]:5000/"),
        ("[::1]", "http://[::1]/"),
        ("::1", "http://[::1]/"),
    ] {
        assert_eq!(
            parse_registry_url(typed).expect(typed).as_str(),
            want,
            "{typed}"
        );
    }
}

#[test]
fn registry_login_every_other_bare_host_stays_https() {
    for (typed, want) in [
        ("ghcr.io", "https://ghcr.io/"),
        (
            "registry.example.com:5000",
            "https://registry.example.com:5000/",
        ),
        ("128.0.0.1:5000", "https://128.0.0.1:5000/"),
        ("192.168.1.10:5000", "https://192.168.1.10:5000/"),
        ("localhost.example.com", "https://localhost.example.com/"),
        ("127.0.0.1.example.com", "https://127.0.0.1.example.com/"),
    ] {
        assert_eq!(
            parse_registry_url(typed).expect(typed).as_str(),
            want,
            "{typed}"
        );
    }
    // A scheme typed by hand wins over the guess.
    assert_eq!(
        parse_registry_url("https://localhost:5000")
            .expect("url")
            .as_str(),
        "https://localhost:5000/"
    );
}

#[test]
fn registry_login_ids_and_keychain_keys_do_not_depend_on_the_scheme() {
    use crate::services::registries::{registry_id, secret_key};
    let https = parse_registry_url("ghcr.io").expect("url");
    assert_eq!(
        secret_key(&registry_id(&https)),
        "registry/ghcr.io/password"
    );
    let local = parse_registry_url("127.0.0.1:56500").expect("url");
    assert_eq!(
        secret_key(&registry_id(&local)),
        "registry/127.0.0.1:56500/password"
    );
    let typed = parse_registry_url("http://127.0.0.1:56500").expect("url");
    assert_eq!(registry_id(&local), registry_id(&typed));
}

#[test]
fn registry_login_says_in_one_line_that_loopback_uses_plain_http() {
    let html = sheet_html(None);
    assert_eq!(
        html.matches("A bare localhost or 127.0.0.1 address uses plain http.")
            .count(),
        1,
        "{html}"
    );
}

#[test]
fn registry_login_shows_a_refusal_once() {
    let reason = "The registry is offline or unreachable";
    let html = sheet_html(Some(reason));
    assert_eq!(html.matches(reason).count(), 1, "{html}");
    assert_eq!(html.matches("role=\"alert\"").count(), 1, "{html}");
    assert!(!sheet_html(None).contains("role=\"alert\""));
}
