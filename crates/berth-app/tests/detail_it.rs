#![allow(clippy::expect_used)]
//! Live inspector against the berthfx fixture (P3-T3 done-when, the part
//! that needs no click): `berthfx-db-1`'s Env tab lists `POSTGRES_PASSWORD`
//! masked, and the value exists only after the reveal. Run with
//! `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only detail_it`
//! from a tree whose fixtures are already up (never `scripts/it.sh` from a
//! worktree: Compose would record the worktree folder).
//!
//! The password is compared with the fixture's `.env` without being
//! printed: a failing assertion names the key, never the value. The engine
//! is only read.

mod common;

use std::sync::Arc;
use std::time::Instant;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::screens::container_detail::model::{env_rows, DetailTab, EnvShown, Reveals};
use berth_app::screens::container_detail::{ContainerDetailPanel, EnvList};
use berth_app::services::detail::{load, provide_detail_store, DetailEntry};
use berth_app::state::{use_app_state, Section, Selection};
use berth_core::Container;
use berth_engine::EngineClient;

/// `BERTHFX_DB_PASSWORD` from `fixtures/compose/berthfx/.env`.
fn fixture_password() -> String {
    let text = include_str!("../../../fixtures/compose/berthfx/.env");
    text.lines()
        .find_map(|line| line.strip_prefix("BERTHFX_DB_PASSWORD="))
        .expect("the fixture .env sets BERTHFX_DB_PASSWORD")
        .trim()
        .trim_matches('"')
        .to_string()
}

#[derive(Clone, PartialEq)]
struct Live {
    container: Container,
    entry: DetailEntry,
}

#[component]
fn LivePanel(live: Live) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    let store = provide_detail_store();
    let seeded = live.clone();
    use_hook(move || {
        state.containers.set(vec![seeded.container.clone()]);
        state.section.set(Section::Containers);
        state
            .selection
            .set(Selection::Containers(vec![seeded.container.id.clone()]));
        let mut entries = store.entries;
        entries
            .write()
            .insert(seeded.container.id.clone(), seeded.entry.clone());
    });
    rsx! {
        ContainerDetailPanel { id: live.container.id.clone(), initial_tab: DetailTab::Env }
    }
}

#[component]
fn RevealedList(rows: Vec<berth_app::screens::container_detail::model::EnvRow>) -> Element {
    rsx! { EnvList { rows, onreveal: |_| {}, onhide: |_| {} } }
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn detail_it_db_env_lists_the_password_masked_and_reveals_it_only_on_request() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
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
    let db = client
        .containers()
        .await
        .expect("list")
        .into_iter()
        .find(|c| c.name == "berthfx-db-1")
        .expect("berthfx-db-1 exists");

    let loaded = load(client.as_ref(), &db, CancellationToken::new())
        .await
        .expect("inspect of berthfx-db-1");
    let expected = fixture_password();
    let actual = loaded
        .detail
        .env
        .iter()
        .find(|e| e.key == "POSTGRES_PASSWORD")
        .expect("POSTGRES_PASSWORD is in the env");
    assert!(
        actual.value == expected,
        "POSTGRES_PASSWORD differs from the fixture's .env"
    );
    // The inspect that feeds the tree carries no value either.
    let inspect = loaded.inspect.to_string();
    assert!(
        !inspect.contains(&expected),
        "the masked inspect still holds the POSTGRES_PASSWORD value"
    );
    assert!(inspect.contains("POSTGRES_PASSWORD=••••"));

    // The Env tab, as rendered before any click.
    let live = Live {
        container: db,
        entry: DetailEntry::Ready(Box::new(loaded.clone())),
    };
    let mut dom = VirtualDom::new_with_props(LivePanel, LivePanelProps { live });
    let _ = dom.rebuild_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("POSTGRES_PASSWORD"), "the key is listed");
    assert!(html.contains("••••"), "the value is masked");
    assert!(
        !html.contains(&expected),
        "the Env tab holds the POSTGRES_PASSWORD value before a reveal"
    );

    // One click reveals that key, and only then does the value exist.
    let now = Instant::now();
    let masked_rows = env_rows(&loaded.detail.env, &Reveals::default(), now);
    assert!(masked_rows.iter().all(|r| r.shown == EnvShown::Masked));
    let mut reveals = Reveals::default();
    reveals.reveal("POSTGRES_PASSWORD", now);
    let rows = env_rows(&loaded.detail.env, &reveals, now);
    let mut list = VirtualDom::new_with_props(RevealedList, RevealedListProps { rows });
    let _ = list.rebuild_to_vec();
    let revealed = dioxus_ssr::render(&list);
    assert!(
        revealed.contains(&expected),
        "the reveal did not show the POSTGRES_PASSWORD value"
    );
    // The other variables stay masked.
    assert!(revealed.contains("Show the value of PATH"));
}
