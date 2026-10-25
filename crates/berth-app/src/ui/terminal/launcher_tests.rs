//! The inspector's Terminal tab.

use dioxus::prelude::*;

use crate::services::exec::{open_container_terminal, ExecCore, TerminalView};

#[component]
fn LauncherRoot(container: String, open_first: bool) -> Element {
    let mut state = crate::state::use_app_state();
    use_context_provider(|| state);
    crate::services::exec::provide_scene_terminals();
    let world = berth_engine::MockWorld::bundled();
    let target = world
        .containers
        .iter()
        .find(|c| c.name == container)
        .cloned()
        .expect("mock container");
    let id = target.id.clone();
    use_hook(move || {
        state.containers.set(world.containers.clone());
        state.projects.set(world.projects.clone());
        if open_first {
            let mut view = consume_context::<TerminalView>();
            let core = consume_context::<ExecCore>();
            open_container_terminal(&mut state, &mut view, &core, None, &target).expect("opened");
        }
    });
    rsx! { crate::ui::terminal::TerminalPanel { container: id } }
}

fn launcher_html(container: &str, open_first: bool) -> String {
    let mut dom = VirtualDom::new_with_props(
        LauncherRoot,
        LauncherRootProps {
            container: container.to_string(),
            open_first,
        },
    );
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn terminal_inspector_tab_offers_to_start_a_stopped_container_and_lists_open_terminals() {
    let running = launcher_html("shop-api-api-1", false);
    assert!(running.contains("Open terminal"), "{running}");
    assert!(!running.contains("Start and open terminal"));
    assert!(
        !running.contains("class=\"term-launch-item\""),
        "no terminals yet"
    );

    let stopped = launcher_html("shop-api-db-1", false);
    assert!(stopped.contains("Start and open terminal"), "{stopped}");

    let listed = launcher_html("shop-api-api-1", true);
    assert!(listed.contains("shop-api/api-1 \u{b7} bash"), "{listed}");
    assert!(listed.contains("class=\"term-launch-item\""), "{listed}");
}
