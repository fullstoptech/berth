//! P4-T5 tests: the create sheet's status, what it offers, and that an
//! invalid draft cannot be sent.

use super::*;

use crate::scenes;

fn markup(frame: &str) -> String {
    let mut out = scenes::render_scene_html(frame).expect("the frame has a scene");
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

fn key(name: &str) -> PendingKey {
    PendingKey {
        subject: berth_core::Subject::Network(name.to_string()),
        action: "network.create",
    }
}

#[test]
fn create_network_status_follows_the_pending_state_of_that_name() {
    let mut pending = std::collections::BTreeMap::new();
    assert_eq!(create_status(&pending, "web"), CreateStatus::Idle);
    pending.insert(
        key("web"),
        PendingState::Running {
            label: "Creating…".to_string(),
        },
    );
    assert_eq!(
        create_status(&pending, "web"),
        CreateStatus::Running("Creating…".to_string())
    );
    assert_eq!(create_status(&pending, "other"), CreateStatus::Idle);
    pending.insert(
        key("web"),
        PendingState::Failed {
            message: "Pool overlaps".to_string(),
        },
    );
    assert_eq!(
        create_status(&pending, "web"),
        CreateStatus::Failed {
            key: key("web"),
            message: "Pool overlaps".to_string()
        }
    );
}

#[test]
fn create_network_sheet_offers_the_drivers_the_switch_and_the_engines_containers() {
    let html = markup("Networks / create / 1440");
    for needle in [
        "aria-label=\"Name\"",
        "aria-label=\"Driver\"",
        "aria-label=\"Subnet\"",
        "aria-label=\"Gateway\"",
        "aria-label=\"Labels\"",
        "aria-label=\"Internal network\"",
        "Attach containers",
    ] {
        assert!(html.contains(needle), "{needle}");
    }
    for driver in DRIVERS {
        assert!(html.contains(&format!(">{driver}<")), "{driver}");
    }
    // The default engine's containers, not another engine's.
    assert!(html.contains("aria-label=\"Attach shop-api-api-1\""));
    assert!(
        !html.contains("aria-label=\"Attach landing-web-1\""),
        "{html}"
    );
}

#[test]
fn create_network_sheet_fields_are_uncontrolled() {
    // D59: the draft's values go in once (`initial_value`) and typing never
    // reads them back, so the inputs carry no controlled `value` binding for
    // the signal: the markup has the typed name once, as the input's value.
    let html = markup("Networks / create / 1440");
    assert_eq!(html.matches("value=\"berthfx-net\"").count(), 1, "{html}");
}

#[test]
fn create_network_sheet_has_cancel_and_a_disabled_create_with_the_reason() {
    let html = markup("Networks / create / 1440");
    assert!(html.contains(">Cancel<"));
    let at = html.rfind(">Create network<").expect("primary");
    let button = &html[html[..at].rfind("<button").expect("a button")..at];
    assert!(
        button.contains("btn-primary") && button.contains("disabled"),
        "{button}"
    );
}
