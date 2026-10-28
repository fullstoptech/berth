use super::*;

fn engine(id: &str, name: &str) -> Engine {
    let mut world = berth_engine::MockWorld::bundled();
    let mut e = world.engines.remove(0);
    e.id = EngineId(id.to_string());
    e.name = name.to_string();
    e
}

#[test]
fn add_engine_build_refuses_empty_duplicate_and_existing() {
    let transport = RemoteTransport::Ssh {
        host: "prod-1".to_string(),
        user: None,
        port: None,
    };
    let now = chrono::Utc::now();
    assert!(build_engine("  ", &transport, &[], now).is_err());
    let existing = [engine("colima:default", "prod-1")];
    assert_eq!(
        build_engine("prod-1", &transport, &existing, now).unwrap_err(),
        "An engine named prod-1 already exists"
    );
    let existing = [engine("ssh:prod-1", "other")];
    assert_eq!(
        build_engine("new", &transport, &existing, now).unwrap_err(),
        "ssh:prod-1 is already an engine"
    );
    let row = build_engine(" prod ", &transport, &[], now).expect("row");
    assert_eq!(row.name, "prod");
    assert_eq!(row.id.0, "ssh:prod-1");
}

#[test]
fn add_engine_tcp_url_parsing_accepts_bare_host_port() {
    assert_eq!(
        parse_tcp_url("10.0.0.5:2376").expect("url").as_str(),
        "tcp://10.0.0.5:2376"
    );
    assert!(parse_tcp_url("https://docker.example.com:2376").is_ok());
    assert!(parse_tcp_url("unix:///tmp/x").is_err());
    assert!(parse_tcp_url("").is_err());
}

#[test]
fn add_engine_draft_transport_validates_fields() {
    let ssh =
        draft_transport("ssh", " prod-1 ", " deploy ", "2222", "", false, "", "", "").expect("ssh");
    assert_eq!(
        ssh,
        RemoteTransport::Ssh {
            host: "prod-1".to_string(),
            user: Some("deploy".to_string()),
            port: Some(2222),
        }
    );
    assert!(draft_transport("ssh", "", "", "", "", false, "", "", "").is_err());
    assert!(draft_transport("ssh", "h", "", "abc", "", false, "", "", "").is_err());
    assert!(draft_transport("tcp", "", "", "", "10.0.0.5:2376", true, "", "c", "k").is_err());
    let tcp = draft_transport("tcp", "", "", "", "10.0.0.5:2376", true, "ca", "ce", "ke")
        .expect("tcp tls");
    assert!(matches!(tcp, RemoteTransport::Tcp { tls: Some(_), .. }));
}

#[test]
fn add_engine_marker_series_is_stable_and_in_range() {
    for id in ["ssh:prod-1", "tcp:10.0.0.5:2376", "colima:default", "a", ""] {
        let one = marker_series(&EngineId(id.to_string()));
        assert!((1..=5).contains(&one), "{id} -> {one}");
        assert_eq!(one, marker_series(&EngineId(id.to_string())));
    }
}
