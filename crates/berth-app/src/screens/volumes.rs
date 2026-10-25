//! Volumes screen (P4-T4, brief 7.3). No frame draws it: the header and the
//! table follow "Containers / selected / 1440", the Delete confirm is the
//! typed-name dialog. Header with count, filter, All / Unused and Create
//! volume; a sortable virtual table of name, project, size ("Calculating…"
//! until `volume_sizes` answers), used by, driver and created; row buttons
//! Browse and Delete (disabled with the reason while a container uses the
//! volume).
//!
//! The model is `services::volumes::build_list` (pure). The screen reads
//! signals and sends clicks to the actions service and the confirm dialog;
//! it never calls an engine (D23). The size fetch runs on the I/O runtime
//! and lands in [`VolumesView::sizes`].

use std::collections::BTreeMap;

use bytesize::ByteSize;
use dioxus::prelude::*;

use berth_core::Subject;

use crate::screens::containers::{list_status, ListStatus};
use crate::services::actions::{disabled_reason, with_view};
use crate::services::confirm;
use crate::services::volumes::{
    build_list, fetch_sizes, needs_sizes, ListInput, SortColumn, UsageFilter, VolumeRow,
};
use crate::services::Services;
use crate::state::{AppState, Overlay, PendingKey, PendingState, Section, Selection};
use crate::ui::{
    scope_includes, Badge, BadgeKind, Banner, BannerTone, Button, ButtonVariant, CellAlign,
    ControlSize, EmptyState, Icon, IconButton, IconButtonTone, IconSize, SearchField,
    SegmentedControl, SegmentedItem, SkeletonRows, SortDirection, TableHeaderCell, VirtualTable,
};

/// The screen's CSS, rendered with it (tokens only): the hand-written
/// lists, then the generated column-drop tiers.
pub const VOLUMES_CSS: &str = concat!(
    include_str!("../../assets/css/volumes.css"),
    include_str!("../../assets/css/volumes_tiers.css")
);

/// The screen's own signals; provided once by the App root (and the scene
/// root) so the inspector's detail panel can read them too.
#[derive(Clone, Copy)]
pub struct VolumesView {
    pub filter: Signal<UsageFilter>,
    pub text: Signal<String>,
    pub sort: Signal<Option<(SortColumn, bool)>>,
    /// Bumped to remount the (uncontrolled) filter field when cleared (D59).
    pub epoch: Signal<u32>,
    /// Sizes the fetch found, by volume name (the lists carry none).
    pub sizes: Signal<BTreeMap<String, ByteSize>>,
    /// The volume whose Browse button was pressed: its detail opens on the
    /// Files tab.
    pub browse: Signal<Option<String>>,
}

impl VolumesView {
    #[must_use]
    pub fn new() -> Self {
        Self {
            filter: Signal::new(UsageFilter::All),
            text: Signal::new(String::new()),
            sort: Signal::new(None),
            epoch: Signal::new(0),
            sizes: Signal::new(BTreeMap::new()),
            browse: Signal::new(None),
        }
    }
}

impl Default for VolumesView {
    fn default() -> Self {
        Self::new()
    }
}

/// Provide the view from the App root (call once, in a component body).
pub fn provide_volumes_view() -> VolumesView {
    use_hook(|| provide_context(VolumesView::new()))
}

/// The view in context, or a fresh one (tests that mount a piece alone).
pub fn view_or_new() -> VolumesView {
    try_consume_context::<VolumesView>().unwrap_or_else(|| provide_context(VolumesView::new()))
}

/// ⌫ on the selected volume: the delete dialog, only on this screen and
/// never over another overlay. A volume in use cannot be deleted, so
/// nothing opens.
pub fn open_delete_for_selection(state: &mut AppState) {
    if *state.section.peek() != Section::Volumes || !state.overlays.peek().is_empty() {
        return;
    }
    let name = match &*state.selection.peek() {
        Selection::Volume(name) => name.clone(),
        _ => return,
    };
    confirm::open_delete_volume(state, &name);
}

/// Open the create sheet once.
pub fn open_create(state: &mut AppState) {
    if !state.overlays.peek().contains(&Overlay::CreateVolume) {
        state.overlays.write().push(Overlay::CreateVolume);
    }
}

fn sort_direction(sort: Option<(SortColumn, bool)>, column: SortColumn) -> SortDirection {
    match sort {
        Some((c, true)) if c == column => SortDirection::Asc,
        Some((c, false)) if c == column => SortDirection::Desc,
        _ => SortDirection::None,
    }
}

/// The sentences the empty table says: (title, body, button).
#[must_use]
pub fn empty_copy(text: &str, filter: UsageFilter) -> (String, String, Option<&'static str>) {
    let typed = text.trim();
    if !typed.is_empty() {
        (
            "No volumes match".to_string(),
            format!("Nothing matches \u{201c}{typed}\u{201d}."),
            Some("Clear filter"),
        )
    } else if filter == UsageFilter::Unused {
        (
            "Every volume is in use".to_string(),
            "A volume no container uses shows here.".to_string(),
            Some("Show all volumes"),
        )
    } else {
        (
            "No volumes yet".to_string(),
            "Volumes keep a container's data when the container goes. Create one, or start a project that declares one."
                .to_string(),
            Some("Create volume"),
        )
    }
}

/// What a failed volume action says in the banner: "Could not delete
/// berthfx-vol: …", per failed key, sorted by name.
#[must_use]
pub fn failure_banners(pending: &BTreeMap<PendingKey, PendingState>) -> Vec<(PendingKey, String)> {
    pending
        .iter()
        .filter_map(|(key, value)| match (&key.subject, value) {
            (Subject::Volume(name), PendingState::Failed { message }) => {
                let verb = if key.action == crate::services::volumes::CREATE {
                    "create"
                } else {
                    "delete"
                };
                Some((key.clone(), format!("Could not {verb} {name}: {message}")))
            }
            _ => None,
        })
        .collect()
}

#[component]
fn VolumeRowView(row: VolumeRow) -> Element {
    let mut state = use_context::<AppState>();
    let view = use_context::<VolumesView>();
    let name = row.name.clone();
    let selected = matches!(&*state.selection.read(), Selection::Volume(n) if *n == name);
    let subject = Subject::Volume(name.clone());
    let (delete_reason, busy) = {
        let reason = with_view(&state, |v| disabled_reason(&subject, "volume.delete", v));
        let pending = state.pending.read();
        let busy = pending.iter().find_map(|(key, value)| match value {
            PendingState::Running { label } if key.subject == subject => Some(label.clone()),
            _ => None,
        });
        (reason, busy)
    };
    let class = if selected {
        "vl-row vl-selected"
    } else {
        "vl-row"
    };
    let size_class = if row.size.is_some() && busy.is_none() {
        "vl-cell vl-size"
    } else {
        "vl-cell vl-size vl-size-wait"
    };
    let size_text = busy.clone().unwrap_or_else(|| row.size_text());
    let created = row.created.map_or_else(
        || "\u{2014}".to_string(),
        |at| berth_core::fmt::age(at, chrono::Utc::now()),
    );
    let used_class = if row.unused() {
        "vl-cell vl-dash"
    } else {
        "vl-cell vl-used"
    };
    let select_name = name.clone();
    let browse_name = name.clone();
    let delete_name = name.clone();
    rsx! {
        div {
            class,
            role: "row",
            "aria-selected": selected.to_string(),
            "aria-busy": busy.is_some().to_string(),
            onclick: move |_| {
                let mut browse = view.browse;
                browse.set(None);
                state.selection.set(Selection::Volume(select_name.clone()));
            },
            div { class: "vl-cell vl-name", title: "{name}",
                Icon { name: "hard-drive", size: IconSize::Sm }
                span { class: "vl-name-text", "{name}" }
            }
            div { class: "vl-cell vl-project vl-muted",
                if let Some(project) = row.project.clone() {
                    "{project}"
                } else {
                    span { class: "vl-dash", "\u{2014}" }
                }
            }
            div { class: size_class, "{size_text}" }
            div { class: used_class, title: "{row.used_by.join(\", \")}", "{row.used_by_text()}" }
            div { class: "vl-cell vl-driver vl-muted", "{row.driver}" }
            div { class: "vl-cell vl-created vl-muted", "{created}" }
            div { class: "vl-cell vl-actions",
                IconButton {
                    icon: "folder",
                    label: format!("Browse files in {name}"),
                    size: ControlSize::Sm,
                    tooltip: "Browse files",
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        let mut browse = view.browse;
                        browse.set(Some(browse_name.clone()));
                        state.selection.set(Selection::Volume(browse_name.clone()));
                    },
                }
                IconButton {
                    icon: "trash-2",
                    label: format!("Delete {name}"),
                    tone: IconButtonTone::Danger,
                    size: ControlSize::Sm,
                    tooltip: delete_reason.clone().unwrap_or_else(|| "Delete\u{2026}".to_string()),
                    disabled: delete_reason.is_some(),
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        confirm::open_delete_volume(&mut state, &delete_name);
                    },
                }
            }
        }
    }
}

#[component]
pub fn VolumesScreen() -> Element {
    let mut state = use_context::<AppState>();
    let view = use_hook(view_or_new);
    let services = try_consume_context::<Services>();

    let model = use_memo(move || {
        let volumes = state.volumes.read();
        let containers = state.containers.read();
        let projects = state.projects.read();
        let scope = state.engine_scope.read();
        let sizes = view.sizes.read();
        let text = view.text.read();
        let filter = *view.filter.read();
        let sort = *view.sort.read();
        build_list(&ListInput {
            volumes: &volumes,
            containers: &containers,
            projects: &projects,
            scope: &scope,
            sizes: &sizes,
            text: &text,
            filter,
            sort,
        })
    });

    // Sizes come from `system df`, which is slow: ask once when the screen
    // opens, and again when a volume shows that has none yet.
    let first = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(true)));
    let _sizes = use_resource(move || {
        let volumes = state.volumes.read().clone();
        let engines: Vec<berth_core::EngineId> = state
            .engines
            .read()
            .iter()
            .filter(|e| e.state == berth_core::EngineState::Running)
            .filter(|e| scope_includes(&state.engine_scope.read(), &e.id))
            .map(|e| e.id.clone())
            .collect();
        let cached = view.sizes.peek().clone();
        let first_run = first.replace(false);
        let services = services.clone();
        async move {
            if !first_run && !needs_sizes(&volumes, &cached) {
                return;
            }
            let Some(services) = services else { return };
            let clients: Vec<_> = engines
                .iter()
                .filter_map(|id| services.client(id))
                .collect();
            if clients.is_empty() {
                return;
            }
            let found = crate::services::files::on_io(fetch_sizes(clients)).await;
            if let Some(found) = found {
                let mut sizes = view.sizes;
                sizes.write().extend(found);
            }
        }
    });

    let engines = state.engines.read().clone();
    let scope = state.engine_scope.read().clone();
    let status = list_status(
        &engines,
        &scope,
        &state.containers.read(),
        *state.snapshot_stale.read(),
    );
    let (total, matched, unused, row_count) = {
        let m = model.read();
        (m.total, m.matched, m.unused, m.rows.len())
    };
    let text = view.text.read().clone();
    let filter = *view.filter.read();
    let sort = *view.sort.read();
    let epoch = *view.epoch.read();
    let counts_total: usize = engines
        .iter()
        .filter(|e| scope_includes(&scope, &e.id))
        .map(|e| e.counts.volumes as usize)
        .sum();
    let shown_total = if status == ListStatus::Ready {
        total
    } else {
        counts_total
    };
    let banners = failure_banners(&state.pending.read());
    let row_height_px = {
        let settings = state.settings.read();
        crate::text_scale::row_height_px(settings.density, settings.text_scale)
    };

    let render_row = use_callback(move |i: usize| -> Element {
        let model = model.read();
        let Some(row) = model.rows.get(i) else {
            return rsx! {};
        };
        rsx! { VolumeRowView { key: "{row.name}", row: row.clone() } }
    });

    let onsort = move |column: SortColumn| {
        let mut sort = view.sort;
        let next = match *sort.peek() {
            Some((c, ascending)) if c == column => (column, !ascending),
            _ => (column, true),
        };
        sort.set(Some(next));
    };
    let header_cell = move |column: SortColumn, label: &'static str, align: CellAlign| {
        rsx! {
            TableHeaderCell {
                label: label.to_string(),
                sortable: true,
                align,
                sort: sort_direction(sort, column),
                onsort: move |()| onsort(column),
            }
        }
    };
    let head = rsx! {
        div { class: "vl-head", role: "row",
            {header_cell(SortColumn::Name, "Name", CellAlign::Left)}
            span { class: "vl-project-head", {header_cell(SortColumn::Project, "Project", CellAlign::Left)} }
            {header_cell(SortColumn::Size, "Size", CellAlign::Right)}
            {header_cell(SortColumn::UsedBy, "Used by", CellAlign::Left)}
            span { class: "vl-driver-head", {header_cell(SortColumn::Driver, "Driver", CellAlign::Left)} }
            span { class: "vl-created-head", {header_cell(SortColumn::Created, "Created", CellAlign::Left)} }
            span {}
        }
    };

    let usage_items: Vec<SegmentedItem> = vec![
        SegmentedItem::with_count(UsageFilter::All.id(), UsageFilter::All.label(), total),
        SegmentedItem::with_count(
            UsageFilter::Unused.id(),
            UsageFilter::Unused.label(),
            unused,
        ),
    ];
    let has_text = !text.trim().is_empty();
    let clear_filter = move || {
        let mut text = view.text;
        text.set(String::new());
        let mut epoch = view.epoch;
        let next = *epoch.peek() + 1;
        epoch.set(next);
        let mut filter = view.filter;
        filter.set(UsageFilter::All);
    };

    rsx! {
        style { "{VOLUMES_CSS}" }
        section { class: "volumes-screen", "aria-label": "Volumes",
            div { class: "vl-header",
                h1 { class: "vl-title", "Volumes" }
                Badge { kind: BadgeKind::Count, text: "{shown_total}" }
                span { class: "vl-spacer" }
                div { class: "vl-filter",
                    for key in [epoch] {
                        SearchField {
                            key: "{key}",
                            initial_value: text.clone(),
                            placeholder: "Filter volumes".to_string(),
                            aria_label: "Filter volumes".to_string(),
                            matched: if has_text { Some(matched) } else { None },
                            total: if has_text { Some(total) } else { None },
                            oninput: move |e: FormEvent| {
                                let mut text = view.text;
                                text.set(e.value());
                            },
                            onclear: move |_| clear_filter(),
                        }
                    }
                }
                div { class: "vl-usage",
                    SegmentedControl {
                        items: usage_items,
                        selected: filter.id().to_string(),
                        aria_label: "Show volumes".to_string(),
                        on_select: move |id: String| {
                            let mut filter = view.filter;
                            filter.set(UsageFilter::from_id(&id));
                        },
                    }
                }
                div { class: "vl-create",
                    Button {
                        label: "Create volume".to_string(),
                        icon: "plus",
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Md,
                        onclick: move |_| open_create(&mut state),
                    }
                }
            }
            if !banners.is_empty() {
                div { class: "vl-banners",
                    for (key, message) in banners {
                        Banner {
                            key: "{key.action}-{message}",
                            message,
                            tone: BannerTone::Danger,
                            action_label: Some("Dismiss".to_string()),
                            onaction: move |()| crate::services::actions::dismiss(&mut state, &key),
                        }
                    }
                }
            }
            match status {
                ListStatus::Loading => rsx! {
                    div { class: "vl-table", role: "table",
                        {head.clone()}
                        div { class: "vl-skeleton", SkeletonRows { count: 8_usize } }
                    }
                },
                ListStatus::EngineStopped(engine) => rsx! {
                    div { class: "vl-empty",
                        EmptyState {
                            title: "Start the engine to see its volumes".to_string(),
                            body: format!("Engine {} is stopped.", engine.name),
                            icon: "hard-drive",
                        }
                    }
                },
                ListStatus::Ready if row_count == 0 => {
                    let (title, body, action) = empty_copy(&text, filter);
                    let create = action == Some("Create volume");
                    rsx! {
                        div { class: "vl-empty",
                            EmptyState {
                                title,
                                body,
                                icon: "hard-drive",
                                action_label: action.map(str::to_string),
                                onaction: move |()| {
                                    if create {
                                        open_create(&mut state);
                                    } else {
                                        clear_filter();
                                    }
                                },
                            }
                        }
                    }
                }
                ListStatus::Ready => rsx! {
                    div { class: "vl-table", role: "table",
                        {head.clone()}
                        VirtualTable {
                            total: row_count,
                            render_row,
                            row_height_px,
                            viewport_height_px: 1100_u32,
                            overscan: 10_usize,
                        }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "volumes_tests.rs"]
mod tests;
