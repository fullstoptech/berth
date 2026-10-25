//! Opening and closing terminal tabs over real signals.

use std::sync::Arc;

use dioxus::prelude::*;

use crate::services::backing::Backing;
use crate::services::exec::{
    close_tab, docker_engine, open_container_terminal, open_docker_tab, open_project_terminals,
    project_terminals_reason, select_tab, use_terminal_view, ExecCore, Kind, Spec, TerminalView,
};
use crate::state::{AppState, BottomTab};

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, TerminalView)>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = crate::state::use_app_state();
    let view = use_terminal_view();
    use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some((state, view))));
    rsx! { div {} }
}

struct Rig {
    dom: VirtualDom,
    state: AppState,
    view: TerminalView,
    core: ExecCore,
    world: berth_engine::MockWorld,
}

fn rig() -> Rig {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (state, view) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the state");
    let world = berth_engine::MockWorld::bundled();
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let (core, _ui) = ExecCore::new(
        crate::services::engines::client_map(),
        store,
        tokio_util::sync::CancellationToken::new(),
    );
    let mut loaded = state;
    dom.in_runtime(|| {
        loaded.containers.set(world.containers.clone());
        loaded.projects.set(world.projects.clone());
        loaded.engines.set(world.engines.clone());
    });
    Rig {
        dom,
        state,
        view,
        core,
        world,
    }
}

impl Rig {
    fn container(&self, name: &str) -> berth_core::Container {
        self.world
            .containers
            .iter()
            .find(|c| c.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("mock container {name}"))
    }

    fn project(&self, name: &str) -> berth_core::Project {
        self.world
            .projects
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("mock project {name}"))
    }

    fn titles(&self) -> Vec<String> {
        self.dom.in_runtime(|| {
            self.state
                .bottom
                .peek()
                .terminals
                .iter()
                .map(|t| t.title.clone())
                .collect()
        })
    }

    fn ids(&self) -> Vec<uuid::Uuid> {
        self.dom.in_runtime(|| {
            self.state
                .bottom
                .peek()
                .terminals
                .iter()
                .map(|t| t.id)
                .collect()
        })
    }
}

#[test]
fn terminal_a_running_container_opens_a_tab_in_the_panel() {
    let rig = rig();
    let api = rig.container("shop-api-api-1");
    let (mut state, mut view) = (rig.state, rig.view);
    let core = rig.core.clone();
    let opened = rig
        .dom
        .in_runtime(|| open_container_terminal(&mut state, &mut view, &core, None, &api));
    let id = opened.expect("opened");
    assert_eq!(rig.titles(), ["shop-api/api-1 \u{b7} bash"]);
    rig.dom.in_runtime(|| {
        let bottom = state.bottom.peek();
        assert!(bottom.open);
        assert_eq!(bottom.tab, BottomTab::Terminal);
        assert_eq!(*view.active.peek(), Some(id));
    });
    assert!(rig.core.view(id).is_some(), "the session's slot exists");
}

#[test]
fn terminal_a_stopped_container_needs_the_actions_service_to_start_first() {
    let rig = rig();
    let db = rig.container("shop-api-db-1");
    let (mut state, mut view) = (rig.state, rig.view);
    let core = rig.core.clone();
    let refused = rig
        .dom
        .in_runtime(|| open_container_terminal(&mut state, &mut view, &core, None, &db));
    assert!(
        refused.is_err(),
        "nothing can start it, so no tab is opened"
    );
    assert!(rig.titles().is_empty());
}

#[test]
fn terminal_a_paused_container_is_refused_with_the_reason() {
    let rig = rig();
    let paused = rig
        .world
        .containers
        .iter()
        .find(|c| c.state == berth_core::ContainerState::Paused)
        .cloned()
        .expect("a paused mock container");
    let (mut state, mut view) = (rig.state, rig.view);
    let core = rig.core.clone();
    let refused = rig
        .dom
        .in_runtime(|| open_container_terminal(&mut state, &mut view, &core, None, &paused));
    assert_eq!(
        refused.expect_err("refused"),
        "Resume the container to open a terminal"
    );
    assert!(rig.titles().is_empty());
}

#[test]
fn terminal_open_all_services_opens_one_tab_per_running_service() {
    let rig = rig();
    let (mut state, mut view) = (rig.state, rig.view);
    let core = rig.core.clone();
    let shop = rig.project("shop-api");
    let opened = rig
        .dom
        .in_runtime(|| open_project_terminals(&mut state, &mut view, &core, &shop.id));
    assert_eq!(
        opened,
        Ok(3),
        "api, redis and mailpit run; worker and db do not"
    );
    let mut titles = rig.titles();
    titles.sort();
    assert_eq!(
        titles,
        [
            "shop-api/api-1 \u{b7} bash",
            "shop-api/mailpit-1 \u{b7} bash",
            "shop-api/redis-1 \u{b7} bash",
        ]
    );
    // A project with nothing running says so, and offers nothing.
    let notebooks = rig.project("ml-notebooks");
    let none = rig
        .dom
        .in_runtime(|| open_project_terminals(&mut state, &mut view, &core, &notebooks.id));
    assert_eq!(none, Err("Nothing is running in this project".to_string()));
    assert_eq!(
        project_terminals_reason(&rig.world.containers, &notebooks, Backing::Live).as_deref(),
        Some("Nothing is running in this project")
    );
    assert_eq!(
        project_terminals_reason(&rig.world.containers, &shop, Backing::Live),
        None
    );
    // "Open terminals for all services" under mock data: refused with the
    // reason, even for a project with services running.
    assert_eq!(
        project_terminals_reason(&rig.world.containers, &shop, Backing::Mock).as_deref(),
        Some("Terminals need a real Docker engine. You're viewing mock data.")
    );
    assert_eq!(rig.titles().len(), 3);
}

#[test]
fn terminal_closing_the_shown_tab_shows_its_neighbour_and_ends_its_session() {
    let rig = rig();
    let (mut state, mut view) = (rig.state, rig.view);
    let core = rig.core.clone();
    let shop = rig.project("shop-api");
    rig.dom
        .in_runtime(|| open_project_terminals(&mut state, &mut view, &core, &shop.id))
        .expect("opened");
    let ids = rig.ids();
    rig.dom
        .in_runtime(|| select_tab(&mut state, &mut view, &core, ids[1]));
    rig.dom
        .in_runtime(|| close_tab(&mut state, &mut view, &core, ids[1]));
    assert!(rig.core.view(ids[1]).is_none(), "the slot is gone");
    rig.dom.in_runtime(|| {
        assert_eq!(state.bottom.peek().terminals.len(), 2);
        assert_eq!(*view.active.peek(), Some(ids[2]), "the next tab takes over");
    });
    rig.dom
        .in_runtime(|| close_tab(&mut state, &mut view, &core, ids[2]));
    rig.dom
        .in_runtime(|| assert_eq!(*view.active.peek(), Some(ids[0])));
}

#[test]
fn terminal_run_as_docker_command_opens_a_tab_on_the_scoped_engine() {
    let rig = rig();
    let (mut state, mut view) = (rig.state, rig.view);
    let core = rig.core.clone();
    let engine = rig.dom.in_runtime(|| {
        state
            .engine_scope
            .set(crate::state::EngineScope::One(berth_core::EngineId(
                "colima:default".to_string(),
            )));
        docker_engine(&state)
    });
    let engine = engine.expect("an engine");
    assert_eq!(engine.0, "colima:default");
    let id = rig
        .dom
        .in_runtime(|| open_docker_tab(&mut state, &mut view, &core, engine.clone(), "ps -a"))
        .expect("opened");
    assert_eq!(rig.titles(), ["docker ps -a"]);
    let Spec { kind, .. } = rig.core.spec(id).expect("spec");
    assert_eq!(
        kind,
        Kind::Docker {
            args: vec!["ps".to_string(), "-a".to_string()]
        }
    );
    let empty = rig
        .dom
        .in_runtime(|| open_docker_tab(&mut state, &mut view, &core, engine, "   "));
    assert!(empty.is_err());
}
