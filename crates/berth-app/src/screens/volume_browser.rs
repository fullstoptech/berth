//! The volume in the inspector (P4-T4): header with Back up, Restore and
//! Delete, tabs Overview and Files. Exported as one self-contained
//! component, `VolumeDetailPanel { name }`, that the shell mounts for
//! `Selection::Volume` (the same way the container detail is mounted).
//!
//! Files is the shared `FileBrowser` (through `ui::files_panel::TargetFiles`)
//! reading the volume through the helper container (D51). Before the first
//! read, backup or restore on an engine, a dialog says "Berth uses a small
//! helper image alpine:3 to read volumes" with its size and pulls it only
//! on a click. Back up writes a `.tar.gz` where the save panel points (a
//! direct export, AD15) and never stops a container. Restore writes into the
//! volume, so it asks with the volume's name typed (D33).
//!
//! All I/O is in `services::{volumes, volume_backup}` and runs on the I/O
//! runtime; this file draws signals and sends clicks.

use std::sync::Arc;

use berth_core::{EngineId, FileTarget, Subject};
use bytes::Bytes;
use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use crate::screens::volumes::{view_or_new, VOLUMES_CSS};
use crate::services::actions::{disabled_reason, dismiss, with_view};
use crate::services::confirm;
use crate::services::files::{on_io, EngineFiles, FileSource};
use crate::services::{volume_backup, volumes, Services};
use crate::state::{AppState, PendingKey, PendingState, Section, Selection, Toast, ToastTone};
use crate::ui::files_panel::TargetFiles;
use crate::ui::{
    Badge, BadgeKind, Button, ButtonVariant, ConfirmDialog, ControlSize, InlineMessage, InlineTone,
    KeyValue, KeyValueList, TabItem, Tabs,
};

/// The container header's classes (`ih-*`, `cd-*`) are shared.
const CONTAINER_DETAIL_CSS: &str = include_str!("../../assets/css/container_detail.css");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Files,
}

impl Tab {
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Files => "files",
        }
    }
}

/// Where the helper image stands on this volume's engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Helper {
    /// Not looked at yet.
    Unknown,
    Checking,
    /// The engine does not have it: the dialog asks.
    Missing,
    /// Being downloaded, 0 to 100.
    Pulling(u8),
    Ready,
    Failed(String),
}

/// What a scene says the helper's state already is (no engine to ask).
#[derive(Clone)]
pub struct HelperPreset(pub Helper);

/// What waits for the helper image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    Files,
    Backup,
    Restore,
}

/// A transfer or restore in progress, in words.
#[derive(Debug, Clone, PartialEq)]
struct Busy {
    text: String,
}

/// A backup file read and counted, waiting for the typed name.
#[derive(Debug, Clone, PartialEq)]
struct PendingRestore {
    file: String,
    files: u64,
    bytes: u64,
    tar: Bytes,
}

/// The file name the save panel offers: `<volume>-<yyyymmdd>.tar.gz`.
#[must_use]
pub fn backup_file_name(volume: &str, today: chrono::NaiveDate) -> String {
    format!("{volume}-{}.tar.gz", today.format("%Y%m%d"))
}

fn push_toast(mut state: AppState, text: String, tone: ToastTone) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone,
        action: None,
    });
}

/// Why nothing can act on a volume the lists no longer have.
const GONE: &str = "This volume no longer exists";

fn client_of(
    services: Option<&Services>,
    engine: Option<&EngineId>,
) -> Result<Arc<dyn berth_engine::EngineClient>, String> {
    let engine = engine.ok_or_else(|| GONE.to_string())?;
    services
        .and_then(|s| s.client(engine))
        .ok_or_else(|| format!("Engine {} is not connected", engine.0))
}

fn source_of(
    services: Option<&Services>,
    engine: Option<&EngineId>,
    volume: &str,
) -> Result<Arc<dyn FileSource>, String> {
    let client = client_of(services, engine)?;
    Ok(Arc::new(EngineFiles::for_target(
        client,
        FileTarget::Volume(volume.to_string()),
    )))
}

/// The Overview rows: what Docker knows about the volume.
#[must_use]
pub fn overview_rows(
    volume: &berth_core::Volume,
    engine_name: Option<&str>,
    project: Option<&str>,
    size: Option<bytesize::ByteSize>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<KeyValue> {
    let created = volume.created.map_or_else(String::new, |at| {
        format!(
            "{} ({})",
            berth_core::fmt::age(at, now),
            at.with_timezone(&chrono::Local).format("%b %-d %Y %H:%M")
        )
    });
    vec![
        KeyValue::mono("Name", volume.name.clone()),
        KeyValue::text("Driver", volume.driver.clone()),
        KeyValue::text("Engine", engine_name.unwrap_or_default()).empty_text("Not known"),
        KeyValue::text("Project", project.unwrap_or_default()).empty_text("\u{2014}"),
        KeyValue::text(
            "Size",
            size.map_or_else(|| "Calculating\u{2026}".to_string(), berth_core::fmt::bytes),
        ),
        KeyValue::text("Created", created).empty_text("Not known"),
        KeyValue::mono("Mountpoint", volume.mountpoint.clone()),
    ]
}

#[component]
pub fn VolumeDetailPanel(name: String) -> Element {
    let mut state = use_context::<AppState>();
    let view = use_hook(view_or_new);
    let services = try_consume_context::<Services>();

    let initial = if view.browse.peek().as_deref() == Some(name.as_str()) {
        Tab::Files
    } else {
        Tab::Overview
    };
    let mut tab = use_signal(move || initial);
    let mut helper =
        use_signal(|| try_consume_context::<HelperPreset>().map_or(Helper::Unknown, |p| p.0));
    let mut busy = use_signal(|| None::<Busy>);
    let mut notice = use_signal(|| None::<String>);
    let mut after = use_signal(|| None::<Next>);
    let mut helper_dialog = use_signal(|| false);
    let mut restore_ask = use_signal(|| None::<PendingRestore>);
    let panel_cancel = use_hook(|| {
        services
            .as_ref()
            .map_or_else(CancellationToken::new, |s| s.cancel.child_token())
    });
    {
        let cancel = panel_cancel.clone();
        use_drop(move || cancel.cancel());
    }

    // The volume as the lists have it, with what the screen derives from it.
    let found = state
        .volumes
        .read()
        .iter()
        .find(|v| v.name == name)
        .cloned();
    let owner = found.as_ref().map(|v| v.engine.clone());
    let engine_name = owner.as_ref().and_then(|id| {
        state
            .engines
            .read()
            .iter()
            .find(|e| e.id == *id)
            .map(|e| e.name.clone())
    });
    let users: Vec<(berth_core::ContainerId, String)> = found
        .as_ref()
        .map(|v| {
            let containers = state.containers.read();
            v.used_by
                .iter()
                .map(|id| {
                    let label = containers
                        .iter()
                        .find(|c| c.id == *id)
                        .map_or_else(|| id.0.chars().take(12).collect(), |c| c.name.clone());
                    (id.clone(), label)
                })
                .collect()
        })
        .unwrap_or_default();
    let project = found.as_ref().and_then(|v| {
        v.project.as_ref().and_then(|id| {
            state
                .projects
                .read()
                .iter()
                .find(|p| p.id == *id)
                .map(|p| p.name.clone())
        })
    });
    let size = found
        .as_ref()
        .and_then(|v| v.size.or_else(|| view.sizes.read().get(&v.name).copied()));

    // --- the helper gate -------------------------------------------------
    let run_after = use_callback({
        let name = name.clone();
        let owner = owner.clone();
        let services = services.clone();
        let cancel = panel_cancel.clone();
        move |next: Next| match next {
            Next::Files => {}
            Next::Backup => {
                let (name, owner, services, cancel) = (
                    name.clone(),
                    owner.clone(),
                    services.clone(),
                    cancel.clone(),
                );
                spawn(async move {
                    notice.set(None);
                    let default = backup_file_name(&name, chrono::Local::now().date_naive());
                    let Some(target) = rfd::AsyncFileDialog::new()
                        .set_title(format!("Back up {name}"))
                        .set_file_name(default)
                        .save_file()
                        .await
                    else {
                        return;
                    };
                    let dest = target.path().to_path_buf();
                    let file = target.file_name();
                    busy.set(Some(Busy {
                        text: format!("Backing up {name}\u{2026}"),
                    }));
                    let result = match source_of(services.as_ref(), owner.as_ref(), &name) {
                        Ok(source) => on_io(volume_backup::backup(source, dest, cancel))
                            .await
                            .unwrap_or_else(|| Err("Cancelled".to_string())),
                        Err(reason) => Err(reason),
                    };
                    busy.set(None);
                    match result {
                        Ok(report) => push_toast(
                            state,
                            format!(
                                "Backed up {name} to {file} ({})",
                                berth_core::fmt::bytes(bytesize::ByteSize::b(report.bytes))
                            ),
                            ToastTone::Success,
                        ),
                        Err(reason) if reason == "Cancelled" => {}
                        Err(reason) => {
                            notice.set(Some(format!("Could not back up {name}: {reason}")))
                        }
                    }
                });
            }
            Next::Restore => {
                let name = name.clone();
                spawn(async move {
                    notice.set(None);
                    let Some(picked) = rfd::AsyncFileDialog::new()
                        .set_title(format!("Restore {name} from a backup"))
                        .add_filter("Backup", &["gz", "tgz", "tar"])
                        .pick_file()
                        .await
                    else {
                        return;
                    };
                    let file = picked.file_name();
                    busy.set(Some(Busy {
                        text: format!("Reading {file}\u{2026}"),
                    }));
                    let read = on_io(volume_backup::read_backup(picked.path().to_path_buf()))
                        .await
                        .unwrap_or_else(|| Err("Cancelled".to_string()));
                    busy.set(None);
                    match read {
                        Ok(backup) => {
                            restore_ask.set(Some(PendingRestore {
                                file,
                                files: backup.info.files,
                                bytes: backup.info.bytes,
                                tar: backup.tar,
                            }));
                        }
                        Err(reason) => notice.set(Some(format!("Could not read {file}: {reason}"))),
                    }
                });
            }
        }
    });

    let ensure = use_callback({
        let owner = owner.clone();
        let services = services.clone();
        move |next: Next| {
            let current = helper.peek().clone();
            match current {
                Helper::Ready => run_after.call(next),
                Helper::Checking | Helper::Pulling(_) => after.set(Some(next)),
                Helper::Unknown | Helper::Missing | Helper::Failed(_) => {
                    let client = match client_of(services.as_ref(), owner.as_ref()) {
                        Ok(client) => client,
                        Err(reason) => {
                            helper.set(Helper::Failed(reason));
                            return;
                        }
                    };
                    after.set(Some(next));
                    helper.set(Helper::Checking);
                    spawn(async move {
                        match volumes::helper_present_io(client).await {
                            Ok(true) => {
                                helper.set(Helper::Ready);
                                let next = after.peek().as_ref().copied();
                                after.set(None);
                                if let Some(next) = next {
                                    run_after.call(next);
                                }
                            }
                            Ok(false) => {
                                helper.set(Helper::Missing);
                                helper_dialog.set(true);
                            }
                            Err(reason) => helper.set(Helper::Failed(reason)),
                        }
                    });
                }
            }
        }
    });

    // The Files tab asks for the helper the first time it is shown.
    use_effect(move || {
        if *tab.read() == Tab::Files && matches!(*helper.peek(), Helper::Unknown) {
            ensure.call(Next::Files);
        }
    });

    let download_helper = {
        let owner = owner.clone();
        let services = services.clone();
        let cancel = panel_cancel.clone();
        move |()| {
            helper_dialog.set(false);
            let client = match client_of(services.as_ref(), owner.as_ref()) {
                Ok(client) => client,
                Err(reason) => {
                    helper.set(Helper::Failed(reason));
                    return;
                }
            };
            helper.set(Helper::Pulling(0));
            let cancel = cancel.clone();
            spawn(async move {
                let result = volumes::pull_helper_io(client, cancel, |p| {
                    let mut helper = helper;
                    helper.set(Helper::Pulling(p));
                })
                .await;
                match result {
                    Ok(()) => {
                        helper.set(Helper::Ready);
                        let next = after.peek().as_ref().copied();
                        after.set(None);
                        if let Some(next) = next {
                            run_after.call(next);
                        }
                    }
                    Err(reason) => {
                        after.set(None);
                        helper.set(Helper::Failed(reason.clone()));
                        notice.set(Some(format!(
                            "Could not download {}: {reason}",
                            volumes::HELPER_IMAGE
                        )));
                    }
                }
            });
        }
    };

    let confirm_restore = {
        let name = name.clone();
        let owner = owner.clone();
        let services = services.clone();
        move |()| {
            let Some(ask) = restore_ask.peek().clone() else {
                return;
            };
            restore_ask.set(None);
            let (name, owner, services) = (name.clone(), owner.clone(), services.clone());
            let effect = confirm::restore_effect(
                &name,
                &ask.file,
                ask.files,
                bytesize::ByteSize::b(ask.bytes),
                &[],
            );
            spawn(async move {
                busy.set(Some(Busy {
                    text: format!("Restoring {name}\u{2026}"),
                }));
                let result = match source_of(services.as_ref(), owner.as_ref(), &name) {
                    Ok(source) => on_io(volume_backup::restore(source, ask.tar))
                        .await
                        .unwrap_or_else(|| Err("Cancelled".to_string())),
                    Err(reason) => Err(reason),
                };
                busy.set(None);
                match result {
                    Ok(()) => {
                        push_toast(state, confirm::result_label(&effect, 1), ToastTone::Success)
                    }
                    Err(reason) => notice.set(Some(format!("Could not restore {name}: {reason}"))),
                }
            });
        }
    };

    // --- the view --------------------------------------------------------
    let Some(volume) = found else {
        return rsx! {
            style { "{VOLUMES_CSS}" }
            div { class: "vd-note", "{name} no longer exists." }
        };
    };
    let subject = Subject::Volume(name.clone());
    let delete_reason = with_view(&state, |v| disabled_reason(&subject, "volume.delete", v));
    let failure: Option<(PendingKey, String)> =
        state
            .pending
            .read()
            .iter()
            .find_map(|(key, value)| match value {
                PendingState::Failed { message } if key.subject == subject => {
                    Some((key.clone(), message.clone()))
                }
                _ => None,
            });
    let working = busy.read().clone();
    let busy_reason = working
        .as_ref()
        .map(|b| format!("{} is still running", b.text));
    let owner_reason = owner.is_none().then(|| GONE.to_string());
    // A mock engine cannot read or write a volume's files (backup, restore).
    let transfer_reason = state
        .backing
        .peek()
        .unsupported_action("volume.backup")
        .or_else(|| busy_reason.clone())
        .or(owner_reason);
    let now = chrono::Utc::now();
    let unused = volume.used_by.is_empty();
    let rows = overview_rows(
        &volume,
        engine_name.as_deref(),
        project.as_deref(),
        size,
        now,
    );
    let created_note = volume
        .created
        .map(|at| format!("created {}", berth_core::fmt::age(at, now)))
        .unwrap_or_default();
    let label_rows: Vec<KeyValue> = volume
        .labels
        .iter()
        .map(|(k, v)| KeyValue::mono(k.clone(), v.clone()))
        .collect();
    let items = vec![
        TabItem::new(Tab::Overview.id(), "Overview"),
        TabItem::new(Tab::Files.id(), "Files"),
    ];
    let selected_tab = *tab.read();
    let helper_now = helper.read().clone();
    let notice_now = notice.read().clone();
    let ask_now = restore_ask.read().clone();
    let restore_dialog = ask_now.as_ref().map(|ask| {
        (
            confirm::restore_title(&name),
            confirm::restore_effect(
                &name,
                &ask.file,
                ask.files,
                bytesize::ByteSize::b(ask.bytes),
                &users.iter().map(|(_, n)| n.clone()).collect::<Vec<_>>(),
            ),
        )
    });
    let delete_name = name.clone();

    rsx! {
        style { "{CONTAINER_DETAIL_CSS}" }
        style { "{VOLUMES_CSS}" }
        div { class: "cd vd",
            header { class: "ih",
                div { class: "ih-title",
                    h2 { class: "ih-name", title: "{name}", "{name}" }
                }
                div { class: "vd-badges",
                    Badge {
                        kind: BadgeKind::Tag,
                        text: if unused { "Unused".to_string() } else { format!("In use by {}", users.len()) },
                    }
                    span { class: "ih-note", "{volume.driver} \u{b7} {created_note}" }
                }
                div { class: "ih-buttons",
                    span { class: "ih-btn",
                        Button {
                            label: "Back up".to_string(),
                            icon: "arrow-down-to-line",
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            disabled: transfer_reason.is_some(),
                            disabled_reason: transfer_reason.clone(),
                            onclick: move |_| ensure.call(Next::Backup),
                        }
                    }
                    span { class: "ih-btn",
                        Button {
                            label: "Restore\u{2026}".to_string(),
                            icon: "arrow-up",
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            disabled: transfer_reason.is_some(),
                            disabled_reason: transfer_reason.clone(),
                            onclick: move |_| ensure.call(Next::Restore),
                        }
                    }
                    span { class: "ih-btn ih-btn-danger",
                        Button {
                            label: "Delete\u{2026}".to_string(),
                            icon: "trash-2",
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            disabled: delete_reason.is_some(),
                            disabled_reason: delete_reason.clone(),
                            onclick: move |_| {
                                confirm::open_delete_volume(&mut state, &delete_name);
                            },
                        }
                    }
                }
            }
            if let Some((key, message)) = failure {
                div { class: "vd-failure",
                    InlineMessage { message, tone: InlineTone::Danger }
                    Button {
                        label: "Dismiss".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        onclick: move |_| dismiss(&mut state, &key),
                    }
                }
            }
            if let Some(message) = notice_now {
                div { class: "vd-failure",
                    InlineMessage { message, tone: InlineTone::Danger }
                }
            }
            if let Some(work) = working {
                div { class: "vd-busy", role: "status",
                    "{work.text}"
                    div { class: "vd-bar", div { class: "vd-bar-fill", style: "width: 100%;" } }
                }
            }
            div { class: "cd-tabs",
                Tabs {
                    items,
                    selected: selected_tab.id().to_string(),
                    aria_label: "Volume detail".to_string(),
                    on_select: move |id: String| {
                        let next = if id == Tab::Files.id() { Tab::Files } else { Tab::Overview };
                        tab.set(next);
                    },
                }
            }
            div { class: "cd-body", role: "tabpanel",
                match selected_tab {
                    Tab::Overview => rsx! {
                        div { class: "vd-body",
                            KeyValueList { rows }
                            h3 { class: "vd-section", "Used by" }
                            if users.is_empty() {
                                p { class: "vd-note", "No container uses this volume." }
                            } else {
                                ul { class: "vd-users",
                                    for (id, label) in users.clone() {
                                        li { key: "{id.0}",
                                            button {
                                                class: "vd-user",
                                                r#type: "button",
                                                onclick: move |_| {
                                                    state.section.set(Section::Containers);
                                                    state.selection.set(Selection::Containers(vec![id.clone()]));
                                                },
                                                "{label}"
                                            }
                                        }
                                    }
                                }
                            }
                            if !label_rows.is_empty() {
                                h3 { class: "vd-section", "Labels" }
                                KeyValueList { rows: label_rows }
                            }
                        }
                    },
                    Tab::Files => rsx! {
                        div { class: "vd-files",
                            match helper_now {
                                Helper::Ready => rsx! {
                                    if let Some(engine) = owner.clone() {
                                        TargetFiles {
                                            key: "{name}",
                                            engine,
                                            target: FileTarget::Volume(name.clone()),
                                            start: "/".to_string(),
                                        }
                                    }
                                },
                                Helper::Unknown | Helper::Checking => rsx! {
                                    div { class: "vd-note", role: "status", "Checking for {volumes::HELPER_IMAGE}\u{2026}" }
                                },
                                Helper::Pulling(percent) => rsx! {
                                    div { class: "vd-busy", role: "status",
                                        "Downloading {volumes::HELPER_IMAGE}\u{2026} {percent}%"
                                        div { class: "vd-bar",
                                            div { class: "vd-bar-fill", style: "width: {percent}%;" }
                                        }
                                    }
                                },
                                Helper::Missing => rsx! {
                                    div { class: "vd-helper",
                                        p { "{confirm::helper_title()}." }
                                        Button {
                                            label: format!("Download {}", volumes::HELPER_IMAGE),
                                            icon: "arrow-down-to-line",
                                            variant: ButtonVariant::Secondary,
                                            size: ControlSize::Sm,
                                            onclick: move |_| {
                                                after.set(Some(Next::Files));
                                                helper_dialog.set(true);
                                            },
                                        }
                                    }
                                },
                                Helper::Failed(reason) => rsx! {
                                    div { class: "vd-helper",
                                        InlineMessage { message: format!("Could not read the volume: {reason}"), tone: InlineTone::Danger }
                                        Button {
                                            label: "Try again".to_string(),
                                            variant: ButtonVariant::Secondary,
                                            size: ControlSize::Sm,
                                            onclick: move |_| {
                                                helper.set(Helper::Unknown);
                                                ensure.call(Next::Files);
                                            },
                                        }
                                    }
                                },
                            }
                        }
                    },
                }
            }
            if *helper_dialog.read() {
                div {
                    class: "overlay-scrim",
                    onclick: move |_| {
                        helper_dialog.set(false);
                        after.set(None);
                    },
                    ConfirmDialog {
                        title: confirm::helper_title(),
                        effect: confirm::helper_effect(),
                        confirm_variant: ButtonVariant::Primary,
                        oncheck: move |_| {},
                        oncancel: move |()| {
                            helper_dialog.set(false);
                            after.set(None);
                        },
                        onconfirm: download_helper,
                    }
                }
            }
            if let Some((title, effect)) = restore_dialog {
                div {
                    class: "overlay-scrim",
                    onclick: move |_| restore_ask.set(None),
                    ConfirmDialog {
                        title,
                        effect,
                        oncheck: move |_| {},
                        oncancel: move |()| restore_ask.set(None),
                        onconfirm: confirm_restore,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "volume_browser_tests.rs"]
mod tests;
