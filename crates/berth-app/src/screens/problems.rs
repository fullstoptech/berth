//! The Problems tab of the bottom panel (P5-T8) and the answer to Explain in a log view.
//!
//! The list is `AppState::problems`, which the problems service keeps (stable order, grouped
//! here by project). Every item is the shared `ProblemItem`; a fix button opens Review changes,
//! a confirm dialog or Clean up (`model::fix_click`). The tab is only a view: it calls no engine.
//!
//! States: empty ("No problems"), checking (no engine has answered yet), the list, and per fix
//! running and failed (a button's loading state and the engine's message under it).

pub mod model;

use berth_core::Problem;
use dioxus::prelude::*;

use crate::services::actions::StateView;
use crate::services::assistant::AssistantService;
use crate::services::plans::PlanStore;
use crate::state::AppState;
use crate::ui::{connection, Connection, ProblemItem, PROBLEM_ITEM_CSS};
use model::{click, groups, problem_view, run_harness_fix};

/// The tab's CSS, rendered with it (tokens only).
pub const PROBLEMS_CSS: &str = include_str!("../../assets/css/problems.css");

/// A list with this many problems or fewer starts with every item open; longer lists open the
/// first only, and a click opens the others.
const OPEN_ALL_UP_TO: usize = 3;

/// Run `f` over a [`StateView`] read from the signals, so the component re-renders when any of
/// them changes (`with_view` peeks and does not subscribe).
fn read_view<R>(state: &AppState, f: impl FnOnce(&StateView<'_>) -> R) -> R {
    let engines = state.engines.read();
    let containers = state.containers.read();
    let projects = state.projects.read();
    let pending = state.pending.read();
    f(&StateView {
        engines: &engines,
        containers: &containers,
        projects: &projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending: &pending,
        backing: *state.backing.peek(),
    })
}

/// What the tab says when there is nothing to list.
fn empty_text(connection: Connection) -> (&'static str, &'static str) {
    match connection {
        Connection::Connecting | Connection::EngineStarting => (
            "Checking\u{2026}",
            "Berth is reading your engines. Problems show up here when it has.",
        ),
        _ => (
            "No problems",
            "Berth checks every engine as things change and lists what it can explain.",
        ),
    }
}

/// The body of the Problems tab.
#[component]
pub fn ProblemsPanel() -> Element {
    let state = use_context::<AppState>();
    let plans = try_consume_context::<PlanStore>();
    let assistant = try_consume_context::<Option<AssistantService>>().flatten();
    let now = chrono::Utc::now();
    let problems = state.problems.read().clone();
    let harnesses = state.harnesses.read().clone();
    let shown = read_view(&state, |view| groups(&problems, view, now, &harnesses));
    let connection = connection(&state.engines.read(), &state.engine_scope.read());
    let open_all = problems.len() <= OPEN_ALL_UP_TO;
    let first = problems.first().map(|p| p.id.0.clone());
    let on_fix = {
        let mut state = state;
        move |fix: String| {
            let problems = state.problems.peek().clone();
            if !run_harness_fix(&mut state, assistant.clone(), &problems, &fix) {
                click(&mut state, plans.as_ref(), &problems, &fix);
            }
        }
    };
    rsx! {
        style { "{PROBLEM_ITEM_CSS}" }
        style { "{PROBLEMS_CSS}" }
        div { class: "problems", role: "region", "aria-label": "Problems",
            if shown.is_empty() {
                {
                    let (title, text) = empty_text(connection);
                    rsx! {
                        div { class: "problems-empty",
                            p { class: "problems-empty-title", "{title}" }
                            p { class: "problems-empty-text", "{text}" }
                        }
                    }
                }
            }
            for group in shown {
                section { key: "{group.label}", class: "problems-group",
                    header { class: "problems-group-head",
                        span { class: "problems-group-name", "{group.label}" }
                        span { class: "problems-group-count", "{group.items.len()}" }
                    }
                    for item in group.items {
                        div { key: "{item.id}", class: "problems-item",
                            {
                                let open = open_all || first.as_deref() == Some(item.id.as_str());
                                rsx! { ProblemItem { problem: item, open, onfix: on_fix.clone() } }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// What a log view's Explain found, shown under the selection bar. `None` while the rules run.
#[component]
pub fn ExplainPanel(problems: Option<Vec<Problem>>, onclose: EventHandler<()>) -> Element {
    let state = use_context::<AppState>();
    let plans = try_consume_context::<PlanStore>();
    let assistant = try_consume_context::<Option<AssistantService>>().flatten();
    let now = chrono::Utc::now();
    let list = problems.clone().unwrap_or_default();
    let harnesses = state.harnesses.read().clone();
    let views = read_view(&state, |view| {
        list.iter()
            .map(|p| problem_view(p, view, now, true, &harnesses))
            .collect::<Vec<_>>()
    });
    let on_fix = {
        let mut state = state;
        let list = list.clone();
        move |fix: String| {
            if !run_harness_fix(&mut state, assistant.clone(), &list, &fix) {
                click(&mut state, plans.as_ref(), &list, &fix);
            }
        }
    };
    rsx! {
        style { "{PROBLEM_ITEM_CSS}" }
        style { "{PROBLEMS_CSS}" }
        div { class: "explain", role: "region", "aria-label": "Explanation",
            div { class: "explain-head",
                span { class: "explain-title", "What the rules say about these lines" }
                button {
                    class: "explain-close",
                    r#type: "button",
                    "aria-label": "Close the explanation",
                    onclick: move |_| onclose.call(()),
                    crate::ui::Icon { name: "x", size: crate::ui::IconSize::Sm }
                }
            }
            if problems.is_none() {
                p { class: "explain-note", "Checking the lines\u{2026}" }
            } else if views.is_empty() {
                p { class: "explain-note",
                    "No known problem matches these lines. Berth explains the 15 kinds it knows: select the error line itself, with the lines around it if you can."
                }
            }
            for item in views {
                div { key: "{item.id}", class: "problems-item",
                    ProblemItem { problem: item, open: true, onfix: on_fix.clone() }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "problems_tests.rs"]
mod tests;
