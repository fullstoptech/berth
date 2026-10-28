use super::*;

#[test]
fn registries_rows_show_login_state_and_dash_for_no_username() {
    let registries = [
        Registry {
            id: "docker.io".to_string(),
            name: "Docker Hub".to_string(),
            url: url::Url::parse("https://index.docker.io").expect("url"),
            username: Some("emma".to_string()),
            status: RegistryStatus::LoggedIn,
        },
        Registry {
            id: "127.0.0.1:56500".to_string(),
            name: "Fixture".to_string(),
            url: url::Url::parse("http://127.0.0.1:56500").expect("url"),
            username: None,
            status: RegistryStatus::Error {
                reason: "Wrong username or password".to_string(),
            },
        },
    ];
    let shown = rows(&registries);
    assert_eq!(shown[0].status, "Logged in");
    assert!(shown[0].logged_in);
    assert_eq!(shown[1].username, "\u{2014}");
    assert_eq!(shown[1].status, "Wrong username or password");
    assert!(!shown[1].logged_in);
    assert_eq!(status_text(&RegistryStatus::LoggedOut), "Logged out");
}
