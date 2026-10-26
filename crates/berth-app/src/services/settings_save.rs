//! Saving settings (P6-T2): every change to `AppState::settings` reaches the store, in order,
//! off the UI path.
//!
//! The App root calls [`use_save_settings`]. A change is one small write; a burst (a text-size
//! key held down, the inspector drag) collapses to the latest value because one task owns the
//! writes and reads the newest settings each time it wakes, so an older write can never land
//! after a newer one. What is not saved: the values loaded at start (nothing changed), anything
//! while a development override is in force (`BERTH_TEXT_SCALE`, `BERTH_INSPECTOR_WIDTH`,
//! `BERTH_THEME`: a forced value must not become the owner's setting) and scenes.

use std::sync::Arc;

use dioxus::prelude::*;

use berth_store::Settings;

use crate::state::AppState;

/// Decides, call by call, whether the settings it is shown need writing.
#[derive(Debug, Default)]
pub struct Tracker {
    last: Option<Settings>,
}

impl Tracker {
    /// `true` when `now` differs from what was last shown and must be written. The first value
    /// shown is the one loaded at start and is never written; while `suppressed`, changes are
    /// followed but not written.
    pub fn changed(&mut self, now: &Settings, suppressed: bool) -> bool {
        let first = self.last.is_none();
        let differs = self.last.as_ref() != Some(now);
        if differs {
            self.last = Some(now.clone());
        }
        differs && !first && !suppressed
    }
}

/// Is a development override in force (or a scene)? Then nothing is written.
#[must_use]
pub fn suppressed() -> bool {
    crate::text_scale::forced_from_env().is_some()
        || crate::ui::dock::forced_width_from_env().is_some()
        || std::env::var_os("BERTH_THEME").is_some()
        || std::env::var_os("BERTH_SCENE").is_some()
}

/// Write `settings` now; the blocking half of the saver (also what tests call).
///
/// # Errors
/// The store refused the write.
pub fn save_now(
    store: &berth_store::Store,
    settings: &Settings,
) -> Result<(), berth_store::StoreError> {
    store.set_settings(settings)
}

/// Save every change of `state.settings` through `store`. Call once from the App root.
pub fn use_save_settings(state: AppState, store: Arc<berth_store::Store>) {
    let tracker = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(Tracker::default())));
    // One writer task: takes the newest value each time it wakes.
    let tx = use_hook(|| {
        let (tx, mut rx) = tokio::sync::watch::channel::<Option<Settings>>(None);
        spawn(async move {
            while rx.changed().await.is_ok() {
                let Some(settings) = rx.borrow_and_update().clone() else {
                    continue;
                };
                let store = Arc::clone(&store);
                let saved = tokio::task::spawn_blocking(move || save_now(&store, &settings)).await;
                if !matches!(saved, Ok(Ok(()))) {
                    tracing::warn!("settings save failed");
                }
            }
        });
        std::rc::Rc::new(tx)
    });
    use_effect(move || {
        let settings = state.settings.read().clone();
        if tracker.borrow_mut().changed(&settings, suppressed()) {
            let _ = tx.send(Some(settings));
        }
    });
}

#[cfg(test)]
#[path = "settings_save_tests.rs"]
mod tests;
