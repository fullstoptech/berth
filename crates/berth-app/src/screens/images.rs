//! Images screen, tab "On this engine" (P4-T1; PLAN §6.11, brief §7.4). No
//! frame draws this tab: the header and tabs follow "Images / find / 1440",
//! the table follows Containers. One row per image of the engines in scope
//! (repo:tag, ID, size, arch, created, used by), a dangling filter, Run and
//! Delete on each row, and Pull image with per-layer progress.
//!
//! The view model below is pure (`build_rows`, `list_rows`, `delete_reason`,
//! `pull_target`): the screen reads signals and the [`ImageStore`], and its
//! buttons go through the store (D23). Delete shows its reason and waits for
//! the confirm model that PLAN §6 has no `EngineAction` for yet.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use berth_core::{Arch, ContainerId, Engine, EngineId, EngineState, Image};

use crate::services::images::{
    ImageFilter, ImageStore, ImagesTab, PullPhase, PullState, Sort, SortColumn, SortDirection,
};
use crate::services::Services;
use crate::state::{AppState, EngineScope, Overlay, Selection};
use crate::ui::progress_bar::{ProgressBar, PROGRESS_BAR_CSS};
use crate::ui::{
    scope_includes, Badge, BadgeKind, Button, ButtonVariant, CellAlign, ControlSize, EmptyState,
    IconButton, IconButtonTone, InlineMessage, InlineTone, SearchField, SegmentedControl,
    SegmentedItem, Select, SelectOption, SkeletonRows, TabItem, TableHeaderCell, Tabs, TextField,
};

/// The screen's CSS: the hand-written rules, the generated column tiers and
/// the progress bar. Colours come from `tokens.css` only.
pub const IMAGES_CSS: &str = concat!(
    include_str!("../../assets/css/images.css"),
    include_str!("../../assets/css/images_tiers.css"),
);

/// The `docker pull --platform` choices; "" is the engine's own.
pub const PLATFORMS: [(&str, &str); 3] = [
    ("", "Engine default"),
    ("linux/arm64", "linux/arm64"),
    ("linux/amd64", "linux/amd64"),
];

// ---------------------------------------------------------------------
// View model (pure)
// ---------------------------------------------------------------------

/// One image of one engine, as the table draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRow {
    pub engine: EngineId,
    pub id: String,
    /// The first twelve hex digits, without `sha256:`.
    pub short_id: String,
    /// The first tag, or empty for a dangling image.
    pub name: String,
    pub tags: Vec<String>,
    pub dangling: bool,
    pub size: bytesize::ByteSize,
    /// "arm64", "amd64", or "—" when the engine did not say.
    pub arch: String,
    /// The image is not the engine's own architecture: it runs emulated.
    pub foreign_arch: bool,
    pub created: DateTime<Utc>,
    pub used_by: Vec<ContainerId>,
}

/// The first twelve hex digits of an image id.
#[must_use]
pub fn short_id(id: &str) -> String {
    id.trim_start_matches("sha256:").chars().take(12).collect()
}

impl ImageRow {
    #[must_use]
    pub fn new(engine: &EngineId, image: &Image, engine_arch: &Arch) -> Self {
        let known =
            !matches!(&image.arch, Arch::Other(name) if name == "unknown" || name.is_empty());
        Self {
            engine: engine.clone(),
            id: image.id.clone(),
            short_id: short_id(&image.id),
            name: image.repo_tags.first().cloned().unwrap_or_default(),
            tags: image.repo_tags.clone(),
            dangling: image.dangling || image.repo_tags.is_empty(),
            size: image.size,
            arch: if known {
                image.arch.docker_name().to_string()
            } else {
                "\u{2014}".to_string()
            },
            foreign_arch: known && image.arch != *engine_arch,
            created: image.created,
            used_by: image.used_by.clone(),
        }
    }

    /// The text the filter field matches: every tag and the short id.
    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.short_id.to_lowercase().contains(&query)
            || self.tags.iter().any(|t| t.to_lowercase().contains(&query))
    }
}

/// Every image of the engines the scope includes, engine by engine. An
/// engine that is known and not running is left out (its list is stale).
#[must_use]
pub fn build_rows(
    engines: &[Engine],
    by_engine: &BTreeMap<EngineId, Vec<Image>>,
    scope: &EngineScope,
) -> Vec<ImageRow> {
    let mut rows = Vec::new();
    for (engine_id, images) in by_engine {
        if !scope_includes(scope, engine_id) {
            continue;
        }
        let engine = engines.iter().find(|e| e.id == *engine_id);
        if engine.is_some_and(|e| e.state != EngineState::Running) {
            continue;
        }
        let arch = engine
            .and_then(|e| e.resources.as_ref())
            .map_or(Arch::Aarch64, |r| r.arch.clone());
        rows.extend(images.iter().map(|i| ImageRow::new(engine_id, i, &arch)));
    }
    rows
}

/// The rows the table shows: the filter text, the dangling filter, then the
/// sort. Ties keep their order, so the engine's own listing holds.
#[must_use]
pub fn list_rows(rows: &[ImageRow], query: &str, filter: ImageFilter, sort: Sort) -> Vec<ImageRow> {
    let mut shown: Vec<ImageRow> = rows
        .iter()
        .filter(|r| r.matches(query))
        .filter(|r| filter == ImageFilter::All || r.dangling)
        .cloned()
        .collect();
    let (column, direction) = sort;
    shown.sort_by(|a, b| {
        // An untagged image has no name: it sorts last either way.
        if column == SortColumn::Name && a.name.is_empty() != b.name.is_empty() {
            return if a.name.is_empty() {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            };
        }
        let order = match column {
            SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortColumn::Size => a.size.cmp(&b.size),
            SortColumn::Created => a.created.cmp(&b.created),
        };
        if direction == SortDirection::Descending {
            order.reverse()
        } else {
            order
        }
    });
    shown
}

/// How a column header's click changes the sort: the same column flips its
/// direction, another starts at its natural one (names up, size and age down).
#[must_use]
pub fn next_sort(current: Sort, clicked: SortColumn) -> Sort {
    if current.0 == clicked {
        let flipped = match current.1 {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        };
        return (clicked, flipped);
    }
    match clicked {
        SortColumn::Name => (clicked, SortDirection::Ascending),
        SortColumn::Size | SortColumn::Created => (clicked, SortDirection::Descending),
    }
}

fn header_direction(sort: Sort, column: SortColumn) -> crate::ui::SortDirection {
    if sort.0 != column {
        return crate::ui::SortDirection::None;
    }
    match sort.1 {
        SortDirection::Ascending => crate::ui::SortDirection::Asc,
        SortDirection::Descending => crate::ui::SortDirection::Desc,
    }
}

/// "—" when nothing uses the image, the container's name when one does,
/// "3 containers" otherwise.
#[must_use]
pub fn used_by_text(used_by: &[ContainerId], containers: &[berth_core::Container]) -> String {
    match used_by {
        [] => "\u{2014}".to_string(),
        [only] => containers
            .iter()
            .find(|c| c.id == *only)
            .map_or_else(|| "1 container".to_string(), |c| c.name.clone()),
        many => format!("{} containers", many.len()),
    }
}

/// Why Delete is off for this row, or `None` when it can be pressed. An
/// image a container uses cannot go (the same words as
/// `actions::disabled_reason`, which the delete itself checks again).
#[must_use]
pub fn delete_reason(row: &ImageRow) -> Option<String> {
    if row.used_by.is_empty() {
        None
    } else {
        Some(format!(
            "Used by {}",
            crate::services::confirm::plural(row.used_by.len(), "container", "containers")
        ))
    }
}

/// Ask first, then delete: opens the confirm dialog of one image (D25a,
/// `confirm::delete_image_model`). Nothing happens for an image the store no
/// longer lists, one that something uses, or while another overlay is open.
pub fn open_delete(state: &mut AppState, store: &ImageStore, engine: &EngineId, image_id: &str) {
    if !state.overlays.peek().is_empty() {
        return;
    }
    let image = store
        .by_engine
        .peek()
        .get(engine)
        .and_then(|images| images.iter().find(|i| i.id == image_id))
        .cloned();
    let Some(image) = image else { return };
    if !image.used_by.is_empty() {
        return;
    }
    let model = crate::services::confirm::delete_image_model(engine, &image);
    state.overlays.write().push(Overlay::Confirm(model));
}

/// ⌫ on the selected image (keys.rs `image.delete`).
pub fn delete_selected(state: &mut AppState) {
    if *state.section.peek() != crate::state::Section::Images {
        return;
    }
    let Selection::Image(id) = state.selection.peek().clone() else {
        return;
    };
    let Some(store) = try_consume_context::<ImageStore>() else {
        return;
    };
    let scope = state.engine_scope.peek().clone();
    if let Some(image) = store.find(&id, &scope) {
        open_delete(state, &store, &image.engine, &id);
    }
}

/// The engine a Pull image goes to: the one the scope names, else the first
/// running one.
#[must_use]
pub fn pull_target(engines: &[Engine], scope: &EngineScope) -> Option<Engine> {
    match scope {
        EngineScope::One(id) => engines.iter().find(|e| e.id == *id).cloned(),
        EngineScope::All => engines
            .iter()
            .find(|e| e.state == EngineState::Running)
            .cloned(),
    }
}

/// The segment counts: All, Dangling.
#[must_use]
pub fn filter_counts(rows: &[ImageRow]) -> [usize; 2] {
    [rows.len(), rows.iter().filter(|r| r.dangling).count()]
}

fn filter_id(filter: ImageFilter) -> &'static str {
    match filter {
        ImageFilter::All => "all",
        ImageFilter::Dangling => "dangling",
    }
}

/// What the main area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenState {
    /// Nothing has answered yet.
    Loading,
    /// An engine answered and has no image.
    Empty,
    Rows,
}

#[must_use]
pub fn screen_state(
    engines: &[Engine],
    by_engine: &BTreeMap<EngineId, Vec<Image>>,
    rows: &[ImageRow],
) -> ScreenState {
    if !rows.is_empty() {
        return ScreenState::Rows;
    }
    let answered = by_engine.keys().any(|id| {
        engines
            .iter()
            .find(|e| e.id == *id)
            .is_none_or(|e| e.state == EngineState::Running)
    });
    if answered {
        ScreenState::Empty
    } else {
        ScreenState::Loading
    }
}

// ---------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------

fn select_image(state: &mut AppState, id: &str) {
    state.selection.set(Selection::Image(id.to_string()));
}

/// Open the Run sheet for an image, once.
pub fn open_run(state: &mut AppState, id: &str) {
    let open = Overlay::Run(id.to_string());
    if !state.overlays.peek().contains(&open) {
        state.overlays.write().push(open);
    }
}

#[component]
fn ImageRowView(
    row: ImageRow,
    selected: bool,
    used_by: String,
    created: String,
    dangling_tag: bool,
) -> Element {
    let mut state = use_context::<AppState>();
    let select_id = row.id.clone();
    let run_id = row.id.clone();
    let reason = delete_reason(&row);
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let delete_engine = row.engine.clone();
    let delete_id = row.id.clone();
    let class = if selected {
        "im-row im-selected"
    } else {
        "im-row"
    };
    let name = if row.name.is_empty() {
        "<none>".to_string()
    } else {
        row.name.clone()
    };
    let shown_name = name.clone();
    let extra = row.tags.len().saturating_sub(1);
    rsx! {
        div {
            class,
            role: "row",
            "aria-selected": selected.to_string(),
            onclick: move |_| select_image(&mut state, &select_id),
            div { class: "im-cell im-name", title: "{name}",
                span { class: if row.name.is_empty() { "im-name-text im-name-none" } else { "im-name-text" },
                    span { "{shown_name}" }
                }
                if extra > 0 {
                    span { class: "im-extra", "+{extra}" }
                }
                if dangling_tag {
                    Badge { kind: BadgeKind::Tag, text: "dangling".to_string() }
                }
            }
            div { class: "im-cell im-id", "{row.short_id}" }
            div { class: "im-cell im-size", "{berth_core::fmt::bytes(row.size)}" }
            div { class: "im-cell im-arch",
                if row.foreign_arch {
                    Badge { kind: BadgeKind::Arch, text: row.arch.clone() }
                } else {
                    span { "{row.arch}" }
                }
            }
            div { class: "im-cell im-created", "{created}" }
            div { class: if row.used_by.is_empty() { "im-cell im-used im-used-none" } else { "im-cell im-used" },
                title: "{used_by}",
                "{used_by}"
            }
            div { class: "im-cell im-actions",
                Button {
                    label: "Run".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        open_run(&mut state, &run_id);
                    },
                }
                IconButton {
                    icon: "trash-2",
                    label: format!("Delete {name}"),
                    tone: IconButtonTone::Danger,
                    size: ControlSize::Sm,
                    tooltip: Some(reason.clone().unwrap_or_else(|| "Delete".to_string())),
                    disabled: reason.is_some(),
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        open_delete(&mut state, &store, &delete_engine, &delete_id);
                    },
                }
            }
        }
    }
}

#[component]
fn Controls(counts: [usize; 2], matched: usize, total: usize, can_pull: bool) -> Element {
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let mut query = store.query;
    let mut rev = store.query_rev;
    let mut filter = store.filter;
    let mut pull_open = store.pull_open;
    let has_query = !query.read().trim().is_empty();
    let rev_key = *rev.read();
    let initial = query.peek().clone();
    let selected = filter_id(*filter.read()).to_string();
    let items = vec![
        SegmentedItem::with_count("all", "All", counts[0]),
        SegmentedItem::with_count("dangling", "Dangling", counts[1]),
    ];
    let pull_pressed = *pull_open.read();
    rsx! {
        div { class: "im-controls",
            div { class: "im-filter",
                for key in [rev_key] {
                    SearchField {
                        key: "{key}",
                        initial_value: initial.clone(),
                        placeholder: "Filter images".to_string(),
                        aria_label: "Filter images".to_string(),
                        matched: has_query.then_some(matched),
                        total: has_query.then_some(total),
                        oninput: move |e: FormEvent| query.set(e.value()),
                        onclear: move |_| {
                            query.set(String::new());
                            rev.set(key + 1);
                        },
                    }
                }
            }
            SegmentedControl {
                items,
                selected,
                aria_label: "Image filter".to_string(),
                on_select: move |id: String| {
                    filter.set(if id == "dangling" { ImageFilter::Dangling } else { ImageFilter::All });
                },
            }
            span { class: "im-spacer" }
            Button {
                label: "Pull image".to_string(),
                variant: ButtonVariant::Primary,
                icon: "arrow-down-to-line",
                disabled: !can_pull,
                disabled_reason: (!can_pull).then(|| "No running engine to pull to".to_string()),
                onclick: move |_| pull_open.set(!pull_pressed),
            }
        }
    }
}

/// One pull as a card: the title and percent, the bytes, one thin bar per
/// layer, and Cancel pull while it runs (frame "Images / find / pulling").
#[component]
pub fn PullCard(pull: PullState) -> Element {
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let key = pull.key();
    let cancel_key = key.clone();
    let dismiss_key = key.clone();
    let (done, seen) = pull.layers_done();
    let title = match &pull.phase {
        PullPhase::Running => format!("Pulling {}", pull.reference),
        PullPhase::Done => format!("Pulled {}", pull.reference),
        PullPhase::Failed(_) => format!("Could not pull {}", pull.reference),
        PullPhase::Cancelled => format!("Cancelled the pull of {}", pull.reference),
    };
    let fraction = match &pull.phase {
        PullPhase::Done => Some(1.0),
        _ => pull.percent().map(|p| f32::from(p) / 100.0),
    };
    let detail = if pull.is_running() {
        let bytes = pull.bytes_label();
        if seen > 0 && !bytes.is_empty() {
            format!("{bytes} \u{b7} {done} of {seen} layers")
        } else {
            bytes
        }
    } else {
        String::new()
    };
    let running = pull.is_running();
    let finished_bar = matches!(pull.phase, PullPhase::Done);
    rsx! {
        div { class: "im-pull-card", role: "group", "aria-label": "{title}",
            if running || finished_bar {
                // The title and percent share the bar's head (frame
                // "Images / find / pulling"); the bytes sit under it.
                ProgressBar {
                    fraction,
                    label: Some(title.clone()),
                    detail: (!detail.is_empty()).then_some(detail),
                }
            } else {
                div { class: "im-pull-head",
                    div { class: "im-pull-title", "{title}" }
                }
            }
            if let PullPhase::Failed(message) = &pull.phase {
                InlineMessage { message: message.clone(), tone: InlineTone::Danger }
            }
            if running && !pull.layers.is_empty() {
                div { class: "im-layers",
                    for layer in pull.layers.clone() {
                        div { key: "{layer.id}", class: "im-layer",
                            div { class: "im-layer-head",
                                span { class: "im-layer-id", "{layer.id}" }
                                span { class: "im-layer-state", "{layer.label()}" }
                            }
                            ProgressBar {
                                fraction: Some(layer.fraction()),
                                thin: true,
                                aria_label: Some(format!("Layer {}", layer.id)),
                            }
                        }
                    }
                }
            } else if running && !pull.note.is_empty() {
                p { class: "im-pull-note", "{pull.note}" }
            }
            div { class: "im-pull-buttons",
                if running {
                    Button {
                        label: "Cancel pull".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Sm,
                        onclick: move |_| store.cancel_pull(&cancel_key),
                    }
                } else {
                    Button {
                        label: "Dismiss".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        onclick: move |_| store.dismiss_pull(&dismiss_key),
                    }
                }
            }
        }
    }
}

/// The form that starts a pull, and every pull's card under it.
#[component]
fn PullPanel(engine: Option<Engine>) -> Element {
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let mut reference = use_signal(String::new);
    let mut platform = use_signal(String::new);
    let mut refused = use_signal(|| None::<String>);
    let mut pull_open = store.pull_open;
    let pulls = store.pulls.read().clone();
    let open = *pull_open.read();
    if !open && pulls.is_empty() {
        return rsx! {};
    }
    let engine_name = engine.as_ref().map(|e| e.name.clone());
    let target = engine.clone();
    let platforms: Vec<SelectOption> = PLATFORMS
        .iter()
        .map(|(value, label)| SelectOption::new(*value, *label))
        .collect();
    rsx! {
        section { class: "im-pull", "aria-label": "Pull image",
            if open {
                div { class: "im-pull-form",
                    div { class: "im-pull-field",
                        TextField {
                            label: "Image".to_string(),
                            placeholder: "alpine:3".to_string(),
                            aria_label: "Image to pull".to_string(),
                            error: refused.read().clone(),
                            oninput: move |e: FormEvent| {
                                reference.set(e.value());
                                refused.set(None);
                            },
                        }
                    }
                    div { class: "im-pull-platform",
                        Select {
                            label: "Platform".to_string(),
                            options: platforms,
                            value: platform.read().clone(),
                            aria_label: "Platform".to_string(),
                            onchange: move |e: FormEvent| platform.set(e.value()),
                        }
                    }
                    Button {
                        label: "Pull".to_string(),
                        variant: ButtonVariant::Primary,
                        icon: "arrow-down-to-line",
                        disabled: target.is_none(),
                        onclick: move |_| {
                            let (Some(engine), Some(services)) = (target.as_ref(), services.as_ref()) else {
                                return;
                            };
                            let chosen = platform.peek().clone();
                            let result = store.pull(
                                services,
                                &engine.id,
                                &reference.peek(),
                                (!chosen.is_empty()).then_some(chosen.as_str()),
                            );
                            match result {
                                Ok(()) => refused.set(None),
                                Err(message) => refused.set(Some(message)),
                            }
                        },
                    }
                    Button {
                        label: "Close".to_string(),
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| pull_open.set(false),
                    }
                }
                if let Some(name) = engine_name {
                    p { class: "im-pull-engine", "Pulls to {name}." }
                }
            }
            for pull in pulls {
                PullCard { key: "{pull.reference}", pull }
            }
        }
    }
}

/// The Images section. The tab, filter and sort live in the [`ImageStore`]
/// so the inspector and the Run sheet share them; everything else is read
/// from `AppState`.
#[component]
pub fn ImagesScreen() -> Element {
    let state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();

    // The store's per-engine lists follow the flat list the engine service
    // keeps current (image events, snapshots).
    let refresh_services = services.clone();
    use_effect(move || {
        let _ = state.images.read();
        let _ = state.engines.read();
        if let Some(services) = refresh_services.as_ref() {
            store.refresh(services);
        }
    });

    let engines = state.engines.read();
    let containers = state.containers.read();
    let scope = state.engine_scope.read().clone();
    let by_engine = store.by_engine.read();
    let rows = build_rows(&engines, &by_engine, &scope);
    let screen = screen_state(&engines, &by_engine, &rows);
    let counts = filter_counts(&rows);
    let query = store.query.read().clone();
    let filter = *store.filter.read();
    let sort = *store.sort.read();
    let shown = list_rows(&rows, &query, filter, sort);
    let selected = match &*state.selection.read() {
        Selection::Image(id) => Some(id.clone()),
        _ => None,
    };
    let now = Utc::now();
    let tab = *store.tab.read();
    let pull_engine = pull_target(&engines, &scope);
    let total = rows.len();
    let matched = list_rows(&rows, &query, ImageFilter::All, sort).len();
    let tab_items = vec![
        TabItem::new("engine", "On this engine"),
        TabItem::new("find", "Find images"),
    ];
    let tab_id = match tab {
        ImagesTab::OnThisEngine => "engine",
        ImagesTab::Find => "find",
    };
    let table_rows: Vec<(ImageRow, bool, String, String)> = shown
        .iter()
        .map(|row| {
            (
                row.clone(),
                selected.as_deref() == Some(row.id.as_str()),
                used_by_text(&row.used_by, &containers),
                berth_core::fmt::age(row.created, now),
            )
        })
        .collect();
    let row_count = table_rows.len();
    let mut tab_signal = store.tab;
    let mut sort_signal = store.sort;
    let mut pull_open = store.pull_open;
    rsx! {
        section { class: "images-screen", "aria-label": "Images",
            style { "{IMAGES_CSS}" }
            style { "{PROGRESS_BAR_CSS}" }
            header { class: "im-header",
                h1 { class: "im-title", "Images" }
                if screen != ScreenState::Loading {
                    Badge { kind: BadgeKind::Count, text: total.to_string() }
                }
            }
            div { class: "im-tabs",
                Tabs {
                    items: tab_items,
                    selected: tab_id.to_string(),
                    aria_label: "Images".to_string(),
                    on_select: move |id: String| {
                        tab_signal.set(if id == "find" { ImagesTab::Find } else { ImagesTab::OnThisEngine });
                    },
                }
            }
            if tab == ImagesTab::Find {
                div { class: "im-find-soon",
                    EmptyState {
                        title: "Find images".to_string(),
                        body: "Search Docker Hub and your registries. This tab is built in the next task."
                            .to_string(),
                        icon: "search",
                    }
                }
            } else {
                Controls { counts, matched, total, can_pull: pull_engine.is_some() }
                PullPanel { engine: pull_engine.clone() }
                match screen {
                    ScreenState::Loading => rsx! {
                        div { class: "im-loading", SkeletonRows { count: 8_usize } }
                    },
                    ScreenState::Empty => rsx! {
                        div { class: "im-empty",
                            EmptyState {
                                title: "No images on this engine".to_string(),
                                body: "Pull one, or start a project and Docker pulls what it needs."
                                    .to_string(),
                                icon: "layers",
                                action_label: Some("Pull image".to_string()),
                                onaction: move |()| pull_open.set(true),
                            }
                        }
                    },
                    ScreenState::Rows => rsx! {
                        div { class: "im-table", role: "table", "aria-rowcount": "{row_count}",
                            div { class: "im-head", role: "row",
                                TableHeaderCell {
                                    label: "Image".to_string(),
                                    sortable: true,
                                    sort: header_direction(sort, SortColumn::Name),
                                    onsort: move |()| {
                                        let next = next_sort(*sort_signal.peek(), SortColumn::Name);
                                        sort_signal.set(next);
                                    },
                                }
                                TableHeaderCell { label: "ID".to_string() }
                                TableHeaderCell {
                                    label: "Size".to_string(),
                                    align: CellAlign::Right,
                                    sortable: true,
                                    sort: header_direction(sort, SortColumn::Size),
                                    onsort: move |()| {
                                        let next = next_sort(*sort_signal.peek(), SortColumn::Size);
                                        sort_signal.set(next);
                                    },
                                }
                                TableHeaderCell { label: "Arch".to_string() }
                                TableHeaderCell {
                                    label: "Created".to_string(),
                                    sortable: true,
                                    sort: header_direction(sort, SortColumn::Created),
                                    onsort: move |()| {
                                        let next = next_sort(*sort_signal.peek(), SortColumn::Created);
                                        sort_signal.set(next);
                                    },
                                }
                                TableHeaderCell { label: "Used by".to_string() }
                                div { class: "table-header-cell", role: "columnheader" }
                            }
                            if row_count == 0 {
                                div { class: "im-no-match",
                                    EmptyState {
                                        title: "No images match".to_string(),
                                        body: "Try a different filter.".to_string(),
                                        icon: "search",
                                    }
                                }
                            }
                            for (row , is_selected , used_by , created) in table_rows {
                                ImageRowView {
                                    key: "{row.engine.0}/{row.id}",
                                    dangling_tag: row.dangling,
                                    row,
                                    selected: is_selected,
                                    used_by,
                                    created,
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
