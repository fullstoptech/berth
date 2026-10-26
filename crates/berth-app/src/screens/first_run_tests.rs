use super::*;

use crate::services::first_run::{provide_first_run_store, Probe};
use crate::state::use_app_state;

fn strip_style(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<style>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</style>") {
            Some(end) => rest = &rest[start + end + "</style>".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn scene(frame: &str) -> String {
    strip_style(&crate::scenes::render_scene_html(frame).expect("scene exists"))
}

#[derive(Props, Clone, PartialEq)]
struct HostProps {
    probe: Option<Probe>,
    engines: Vec<berth_core::Engine>,
    harnesses: Vec<berth_core::Harness>,
    full_screen: bool,
}

#[component]
fn Host(props: HostProps) -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let mut store = provide_first_run_store();
    let mut shown = state;
    use_hook(move || {
        store.probe.set(props.probe.clone());
        store.check_sockets.set(false);
        shown.engines.set(props.engines.clone());
        shown.harnesses.set(props.harnesses.clone());
    });
    if props.full_screen {
        rsx! { FirstRunScreen {} }
    } else {
        rsx! { FirstRunCard {} }
    }
}

fn render(probe: Option<Probe>, engines: Vec<berth_core::Engine>, full_screen: bool) -> String {
    render_with(probe, engines, Vec::new(), full_screen)
}

fn render_with(
    probe: Option<Probe>,
    engines: Vec<berth_core::Engine>,
    harnesses: Vec<berth_core::Harness>,
    full_screen: bool,
) -> String {
    let mut dom = VirtualDom::new_with_props(
        Host,
        HostProps {
            probe,
            engines,
            harnesses,
            full_screen,
        },
    );
    let _ = dom.rebuild_to_vec();
    strip_style(&dioxus_ssr::render(&dom))
}

fn colima(state: berth_core::EngineState) -> berth_core::Engine {
    berth_core::Engine {
        id: berth_core::EngineId("colima:default".to_string()),
        name: "default".to_string(),
        kind: berth_core::EngineKind::Colima {
            profile: "default".to_string(),
        },
        state,
        docker_host: None,
        docker_version: Some("28.4.0".to_string()),
        resources: None,
        counts: berth_core::ResourceCounts::default(),
        owner: berth_core::OwnerId::Local,
    }
}

// --- The card on Projects empty ---

#[test]
fn first_run_card_says_so_until_the_first_scan_answers() {
    let html = render(None, Vec::new(), false);
    assert!(html.contains("First-run checks"), "{html}");
    assert!(html.contains("Checking this Mac"), "{html}");
    assert!(!html.contains("fr-row"), "{html}");
}

#[test]
fn first_run_card_lists_the_rows_of_the_frame_in_order() {
    let html = scene("Projects / empty / 1440");
    let at = |needle: &str| {
        html.find(needle)
            .unwrap_or_else(|| panic!("{needle}: {html}"))
    };
    let order = [
        at(">Colima<"),
        at(">Docker<"),
        at(">Compose plugin<"),
        at(">Claude Code<"),
        at(">Muse Code<"),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert!(html.contains("0.10.3 \u{b7} running"), "{html}");
    assert!(html.contains(">28.4.0<"), "{html}");
    assert!(html.contains(HARNESS_NOTE), "{html}");
    // Everything passes in the frame: no failing mark, no button in the card.
    assert!(
        !html.contains("fr-fail") && !html.contains("fr-warn"),
        "{html}"
    );
    assert!(!html.contains("Start default"), "{html}");
}

#[test]
fn first_run_card_with_a_stopped_profile_offers_start_default() {
    let html = render(
        Some(Probe::all_found()),
        vec![colima(berth_core::EngineState::Stopped)],
        false,
    );
    assert!(html.contains("fr-warn"), "{html}");
    assert!(html.contains("0.10.3 \u{b7} stopped"), "{html}");
    assert!(html.contains("Start default"), "{html}");
}

#[test]
fn first_run_card_with_no_compose_plugin_shows_the_hint_without_a_setup_screen() {
    let html = render(
        Some(Probe {
            compose_version: None,
            ..Probe::all_found()
        }),
        vec![colima(berth_core::EngineState::Running)],
        false,
    );
    assert!(html.contains("fr-fail"), "{html}");
    assert!(html.contains("brew install docker-compose"), "{html}");
}

#[test]
fn first_run_an_assistant_that_is_not_installed_is_muted_not_failed() {
    let harness = |id: &str, name: &str, version: Option<&str>| berth_core::Harness {
        id: berth_core::HarnessId(id.to_string()),
        name: name.to_string(),
        binary: std::path::PathBuf::from(id),
        version: version.map(str::to_string),
        transport: berth_core::HarnessTransport::MuseMsp,
        available: version.is_some(),
    };
    let html = render_with(
        Some(Probe::all_found()),
        vec![colima(berth_core::EngineState::Running)],
        vec![
            harness("claude-code", "Claude Code", Some("2.1.309")),
            harness("muse", "Muse Code", None),
        ],
        false,
    );
    assert!(html.contains("fr-absent"), "{html}");
    assert!(
        html.contains("Muse Code, not installed, not found"),
        "{html}"
    );
    assert!(html.contains("Claude Code, passed, 2.1.309"), "{html}");
    assert!(
        !html.contains("fr-fail"),
        "an absent assistant is not a failure: {html}"
    );
}

// --- The full-screen setup ---

#[test]
fn first_run_screen_with_colima_missing_shows_the_install_command_and_copy() {
    let html = scene("First run / colima missing / 1440");
    assert!(html.contains("Set up Berth"), "{html}");
    assert!(html.contains("fr-fail"), "{html}");
    assert!(html.contains("not found"), "{html}");
    assert!(html.contains(INSTALL_COMMAND), "{html}");
    assert!(html.contains(">Copy<"), "{html}");
    assert!(html.contains("Check again"), "{html}");
    assert!(html.contains("Continue"), "{html}");
    assert!(
        html.contains("Berth never installs anything itself"),
        "{html}"
    );
    assert!(!html.contains("Start default"), "{html}");
}

#[test]
fn first_run_screen_with_docker_missing_also_shows_the_install_command() {
    let html = render(
        Some(Probe {
            docker: None,
            docker_version: None,
            compose_version: None,
            ..Probe::all_found()
        }),
        vec![colima(berth_core::EngineState::Running)],
        true,
    );
    assert!(html.contains(INSTALL_COMMAND), "{html}");
    assert!(html.contains("Docker"), "{html}");
    assert!(
        html.matches("fr-fail").count() >= 2,
        "docker and compose fail: {html}"
    );
}

#[test]
fn first_run_screen_with_everything_present_has_no_install_block() {
    let html = render(
        Some(Probe::all_found()),
        vec![colima(berth_core::EngineState::Running)],
        true,
    );
    assert!(!html.contains(INSTALL_COMMAND), "{html}");
    assert!(html.contains("Continue"), "{html}");
}

#[test]
fn first_run_screen_with_a_stopped_default_offers_start_default() {
    let html = scene("First run / default stopped / 1440");
    assert!(html.contains("Start default"), "{html}");
    assert!(html.contains("0.10.3 \u{b7} stopped"), "{html}");
    assert!(!html.contains(INSTALL_COMMAND), "{html}");
}

#[test]
fn first_run_screen_with_no_profile_says_what_start_will_create() {
    let html = render(Some(Probe::all_found()), Vec::new(), true);
    assert!(html.contains("no profile yet"), "{html}");
    assert!(html.contains("Start default (2 CPU, 8 GB)"), "{html}");
}

#[test]
fn first_run_section_is_routed_to_the_setup_screen_and_settings_to_settings() {
    let html = scene("First run / colima missing / 1440");
    assert!(html.contains("first-run-screen"), "{html}");
    let settings = scene("Settings / default / 1440");
    assert!(settings.contains("settings-screen"), "{settings}");
    assert!(!settings.contains("first-run-screen"), "{settings}");
}
