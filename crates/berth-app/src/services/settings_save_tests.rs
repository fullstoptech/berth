use super::*;

use berth_store::{Density, InspectorDock, Theme};

fn defaults() -> Settings {
    Settings::default()
}

// --- When a change is written ---

#[test]
fn settings_save_the_value_loaded_at_start_is_not_written() {
    let mut tracker = Tracker::default();
    assert!(!tracker.changed(&defaults(), false));
}

#[test]
fn settings_save_an_unchanged_value_is_not_written_again() {
    let mut tracker = Tracker::default();
    assert!(!tracker.changed(&defaults(), false));
    assert!(!tracker.changed(&defaults(), false));
}

#[test]
fn settings_save_every_changed_field_is_written() {
    let base = defaults();
    let edits: Vec<(&str, Settings)> = vec![
        (
            "theme",
            Settings {
                theme: Theme::Dark,
                ..base.clone()
            },
        ),
        (
            "density",
            Settings {
                density: Density::Compact,
                ..base.clone()
            },
        ),
        (
            "text_scale",
            Settings {
                text_scale: 1.3,
                ..base.clone()
            },
        ),
        (
            "dock",
            Settings {
                inspector_dock: InspectorDock::Bottom,
                ..base.clone()
            },
        ),
        (
            "width",
            Settings {
                inspector_width_px: 480,
                ..base.clone()
            },
        ),
        (
            "split",
            Settings {
                inspector_split_pct: 55,
                ..base.clone()
            },
        ),
        (
            "dock icon",
            Settings {
                show_dock_icon: !base.show_dock_icon,
                ..base.clone()
            },
        ),
        (
            "update check",
            Settings {
                update_check: !base.update_check,
                ..base.clone()
            },
        ),
        (
            "terminal",
            Settings {
                terminal_app: Some("iTerm".into()),
                ..base.clone()
            },
        ),
        (
            "editor",
            Settings {
                editor: Some("zed".into()),
                ..base.clone()
            },
        ),
        (
            "log buffer",
            Settings {
                log_buffer_lines: 50_000,
                ..base.clone()
            },
        ),
        (
            "last checked",
            Settings {
                update_last_checked: chrono::DateTime::from_timestamp(1_800_000_000, 0),
                ..base.clone()
            },
        ),
    ];
    for (name, edited) in edits {
        let mut tracker = Tracker::default();
        tracker.changed(&base, false);
        assert!(
            tracker.changed(&edited, false),
            "{name} change is not written"
        );
    }
}

#[test]
fn settings_save_a_forced_override_is_followed_but_never_written() {
    let mut tracker = Tracker::default();
    tracker.changed(&defaults(), true);
    let forced = Settings {
        text_scale: 1.5,
        ..defaults()
    };
    assert!(!tracker.changed(&forced, true), "suppressed");
    // Once the override is gone, only a real change writes.
    assert!(!tracker.changed(&forced, false));
    let real = Settings {
        theme: Theme::Dark,
        ..forced
    };
    assert!(tracker.changed(&real, false));
}

// --- Through the store ---

#[test]
fn settings_save_now_survives_closing_and_reopening_the_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("berth.db");
    let edited = Settings {
        theme: Theme::Dark,
        density: Density::Compact,
        text_scale: 1.2,
        inspector_dock: InspectorDock::Bottom,
        show_dock_icon: false,
        update_check: false,
        terminal_app: Some("Ghostty".to_string()),
        editor: Some("code".to_string()),
        log_buffer_lines: 250_000,
        ..defaults()
    };
    {
        let store = berth_store::Store::open(&file).expect("open");
        save_now(&store, &edited).expect("save");
    }
    let reopened = berth_store::Store::open(&file).expect("reopen");
    assert_eq!(reopened.settings().expect("settings"), edited);
}

// --- The effect, end to end: a change to the signal reaches the file ---

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[derive(Props, Clone)]
struct HostProps {
    store: Arc<berth_store::Store>,
}

impl PartialEq for HostProps {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.store, &other.store)
    }
}

#[component]
fn Host(props: HostProps) -> Element {
    let state = crate::state::use_app_state();
    use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some(state)));
    use_save_settings(state, props.store.clone());
    rsx! { div {} }
}

/// Drive the virtual dom (effects and spawned tasks) for a short while.
async fn pump(dom: &mut VirtualDom, millis: u64) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(millis);
    loop {
        tokio::select! {
            () = dom.wait_for_work() => { let _ = dom.render_immediate_to_vec(); }
            () = tokio::time::sleep_until(deadline) => return,
        }
    }
}

#[tokio::test]
async fn settings_save_a_change_to_the_signal_is_written_to_the_store() {
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let mut dom = VirtualDom::new_with_props(
        Host,
        HostProps {
            store: Arc::clone(&store),
        },
    );
    let _ = dom.rebuild_to_vec();
    let mut state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("host captured the state");
    pump(&mut dom, 50).await;
    // Loading changed nothing: nothing was written yet.
    assert_eq!(store.settings().expect("settings"), defaults());
    dom.in_runtime(|| {
        let mut settings = state.settings.write();
        settings.theme = Theme::Dark;
        settings.text_scale = 1.3;
    });
    pump(&mut dom, 300).await;
    let saved = store.settings().expect("settings");
    assert_eq!(saved.theme, Theme::Dark);
    assert!((saved.text_scale - 1.3).abs() < f32::EPSILON);
}

#[tokio::test]
async fn settings_save_two_quick_changes_leave_the_latest_in_the_store() {
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let mut dom = VirtualDom::new_with_props(
        Host,
        HostProps {
            store: Arc::clone(&store),
        },
    );
    let _ = dom.rebuild_to_vec();
    let mut state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("host captured the state");
    pump(&mut dom, 50).await;
    for scale in [1.1, 1.2, 1.3, 1.5] {
        dom.in_runtime(|| state.settings.write().text_scale = scale);
        pump(&mut dom, 5).await;
    }
    pump(&mut dom, 300).await;
    assert!((store.settings().expect("settings").text_scale - 1.5).abs() < f32::EPSILON);
}
