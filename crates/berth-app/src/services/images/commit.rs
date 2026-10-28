//! Save a container as an image (P8-T4): `docker commit` behind the ⋯ menu's
//! "Save as image". The engine pauses the container while it commits, so
//! nothing half-written lands in the image.
//!
//! Like the other image operations it takes a child of the services'
//! `CancellationToken`, shows as a pending entry on the image while it runs,
//! writes one Activity entry when it ends and re-lists the engines' images.
//! The container's environment is part of the image the engine makes, so the
//! sheet says that in its help text; Berth neither reads nor logs a value.

use berth_core::{Container, EngineId};

use super::{check_reference, io, with_default_tag, ImageStore};
use crate::services::Services;

/// The Activity entry's action id and the pending key's.
pub const COMMIT_ACTION: &str = "image.commit";

/// `repo` and `tag` of what the user typed in the Save as image field:
/// `berthfx/web-snap:1`. A missing tag means `latest`, as it does for
/// `docker commit`; a digest names no new image, so it is refused.
///
/// # Errors
/// A sentence saying what is wrong with the reference.
pub fn commit_reference(text: &str) -> Result<(String, String), String> {
    let checked = check_reference(text)?;
    if checked.contains('@') {
        return Err("Name the new image with a tag such as my-app:1, not a digest".to_string());
    }
    let full = with_default_tag(&checked);
    // `check_reference` guarantees a tag only after the last `/`; `with_default_tag` adds one.
    full.rsplit_once(':')
        .map(|(repo, tag)| (repo.to_string(), tag.to_string()))
        .ok_or_else(|| "Name the new image with a tag such as my-app:1".to_string())
}

impl ImageStore {
    /// Commit `container` as the image `reference` (`repo:tag`). `done` hears
    /// the full reference, or the reason (`"Cancelled"` after
    /// [`Self::cancel_transfer`] with [`COMMIT_ACTION`]). The image lists are
    /// read again after a success.
    pub fn save_as_image(
        &self,
        services: &Services,
        container: &Container,
        reference: &str,
        done: impl FnOnce(Result<String, String>) + 'static,
    ) {
        let (repo, tag) = match commit_reference(reference) {
            Ok(parts) => parts,
            Err(reason) => {
                done(Err(reason));
                return;
            }
        };
        let engine: EngineId = container.engine.clone();
        let Some(client) = services.client(&engine) else {
            done(Err(format!("Engine {} is not connected", engine.0)));
            return;
        };
        let name = format!("{repo}:{tag}");
        let cancel = self.begin_op(
            services,
            &engine,
            COMMIT_ACTION,
            &name,
            format!("Saving {} as {name}", container.name),
        );
        let app = dioxus::prelude::try_consume_context::<crate::state::AppState>();
        let store = *self;
        let services = services.clone();
        let id = container.id.clone();
        let work_cancel = cancel.clone();
        io(
            async move {
                tokio::select! {
                    biased;
                    () = work_cancel.cancelled() => Err("Cancelled".to_string()),
                    made = client.commit(&id, &repo, &tag) => {
                        made.map(|_| ()).map_err(|e| e.to_string())
                    }
                }
            },
            move |result: Result<(), String>| {
                store.end_op(app, &engine, COMMIT_ACTION, &name);
                if !cancel.is_cancelled() {
                    Self::record(
                        &services,
                        &engine,
                        COMMIT_ACTION,
                        &name,
                        super::build::outcome_of(&result),
                    );
                }
                if result.is_ok() {
                    store.refresh(&services);
                }
                done(result.map(|()| name));
            },
        );
    }
}

/// The reference the Save as image field starts with: the container's name
/// made into a valid repository, then `-snap:1` (`shop-api-web-1` becomes
/// `shop-api-web-1-snap:1`).
#[must_use]
pub fn default_reference(container_name: &str) -> String {
    let repo: String = container_name
        .trim_start_matches('/')
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let repo = repo.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    if repo.is_empty() {
        "container-snap:1".to_string()
    } else {
        format!("{repo}-snap:1")
    }
}

#[cfg(test)]
#[path = "commit_tests.rs"]
mod tests;
