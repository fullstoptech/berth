//! The container Files tab (P3-T8): `FileBrowser` fed by
//! `services::files`. Mount it with a container id:
//!
//! ```ignore
//! crate::ui::FilesPanel { container: c.id.clone() }
//! ```
//!
//! Folders are read through the archive API, so it works on a stopped
//! container and without a shell. Download opens the save panel first, then
//! reads the file and writes it where the panel pointed. Upload opens the
//! file panel, then asks (`confirm::upload_effect`) because a file of the
//! same name is overwritten; only a click on the dialog's button writes.
//! Needs `AppState`; for live data a `Services` in context, or a
//! `FilesBackend` (scenes and tests).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use berth_core::{ContainerId, EngineId, FileTarget, Mount};
use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use super::file_browser::{FileBrowser, Listing, FILE_BROWSER_CSS};
use super::{ConfirmDialog, InlineMessage, InlineTone};
use crate::services::confirm;
use crate::services::files::{self, EngineFiles, FileSource, FilesBackend, FilesError};
use crate::services::Services;
use crate::state::{AppState, Toast, ToastTone};

/// Where the tab opens: the first mounted folder (that is where a
/// container keeps its data), else the root. Sockets are not folders.
#[must_use]
pub fn start_dir(mounts: &[Mount]) -> String {
    mounts
        .iter()
        .map(|m| m.destination.as_str())
        .find(|d| d.starts_with('/') && !d.ends_with(".sock"))
        .map_or_else(|| "/".to_string(), files::normalize_dir)
}

/// A file waiting for the dialog's answer.
struct PendingUpload {
    dir: String,
    name: String,
    contents: Arc<Vec<u8>>,
    replaces: bool,
}

/// Where a panel finds its files, resolved at the moment of use so an
/// engine that connects late is picked up.
#[derive(Clone)]
struct Backend {
    over: Option<FilesBackend>,
    services: Option<Services>,
    engine: EngineId,
    target: FileTarget,
}

impl Backend {
    fn source(&self) -> Result<Arc<dyn FileSource>, FilesError> {
        if let Some(over) = &self.over {
            return Ok(Arc::clone(&over.0));
        }
        let services = self
            .services
            .as_ref()
            .ok_or_else(|| FilesError::Engine("Files are not connected".to_string()))?;
        let client = services.client(&self.engine).ok_or_else(|| {
            FilesError::Engine(format!("Engine {} is not connected", self.engine.0))
        })?;
        Ok(Arc::new(EngineFiles::for_target(
            client,
            self.target.clone(),
        )))
    }
}

fn push_toast(mut state: AppState, text: String, tone: ToastTone) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone,
        action: None,
    });
}

#[component]
pub fn FilesPanel(container: ContainerId) -> Element {
    let state = use_context::<AppState>();
    let found = state
        .containers
        .read()
        .iter()
        .find(|c| c.id == container)
        .map(|c| (c.engine.clone(), start_dir(&c.mounts), c.name.clone()));
    let Some((engine, start, _)) = found else {
        let name = container.0.clone();
        return rsx! {
            style { "{FILE_BROWSER_CSS}" }
            InlineMessage { message: format!("{name} was removed."), tone: InlineTone::Warning }
        };
    };
    rsx! {
        TargetFiles { engine, target: FileTarget::Container(container), start }
    }
}

/// The files of any target (a container, or a volume through the helper),
/// starting in `start`. The Files tab and the volume browser mount this.
#[component]
pub fn TargetFiles(engine: EngineId, target: FileTarget, start: String) -> Element {
    let state = use_context::<AppState>();
    let backend = use_hook({
        move || Backend {
            over: try_consume_context::<FilesBackend>(),
            services: try_consume_context::<Services>(),
            engine,
            target,
        }
    });
    let panel_cancel = use_hook(|| {
        try_consume_context::<Services>()
            .map_or_else(CancellationToken::new, |s| s.cancel.child_token())
    });
    {
        let cancel = panel_cancel.clone();
        use_drop(move || cancel.cancel());
    }
    let load_token = use_hook(|| Rc::new(RefCell::new(CancellationToken::new())));

    let mut dir = use_signal(move || start);
    let mut reload = use_signal(|| 0_u32);
    let mut listing = use_signal(|| Listing::Loading);
    let mut listing_key = use_signal(|| 0_u64);
    let mut busy = use_signal(|| None::<String>);
    let mut notice = use_signal(|| None::<String>);
    let mut pending = use_signal(|| None::<PendingUpload>);

    // Read the folder again whenever it, or the reload counter, changes.
    let _loader = use_resource({
        let backend = backend.clone();
        let cancel = panel_cancel.clone();
        let load_token = Rc::clone(&load_token);
        move || {
            let target = dir.read().clone();
            let _tick = *reload.read();
            let backend = backend.clone();
            let cancel = cancel.clone();
            let load_token = Rc::clone(&load_token);
            async move {
                listing.set(Listing::Loading);
                let source = match backend.source() {
                    Ok(source) => source,
                    Err(err) => {
                        listing.set(Listing::Failed(err.to_string()));
                        return;
                    }
                };
                let token = cancel.child_token();
                {
                    let mut slot = load_token.borrow_mut();
                    slot.cancel();
                    *slot = token.clone();
                }
                match files::list_dir(source, target, token).await {
                    Ok(entries) => {
                        *listing_key.write() += 1;
                        listing.set(Listing::Ready(entries));
                    }
                    Err(FilesError::Cancelled) => {}
                    Err(err) => listing.set(Listing::Failed(err.to_string())),
                }
            }
        }
    });

    let on_navigate = move |path: String| {
        notice.set(None);
        dir.set(files::normalize_dir(&path));
    };
    let on_refresh = move |()| {
        notice.set(None);
        *reload.write() += 1;
    };

    let on_download = {
        let backend = backend.clone();
        let cancel = panel_cancel.clone();
        move |file: String| {
            if busy.peek().is_some() {
                return;
            }
            let backend = backend.clone();
            let token = cancel.child_token();
            let path = files::join_path(&dir.peek(), &file);
            spawn(async move {
                notice.set(None);
                let Some(target) = rfd::AsyncFileDialog::new()
                    .set_title("Download file")
                    .set_file_name(&file)
                    .save_file()
                    .await
                else {
                    return;
                };
                busy.set(Some(format!("Downloading {file}\u{2026}")));
                let result = async {
                    let source = backend.source()?;
                    let contents = files::read_file(source, path, token).await?;
                    let size = contents.len() as u64;
                    files::save_local(target.path().to_path_buf(), contents).await?;
                    Ok::<u64, FilesError>(size)
                }
                .await;
                busy.set(None);
                match result {
                    Ok(size) => push_toast(
                        state,
                        format!(
                            "Saved {file} ({})",
                            berth_core::fmt::bytes(bytesize::ByteSize::b(size))
                        ),
                        ToastTone::Success,
                    ),
                    Err(FilesError::Cancelled) => {}
                    Err(err) => notice.set(Some(format!("Could not download {file}: {err}"))),
                }
            });
        }
    };

    let on_upload = move |()| {
        if busy.peek().is_some() || pending.peek().is_some() {
            return;
        }
        let target_dir = dir.peek().clone();
        spawn(async move {
            notice.set(None);
            let Some(picked) = rfd::AsyncFileDialog::new()
                .set_title(format!("Upload to {target_dir}"))
                .pick_file()
                .await
            else {
                return;
            };
            let file = picked.file_name();
            busy.set(Some(format!("Reading {file}\u{2026}")));
            let loaded = files::load_local(picked.path().to_path_buf()).await;
            busy.set(None);
            let contents = match loaded {
                Ok(contents) => contents,
                Err(err) => {
                    notice.set(Some(format!("Could not upload {file}: {err}")));
                    return;
                }
            };
            if !files::valid_file_name(&file) {
                notice.set(Some(format!(
                    "Could not upload {file}: {}",
                    FilesError::InvalidName(file.clone())
                )));
                return;
            }
            let replaces = matches!(
                &*listing.peek(),
                Listing::Ready(entries) if entries.iter().any(|e| e.name == file)
            );
            pending.set(Some(PendingUpload {
                dir: target_dir,
                name: file,
                contents: Arc::new(contents),
                replaces,
            }));
        });
    };

    let on_confirm_upload = {
        let backend = backend.clone();
        move |()| {
            let Some(upload) = pending.write().take() else {
                return;
            };
            let backend = backend.clone();
            let effect = confirm::upload_effect(
                &upload.dir,
                &upload.name,
                bytesize::ByteSize::b(upload.contents.len() as u64),
                upload.replaces,
            );
            spawn(async move {
                busy.set(Some(format!("Uploading {}\u{2026}", upload.name)));
                let contents = Arc::try_unwrap(upload.contents).unwrap_or_else(|a| (*a).clone());
                let result = async {
                    let source = backend.source()?;
                    files::write_file(
                        source,
                        upload.dir.clone(),
                        upload.name.clone(),
                        contents,
                        chrono::Utc::now(),
                    )
                    .await
                }
                .await;
                busy.set(None);
                match result {
                    Ok(()) => {
                        push_toast(state, confirm::result_label(&effect, 1), ToastTone::Success);
                        if *dir.peek() == upload.dir {
                            *reload.write() += 1;
                        }
                    }
                    Err(err) => {
                        notice.set(Some(format!("Could not upload {}: {err}", upload.name)));
                    }
                }
            });
        }
    };

    let upload_blocker = if busy.read().is_some() {
        Some("Wait for the current transfer to finish".to_string())
    } else {
        None
    };
    let dialog = pending.read().as_ref().map(|upload| {
        (
            confirm::upload_title(&upload.dir, &upload.name),
            confirm::upload_effect(
                &upload.dir,
                &upload.name,
                bytesize::ByteSize::b(upload.contents.len() as u64),
                upload.replaces,
            ),
        )
    });
    rsx! {
        FileBrowser {
            dir: dir.read().clone(),
            listing: listing.read().clone(),
            listing_key: *listing_key.read(),
            busy: busy.read().clone(),
            notice: notice.read().clone(),
            upload_blocker,
            on_navigate,
            on_download,
            on_upload,
            on_refresh,
        }
        if let Some((title, effect)) = dialog {
            div {
                class: "overlay-scrim",
                onclick: move |_| pending.set(None),
                ConfirmDialog {
                    title,
                    effect,
                    oncheck: move |_| {},
                    oncancel: move |()| pending.set(None),
                    onconfirm: on_confirm_upload,
                }
            }
        }
    }
}

/// The "Files / panel" dev scene: the panel at the inspector's width,
/// under the container's name.
#[component]
pub fn FilesPanelScene(id: ContainerId) -> Element {
    let state = use_context::<AppState>();
    let name = state
        .containers
        .read()
        .iter()
        .find(|c| c.id == id)
        .map_or_else(|| id.0.clone(), |c| c.name.clone());
    rsx! {
        style { "{FILE_BROWSER_CSS}" }
        div { class: "dev-files-scene",
            h2 { "Files / panel" }
            p { class: "dev-files-name", "{name}" }
            aside { class: "dev-files-card", FilesPanel { container: id } }
        }
    }
}
