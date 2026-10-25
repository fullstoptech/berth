//! ColumnBrowser (P5-T4, PLAN §7.3, D36): the Finder-style columns of the Open
//! folder sheet. One column per folder from the home down to the folder being
//! looked at; each row is a folder with a marker (In Berth, Compose,
//! Dockerfile) or its item count and a chevron.
//!
//! This is markup over a [`Browser`] signal: it reads the model and reports
//! clicks. The sheet owns the I/O. A column is a virtual list (D24): only the
//! rows in view plus an overscan are in the DOM, so a folder of 5,000
//! subfolders costs a screen of rows.
//!
//! States: a column being read (skeleton, with a note when slow), a folder
//! with nothing to show (empty or nothing matches the filter), and a folder
//! that cannot be read (no access with the macOS words, not found, other).

use std::path::PathBuf;

use dioxus::prelude::*;

use super::file_browser::item_count;
use super::virtual_table::{bottom_spacer_px, top_spacer_px, visible_range, DEFAULT_OVERSCAN};
use super::{Button, ButtonVariant, ControlSize, Icon, IconSize};
use crate::services::folders::{
    folder_name, marker_of, Browser, ColumnState, Entry, FolderError, Known, Marker,
};

/// The CSS of the browser, rendered with it (tokens only).
pub const COLUMN_BROWSER_CSS: &str = include_str!("../../assets/css/column_browser.css");

/// Row height in px at the default text size; the stylesheet scales it.
pub const ROW_HEIGHT_BASE_PX: u32 = 28;

/// The text of the right-hand side of a row: the marker, else the count.
#[must_use]
pub fn meta_text(entry: &Entry, known: bool) -> Option<(Option<Marker>, String)> {
    let details = entry.details?;
    match marker_of(details.markers, known) {
        Some(marker) => Some((Some(marker), marker.label().to_string())),
        None => details.items.map(|n| (None, item_count(n))),
    }
}

fn marker_class(marker: Option<Marker>) -> &'static str {
    match marker {
        Some(Marker::InBerth) => "cb-meta cb-meta-known",
        Some(Marker::Compose | Marker::Dockerfile) => "cb-meta cb-meta-marker",
        None => "cb-meta",
    }
}

#[component]
pub fn ColumnBrowser(
    browser: Signal<Browser>,
    known: Memo<Known>,
    #[props(default = ROW_HEIGHT_BASE_PX)] row_height_px: u32,
    #[props(default = 540)] viewport_height_px: u32,
    on_select: EventHandler<(usize, PathBuf)>,
    on_open: EventHandler<(usize, PathBuf)>,
    on_open_settings: EventHandler<()>,
    on_system_dialog: EventHandler<PathBuf>,
    on_other: EventHandler<()>,
) -> Element {
    let failure = browser.read().failure().cloned();
    let folders: Vec<PathBuf> = browser
        .read()
        .columns
        .iter()
        .map(|c| c.folder.clone())
        .collect();
    let count = folders.len();

    // A new column comes in at the right edge: keep it in view.
    use_effect(move || {
        let _ = browser.read().columns.len();
        spawn(async move {
            let _ = document::eval(
                "var e = document.getElementById('cb-columns'); if (e) { e.scrollLeft = e.scrollWidth; }",
            )
            .await;
        });
    });

    rsx! {
        style { "{COLUMN_BROWSER_CSS}" }
        if let Some(err) = failure {
            FailureState {
                err,
                on_open_settings,
                on_system_dialog,
                on_other,
            }
        } else {
            div { class: "cb", id: "cb-columns", role: "group", "aria-label": "Folders", "data-columns": "{count}",
                for (index, folder) in folders.into_iter().enumerate() {
                    ColumnList {
                        key: "{folder.display()}",
                        index,
                        browser,
                        known,
                        row_height_px,
                        viewport_height_px,
                        on_select,
                        on_open,
                    }
                }
            }
        }
    }
}

#[component]
fn ColumnList(
    index: usize,
    browser: Signal<Browser>,
    known: Memo<Known>,
    row_height_px: u32,
    viewport_height_px: u32,
    on_select: EventHandler<(usize, PathBuf)>,
    on_open: EventHandler<(usize, PathBuf)>,
) -> Element {
    let mut scroll_top = use_signal(|| 0.0_f64);
    let list_id = format!("cb-col-{index}");

    // Keyboard moves can leave the highlighted row out of view: scroll to it.
    let selected_row = use_memo(move || {
        let b = browser.read();
        let col = b.columns.get(index)?;
        let ColumnState::Ready(entries) = &col.state else {
            return None;
        };
        let selected = col.selected.as_ref()?;
        b.visible(index)
            .iter()
            .position(|&i| entries[i].path == *selected)
    });
    let scroll_id = list_id.clone();
    use_effect(move || {
        let Some(row) = selected_row() else { return };
        let top = f64::from(row_height_px) * row as f64;
        let bottom = top + f64::from(row_height_px);
        let seen = *scroll_top.peek();
        let view = f64::from(viewport_height_px);
        let target = if top < seen {
            top
        } else if bottom > seen + view {
            bottom - view
        } else {
            return;
        };
        let js = format!(
            "var e = document.getElementById('{scroll_id}'); if (e) {{ e.scrollTop = {target}; }}"
        );
        spawn(async move {
            let _ = document::eval(&js).await;
        });
    });

    let b = browser.read();
    let Some(column) = b.columns.get(index) else {
        return rsx! {};
    };
    let active = index + 1 == b.columns.len();
    let col_class = if active {
        "cb-col cb-col-active"
    } else {
        "cb-col"
    };
    let name = folder_name(&column.folder);

    match &column.state {
        ColumnState::Loading { slow } => rsx! {
            div { class: col_class, "aria-busy": "true", "data-state": "loading",
                div { class: "cb-skeleton", role: "status",
                    span { class: "cb-sr-only", "Reading {name}\u{2026}" }
                    for i in 0..9 {
                        div { key: "{i}", class: "cb-skeleton-row", "aria-hidden": "true",
                            span { class: "cb-skeleton-box cb-skeleton-icon" }
                            span { class: "cb-skeleton-box cb-skeleton-bar", style: "width: {40 + (i * 37) % 45}%;" }
                        }
                    }
                }
                if *slow {
                    p { class: "cb-note", role: "status",
                        "{name} is slow to answer. Berth keeps reading; you can pick another folder meanwhile."
                    }
                } else {
                    p { class: "cb-note", "Reading {name}\u{2026}" }
                }
            }
        },
        ColumnState::Failed(err) => rsx! {
            div { class: col_class, "data-state": "failed",
                p { class: "cb-note", "{err.headline()}" }
            }
        },
        ColumnState::Ready(entries) => {
            let rows = b.visible(index);
            if rows.is_empty() {
                let note = if entries.is_empty() {
                    format!("No folders in {name}")
                } else if active && !b.filter.trim().is_empty() {
                    format!(
                        "Nothing in {name} matches \u{201c}{}\u{201d}",
                        b.filter.trim()
                    )
                } else {
                    format!("Only hidden folders in {name}")
                };
                return rsx! {
                    div { class: col_class, "data-state": "empty",
                        p { class: "cb-note", "{note}" }
                    }
                };
            }
            let total = rows.len();
            let range = visible_range(
                *scroll_top.read(),
                f64::from(viewport_height_px),
                row_height_px,
                total,
                DEFAULT_OVERSCAN,
            );
            let top_px = top_spacer_px(range.start, row_height_px);
            let bottom_px = bottom_spacer_px(total, range.end, row_height_px);
            let known_now = known.read();
            let selected = column.selected.clone();
            rsx! {
                div { class: col_class, "data-state": "ready",
                    div {
                        class: "cb-list",
                        id: "{list_id}",
                        role: "listbox",
                        "aria-label": "Folders in {name}",
                        "aria-rowcount": "{total}",
                        style: "max-height: {viewport_height_px}px;",
                        onscroll: move |e| scroll_top.set(e.data.scroll_top()),
                        div { class: "cb-spacer", style: "height: {top_px}px;" }
                        for at in range {
                            {
                                let entry = &entries[rows[at]];
                                let is_known = known_now.contains(&entry.path);
                                let meta = meta_text(entry, is_known);
                                let is_selected = selected.as_ref() == Some(&entry.path);
                                let row_class = match (is_selected, active) {
                                    (true, true) => "cb-row cb-row-selected cb-row-focus",
                                    (true, false) => "cb-row cb-row-selected",
                                    _ => "cb-row",
                                };
                                let click_path = entry.path.clone();
                                let open_path = entry.path.clone();
                                let chevron_path = entry.path.clone();
                                let label = entry.name.clone();
                                rsx! {
                                    div {
                                        key: "{entry.path.display()}",
                                        class: row_class,
                                        role: "option",
                                        "aria-selected": "{is_selected}",
                                        "aria-posinset": "{at + 1}",
                                        title: "{entry.name}",
                                        style: "height: {row_height_px}px;",
                                        onclick: move |_| on_select.call((index, click_path.clone())),
                                        ondoubleclick: move |_| on_open.call((index, open_path.clone())),
                                        span { class: "cb-icon",
                                            Icon { name: "folder", size: IconSize::Sm }
                                        }
                                        span { class: "cb-name", "{label}" }
                                        if let Some((marker, text)) = meta {
                                            span { class: marker_class(marker), "{text}" }
                                        }
                                        button {
                                            class: "cb-chevron",
                                            r#type: "button",
                                            tabindex: "-1",
                                            "aria-label": "Open {label}",
                                            onclick: move |e| {
                                                e.stop_propagation();
                                                on_open.call((index, chevron_path.clone()));
                                            },
                                            Icon { name: "chevron-right", size: IconSize::Sm }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "cb-spacer", style: "height: {bottom_px}px;" }
                    }
                }
            }
        }
    }
}

/// A folder that cannot be read: the words, and what to do about it.
#[component]
fn FailureState(
    err: FolderError,
    on_open_settings: EventHandler<()>,
    on_system_dialog: EventHandler<PathBuf>,
    on_other: EventHandler<()>,
) -> Element {
    let name = err.folder_name();
    let folder = err.folder().to_path_buf();
    let no_access = err.is_no_access();
    let (lines, state): (Vec<String>, &str) = match &err {
        FolderError::NoAccess(_) => (
            vec![
                "macOS asks you to allow access.".to_string(),
                "Allow it in System Settings \u{2192} Privacy & Security \u{2192} Files and Folders."
                    .to_string(),
                "No need to restart Berth after allowing.".to_string(),
            ],
            "no-access",
        ),
        FolderError::NotFound(_) => (
            vec!["Check the path in the bar above, or pick a place on the left.".to_string()],
            "not-found",
        ),
        FolderError::NotAFolder(_) => (
            vec!["A project is a folder. Pick the folder that holds it.".to_string()],
            "not-a-folder",
        ),
        FolderError::Failed(_, message) => (vec![message.clone()], "failed"),
    };
    rsx! {
        div { class: "cb-failure", role: "alert", "data-state": state,
            span { class: "cb-failure-icon",
                Icon { name: "folder", size: IconSize::Md }
            }
            h3 { class: "cb-failure-title", "{err.headline()}" }
            div { class: "cb-failure-body",
                for line in lines {
                    p { key: "{line}", "{line}" }
                }
            }
            div { class: "cb-failure-actions",
                if no_access {
                    Button {
                        label: "Open System Settings",
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Md,
                        onclick: move |_| on_open_settings.call(()),
                    }
                }
                Button {
                    label: "Choose another folder",
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| on_other.call(()),
                }
            }
            if no_access {
                button {
                    class: "cb-failure-link",
                    r#type: "button",
                    onclick: move |_| on_system_dialog.call(folder.clone()),
                    span { "Choose {name} with the system dialog" }
                    span { class: "cb-failure-hint", "Grants access in one step" }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "column_browser_tests.rs"]
mod tests;
