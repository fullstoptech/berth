//! Assistant tab actions (D77: ⌘. stops the turn here, but
//! `container.start_stop` carries the shortcut string).

use super::{def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::Chord;
use crate::state::{AppState, BottomTab};

pub const DEFS: &[ActionDef] = &[
    def(
        "assistant.stop_turn",
        "Stop the assistant turn",
        None,
        S::Assistant,
    ),
    def("assistant.approve_tool", "Approve", None, S::Assistant),
    def("assistant.deny_tool", "Deny", None, S::Assistant),
    def("assistant.open", "Open assistant", None, S::Assistant),
];

const HANDLERS: &[(&str, Handler)] = &[
    // ⌘. reaches the running turn of the selected project's session (D77).
    ("assistant.stop_turn", stop_turn as Handler),
    ("assistant.open", |state, _| {
        use dioxus::prelude::WritableExt;
        let mut bottom = state.bottom.write();
        bottom.open = true;
        bottom.tab = BottomTab::Assistant;
    }),
];

fn stop_turn(state: &mut AppState, _: &str) {
    use dioxus::prelude::ReadableExt;
    let Some(service) = dioxus::prelude::try_consume_context::<
        Option<crate::services::assistant::AssistantService>,
    >()
    .flatten() else {
        return;
    };
    let Some(project) = crate::screens::assistant::current_project(state) else {
        return;
    };
    let sessions = state.assistant.peek();
    if let Some(session) = crate::services::assistant::session_for_project(&sessions, &project.id) {
        let id = session.id;
        drop(sessions);
        service.interrupt(id);
    }
}

const CHORDS: &[Chord] = &[Chord::cmd(&["."], "assistant.stop_turn").when(|ctx| ctx.in_assistant)];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    ..Area::EMPTY
};
