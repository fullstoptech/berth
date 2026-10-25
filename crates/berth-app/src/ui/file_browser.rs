//! FileBrowser (P3-T8, PLAN D51): one folder of a container, as a
//! breadcrumb trail over a virtual list with name, size and modified, a
//! download button on each file, and Upload and Refresh in the header.
//!
//! This is markup over a [`Listing`]; it holds no I/O and no state of its
//! own. `files_panel::FilesPanel` feeds it from `services::files`. The
//! states: reading, failed (with the engine's words and Retry), empty, and
//! a list of any length (rows are windowed, so 5,000 entries cost a screen
//! of rows).

use berth_core::{FileEntry, FileKind};
use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;

use super::{
    Button, ButtonVariant, ControlSize, EmptyState, IconButton, InlineMessage, InlineTone,
    TextField, VirtualTable,
};
use crate::services::files::{breadcrumbs, join_path, normalize_dir, parent_dir};
use crate::state::AppState;

/// The CSS of the browser, rendered with it (tokens only).
pub const FILE_BROWSER_CSS: &str = include_str!("../../assets/css/file_browser.css");

/// What the folder view shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Listing {
    /// The archive is being read.
    Loading,
    /// The entries, folders first.
    Ready(Vec<FileEntry>),
    /// The engine's words for why not.
    Failed(String),
}

/// "5,000" for a count.
#[must_use]
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "1 item" or "5,000 items".
#[must_use]
pub fn item_count(n: usize) -> String {
    if n == 1 {
        "1 item".to_string()
    } else {
        format!("{} items", thousands(n))
    }
}

/// The size column: bytes for a file, a dash for anything else.
#[must_use]
pub fn size_text(entry: &FileEntry) -> String {
    match (entry.kind, entry.size) {
        (FileKind::File, Some(size)) => berth_core::fmt::bytes(size),
        _ => "\u{2014}".to_string(),
    }
}

/// The modified column in `tz`: "Oct 24 14:03" within the year of `now`,
/// "Oct 24 2025" before it, a dash when the archive had no time.
#[must_use]
pub fn modified_text_in<Tz: TimeZone>(
    modified: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    tz: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Datelike;
    let Some(at) = modified else {
        return "\u{2014}".to_string();
    };
    let at = at.with_timezone(tz);
    if at.year() == now.with_timezone(tz).year() {
        at.format("%b %-d %H:%M").to_string()
    } else {
        at.format("%b %-d %Y").to_string()
    }
}

fn kind_icon(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Dir => "folder",
        FileKind::File | FileKind::Link | FileKind::Other => "file-text",
    }
}

fn kind_note(kind: FileKind) -> Option<&'static str> {
    match kind {
        FileKind::Link => Some("link"),
        FileKind::Other => Some("special"),
        FileKind::File | FileKind::Dir => None,
    }
}

/// Why a row cannot be downloaded, `None` for a file.
fn download_blocker(entry: &FileEntry) -> Option<&'static str> {
    match entry.kind {
        FileKind::File => None,
        FileKind::Dir => Some("Folders cannot be downloaded yet"),
        FileKind::Link => Some("Links cannot be downloaded"),
        FileKind::Other => Some("Only regular files can be downloaded"),
    }
}

#[component]
fn FileRow(
    dir: String,
    entry: FileEntry,
    now: DateTime<Utc>,
    on_navigate: EventHandler<String>,
    on_download: EventHandler<String>,
) -> Element {
    let name = entry.name.clone();
    let blocker = download_blocker(&entry);
    let modified = modified_text_in(entry.modified, now, &chrono::Local);
    let size = size_text(&entry);
    let icon = kind_icon(entry.kind);
    let note = kind_note(entry.kind);
    let is_dir = entry.kind == FileKind::Dir;
    let open_path = join_path(&dir, &name);
    rsx! {
        div { class: "fb-row",
            div { class: "fb-cell fb-name",
                svg { class: "icon icon-sm fb-icon", "aria-hidden": "true",
                    r#use { href: "/icons.svg#{icon}" }
                }
                if is_dir {
                    button {
                        class: "fb-open",
                        r#type: "button",
                        title: "{name}",
                        onclick: move |_| on_navigate.call(open_path.clone()),
                        "{name}"
                    }
                } else {
                    span { class: "fb-file-name", title: "{name}", "{name}" }
                }
                if let Some(note) = note {
                    span { class: "fb-note-tag", "{note}" }
                }
            }
            div { class: "fb-cell fb-size", "{size}" }
            div { class: "fb-cell fb-modified", "{modified}" }
            div { class: "fb-cell fb-action",
                if !is_dir {
                    IconButton {
                        icon: "arrow-down-to-line",
                        label: format!("Download {name}"),
                        size: ControlSize::Sm,
                        tooltip: blocker.map_or_else(|| "Download".to_string(), str::to_string),
                        disabled: blocker.is_some(),
                        onclick: move |_| on_download.call(name.clone()),
                    }
                }
            }
        }
    }
}

/// The breadcrumb trail: every crumb but the last is a button.
#[component]
fn Crumbs(dir: String, on_navigate: EventHandler<String>) -> Element {
    let crumbs = breadcrumbs(&dir);
    let last = crumbs.len().saturating_sub(1);
    rsx! {
        nav { class: "fb-crumbs", "aria-label": "Folder",
            for (index, crumb) in crumbs.into_iter().enumerate() {
                if index > 0 {
                    svg { class: "icon icon-sm fb-crumb-sep", "aria-hidden": "true",
                        r#use { href: "/icons.svg#chevron-right" }
                    }
                }
                if index == last {
                    span { key: "{crumb.path}", class: "fb-crumb fb-crumb-here", "aria-current": "page", "{crumb.label}" }
                } else {
                    button {
                        key: "{crumb.path}",
                        class: "fb-crumb",
                        r#type: "button",
                        onclick: move |_| on_navigate.call(crumb.path.clone()),
                        "{crumb.label}"
                    }
                }
            }
        }
    }
}

/// One folder of a container. `listing_key` changes with every listing, so
/// the rows remount and scroll back to the top.
#[component]
pub fn FileBrowser(
    dir: String,
    listing: Listing,
    listing_key: u64,
    #[props(default)] busy: Option<String>,
    #[props(default)] notice: Option<String>,
    #[props(default)] upload_blocker: Option<String>,
    on_navigate: EventHandler<String>,
    on_download: EventHandler<String>,
    on_upload: EventHandler<()>,
    on_refresh: EventHandler<()>,
) -> Element {
    let dir = normalize_dir(&dir);
    let at_root = dir == "/";
    let up = parent_dir(&dir);
    let row_height_px = try_consume_context::<AppState>().map_or(28, |state| {
        let settings = state.settings.read();
        crate::text_scale::row_height_px(settings.density, settings.text_scale)
    });
    let now = Utc::now();

    let entries: Vec<FileEntry> = match &listing {
        Listing::Ready(entries) => entries.clone(),
        _ => Vec::new(),
    };
    let total = entries.len();
    let row_dir = dir.clone();
    let render_row = use_callback(move |i: usize| -> Element {
        let Some(entry) = entries.get(i) else {
            return rsx! {};
        };
        rsx! {
            FileRow {
                dir: row_dir.clone(),
                entry: entry.clone(),
                now,
                on_navigate,
                on_download,
            }
        }
    });

    let footer = match (&listing, &busy) {
        (_, Some(text)) => text.clone(),
        (Listing::Ready(_), None) => item_count(total),
        (Listing::Loading, None) => format!("Reading {dir}\u{2026}"),
        (Listing::Failed(_), None) => String::new(),
    };
    let goto_dir = dir.clone();
    rsx! {
        style { "{FILE_BROWSER_CSS}" }
        section { class: "fb", "aria-label": "Files",
            div { class: "fb-head",
                Crumbs { dir: dir.clone(), on_navigate }
                div { class: "fb-tools",
                    IconButton {
                        icon: "chevron-up",
                        label: "Up one folder".to_string(),
                        size: ControlSize::Sm,
                        tooltip: if at_root { "Already at the top".to_string() } else { format!("Up to {up}") },
                        disabled: at_root,
                        onclick: move |_| on_navigate.call(up.clone()),
                    }
                    IconButton {
                        icon: "rotate-cw",
                        label: "Refresh".to_string(),
                        size: ControlSize::Sm,
                        tooltip: "Read the folder again".to_string(),
                        loading: matches!(listing, Listing::Loading),
                        onclick: move |_| on_refresh.call(()),
                    }
                    Button {
                        label: "Upload".to_string(),
                        icon: "arrow-up",
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Sm,
                        disabled: upload_blocker.is_some(),
                        disabled_reason: upload_blocker.clone(),
                        onclick: move |_| on_upload.call(()),
                    }
                }
            }
            // The field keeps what was typed (D59); a new folder remounts it
            // with that folder's path.
            for key in [goto_dir.clone()] {
                div { key: "{key}", class: "fb-goto",
                    TextField {
                        initial_value: key.clone(),
                        aria_label: "Go to folder".to_string(),
                        prefix_icon: "folder",
                        placeholder: "/path/to/folder".to_string(),
                        onchange: move |e: FormEvent| {
                            let typed = e.value();
                            if !typed.trim().is_empty() {
                                on_navigate.call(normalize_dir(&typed));
                            }
                        },
                    }
                }
            }
            if let Some(message) = notice {
                div { class: "fb-notice",
                    InlineMessage { message, tone: InlineTone::Danger }
                }
            }
            match listing {
                Listing::Loading => rsx! {
                    div { class: "fb-state", role: "status", "aria-busy": "true",
                        "Reading {dir}\u{2026}"
                    }
                },
                Listing::Failed(message) => rsx! {
                    div { class: "fb-state fb-failed",
                        InlineMessage { message: format!("Could not read {dir}: {message}") }
                        Button {
                            label: "Retry".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            onclick: move |_| on_refresh.call(()),
                        }
                    }
                },
                Listing::Ready(_) if total == 0 => rsx! {
                    div { class: "fb-state",
                        EmptyState {
                            title: "This folder is empty".to_string(),
                            body: "Upload a file to put something here.".to_string(),
                            icon: "folder",
                        }
                    }
                },
                Listing::Ready(_) => rsx! {
                    div { class: "fb-table", role: "table",
                        div { class: "fb-cols", role: "row",
                            span { class: "fb-col", role: "columnheader", "Name" }
                            span { class: "fb-col fb-col-right", role: "columnheader", "Size" }
                            span { class: "fb-col", role: "columnheader", "Modified" }
                            span { class: "fb-col" }
                        }
                        for key in [listing_key] {
                            VirtualTable {
                                key: "{key}",
                                total,
                                render_row,
                                row_height_px,
                                viewport_height_px: 1100_u32,
                                overscan: 10_usize,
                            }
                        }
                    }
                },
            }
            div { class: "fb-foot", role: "status", "{footer}" }
        }
    }
}

#[cfg(test)]
#[path = "file_browser_tests.rs"]
mod tests;
