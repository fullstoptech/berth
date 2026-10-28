//! A log view: toolbar, selection bar and a virtualized list over one [`LogHandle`] (PLAN §7.6).
//!
//! Only a window of rows is rendered (a screenful plus overscan, from `visible_range`), and
//! the list is one version-signal read per batch: the session task applies lines, the handle
//! bumps the signal once, and this component reads the window under a short lock. Follow pins
//! the scroller to the bottom from the page itself (a `MutationObserver`), so a batch never
//! waits on a round trip to Rust; while Follow is on the window is always the tail, so the
//! rows on screen never lag the pin. Everything that walks the whole ring (filter, search,
//! Copy, Save) is a command the session task runs.

use std::collections::BTreeSet;
use std::ops::Range;

use berth_logs::{Fold, Level, LogView, Query, RowModel, SearchSummary};
use dioxus::prelude::*;

use super::banner::{Banner, BannerTone};
use super::button::{Button, ButtonVariant};
use super::inline_message::{InlineMessage, InlineTone};
use super::log_line::{row_tone, LogLine, RowClick, RowOpts, RowTone};
use super::log_toolbar::{
    build_filter, search_count_label, LogToolbar, StreamChoice, TimeRange, ToolbarEvent,
    ToolbarModel,
};
use super::virtual_table::visible_range;
use crate::screens::problems::ExplainPanel;
use crate::services::assistant::AssistantService;
use crate::services::assistant_entry;
use crate::services::logs::{save_file_name, write_lines, LogCore, LogHandle, LogStatus};
use crate::services::problems::{ExplainLine, ProblemsService, World};
use crate::services::Services;
use crate::state::{AppState, Toast, ToastTone};

/// Rows rendered above and below the viewport.
pub const OVERSCAN: usize = 12;
/// Row height in px before text scaling: comfortable and compact density.
pub const ROW_PX: u32 = 22;
pub const ROW_PX_COMPACT: u32 = 18;
/// Viewport height assumed until the scroller reports its own.
const DEFAULT_VIEWPORT_PX: f64 = 640.0;

/// A closed range of sequence numbers; every visible row whose `seq` falls in it is selected.
pub type Span = (u64, u64);

/// Where the view starts: the toggles, the filters and the search text.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewPrefs {
    pub wrap: bool,
    pub time: bool,
    pub raw: bool,
    pub regex: bool,
    pub case_sensitive: bool,
    pub query: String,
    pub levels: BTreeSet<Level>,
    pub stream: StreamChoice,
    pub range: TimeRange,
    pub selection: Vec<Span>,
    pub focus: Option<u64>,
    /// What Explain showed (a scene or a test starts there): `Some(None)` while the rules run.
    pub explained: Option<Option<Vec<berth_core::Problem>>>,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            wrap: false,
            time: true,
            raw: false,
            regex: false,
            case_sensitive: false,
            query: String::new(),
            levels: BTreeSet::new(),
            stream: StreamChoice::All,
            range: TimeRange::All,
            selection: Vec::new(),
            focus: None,
            explained: None,
        }
    }
}

// ---------------------------------------------------------------------
// Keys: F8 reaches the view through one global signal
// ---------------------------------------------------------------------

/// `(presses so far, forward?)`: bumped by `keys::dispatch` on F8 and Shift+F8.
static ERROR_JUMP: GlobalSignal<(u64, bool)> = Signal::global(|| (0, true));
/// The id of the view F8 acts on: the one last mounted or clicked.
static ACTIVE_VIEW: GlobalSignal<u64> = Signal::global(|| 0);

/// F8 (`forward`) or Shift+F8 pressed: the active log view jumps to the next or previous error.
pub fn request_error_jump(forward: bool) {
    let mut jump = ERROR_JUMP.write();
    jump.0 += 1;
    jump.1 = forward;
}

/// The log-view part of `keys::dispatch`.
pub fn dispatch(action: &str) {
    match action {
        "logs.next_error" => request_error_jump(true),
        "logs.prev_error" => request_error_jump(false),
        _ => {}
    }
}

// ---------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------

/// Row height in px for the density and text scale.
#[must_use]
pub fn row_px(compact: bool, scale: f32) -> u32 {
    crate::text_scale::scaled_px(if compact { ROW_PX_COMPACT } else { ROW_PX }, scale)
}

/// Is this row inside one of the selected spans?
#[must_use]
pub fn is_selected(spans: &[Span], seq: u64) -> bool {
    spans.iter().any(|&(lo, hi)| lo <= seq && seq <= hi)
}

/// The selection after a click: a plain click selects that row; Shift extends from the anchor;
/// ⌘ or ⌃ adds or removes one row (splitting a range when the row is inside one).
#[must_use]
pub fn click_selection(
    spans: &[Span],
    anchor: Option<u64>,
    click: RowClick,
) -> (Vec<Span>, Option<u64>) {
    let seq = click.seq;
    if click.shift {
        let from = anchor.unwrap_or(seq);
        return (vec![(from.min(seq), from.max(seq))], Some(from));
    }
    if click.toggle {
        let mut next = Vec::with_capacity(spans.len() + 1);
        let mut removed = false;
        for &(lo, hi) in spans {
            if lo <= seq && seq <= hi {
                removed = true;
                if lo < seq {
                    next.push((lo, seq - 1));
                }
                if seq < hi {
                    next.push((seq + 1, hi));
                }
            } else {
                next.push((lo, hi));
            }
        }
        if !removed {
            next.push((seq, seq));
        }
        return (next, Some(seq));
    }
    (vec![(seq, seq)], Some(seq))
}

/// The index of the first visible row whose `seq` is at least `seq` (the number of rows before
/// it), by binary search over `LogView::rows`.
#[must_use]
pub fn lower_bound(view: &LogView, seq: u64) -> usize {
    let (mut lo, mut hi) = (0, view.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        match view.rows(mid..mid + 1).first() {
            Some(row) if row.seq < seq => lo = mid + 1,
            _ => hi = mid,
        }
    }
    lo
}

/// Rows covered by the spans, counted over what is visible now.
#[must_use]
pub fn selected_rows(view: &LogView, spans: &[Span]) -> usize {
    spans
        .iter()
        .map(|&(lo, hi)| {
            lower_bound(view, hi.saturating_add(1)).saturating_sub(lower_bound(view, lo))
        })
        .sum()
}

/// The text of the selected rows, spans in order, one line per row (a closed trace is written
/// out in full, as `LogView::text` does).
#[must_use]
pub fn selected_text(view: &LogView, spans: &[Span], timestamps: bool) -> String {
    spans
        .iter()
        .map(|&(lo, hi)| {
            let range = lower_bound(view, lo)..lower_bound(view, hi.saturating_add(1));
            view.text(Some(range), timestamps)
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The selected rows as the rules read them: the container's name as the view shows it, and the
/// line. A trace's folded members are not listed; the head line carries the error.
#[must_use]
pub fn selected_lines(view: &LogView, spans: &[Span]) -> Vec<ExplainLine> {
    spans
        .iter()
        .flat_map(|&(lo, hi)| {
            let range = lower_bound(view, lo)..lower_bound(view, hi.saturating_add(1));
            view.rows(range)
        })
        .map(|row| ExplainLine {
            container: row.container,
            text: row.text,
        })
        .collect()
}

/// What the rules say about the selected rows of a view: the rows are read inside the session
/// task, the rules run off the UI thread.
pub async fn explain_selection(
    core: std::sync::Arc<LogCore>,
    spans: Vec<Span>,
    service: ProblemsService,
    world: World,
    services: Option<Services>,
) -> Vec<berth_core::Problem> {
    let lines = core
        .run(move |view| selected_lines(view, &spans))
        .await
        .unwrap_or_default();
    service.explain(world, services.as_ref(), lines).await
}

/// The tone of each row in a window, and whether it is an open trace head. A trace member takes
/// the tone of its head (the head is just above it in the window; a member whose head is above
/// the window reads plain, which is off screen).
#[must_use]
pub fn window_tones(rows: &[RowModel]) -> Vec<(RowTone, bool)> {
    let mut head: Option<(u64, RowTone)> = None;
    rows.iter()
        .enumerate()
        .map(|(index, row)| match row.fold {
            Fold::Member { head: head_seq } => {
                let tone = head
                    .filter(|(seq, _)| *seq == head_seq)
                    .map_or(RowTone::Plain, |(_, tone)| tone);
                (tone, false)
            }
            _ => {
                let tone = row_tone(row);
                head = Some((row.seq, tone));
                let expanded = matches!(row.fold, Fold::Head { .. })
                    && rows.get(index + 1).is_some_and(
                        |next| matches!(next.fold, Fold::Member { head } if head == row.seq),
                    );
                (tone, expanded)
            }
        })
        .collect()
}

/// The `grid-template-columns` of a row for the columns that are on.
#[must_use]
pub fn grid_columns(time: bool, container: bool) -> String {
    let mut cols = vec!["calc(28px * var(--text-scale, 1))".to_string()];
    if time {
        cols.push("calc(108px * var(--text-scale, 1))".to_string());
    }
    if container {
        cols.push("calc(180px * var(--text-scale, 1))".to_string());
    }
    cols.push("calc(52px * var(--text-scale, 1))".to_string());
    cols.push("minmax(0, 1fr)".to_string());
    cols.join(" ")
}

/// The scroll element's DOM id.
#[must_use]
pub fn scroller_id(core_id: u64) -> String {
    format!("log-scroll-{core_id}")
}

/// What the page runs once per scroller: while `data-follow="1"` every change inside it pins it
/// to the bottom before the next paint, and a scroll upwards (the user's) ends Follow at once and
/// tells Rust.
#[must_use]
pub fn follow_script(core_id: u64) -> String {
    format!(
        r#"(() => {{
  const el = document.getElementById("{id}");
  if (!el) return;
  let top = el.scrollTop, height = el.scrollHeight;
  const pin = () => {{ if (el.dataset.follow === "1") el.scrollTop = el.scrollHeight; }};
  new MutationObserver(pin).observe(el, {{ childList: true, subtree: true, attributes: true, attributeFilter: ["style", "data-follow"] }});
  el.addEventListener("scroll", () => {{
    const grew = el.scrollHeight >= height;
    const up = grew && el.scrollTop < top - 2;
    top = el.scrollTop; height = el.scrollHeight;
    if (up && el.dataset.follow === "1") {{ el.dataset.follow = "0"; dioxus.send("unfollow"); }}
  }}, {{ passive: true }});
  pin();
}})();"#,
        id = scroller_id(core_id)
    )
}

fn scroll_to_script(core_id: u64, top: f64) -> String {
    format!(
        "const el = document.getElementById(\"{}\"); if (el) el.scrollTop = {top};",
        scroller_id(core_id)
    )
}

fn copy_text(text: &str) {
    let quoted = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    let script = format!("navigator.clipboard.writeText({quoted});");
    spawn(async move {
        let _ = document::eval(&script).await;
    });
}

fn toast(mut state: AppState, text: String, tone: ToastTone) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone,
        action: None,
    });
}

/// "1,234": thousands grouped, for the line counts in toasts.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What one render of the view reads, taken under one short lock.
struct Snapshot {
    len: usize,
    range: Range<usize>,
    rows: Vec<RowModel>,
    counts: berth_logs::LevelCounts,
    summary: SearchSummary,
    follow: bool,
    dropped: u64,
    selected_lines: usize,
}

fn snapshot(
    view: &LogView,
    row_h: u32,
    viewport: f64,
    scroll_top: f64,
    selection: &[Span],
) -> Snapshot {
    let len = view.len();
    let follow = view.follow();
    let top = if follow {
        // Following: the window is the tail, whatever the scroll signal last said.
        (len as f64 * f64::from(row_h) - viewport).max(0.0)
    } else {
        scroll_top
    };
    let range = visible_range(top, viewport, row_h, len, OVERSCAN);
    Snapshot {
        len,
        rows: view.rows(range.clone()),
        range,
        counts: view.counts().clone(),
        summary: view.search_summary(),
        follow,
        dropped: view.dropped(),
        selected_lines: selected_rows(view, selection),
    }
}

// ---------------------------------------------------------------------
// Commands the view sends to the session task
// ---------------------------------------------------------------------

/// What F8 / search stepping found: the row to show and its sequence number.
type Found = Option<(usize, u64)>;

fn found_at(view: &LogView, row: Option<usize>) -> Found {
    let row = row?;
    view.rows(row..row + 1).first().map(|r| (row, r.seq))
}

/// Starts (or, with empty text, ends) the search. Typing fast queues searches; the core runs
/// only the newest.
async fn run_search(core: std::sync::Arc<LogCore>, text: String, regex: bool, case: bool) {
    let query = (!text.is_empty()).then_some(Query {
        text,
        regex,
        case_sensitive: case,
    });
    let _ = core.set_search(query).await;
}

// ---------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------

/// A log view over one session. `merged` shows the container column (more than one container).
#[component]
pub fn LogViewer(
    handle: LogHandle,
    view_name: String,
    #[props(default)] merged: bool,
    #[props(default)] prefs: ViewPrefs,
) -> Element {
    let mut state = use_context::<AppState>();
    let core_id = handle.core.id();
    let services = try_consume_context::<Services>();
    let assistant = try_consume_context::<Option<AssistantService>>().flatten();
    let mut ask_refusal = use_signal(|| None::<String>);
    let problems_service = use_hook(|| {
        try_consume_context::<ProblemsService>().unwrap_or_else(ProblemsService::inert)
    });
    // What Explain found for the selected lines: `Some(None)` while the rules run.
    let mut explained = use_signal(|| prefs.explained.clone());

    let mut wrap = use_signal(|| prefs.wrap);
    let mut time = use_signal(|| prefs.time);
    let mut raw = use_signal(|| prefs.raw);
    let mut regex = use_signal(|| prefs.regex);
    let mut case = use_signal(|| prefs.case_sensitive);
    let mut query = use_signal(|| prefs.query.clone());
    let mut epoch = use_signal(|| 0_u32);
    let mut levels = use_signal(|| prefs.levels.clone());
    let mut stream = use_signal(|| prefs.stream);
    let mut range = use_signal(|| prefs.range);
    let mut selection = use_signal(|| prefs.selection.clone());
    let mut anchor = use_signal(|| None::<u64>);
    let mut focus = use_signal(|| prefs.focus);
    let mut json_open = use_signal(BTreeSet::<u64>::new);
    let mut scroll_top = use_signal(|| 0.0_f64);
    let mut viewport = use_signal(|| DEFAULT_VIEWPORT_PX);
    let mut menu_open = use_signal(|| false);

    // F8 acts on the view that was last mounted or clicked.
    use_effect(move || {
        *ACTIVE_VIEW.write() = core_id;
    });
    let seen_jump = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(ERROR_JUMP.peek().0)));
    let jump_handle = handle.clone();
    use_effect(move || {
        let (count, forward) = *ERROR_JUMP.read();
        if count == seen_jump.get() {
            return;
        }
        seen_jump.set(count);
        if *ACTIVE_VIEW.peek() != core_id {
            return;
        }
        let handle = jump_handle.clone();
        spawn(async move {
            jump_error(
                handle,
                forward,
                focus,
                scroll_top,
                viewport,
                state_row_px(state),
            )
            .await;
        });
    });

    let (compact, scale) = {
        let settings = state.settings.read();
        (
            settings.density == berth_store::Density::Compact,
            settings.text_scale,
        )
    };
    let row_h = row_px(compact, scale);
    let selection_now = selection.read().clone();
    let snap = handle.read(|view| {
        snapshot(
            view,
            row_h,
            *viewport.read(),
            *scroll_top.read(),
            &selection_now,
        )
    });
    let status = handle.core.status();
    let ask_harness = assistant_entry::first_available(&state.harnesses.read());
    let ask_label = ask_harness.as_ref().map_or_else(
        || "Ask a harness".to_string(),
        |h| format!("Ask {}", h.name),
    );
    // The project is only known once the selected lines are read; the
    // click reports it when none of them has a folder.
    let ask_disabled = if assistant.is_none() {
        Some("The assistant is not connected".to_string())
    } else if ask_harness.is_none() {
        Some("Needs a harness: install Claude Code or muse".to_string())
    } else {
        None
    };
    let search_error = handle.core.search_error();
    let new_lines = handle.core.new_lines();

    // ---- toolbar events ----
    let toolbar_handle = handle.clone();
    let save_name = view_name.clone();
    let on_toolbar = use_callback(move |event: ToolbarEvent| {
        let handle = toolbar_handle.clone();
        let core = std::sync::Arc::clone(&handle.core);
        match event {
            ToolbarEvent::Follow => {
                let on = !core.with_view(LogView::follow).unwrap_or(false);
                spawn(async move {
                    let _ = core.set_follow(on).await;
                });
            }
            ToolbarEvent::JumpToLatest => {
                spawn(async move {
                    let _ = core.set_follow(true).await;
                });
            }
            ToolbarEvent::Wrap => wrap.set(!wrap()),
            ToolbarEvent::Time => time.set(!time()),
            ToolbarEvent::Raw => raw.set(!raw()),
            ToolbarEvent::Search(text) => {
                query.set(text.clone());
                spawn(run_search(core, text, regex(), case()));
            }
            ToolbarEvent::ClearSearch => {
                query.set(String::new());
                epoch.set(epoch() + 1);
                spawn(run_search(core, String::new(), regex(), case()));
            }
            ToolbarEvent::NextMatch | ToolbarEvent::PrevMatch => {
                let forward = event == ToolbarEvent::NextMatch;
                spawn(async move {
                    let found = core
                        .run(move |view| {
                            let row = if forward {
                                view.next_match()
                            } else {
                                view.prev_match()
                            };
                            found_at(view, row)
                        })
                        .await
                        .flatten();
                    show_found(
                        &handle,
                        found,
                        focus,
                        scroll_top,
                        viewport,
                        state_row_px(state),
                    )
                    .await;
                });
            }
            ToolbarEvent::Level(level) => {
                let mut set = levels.read().clone();
                if !set.remove(&level) {
                    set.insert(level);
                }
                levels.set(set.clone());
                spawn(apply_filter(core, set, stream(), range()));
            }
            ToolbarEvent::Stream(choice) => {
                stream.set(choice);
                spawn(apply_filter(core, levels.read().clone(), choice, range()));
            }
            ToolbarEvent::Range(choice) => {
                range.set(choice);
                spawn(apply_filter(core, levels.read().clone(), stream(), choice));
            }
            ToolbarEvent::NextError | ToolbarEvent::PrevError => {
                let forward = event == ToolbarEvent::NextError;
                spawn(jump_error(
                    handle,
                    forward,
                    focus,
                    scroll_top,
                    viewport,
                    state_row_px(state),
                ));
            }
            ToolbarEvent::ToggleMenu => menu_open.set(!menu_open()),
            ToolbarEvent::CloseMenu => menu_open.set(false),
            ToolbarEvent::Menu(id) => {
                menu_open.set(false);
                match id.as_str() {
                    "save" => {
                        let name = save_name.clone();
                        spawn(save_to_file(core, name, time(), state));
                    }
                    "copy-all" => {
                        let timestamps = time();
                        spawn(async move {
                            if let Some(text) = core.run(move |v| v.text(None, timestamps)).await {
                                copy_text(&text);
                            }
                        });
                    }
                    "clear" => {
                        selection.set(Vec::new());
                        focus.set(None);
                        spawn(async move {
                            let _ = core.run(LogView::clear).await;
                        });
                    }
                    "regex" => {
                        regex.set(!regex());
                        spawn(run_search(core, query(), regex(), case()));
                    }
                    "case" => {
                        case.set(!case());
                        spawn(run_search(core, query(), regex(), case()));
                    }
                    _ => {}
                }
            }
        }
    });

    // ---- row events ----
    let on_select = use_callback(move |click: RowClick| {
        let (next, new_anchor) = click_selection(&selection.peek(), *anchor.peek(), click);
        selection.set(next);
        anchor.set(new_anchor);
        focus.set(Some(click.seq));
    });
    let fold_core = std::sync::Arc::clone(&handle.core);
    let on_fold = use_callback(move |seq: u64| {
        let core = std::sync::Arc::clone(&fold_core);
        spawn(async move {
            let _ = core.run(move |view| view.toggle_fold(seq)).await;
        });
    });
    let on_json = use_callback(move |seq: u64| {
        let mut open = json_open.write();
        if !open.remove(&seq) {
            open.insert(seq);
        }
    });

    // ---- selection bar events ----
    let copy_handle = handle.clone();
    let copy_selection = move |_| {
        spawn(copy_selected(
            std::sync::Arc::clone(&copy_handle.core),
            selection.peek().clone(),
            time(),
        ));
    };

    // Ask about these lines (P7-T6): the selection becomes log chips on
    // the project's assistant session; what Explain found rides along.
    let ask_core = std::sync::Arc::clone(&handle.core);
    let on_ask = {
        let assistant = assistant.clone();
        let harness = ask_harness.clone();
        move |_| {
            let (Some(service), Some(harness)) = (assistant.clone(), harness.clone()) else {
                return;
            };
            let core = std::sync::Arc::clone(&ask_core);
            let spans = selection.peek().clone();
            let containers = state.containers.peek().clone();
            let projects = state.projects.peek().clone();
            let rules_say: Vec<String> = explained
                .peek()
                .clone()
                .flatten()
                .unwrap_or_default()
                .iter()
                .map(|p| p.title.clone())
                .collect();
            let model = assistant_entry::saved_model(&state.settings.peek(), &harness);
            ask_refusal.set(None);
            spawn(async move {
                let found = core
                    .run(move |view| selected_lines(view, &spans))
                    .await
                    .unwrap_or_default();
                let lines: Vec<(berth_core::ContainerId, String)> = found
                    .into_iter()
                    .map(|l| (berth_core::ContainerId(l.container), l.text))
                    .collect();
                let project = lines.iter().find_map(|(id, _)| {
                    assistant_entry::project_of_container(&containers, &projects, id)
                        .filter(|p| p.folder.is_some())
                        .cloned()
                });
                let Some(project) = project else {
                    ask_refusal.set(Some(
                        "Ask needs a project with a folder on this Mac".to_string(),
                    ));
                    return;
                };
                let Some(mut entry) = assistant_entry::logs_entry(
                    &project,
                    &harness,
                    &lines,
                    &containers,
                    &rules_say,
                ) else {
                    ask_refusal.set(Some("Select a line first".to_string()));
                    return;
                };
                entry.request.model = model;
                match assistant_entry::run(&service, entry).await {
                    Ok(_) => assistant_entry::show(&mut state, &project.id),
                    Err(reason) => ask_refusal.set(Some(reason)),
                }
            });
        }
    };

    let explain_core = std::sync::Arc::clone(&handle.core);
    let on_explain = move |_| {
        let core = std::sync::Arc::clone(&explain_core);
        let spans = selection.peek().clone();
        let service = problems_service.clone();
        let services = services.clone();
        explained.set(Some(None));
        spawn(async move {
            let found = explain_selection(core, spans, service, World::of(&state), services).await;
            explained.set(Some(Some(found)));
        });
    };

    let Some(snap) = snap else {
        return rsx! {
            div { class: "log-viewer", role: "region", "aria-label": "Logs",
                div { class: "log-empty", "Waiting for the first line\u{2026}" }
            }
        };
    };

    let toolbar = ToolbarModel {
        follow: snap.follow,
        wrap: wrap(),
        time: time(),
        raw: raw(),
        regex: regex(),
        case_sensitive: case(),
        levels: levels.read().clone(),
        counts: snap.counts.clone(),
        stream: stream(),
        range: range(),
        query: query.read().clone(),
        epoch: epoch(),
        has_query: !query.read().is_empty(),
        search_label: search_count_label(&snap.summary),
        new_lines,
        menu_open: menu_open(),
    };
    let tones = window_tones(&snap.rows);
    let top_px = snap.range.start as u64 * u64::from(row_h);
    let bottom_px = snap.len.saturating_sub(snap.range.end) as u64 * u64::from(row_h);
    let selection_for_rows = selection.read().clone();
    let json_open_now = json_open.read().clone();
    let focus_now = *focus.read();
    let columns = grid_columns(time(), merged);
    let scroller = scroller_id(core_id);
    let style = format!("--log-row: {row_h}px; --log-cols: {columns};");
    let filtered =
        !levels.read().is_empty() || stream() != StreamChoice::All || range() != TimeRange::All;
    let empty_text = if snap.len > 0 {
        None
    } else if status == LogStatus::Connecting {
        Some("Waiting for the first line\u{2026}")
    } else if filtered {
        Some("No lines match the filters")
    } else if status == LogStatus::Ended {
        Some("Nothing was written")
    } else {
        Some("No lines yet")
    };
    let wrap_class = if wrap() { " log-wrap" } else { "" };
    let selected_count = snap.selected_lines;
    let selection_open = selected_count > 0;
    let dropped = snap.dropped;
    let follow_attr = if snap.follow { "1" } else { "0" };

    rsx! {
        div {
            class: "log-viewer",
            role: "region",
            "aria-label": "Logs",
            onmousedown: move |_| {
                *ACTIVE_VIEW.write() = core_id;
            },
            onkeydown: {
                let handle = handle.clone();
                move |e| {
                    use dioxus::html::ModifiersInteraction;
                    let mods = e.data.modifiers();
                    let command = mods.contains(dioxus::html::Modifiers::META)
                        || mods.contains(dioxus::html::Modifiers::CONTROL);
                    match e.key() {
                        Key::Escape if !selection.peek().is_empty() => {
                            selection.set(Vec::new());
                            e.stop_propagation();
                        }
                        Key::Character(c)
                            if command
                                && c.eq_ignore_ascii_case("c")
                                && !selection.peek().is_empty() =>
                        {
                            e.prevent_default();
                            spawn(copy_selected(
                                std::sync::Arc::clone(&handle.core),
                                selection.peek().clone(),
                                time(),
                            ));
                        }
                        _ => {}
                    }
                }
            },
            LogToolbar { model: toolbar, onevent: on_toolbar }
            if selection_open {
                div { class: "log-selbar", role: "group", "aria-label": "Selected lines",
                    span { class: "log-selcount",
                        if selected_count == 1 {
                            "1 line selected"
                        } else {
                            "{grouped(selected_count)} lines selected"
                        }
                    }
                    span { class: "log-divider", "aria-hidden": "true" }
                    Button {
                        label: "Explain".to_string(),
                        variant: ButtonVariant::Ghost,
                        icon: "sparkles",
                        onclick: on_explain,
                    }
                    Button {
                        label: ask_label.clone(),
                        variant: ButtonVariant::Secondary,
                        icon: "sparkles",
                        disabled: ask_disabled.is_some(),
                        disabled_reason: ask_disabled.clone().unwrap_or_default(),
                        onclick: on_ask,
                    }
                    Button {
                        label: "Copy".to_string(),
                        variant: ButtonVariant::Ghost,
                        icon: "copy",
                        onclick: copy_selection,
                    }
                    Button {
                        label: "Clear".to_string(),
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| selection.set(Vec::new()),
                    }
                }
            }
            if let Some(reason) = ask_refusal() {
                InlineMessage { message: reason, tone: InlineTone::Warning }
            }
            if let Some(result) = explained.read().clone() {
                ExplainPanel { problems: result, onclose: move |()| explained.set(None) }
            }
            if let LogStatus::Failed(message) = &status {
                InlineMessage { message: message.clone(), tone: InlineTone::Warning }
            }
            if let Some(message) = &search_error {
                InlineMessage { message: format!("Invalid pattern: {message}"), tone: InlineTone::Danger }
            }
            if dropped > 0 {
                div { class: "log-dropped", role: "status", "data-dropped": "{dropped}",
                    Banner {
                        message: "Older lines dropped; Save to keep all".to_string(),
                        tone: BannerTone::Info,
                    }
                }
            }
            div {
                class: "log-list{wrap_class}",
                id: "{scroller}",
                style,
                tabindex: "0",
                role: "table", "aria-label": "Log lines",
                "aria-rowcount": "{snap.len}",
                "data-follow": follow_attr,
                onmounted: {
                    let handle = handle.clone();
                    move |_| {
                        let core = std::sync::Arc::clone(&handle.core);
                        spawn(async move {
                            let mut eval = document::eval(&follow_script(core_id));
                            while let Ok(message) = eval.recv::<String>().await {
                                if message == "unfollow" {
                                    let _ = core.set_follow(false).await;
                                }
                            }
                        });
                    }
                },
                onscroll: move |e| {
                    // Only a real change writes: the pin scrolls the page itself, and that must
                    // not echo back as a re-render storm.
                    let top = e.data.scroll_top();
                    if (*scroll_top.peek() - top).abs() >= 1.0 {
                        scroll_top.set(top);
                    }
                    let height = f64::from(e.data.client_height());
                    if height > 0.0 && (*viewport.peek() - height).abs() >= 1.0 {
                        viewport.set(height);
                    }
                },
                div { class: "log-spacer-top", style: "height: {top_px}px;" }
                for (index, row) in snap.rows.iter().enumerate() {
                    LogLine {
                        key: "{row.seq}",
                        row: row.clone(),
                        opts: RowOpts {
                            tone: tones[index].0,
                            time: time(),
                            seconds_only: false,
                            container: merged,
                            raw: raw(),
                            wrap: wrap(),
                            selected: is_selected(&selection_for_rows, row.seq),
                            focused: focus_now == Some(row.seq),
                            json_open: json_open_now.contains(&row.seq),
                            expanded: tones[index].1,
                        },
                        onselect: on_select,
                        ontoggle_fold: on_fold,
                        ontoggle_json: on_json,
                    }
                }
                div { class: "log-spacer-bottom", style: "height: {bottom_px}px;" }
                if let Some(text) = empty_text {
                    div { class: "log-empty", "{text}" }
                }
            }
        }
    }
}

fn state_row_px(state: AppState) -> u32 {
    let settings = state.settings.peek();
    row_px(
        settings.density == berth_store::Density::Compact,
        settings.text_scale,
    )
}

/// Copies the selected rows to the clipboard (the text is made in the session task).
async fn copy_selected(core: std::sync::Arc<LogCore>, spans: Vec<Span>, timestamps: bool) {
    let text = core
        .run(move |view| selected_text(view, &spans, timestamps))
        .await
        .unwrap_or_default();
    if !text.is_empty() {
        copy_text(&text);
    }
}

async fn apply_filter(
    core: std::sync::Arc<LogCore>,
    levels: BTreeSet<Level>,
    stream: StreamChoice,
    range: TimeRange,
) {
    let filter = build_filter(&levels, stream, range, chrono::Utc::now());
    let _ = core.run(move |view| view.set_filter(filter)).await;
}

/// Shows a row F8 or search found: Follow ends, the row is centred and marked.
async fn show_found(
    handle: &LogHandle,
    found: Found,
    mut focus: Signal<Option<u64>>,
    mut scroll_top: Signal<f64>,
    viewport: Signal<f64>,
    row_h: u32,
) {
    let Some((row, seq)) = found else {
        return;
    };
    let _ = handle.core.set_follow(false).await;
    focus.set(Some(seq));
    let top = (row as f64 * f64::from(row_h) - *viewport.peek() / 2.0).max(0.0);
    scroll_top.set(top);
    let script = scroll_to_script(handle.core.id(), top);
    let _ = document::eval(&script).await;
}

async fn jump_error(
    handle: LogHandle,
    forward: bool,
    focus: Signal<Option<u64>>,
    scroll_top: Signal<f64>,
    viewport: Signal<f64>,
    row_h: u32,
) {
    let from_seq = *focus.peek();
    let top = *scroll_top.peek();
    let found = handle
        .core
        .run(move |view| error_target(view, forward, from_seq, top, row_h))
        .await
        .flatten();
    show_found(&handle, found, focus, scroll_top, viewport, row_h).await;
}

/// F8 (`forward`) or Shift+F8: the error row after (or before) the row last jumped to; when
/// there is none yet, after the first row on screen (`scroll_top` px down). Wraps at the ends.
#[must_use]
pub fn error_target(
    view: &LogView,
    forward: bool,
    from_seq: Option<u64>,
    scroll_top: f64,
    row_h: u32,
) -> Found {
    let current = from_seq.map(|seq| lower_bound(view, seq));
    let row = if forward {
        let after = current.unwrap_or_else(|| {
            let first_visible = (scroll_top / f64::from(row_h.max(1))).floor();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let first_visible = first_visible.max(0.0) as usize;
            // Row 0 itself may be the error: ask from before it.
            first_visible.checked_sub(1).unwrap_or(usize::MAX)
        });
        view.next_error(after)
    } else {
        view.prev_error(current.unwrap_or(view.len()))
    };
    found_at(view, row)
}

/// Save: the save panel, then every line of the view to the chosen file.
async fn save_to_file(
    core: std::sync::Arc<LogCore>,
    name: String,
    timestamps: bool,
    state: AppState,
) {
    let suggested = save_file_name(&name, chrono::Utc::now());
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_file_name(suggested)
        .save_file()
        .await
    else {
        return;
    };
    let path = file.path().to_path_buf();
    let Some(text) = core.run(move |view| view.text(None, timestamps)).await else {
        return;
    };
    match write_lines(&path, &text).await {
        Ok(lines) => toast(
            state,
            format!(
                "Saved {} lines to {}",
                grouped(lines),
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned()
                )
            ),
            ToastTone::Success,
        ),
        Err(err) => {
            tracing::warn!(%err, "saving a log failed");
            toast(
                state,
                format!("Could not save the log: {err}"),
                ToastTone::Danger,
            );
        }
    }
}

#[cfg(test)]
#[path = "log_view_tests.rs"]
mod tests;
