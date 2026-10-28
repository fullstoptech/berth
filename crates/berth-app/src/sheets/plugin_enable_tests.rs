//! P9-T2 tests for the permission request sheet, read as markup.

use crate::scenes;

fn sheet(frame: &str) -> String {
    let mut html = scenes::render_scene_html(frame).expect("the frame has a scene");
    while let (Some(a), Some(b)) = (html.find("<style>"), html.find("</style>")) {
        if b < a {
            break;
        }
        html.replace_range(a..b + "</style>".len(), "");
    }
    let start = html.find("class=\"pl-sheet\"").expect("the sheet is drawn");
    html[start..].to_string()
}

#[test]
fn plugin_enable_lists_every_permission_and_the_hosts_it_names() {
    let html = sheet("Dev / plugins / enable sheet");
    for needle in [
        "aria-label=\"Enable Coolify\"",
        "Enable Coolify",
        "Coolify 0.6.0 is a separate program",
        "Connect to any address on the network",
        "Read these stored secrets: coolify/token",
        "Offer MCP servers to the Assistant",
        "Addresses it says it contacts",
        ">Any address<",
        "does not block other addresses",
        "Cancel",
    ] {
        assert!(html.contains(needle), "missing {needle:?}: {html}");
    }
}

#[test]
fn plugin_enable_never_shows_a_bare_star_as_an_address() {
    let html = sheet("Dev / plugins / enable sheet");
    assert!(!html.contains(">*<"), "{html}");
}

#[test]
fn plugin_enable_tells_a_plugin_that_needs_settings_to_open_configure_next() {
    let html = sheet("Dev / plugins / enable sheet");
    assert!(
        html.contains("open Configure to finish setting it up"),
        "{html}"
    );
}

#[test]
fn plugin_enable_for_a_plugin_that_is_gone_offers_only_close() {
    // The scene's plugin id is not in an empty list.
    let html = {
        let mut dom = dioxus::prelude::VirtualDom::new(|| {
            use dioxus::prelude::*;
            let _ = crate::services::plugins::provide_plugins_view();
            let state = crate::state::use_app_state();
            use_context_provider(|| state);
            rsx! { super::PluginEnableSheet { id: "missing".to_string() } }
        });
        let _ = dom.rebuild_to_vec();
        dioxus_ssr::render(&dom)
    };
    assert!(
        html.contains("That plugin is no longer installed."),
        "{html}"
    );
    assert!(html.contains(">Close<"), "{html}");
    assert!(!html.contains("Enable missing"), "{html}");
}
