#![allow(clippy::expect_used)]
//! Live Containers screen against the berthfx fixture (P2-T7 done-when,
//! the parts that need no click). Run with `scripts/it.sh -p berth-app
//! containers_it`, which brings the fixtures up first.
//!
//! What the owner's by-hand checks do, as far as a headless run can:
//!
//! - filtering "berthfx" leaves 4 rows (model and rendered screen);
//! - the web container's port link is `http://localhost:56080`, the
//!   command a click runs is `open http://localhost:56080`, and that URL
//!   really serves the nginx page;
//! - J and K step through exactly those 4 rows.
//!
//! Nothing is stopped, removed or restarted; the engine is only read.

mod common;

use std::io::{Read, Write};
use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::screens::containers::{
    build_list, open_command, port_url, provide_containers_view, step_selection, ContainersScreen,
    ContainersView, Filters, GroupBy, ListInput, ListRow,
};
use berth_app::services::engines::{client_map, run_engine, Applier, EngineUpdate};
use berth_app::state::{
    use_app_state, AppState, EngineScope, PendingKey, PendingState, Section, Selection,
};
use berth_core::{Container, Engine, Project};
use berth_engine::EngineClient;

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { div {} }
}

fn harness() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    (dom, state)
}

#[derive(Clone, PartialEq)]
struct Live {
    engines: Vec<Engine>,
    containers: Vec<Container>,
    projects: Vec<Project>,
    filter: String,
}

#[component]
fn LiveScreen(live: Live) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    let view: ContainersView = provide_containers_view();
    use_hook(move || {
        state.engines.set(live.engines.clone());
        if let Some(engine) = live.engines.iter().find(|e| e.id.0 == "colima:default") {
            state.engine_scope.set(EngineScope::One(engine.id.clone()));
        }
        state.containers.set(live.containers.clone());
        state.projects.set(live.projects.clone());
        state.section.set(Section::Containers);
        let mut filters = view.filters;
        filters.write().text = live.filter.clone();
    });
    rsx! { ContainersScreen {} }
}

fn render_live(live: Live) -> String {
    let mut dom = VirtualDom::new_with_props(LiveScreen, LiveScreenProps { live });
    let _ = dom.rebuild_to_vec();
    let mut html = dioxus_ssr::render(&dom);
    while let (Some(a), Some(b)) = (html.find("<style>"), html.find("</style>")) {
        if b < a {
            break;
        }
        html.replace_range(a..b + "</style>".len(), "");
    }
    html
}

fn http_get(port: u16) -> String {
    let mut stream =
        std::net::TcpStream::connect(("127.0.0.1", port)).expect("fixture port accepts");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("timeout");
    stream
        .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .expect("request");
    let mut body = String::new();
    let _ = stream.read_to_string(&mut body);
    body
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1 via scripts/it.sh"]
async fn containers_it_filter_berthfx_shows_four_rows_and_the_port_link_opens_nginx() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let (dom, mut state) = harness();
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search);
    let engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running")
        .clone();
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let _clients = client_map();
    let cancel = CancellationToken::new();

    let mut applier = Applier::new(Vec::new());
    dom.in_runtime(|| {
        applier.apply(&mut state, EngineUpdate::EnginesDiscovered(engines.clone()));
    });
    let (tx, mut updates) = tokio::sync::mpsc::channel(256);
    let task = tokio::spawn(run_engine(
        Arc::clone(&client),
        Arc::clone(&store),
        Vec::new(),
        false,
        tx,
        cancel.clone(),
        std::time::Duration::from_millis(200),
    ));
    // Read the engine until the berthfx containers are in the signal.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let have = dom.in_runtime(|| {
            state
                .containers
                .peek()
                .iter()
                .filter(|c| c.name.starts_with("berthfx-"))
                .count()
        });
        if have >= 4 || tokio::time::Instant::now() >= deadline {
            break;
        }
        if let Ok(Some(update)) =
            tokio::time::timeout(std::time::Duration::from_millis(500), updates.recv()).await
        {
            let mut handle = state;
            dom.in_runtime(|| applier.apply(&mut handle, update));
        }
    }
    cancel.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;

    let (containers, projects, engines_now) = dom.in_runtime(|| {
        (
            state.containers.peek().clone(),
            state.projects.peek().clone(),
            state.engines.peek().clone(),
        )
    });
    let filters = Filters {
        text: "berthfx".to_string(),
        ..Filters::default()
    };
    let scope = EngineScope::One(engine.id.clone());
    let collapsed = std::collections::BTreeMap::new();
    let pending: std::collections::BTreeMap<PendingKey, PendingState> =
        std::collections::BTreeMap::new();
    let model = build_list(&ListInput {
        containers: &containers,
        projects: &projects,
        engines: &engines_now,
        scope: &scope,
        filters: &filters,
        group_by: GroupBy::Project,
        sort: None,
        collapsed: &collapsed,
        pending: &pending,
    });
    let order = model.selection_order();
    let group_names: Vec<String> = model
        .rows
        .iter()
        .filter_map(|r| match r {
            ListRow::Group(g) => Some(g.name.clone()),
            _ => None,
        })
        .collect();

    // The rendered screen shows the same rows.
    let html = render_live(Live {
        engines: engines_now.clone(),
        containers: containers.clone(),
        projects: projects.clone(),
        filter: "berthfx".to_string(),
    });
    let rendered_rows = html.matches("class=\"ct-row").count();

    // The web container's link and the command a click runs.
    let web = containers
        .iter()
        .find(|c| c.name == "berthfx-web-1")
        .expect("berthfx-web-1");
    let url = web
        .ports
        .iter()
        .find_map(port_url)
        .expect("web publishes a port");
    let (program, args) = open_command(&url);
    let page = http_get(56080);

    // J then K over the four rows.
    let first = step_selection(&order, &Selection::None, true);
    let second = step_selection(&order, &Selection::Containers(vec![order[0].clone()]), true);
    let back = step_selection(
        &order,
        &Selection::Containers(vec![order[1].clone()]),
        false,
    );

    println!(
        "containers_it: matched={} of {} order={} groups={group_names:?} rendered_rows={rendered_rows} \
         url={url} cmd={program} {args:?} nginx_in_page={} link_in_html={}",
        model.matched,
        model.total,
        order.len(),
        page.to_lowercase().contains("nginx"),
        html.contains("href=\"http://localhost:56080\""),
    );

    assert_eq!(model.matched, 4, "four berthfx containers");
    assert_eq!(order.len(), 4);
    // One tree owns the fixtures, so berthfx is one project with its four
    // containers (not several projects of the same name).
    assert_eq!(group_names, ["berthfx"], "exactly one berthfx group");
    let berthfx: Vec<&Project> = projects.iter().filter(|p| p.name == "berthfx").collect();
    assert_eq!(berthfx.len(), 1, "exactly one berthfx project: {berthfx:?}");
    assert_eq!(
        berthfx[0]
            .services
            .iter()
            .map(|s| s.containers.len())
            .sum::<usize>(),
        4,
        "the berthfx project holds the four fixture containers"
    );
    assert_eq!(rendered_rows, 4, "the screen renders the four rows");
    assert!(html.contains("4 of "), "the filter field counts matches");
    assert_eq!(url, "http://localhost:56080");
    assert_eq!(args, ["http://localhost:56080"]);
    assert!(
        html.contains("href=\"http://localhost:56080\""),
        "the port renders as a link"
    );
    assert!(
        page.to_lowercase().contains("nginx"),
        "the link target serves the nginx page"
    );
    assert_eq!(first.as_ref(), Some(&order[0]));
    assert_eq!(second.as_ref(), Some(&order[1]));
    assert_eq!(back.as_ref(), Some(&order[0]));
}
