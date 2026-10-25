//! The terminal UI (P3-T6; frames "Container / terminal / 1440" and "Container /
//! terminal / no shell / 1440"; PLAN D41, §7.5).
//!
//! `TerminalBody` is the bottom panel's Terminal tab: a strip of tabs, a
//! toolbar and the xterm host. `TerminalPane` is one session's view and is
//! all a pop-out window needs. `TerminalPanel` is the small launcher the
//! inspector's Terminal tab shows. Sessions live in `services::exec`; a
//! pane only draws what the service pushes and sends back what is typed.

use dioxus::prelude::*;
use futures::StreamExt;
use uuid::Uuid;

use super::button::{Button, ButtonVariant};
use super::empty_state::EmptyState;
use super::icon::{Icon, IconSize};
use super::ControlSize;
use crate::bridge::terminal::host_id;
use crate::bridge::{Bridge, Event, TermCommand, TermEvent, TermOptions, Theme};
use crate::services::exec::{
    close_tab, open_container_terminal, open_tab, select_tab, tab_label, terminal_disabled_reason,
    view_id, Attached, ExecCore, Kind, PaneView, Status, TerminalView, ViewMsg, SHELL_CHOICES,
};
use crate::state::{AppState, BottomTab, Section, Selection, Toast, ToastTone};

pub const TERMINAL_CSS: &str = include_str!("../../assets/css/terminal.css");

/// A toolbar button that can give way to the ⋯ menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Reconnect,
    Clear,
    Copy,
    Split,
    PopOut,
    OpenApp,
}

impl Tool {
    /// In toolbar order.
    pub const ALL: [Tool; 6] = [
        Tool::Reconnect,
        Tool::Clear,
        Tool::Copy,
        Tool::Split,
        Tool::PopOut,
        Tool::OpenApp,
    ];

    /// The order buttons give way as the pane narrows, first to go first:
    /// the widest and rarest, then the ones a session needs least.
    pub const GIVE_WAY: [Tool; 6] = [
        Tool::OpenApp,
        Tool::PopOut,
        Tool::Split,
        Tool::Copy,
        Tool::Clear,
        Tool::Reconnect,
    ];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Reconnect => "reconnect",
            Self::Clear => "clear",
            Self::Copy => "copy",
            Self::Split => "split",
            Self::PopOut => "pop-out",
            Self::OpenApp => "open-app",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Reconnect => "Reconnect",
            Self::Clear => "Clear",
            Self::Copy => "Copy",
            Self::Split => "Split",
            Self::PopOut => "Pop out",
            Self::OpenApp => "Open in Terminal app",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.id() == id)
    }

    /// The button's width in px at 100 % text, measured from the frame.
    fn width(self) -> f32 {
        match self {
            Self::Reconnect => 84.0,
            Self::Clear => 52.0,
            Self::Copy => 50.0,
            Self::Split => 48.0,
            Self::PopOut => 64.0,
            Self::OpenApp => 172.0,
        }
    }
}

/// What the Shell, User and Working directory pickers take of the toolbar at
/// 100 % text (two selects and the shortest path field).
const PICKERS_PX: f32 = 100.0 + 112.0 + 60.0 + 2.0 * 8.0;
/// The ⋯ button, and the toolbar's padding and gaps.
const MORE_PX: f32 = 28.0;
const TOOLBAR_PAD_PX: f32 = 16.0;
const TOOL_GAP_PX: f32 = 8.0;

/// The toolbar buttons that fit a pane `pane_px` wide, and the ones that go
/// to the ⋯ menu (both in toolbar order). `tools` are the buttons this
/// session has; `pickers` says the Shell, User and directory fields are
/// there. Buttons give way in [`Tool::GIVE_WAY`] order until the rest and the
/// ⋯ button fit. `None` (a window of its own) keeps everything.
#[must_use]
pub fn toolbar_tools(
    pane_px: Option<u32>,
    text_scale: f32,
    pickers: bool,
    tools: &[Tool],
) -> (Vec<Tool>, Vec<Tool>) {
    let Some(pane_px) = pane_px else {
        return (tools.to_vec(), Vec::new());
    };
    let room = pane_px as f32 - TOOLBAR_PAD_PX * text_scale.min(1.0);
    let fixed = if pickers {
        PICKERS_PX * text_scale
    } else {
        0.0
    };
    let mut shown: Vec<Tool> = tools.to_vec();
    let mut moved: Vec<Tool> = Vec::new();
    loop {
        let buttons: f32 = shown
            .iter()
            .map(|tool| tool.width() * text_scale + TOOL_GAP_PX)
            .sum();
        let more = if moved.is_empty() {
            0.0
        } else {
            MORE_PX * text_scale + TOOL_GAP_PX
        };
        if fixed + buttons + more <= room || shown.is_empty() {
            break;
        }
        let Some(next) = Tool::GIVE_WAY.into_iter().find(|tool| shown.contains(tool)) else {
            break;
        };
        shown.retain(|tool| *tool != next);
        moved.push(next);
    }
    moved.sort_by_key(|tool| Tool::ALL.iter().position(|t| t == tool));
    (shown, moved)
}

/// The window's one bridge, for the panes below [`TerminalHost`].
#[derive(Clone, Copy)]
pub struct TermBridge(pub Bridge);

/// Largest piece of output sent to the page in one message.
const WRITE_CHUNK: usize = 32 * 1024;

// ---------------------------------------------------------------------
// Pure pieces
// ---------------------------------------------------------------------

/// What the pump does for a run of service messages.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Write(Vec<u8>),
    Clear,
    View(Box<PaneView>),
}

/// Merge consecutive output into as few writes as `WRITE_CHUNK` allows. A
/// clear drops what came before it; a view change keeps its place.
#[must_use]
pub fn coalesce(messages: Vec<ViewMsg>) -> Vec<Paint> {
    fn flush(pending: &mut Vec<u8>, out: &mut Vec<Paint>) {
        for chunk in pending.chunks(WRITE_CHUNK) {
            out.push(Paint::Write(chunk.to_vec()));
        }
        pending.clear();
    }
    let mut out = Vec::new();
    let mut pending = Vec::new();
    for message in messages {
        match message {
            ViewMsg::Replay(bytes) | ViewMsg::Output(bytes) => pending.extend(bytes),
            ViewMsg::Snapshot(view) => {
                flush(&mut pending, &mut out);
                out.push(Paint::View(view));
            }
            ViewMsg::Clear => {
                pending.clear();
                out.push(Paint::Clear);
            }
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// Does a pane keep this key, so the shell's shortcuts never see it? Typing
/// goes to the terminal (J, K, Space and ⌫ would otherwise act on the
/// list); ⌘ chords and ⌃` still reach Berth.
#[must_use]
pub fn pane_keeps_key(key: &str, meta: bool, ctrl: bool) -> bool {
    !(meta || (ctrl && key == "`"))
}

/// The user picker's choices (default is the image's own user).
const USERS: [(&str, &str); 2] = [("", "default"), ("root", "root")];

// ---------------------------------------------------------------------
// Host
// ---------------------------------------------------------------------

/// One bridge per window: starts it, pumps xterm's messages into the
/// sessions and provides [`TermBridge`] to the panes below.
#[component]
pub fn TerminalHost(core: ExecCore, children: Element) -> Element {
    let bridge = use_hook(move || {
        let (bridge, mut events) = Bridge::start();
        spawn(async move {
            while let Some(event) = events.next().await {
                if let Event::Term(event) = event {
                    // A host that just mounted takes the keyboard.
                    let focus = match &event {
                        TermEvent::Ready { id } => Some(id.clone()),
                        _ => None,
                    };
                    core.term_event(event);
                    if let Some(id) = focus {
                        bridge.send(TermCommand::Focus { id });
                    }
                }
            }
        });
        bridge
    });
    use_context_provider(|| TermBridge(bridge));
    rsx! {
        style { "{TERMINAL_CSS}" }
        {children}
    }
}

// ---------------------------------------------------------------------
// One session
// ---------------------------------------------------------------------

fn push_toast(state: Option<AppState>, text: String, tone: ToastTone) {
    if let Some(mut state) = state {
        state.toasts.write().push(Toast {
            id: Uuid::now_v7(),
            text,
            tone,
            action: None,
        });
    }
}

fn open_in_terminal_app(core: &ExecCore, view: &PaneView, state: Option<AppState>) {
    let Kind::Exec(exec) = &view.spec.kind else {
        return;
    };
    let Some(docker_host) = core.docker_host(&view.spec.engine) else {
        push_toast(
            state,
            "The engine is not connected".to_string(),
            ToastTone::Danger,
        );
        return;
    };
    let target = crate::platform::ExecTarget {
        docker_host,
        container: exec.container.0.clone(),
        shell: view
            .shell_used
            .clone()
            .or_else(|| exec.shell.clone())
            .unwrap_or_else(|| "bash".to_string()),
        user: exec.user.clone(),
        workdir: exec.workdir.clone(),
    };
    spawn(async move {
        if let Err(message) = crate::platform::open_in_terminal_app(target).await {
            push_toast(state, message, ToastTone::Danger);
        }
    });
}

/// Move a tab into its own window. The tab leaves the panel; closing the
/// window ends the session.
fn pop_out(state: &mut AppState, view: &mut TerminalView, core: &ExecCore, tab: Uuid) {
    let Some(pane) = core.view(tab) else { return };
    let settings = state.settings.peek().clone();
    {
        let mut bottom = state.bottom.write();
        bottom.terminals.retain(|t| t.id != tab);
        if *view.active.peek() == Some(tab) {
            let next = bottom.terminals.first().map(|t| t.id);
            view.active.set(next);
        }
    }
    if *view.split.peek() == Some(tab) {
        let mut split = view.split;
        split.set(None);
    }
    let dom = VirtualDom::new_with_props(
        PopoutRoot,
        PopoutProps {
            core: core.clone(),
            tab,
            theme: settings.theme,
            text_scale: settings.text_scale,
        },
    );
    let window = dioxus::desktop::WindowBuilder::new()
        .with_title(pane.title)
        .with_inner_size(dioxus::desktop::LogicalSize::new(900.0, 560.0));
    let config = dioxus::desktop::Config::new()
        .with_window(window)
        .with_custom_head(crate::assets::head_html());
    drop(dioxus::desktop::window().new_window(dom, config));
}

#[derive(Clone, PartialEq, Props)]
struct PopoutProps {
    core: ExecCore,
    tab: Uuid,
    theme: berth_store::Theme,
    text_scale: f32,
}

/// The root of a pop-out window: the tokens, one host, one pane.
#[component]
fn PopoutRoot(props: PopoutProps) -> Element {
    crate::assets::use_assets();
    crate::bridge::use_web_assets();
    let script = crate::state::theme_script(
        props.theme,
        berth_store::Density::Comfortable,
        props.text_scale,
    );
    use_effect(move || {
        let script = script.clone();
        spawn(async move {
            let _ = document::eval(&script).await;
        });
    });
    let core = props.core.clone();
    use_context_provider(|| core.clone());
    let tab = props.tab;
    use_drop(move || core.close(tab));
    rsx! {
        style { "{crate::assets::FONTS_CSS}" }
        div { class: "term-popout",
            TerminalHost { core: props.core.clone(),
                TerminalPane {
                    tab: props.tab,
                    theme: Theme::from(props.theme),
                    font_size: 12.0 * props.text_scale,
                    popped: true,
                }
            }
        }
    }
}

/// One session: toolbar, xterm host and the no-shell or error state.
#[component]
pub fn TerminalPane(
    tab: Uuid,
    theme: Theme,
    font_size: f32,
    #[props(default = true)] toolbar: bool,
    #[props(default)] popped: bool,
    /// The pane's width in px, which decides how many toolbar buttons give
    /// way to the ⋯ menu; `None` keeps them all.
    #[props(default)]
    pane_px: Option<u32>,
) -> Element {
    let core = use_context::<ExecCore>();
    let bridge = use_context::<TermBridge>().0;
    let state = try_use_context::<AppState>();
    let view_ctx = try_use_context::<TerminalView>();
    let mut view = use_signal(|| core.view(tab));
    let attach_core = core.clone();
    let viewer = use_hook(move || {
        let Attached {
            viewer,
            view: first,
            mut rx,
        } = attach_core.attach(tab)?;
        view.set(Some(first));
        let id = view_id(tab, viewer);
        bridge.send(TermCommand::Create {
            id: id.clone(),
            options: TermOptions::new(font_size, theme),
        });
        spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                for paint in coalesce(batch) {
                    match paint {
                        Paint::Write(data) => bridge.send(TermCommand::Write {
                            id: id.clone(),
                            data,
                        }),
                        Paint::Clear => bridge.send(TermCommand::Clear { id: id.clone() }),
                        Paint::View(next) => view.set(Some(*next)),
                    }
                }
            }
        });
        Some(viewer)
    });
    let drop_core = core.clone();
    use_drop(move || {
        if let Some(viewer) = viewer {
            drop_core.detach(tab, viewer);
            bridge.send(TermCommand::Dispose {
                id: view_id(tab, viewer),
            });
        }
    });
    use_effect(use_reactive!(|theme| {
        bridge.send(TermCommand::SetTheme(theme));
    }));

    let Some(pane) = view.read().clone() else {
        return rsx! {};
    };
    let Some(viewer) = viewer else {
        return rsx! {};
    };
    let js_id = view_id(tab, viewer);
    let is_exec = matches!(pane.spec.kind, Kind::Exec(_));
    let exec = match &pane.spec.kind {
        Kind::Exec(exec) => Some(exec.clone()),
        Kind::Docker { .. } => None,
    };
    let shell_now = pane
        .shell_used
        .clone()
        .or_else(|| exec.as_ref().and_then(|e| e.shell.clone()))
        .unwrap_or_else(|| "bash".to_string());
    let user_now = exec
        .as_ref()
        .and_then(|e| e.user.clone())
        .unwrap_or_default();
    let workdir_now = exec
        .as_ref()
        .and_then(|e| e.workdir.clone())
        .unwrap_or_default();
    // The image has no shell at all: the toolbar has nothing to offer.
    let no_shell = matches!(&pane.status, Status::NoShell { tried } if tried.len() > 1);
    let splittable = !popped && is_exec && view_ctx.is_some() && state.is_some();
    let can_pop_out = state.is_some() && view_ctx.is_some();
    let text_scale = state.map_or(1.0, |s| s.settings.read().text_scale);
    let has_tools: Vec<Tool> = Tool::ALL
        .into_iter()
        .filter(|tool| match tool {
            Tool::Split | Tool::PopOut => !popped,
            Tool::OpenApp => is_exec,
            _ => true,
        })
        .collect();
    let (inline_tools, menu_tools) = toolbar_tools(pane_px, text_scale, is_exec, &has_tools);
    let run_tool = {
        let core = core.clone();
        let pane = pane.clone();
        let js_id = js_id.clone();
        move |tool: Tool| match tool {
            Tool::Reconnect => core.reconnect(tab),
            Tool::Clear => core.clear(tab),
            Tool::Copy => bridge.send(TermCommand::CopySelection { id: js_id.clone() }),
            Tool::Split => {
                if let (Some(mut state), Some(mut ctx)) = (state, view_ctx) {
                    split(&mut state, &mut ctx, &core, tab);
                }
            }
            Tool::PopOut => {
                if let (Some(mut state), Some(mut ctx)) = (state, view_ctx) {
                    pop_out(&mut state, &mut ctx, &core, tab);
                }
            }
            Tool::OpenApp => open_in_terminal_app(&core, &pane, state),
        }
    };
    let menu_entries: Vec<super::MenuEntry> = menu_tools
        .iter()
        .map(|tool| {
            let entry = super::MenuEntry::new(tool.id(), tool.label());
            match tool {
                Tool::Split if !splittable => entry.disabled("Needs a shell in a container"),
                Tool::PopOut if !can_pop_out => entry.disabled("Not available here"),
                _ => entry,
            }
        })
        .collect();
    let mut more_open = use_signal(|| false);

    let on_shell = {
        let core = core.clone();
        move |e: FormEvent| {
            let value = e.value();
            core.configure(tab, |exec| exec.shell = Some(value));
        }
    };
    let on_user = {
        let core = core.clone();
        move |e: FormEvent| {
            let value = e.value();
            core.configure(tab, |exec| {
                exec.user = (!value.is_empty()).then_some(value);
            });
        }
    };
    let on_workdir = {
        let core = core.clone();
        move |e: FormEvent| {
            let value = e.value().trim().to_string();
            core.configure(tab, |exec| {
                exec.workdir = (!value.is_empty()).then_some(value);
            });
        }
    };
    let screen_keys = move |e: KeyboardEvent| {
        use dioxus::html::ModifiersInteraction;
        let mods = e.data.modifiers();
        if pane_keeps_key(
            &e.data.key().to_string(),
            mods.contains(dioxus::html::Modifiers::META),
            mods.contains(dioxus::html::Modifiers::CONTROL),
        ) {
            e.stop_propagation();
        }
    };

    rsx! {
        div { class: "term-pane",
            if toolbar && !no_shell {
                div { class: "term-toolbar", role: "toolbar", "aria-label": "Terminal",
                    if is_exec {
                        label { class: "term-pick",
                            span { class: "term-pick-label", "Shell" }
                            select {
                                class: "term-pick-input",
                                "aria-label": "Shell",
                                onchange: on_shell,
                                for choice in SHELL_CHOICES {
                                    option {
                                        key: "{choice}",
                                        value: "{choice}",
                                        selected: choice == shell_now,
                                        "{choice}"
                                    }
                                }
                            }
                            span { class: "term-pick-chevron",
                                Icon { name: "chevron-down", size: IconSize::Sm }
                            }
                        }
                        label { class: "term-pick",
                            span { class: "term-pick-label", "User" }
                            select {
                                class: "term-pick-input",
                                "aria-label": "User",
                                onchange: on_user,
                                for (value , label) in USERS {
                                    option {
                                        key: "{label}",
                                        value: "{value}",
                                        selected: value == user_now,
                                        "{label}"
                                    }
                                }
                            }
                            span { class: "term-pick-chevron",
                                Icon { name: "chevron-down", size: IconSize::Sm }
                            }
                        }
                        input {
                            class: "term-workdir",
                            r#type: "text",
                            "aria-label": "Working directory",
                            placeholder: "Working dir",
                            initial_value: "{workdir_now}",
                            spellcheck: "false",
                            size: "{workdir_now.chars().count().max(6)}",
                            onkeydown: |e| e.stop_propagation(),
                            onchange: on_workdir,
                        }
                    }
                    div { class: "term-spacer" }
                    if inline_tools.contains(&Tool::Reconnect) {
                    Button {
                        label: "Reconnect",
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        onclick: {
                            let core = core.clone();
                            move |_| core.reconnect(tab)
                        },
                    }
                    }
                    if inline_tools.contains(&Tool::Clear) {
                    Button {
                        label: "Clear",
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        onclick: {
                            let core = core.clone();
                            move |_| core.clear(tab)
                        },
                    }
                    }
                    if inline_tools.contains(&Tool::Copy) {
                    Button {
                        label: "Copy",
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        onclick: {
                            let js_id = js_id.clone();
                            move |_| bridge.send(TermCommand::CopySelection { id: js_id.clone() })
                        },
                    }
                    }
                    if !popped && inline_tools.contains(&Tool::Split) {
                        Button {
                            label: "Split",
                            variant: ButtonVariant::Ghost,
                            size: ControlSize::Sm,
                            disabled: !splittable,
                            disabled_reason: (!splittable)
                                .then(|| "Split needs a shell in a container".to_string()),
                            onclick: {
                                let core = core.clone();
                                move |_| {
                                    if let (Some(mut state), Some(mut ctx)) = (state, view_ctx) {
                                        split(&mut state, &mut ctx, &core, tab);
                                    }
                                }
                            },
                        }
                    }
                    if !popped && inline_tools.contains(&Tool::PopOut) {
                        Button {
                            label: "Pop out",
                            variant: ButtonVariant::Ghost,
                            size: ControlSize::Sm,
                            disabled: !(state.is_some() && view_ctx.is_some()),
                            onclick: {
                                let core = core.clone();
                                move |_| {
                                    if let (Some(mut state), Some(mut ctx)) = (state, view_ctx) {
                                        pop_out(&mut state, &mut ctx, &core, tab);
                                    }
                                }
                            },
                        }
                    }
                    if is_exec && inline_tools.contains(&Tool::OpenApp) {
                        Button {
                            label: "Open in Terminal app",
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            icon: "open-external",
                            onclick: {
                                let core = core.clone();
                                let pane = pane.clone();
                                move |_| open_in_terminal_app(&core, &pane, state)
                            },
                        }
                    }
                    if !menu_entries.is_empty() {
                        div { class: "term-more",
                            super::IconButton {
                                icon: "more-horizontal",
                                label: "More terminal actions".to_string(),
                                tooltip: "More".to_string(),
                                size: ControlSize::Sm,
                                pressed: more_open(),
                                onclick: move |_| {
                                    let open = *more_open.peek();
                                    more_open.set(!open);
                                },
                            }
                            if more_open() {
                                div { class: "term-more-menu",
                                    super::Menu {
                                        items: menu_entries.clone(),
                                        aria_label: "More terminal actions".to_string(),
                                        on_select: {
                                            let run_tool = run_tool.clone();
                                            move |id: String| {
                                                more_open.set(false);
                                                if let Some(tool) = Tool::from_id(&id) {
                                                    run_tool(tool);
                                                }
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "term-screen", onkeydown: screen_keys,
                div { id: host_id(&js_id), class: "berth-term" }
                match &pane.status {
                    Status::Connecting => rsx! { div { class: "term-note", "Connecting\u{2026}" } },
                    Status::Starting => rsx! {
                        div { class: "term-note", "Starting the container\u{2026}" }
                    },
                    Status::Failed(message) => rsx! {
                        div { class: "term-note term-note-danger", role: "alert",
                            Icon { name: "alert-circle", size: IconSize::Sm }
                            span { "{message}" }
                        }
                    },
                    Status::NoShell { tried } if exec.is_some() && tried.len() < 2 => rsx! {
                        div { class: "term-note term-note-danger", role: "alert",
                            Icon { name: "alert-circle", size: IconSize::Sm }
                            span { "{shell_now} is not in this image. Pick another shell." }
                        }
                    },
                    Status::NoShell { .. } => rsx! {
                        NoShell {
                            image: exec.as_ref().map(|e| e.image.clone()).unwrap_or_default(),
                            container: exec.as_ref().map(|e| e.container.clone()),
                        }
                    },
                    Status::Live | Status::Exited => rsx! {},
                }
            }
        }
    }
}

/// The state for an image with neither bash nor sh (frame "no shell").
#[component]
fn NoShell(image: String, container: Option<berth_core::ContainerId>) -> Element {
    let state = try_use_context::<AppState>();
    let image_name = image
        .rsplit_once(':')
        .map_or(image.as_str(), |(name, _)| name);
    rsx! {
        div { class: "term-noshell",
            div { class: "term-noshell-icon",
                Icon { name: "terminal", size: IconSize::Md }
            }
            h3 { class: "term-noshell-title", "This image has no shell" }
            p { class: "term-noshell-body",
                "{image_name} is built without bash or sh. Browse its files instead."
            }
            div { class: "term-noshell-actions",
                Button {
                    label: "Files",
                    variant: ButtonVariant::Primary,
                    disabled: container.is_none() || state.is_none(),
                    onclick: move |_| {
                        // Files is a tab of the container's inspector (P3-T8):
                        // select the container so the inspector shows it.
                        if let (Some(mut state), Some(id)) = (state, container.clone()) {
                            state.section.set(Section::Containers);
                            state.selection.set(Selection::Containers(vec![id]));
                        }
                    },
                }
                Button {
                    label: "Open a debug shell\u{2026}",
                    variant: ButtonVariant::Secondary,
                    disabled: true,
                    disabled_reason: "Debug shells are not built yet".to_string(),
                }
            }
            p { class: "term-noshell-note",
                "Starts a temporary busybox container that shares this container\u{2019}s processes and files; removed when you exit."
            }
        }
    }
}

/// Open a sibling session beside the active one (or close the split).
fn split(state: &mut AppState, view: &mut TerminalView, core: &ExecCore, tab: Uuid) {
    if view.split.peek().is_some() {
        let mut split = view.split;
        split.set(None);
        return;
    }
    let Some(spec) = core.spec(tab) else { return };
    let before = *view.active.peek();
    let id = open_tab(state, view, core, spec);
    view.active.set(before);
    let mut split = view.split;
    split.set(Some(id));
}

// ---------------------------------------------------------------------
// The bottom panel's Terminal tab
// ---------------------------------------------------------------------

/// The tab shown: the chosen one, else the first.
#[must_use]
pub fn effective_active(tabs: &[crate::state::TerminalTab], chosen: Option<Uuid>) -> Option<Uuid> {
    chosen
        .filter(|id| tabs.iter().any(|t| t.id == *id))
        .or_else(|| tabs.first().map(|t| t.id))
}

#[component]
pub fn TerminalBody() -> Element {
    let state = use_context::<AppState>();
    let (Some(core), Some(view)) = (
        try_use_context::<ExecCore>(),
        try_use_context::<TerminalView>(),
    ) else {
        return rsx! {
            style { "{TERMINAL_CSS}" }
            EmptyState {
                title: "No terminals".to_string(),
                body: "Terminals are not available here.".to_string(),
                icon: "terminal",
            }
        };
    };
    let tabs = state.bottom.read().terminals.clone();
    let active = effective_active(&tabs, *view.active.read());
    let split_tab = (*view.split.read()).filter(|id| tabs.iter().any(|t| t.id == *id));
    let settings = state.settings.read().clone();
    let theme = Theme::from(settings.theme);
    let font_size = 12.0 * settings.text_scale;
    // The width the panel has (the whole region, less the inspector beside it).
    let pane_px = super::dock::dock_view().map_or(1440 - 220, |view| view.layout.read().panel_px);

    // Looking at a tab reads it.
    let seen_core = core.clone();
    use_effect(move || {
        let Some(id) = effective_active(&state.bottom.peek().terminals, *view.active.read()) else {
            return;
        };
        let unread = state
            .bottom
            .peek()
            .terminals
            .iter()
            .any(|t| t.id == id && t.unread);
        if unread {
            let mut bottom = state.bottom;
            for tab in bottom.write().terminals.iter_mut().filter(|t| t.id == id) {
                tab.unread = false;
            }
        }
        seen_core.seen(id);
    });

    let mut shown: Vec<(Uuid, bool)> = Vec::new();
    if let Some(id) = active {
        shown.push((id, true));
    }
    if let Some(id) = split_tab.filter(|id| Some(*id) != active) {
        shown.push((id, false));
    }

    rsx! {
        style { "{TERMINAL_CSS}" }
        TerminalHost { core: core.clone(),
            div { class: "term",
                if tabs.is_empty() {
                    EmptyState {
                        title: "No terminals open".to_string(),
                        body: "Open one from a container\u{2019}s Terminal button, or select a container and press \u{2318}T."
                            .to_string(),
                        icon: "terminal",
                    }
                } else {
                    TerminalTabs { tabs: tabs.clone(), active, core: core.clone() }
                    div { class: if shown.len() > 1 { "term-split" } else { "term-single" },
                        // One keyed pane per shown tab; a different tab remounts.
                        for (id , primary) in shown.clone() {
                            TerminalPane {
                                key: "{id}",
                                tab: id,
                                theme,
                                font_size,
                                toolbar: primary,
                                pane_px: pane_px / u32::try_from(shown.len().max(1)).unwrap_or(1),
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TerminalTabs(
    tabs: Vec<crate::state::TerminalTab>,
    active: Option<Uuid>,
    core: ExecCore,
) -> Element {
    let mut state = use_context::<AppState>();
    let Some(mut view) = try_use_context::<TerminalView>() else {
        return rsx! {};
    };
    let add_core = core.clone();
    rsx! {
        div { class: "tabs tabs-panel term-tabs", role: "tablist", "aria-label": "Terminals",
            for tab in tabs.iter() {
                {
                    let id = tab.id;
                    let selected = Some(id) == active;
                    let label = tab_label(tab);
                    let class = match (selected, tab.exited) {
                        (true, _) => "tab tab-selected",
                        (false, true) => "tab term-tab-exited",
                        (false, false) => "tab",
                    };
                    let select_core = core.clone();
                    let close_core = core.clone();
                    let close_label = format!("Close {label}");
                    rsx! {
                        span { key: "{id}", class: "tab-wrap",
                            button {
                                class,
                                r#type: "button",
                                role: "tab",
                                "aria-selected": selected.to_string(),
                                onclick: move |_| select_tab(&mut state, &mut view, &select_core, id),
                                span { "{label}" }
                                if tab.unread {
                                    span { class: "term-dot", title: "New output" }
                                }
                            }
                            button {
                                class: "tab-close",
                                r#type: "button",
                                "aria-label": "{close_label}",
                                onclick: move |_| close_tab(&mut state, &mut view, &close_core, id),
                                Icon { name: "x", size: IconSize::Sm }
                            }
                        }
                    }
                }
            }
            button {
                class: "term-add",
                r#type: "button",
                "aria-label": "New terminal",
                title: "New terminal \u{b7} \u{2318}T",
                onclick: move |_| {
                    // A sibling of the active tab, else the selected container's.
                    let spec = active.and_then(|id| add_core.spec(id));
                    match spec {
                        Some(spec) => {
                            open_tab(&mut state, &mut view, &add_core, spec);
                        }
                        None => crate::services::exec::dispatch_open(&mut state),
                    }
                },
                Icon { name: "plus", size: IconSize::Sm }
            }
        }
    }
}

// ---------------------------------------------------------------------
// The inspector's Terminal tab
// ---------------------------------------------------------------------

/// Mounted in the inspector's Terminal tab: lists the container's terminals
/// and opens a new one (starting the container first when it is stopped).
/// The terminals themselves live in the bottom panel, so a session is never
/// drawn twice.
#[component]
pub fn TerminalPanel(container: berth_core::ContainerId) -> Element {
    let mut state = use_context::<AppState>();
    let core = try_use_context::<ExecCore>();
    let view = try_use_context::<TerminalView>();
    let actions = try_use_context::<crate::services::actions::ActionsService>();
    let found = state
        .containers
        .read()
        .iter()
        .find(|c| c.id == container)
        .cloned();
    let (Some(core), Some(mut view), Some(found)) = (core, view, found) else {
        return rsx! {
            style { "{TERMINAL_CSS}" }
            div { class: "term-launch",
                p { class: "term-launch-note", "Terminals are not available for this container." }
            }
        };
    };
    let reason = terminal_disabled_reason(found.state);
    let stopped = matches!(
        found.state,
        berth_core::ContainerState::Created | berth_core::ContainerState::Exited
    );
    let label = if stopped {
        "Start and open terminal"
    } else {
        "Open terminal"
    };
    let mine: Vec<(Uuid, String, bool)> = state
        .bottom
        .read()
        .terminals
        .iter()
        .filter(|t| {
            matches!(
                core.spec(t.id).map(|s| s.kind),
                Some(Kind::Exec(exec)) if exec.container == container
            )
        })
        .map(|t| (t.id, tab_label(t), t.exited))
        .collect();
    let open_core = core.clone();
    let open_container = found.clone();
    rsx! {
        style { "{TERMINAL_CSS}" }
        div { class: "term-launch",
            p { class: "term-launch-note",
                "Terminals open in the bottom panel, so they stay while you look at other things."
            }
            Button {
                label,
                variant: ButtonVariant::Primary,
                icon: "terminal",
                disabled: reason.is_some(),
                disabled_reason: reason,
                onclick: move |_| {
                    let _ = open_container_terminal(
                        &mut state,
                        &mut view,
                        &open_core,
                        actions.as_ref(),
                        &open_container,
                    );
                },
            }
            if !mine.is_empty() {
                ul { class: "term-launch-list",
                    for (id , name , exited) in mine {
                        li { key: "{id}",
                            button {
                                class: if exited { "term-launch-item term-tab-exited" } else { "term-launch-item" },
                                r#type: "button",
                                onclick: {
                                    let core = core.clone();
                                    move |_| {
                                        state.bottom.write().open = true;
                                        state.bottom.write().tab = BottomTab::Terminal;
                                        select_tab(&mut state, &mut view, &core, id);
                                    }
                                },
                                Icon { name: "terminal", size: IconSize::Sm }
                                span { "{name}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
