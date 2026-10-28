//! Tests of the From Git sheet and the Git panel through their scenes (P8-T5).

use super::*;
use crate::scenes::render_scene_html;

fn scene(frame: &str) -> String {
    render_scene_html(frame).unwrap_or_else(|| panic!("scene {frame:?} renders"))
}

#[test]
fn git_deploy_new_project_offers_from_git_enabled() {
    let html = scene("New project / sheet / 1440");
    assert!(html.contains(">From Git<"), "{html}");
    assert!(
        html.contains("Clone a repository, then deploy it."),
        "{html}"
    );
    assert!(
        !html.contains("np-choice\" type=\"button\" disabled"),
        "no choice is disabled any more"
    );
}

#[test]
fn git_deploy_form_asks_for_address_branch_key_engine_and_folder() {
    let html = scene("New project / from git / 1440");
    for text in [
        "aria-label=\"Repository address\"",
        "aria-label=\"Branch\"",
        "aria-label=\"SSH key\"",
        "aria-label=\"Target engine\"",
        "Use whatever my ssh-agent offers",
        ">id_ed25519<",
        ">id_work<",
        "Berth never reads the key",
        "Clone into",
        "Choose folder",
        ">Clone<",
    ] {
        assert!(html.contains(text), "missing {text:?}\n{html}");
    }
    // The typed fields are uncontrolled (D59): an initial value, never a bound `value`.
    assert!(
        html.contains("initial_value=\"git@github.com:acme/shop-api.git\""),
        "{html}"
    );
    assert!(
        !html.contains("class=\"field-input\" type=\"text\" value="),
        "{html}"
    );
    // Everything is filled in, so Clone is on.
    let clone_button = html.split(">Clone<").next().expect("clone button");
    assert!(
        !clone_button
            .rsplit("<button")
            .next()
            .expect("tag")
            .contains("disabled"),
        "Clone is enabled for a complete form"
    );
}

#[test]
fn git_deploy_cloning_shows_the_phase_and_a_cancel() {
    let html = scene("New project / from git / cloning / 1440");
    assert!(
        html.contains("Cloning git@github.com:acme/shop-api.git")
            || html.contains("Cloning ssh://"),
        "{html}"
    );
    assert!(html.contains("Receiving objects 64%"), "{html}");
    assert!(html.contains("aria-label=\"Clone progress\""), "{html}");
    assert!(html.contains(">Cancel<"), "{html}");
    assert!(
        !html.contains("aria-label=\"Repository address\""),
        "the form is gone while cloning"
    );
}

#[test]
fn git_deploy_auth_failure_says_what_git_said_and_the_fix() {
    let html = scene("New project / from git / clone failed / 1440");
    assert!(html.contains("git could not sign in"), "{html}");
    assert!(html.contains("Permission denied (publickey)"), "{html}");
    assert!(html.contains("Run ssh-add for it"), "{html}");
    assert!(html.contains(">Edit the address<"), "{html}");
}

#[test]
fn git_deploy_wrong_address_failure_points_at_the_address() {
    let html = scene("New project / from git / wrong address / 1440");
    assert!(html.contains("found nothing at that address"), "{html}");
    assert!(html.contains("shop-apy.git"), "{html}");
    assert!(
        html.contains("Check the address and the branch name"),
        "{html}"
    );
}

#[test]
fn git_deploy_invalid_compose_keeps_the_clone_and_offers_the_folder() {
    let html = scene("New project / from git / invalid compose / 1440");
    assert!(html.contains("its Compose file does not load"), "{html}");
    assert!(html.contains("services.api.ports must be a list"), "{html}");
    assert!(html.contains("~/Dev/shop-api"), "{html}");
    assert!(html.contains(">Open folder<"), "{html}");
    assert!(html.contains(">Close<"), "{html}");
}

#[test]
fn git_deploy_project_detail_lists_the_new_commits_before_pulling() {
    let html = scene("Project / git / 1440");
    for text in [
        "aria-label=\"Git\"",
        "d544a3d",
        "2 new commits",
        "a41c9f0",
        "api: add the health endpoint",
        "7be20d1",
        ">Check for new commits<",
        ">Pull and redeploy<",
        "Auto-redeploy",
        "Every 15 minutes",
    ] {
        assert!(html.contains(text), "missing {text:?}\n{html}");
    }
}

#[test]
fn git_deploy_project_row_shows_the_commit_and_new_commits() {
    let html = scene("Projects / git / 1440");
    assert!(html.contains("project-commit"), "{html}");
    assert!(html.contains("d544a3d"), "{html}");
    assert!(html.contains("2 new commits"), "{html}");
}

#[test]
fn git_deploy_engines_offered_are_running_and_local() {
    use berth_core::{Engine, EngineId, EngineState, OwnerId, ResourceCounts};
    let make = |id: &str, state: EngineState, kind: EngineKind| Engine {
        id: EngineId(id.to_string()),
        name: id.to_string(),
        kind,
        state,
        docker_host: None,
        docker_version: None,
        resources: None,
        counts: ResourceCounts::default(),
        owner: OwnerId::Local,
    };
    let engines = [
        make(
            "colima:default",
            EngineState::Running,
            EngineKind::Colima {
                profile: "default".to_string(),
            },
        ),
        make(
            "colima:off",
            EngineState::Stopped,
            EngineKind::Colima {
                profile: "off".to_string(),
            },
        ),
        make(
            "ssh:box",
            EngineState::Running,
            EngineKind::Remote {
                transport: berth_core::RemoteTransport::Ssh {
                    host: "box".to_string(),
                    user: None,
                    port: None,
                },
            },
        ),
    ];
    let offered = clone_engines(&engines);
    assert_eq!(
        offered
            .iter()
            .map(|(id, _)| id.0.as_str())
            .collect::<Vec<_>>(),
        ["colima:default"]
    );
    assert_eq!(
        progress_text("Receiving objects", Some(64)),
        "Receiving objects 64%"
    );
    assert_eq!(progress_text("Starting", None), "Starting");
}
