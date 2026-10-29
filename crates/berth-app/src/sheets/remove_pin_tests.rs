//! P10-T3 tests for Remove from Berth: the dialog says exactly what the action does (it unpins
//! the folder and nothing else), its button and result come from the same summary, and the
//! confirmed action deletes the pin and leaves the folder on disk alone.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use berth_core::{EffectKind, EngineId, PinnedFolder};

use super::*;
use crate::state::use_app_state;

fn engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn pin_of(folder: &Path) -> PinnedFolder {
    PinnedFolder {
        folder: folder.to_path_buf(),
        engine: engine(),
        added: chrono::Utc::now(),
    }
}

fn services_with(store: Arc<berth_store::Store>) -> Services {
    Services {
        store,
        clients: crate::services::engines::client_map(),
        cancel: tokio_util::sync::CancellationToken::new(),
    }
}

#[test]
fn remove_pin_dialog_names_the_folder_and_says_nothing_but_the_pin_goes() {
    let home = Path::new("/Users/berth");
    let (title, effect) = dialog_of("clone", Path::new("/Users/berth/Dev/clone"), home);
    assert_eq!(title, "Remove clone from Berth?");
    let lines: Vec<(EffectKind, &str)> = effect
        .lines
        .iter()
        .map(|l| (l.kind, l.text.as_str()))
        .collect();
    assert_eq!(
        lines,
        [
            (
                EffectKind::Goes,
                "clone from Berth's Projects list (the pin of ~/Dev/clone)"
            ),
            (
                EffectKind::Stays,
                "The folder and every file in ~/Dev/clone"
            ),
            (
                EffectKind::Stays,
                "Docker containers, images and volumes: nothing was created for this project"
            ),
        ]
    );
    // One summary behind the button and the result.
    assert_eq!(
        crate::services::confirm::button_label(&effect),
        "Unpin folder"
    );
    assert_eq!(result_label(&effect, 1), "Unpinned 1 folder");
    assert!(effect.typed_name.is_none(), "an unpin needs no typed name");
}

#[test]
fn remove_pin_unpins_the_folder_and_leaves_its_files_and_other_pins() {
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let dir = tempfile::tempdir().expect("tempdir");
    let clone: PathBuf = dir.path().join("clone");
    let other: PathBuf = dir.path().join("other");
    std::fs::create_dir_all(&clone).expect("mkdir");
    std::fs::create_dir_all(&other).expect("mkdir");
    std::fs::write(clone.join("README.md"), "keep me").expect("write");
    store.pin(&pin_of(&clone)).expect("pin");
    store.pin(&pin_of(&other)).expect("pin");

    let services = services_with(Arc::clone(&store));
    let (_, effect) = dialog_of("clone", &clone, dir.path());
    let (text, tone) = unpin(Some(&services), &clone, &effect);

    assert_eq!(text, "Unpinned 1 folder");
    assert_eq!(tone, ToastTone::Success);
    let pins: Vec<PathBuf> = store
        .pins()
        .expect("pins")
        .into_iter()
        .map(|p| p.folder)
        .collect();
    assert_eq!(pins, vec![other], "only the confirmed folder is unpinned");
    assert_eq!(
        std::fs::read_to_string(clone.join("README.md")).expect("file"),
        "keep me",
        "the folder and its files are never touched"
    );
}

#[test]
fn remove_pin_without_a_store_says_so_instead_of_claiming_success() {
    let (_, effect) = dialog_of("clone", Path::new("/x/clone"), Path::new("/x"));
    let (text, tone) = unpin(None, Path::new("/x/clone"), &effect);
    assert_eq!(tone, ToastTone::Danger);
    assert!(!text.starts_with("Unpinned"), "{text}");
}

thread_local! {
    static PROJECT: std::cell::RefCell<Option<berth_core::Project>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    use_hook(move || {
        if let Some(project) = PROJECT.with(|slot| slot.borrow().clone()) {
            state.projects.set(vec![project]);
        }
    });
    let id = PROJECT
        .with(|slot| slot.borrow().as_ref().map(|p| p.id.clone()))
        .expect("project");
    rsx! { RemovePinSheet { id } }
}

fn render_sheet(project: berth_core::Project) -> String {
    PROJECT.with(|slot| *slot.borrow_mut() = Some(project));
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn remove_pin_sheet_renders_the_confirm_with_its_rows_and_button() {
    let world = berth_engine::MockWorld::bundled();
    let project = world
        .projects
        .iter()
        .find(|p| p.name == "docker-manger")
        .cloned()
        .expect("a pinned folder with no containers");
    let html = render_sheet(project);
    assert!(html.contains("Remove docker-manger from Berth?"), "{html}");
    assert!(html.contains("Removes"), "{html}");
    assert!(html.contains("Keeps"), "{html}");
    assert!(html.contains("The folder and every file in"), "{html}");
    assert!(html.contains("Unpin folder"), "{html}");
    assert!(html.contains("Cancel"), "{html}");
}
