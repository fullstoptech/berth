//! Scenes of the Open folder sheet (P5-T4). The sheet reads no disk under a
//! scene: the model stands in for what the folders would answer, built from
//! this Mac's home so paths read `~/Dev/side`.

use std::path::{Path, PathBuf};

use berth_core::{
    Confidence, Detection, FoundFile, FoundKind, ServiceGuess, ServiceKind, StackGuess,
};
use berth_rules::detect::FolderMarkers;

use super::*;
use crate::services::folders::{
    Browser, Column, ColumnState, Details, Entry, FolderError, Place, PlaceKind, Preview,
};
use crate::sheets::open_folder::{DevOpenFolder, PreviewState};

fn entry(parent: &Path, name: &str, items: usize, markers: FolderMarkers) -> Entry {
    Entry {
        name: name.to_string(),
        path: parent.join(name),
        hidden: name.starts_with('.'),
        details: Some(Details {
            markers,
            items: Some(items),
        }),
    }
}

fn plain(parent: &Path, name: &str, items: usize) -> Entry {
    entry(parent, name, items, FolderMarkers::default())
}

fn column(folder: &Path, entries: Vec<Entry>, selected: Option<PathBuf>) -> Column {
    Column {
        folder: folder.to_path_buf(),
        state: ColumnState::Ready(entries),
        selected,
    }
}

fn place(name: &str, path: PathBuf, kind: PlaceKind) -> Place {
    Place {
        name: name.to_string(),
        path,
        kind,
    }
}

/// What the frames' left column shows: the Dev favourite, three recents and
/// the startup disk.
fn dev_places(home: &Path) -> (Vec<PathBuf>, Vec<Place>, Vec<Place>) {
    (
        vec![home.join("Dev")],
        vec![place(
            "Macintosh HD",
            PathBuf::from("/Volumes/Macintosh HD"),
            PlaceKind::Volume,
        )],
        vec![
            place(
                "shop-api",
                home.join("Dev/clients/acme/shop-api"),
                PlaceKind::Standard,
            ),
            place("analytics", home.join("Dev/analytics"), PlaceKind::Standard),
            place(
                "berth-site",
                home.join("Dev/clients/berth-site"),
                PlaceKind::Standard,
            ),
        ],
    )
}

/// `~` -> `Dev` -> `side`, with `new-app` highlighted and its detection in
/// the preview ("Open folder / browser / 1440").
fn browser_world(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    let home = berth_sys::paths::home();
    let dev = home.join("Dev");
    let side = dev.join("side");
    let new_app = side.join("new-app");
    let compose = FolderMarkers {
        compose: true,
        dockerfile: false,
    };
    let dockerfile = FolderMarkers {
        compose: false,
        dockerfile: true,
    };

    let columns = vec![
        column(
            &home,
            vec![
                plain(&home, "Desktop", 4),
                plain(&home, "Dev", 6),
                plain(&home, "Documents", 23),
                plain(&home, "Downloads", 58),
                plain(&home, "Movies", 3),
                plain(&home, "Music", 12),
                plain(&home, "Pictures", 9),
            ],
            Some(dev.clone()),
        ),
        column(
            &dev,
            vec![
                plain(&dev, "analytics", 7),
                plain(&dev, "app-ideas", 5),
                plain(&dev, "clients", 2),
                plain(&dev, "docs-site", 9),
                plain(&dev, "research", 1),
                plain(&dev, "side", 4),
            ],
            Some(side.clone()),
        ),
        column(
            &side,
            vec![
                entry(&side, "landing-v2", 8, compose),
                entry(&side, "new-app", 4, dockerfile),
                plain(&side, "old-api", 3),
                plain(&side, "scratch", 2),
            ],
            Some(new_app.clone()),
        ),
    ];
    let detection = Detection {
        folder: new_app.clone(),
        found: vec![
            FoundFile {
                kind: FoundKind::Dockerfile {
                    from: Some("node:22-alpine".to_string()),
                },
                path: new_app.join("Dockerfile"),
            },
            FoundFile {
                kind: FoundKind::Env { keys: 6 },
                path: new_app.join(".env"),
            },
        ],
        stack: vec![StackGuess::Node {
            pm: "pnpm".to_string(),
            framework: Some("Next.js".to_string()),
            framework_version: Some("15".to_string()),
            version: Some("22".to_string()),
        }],
        services_needed: vec![
            ServiceGuess {
                kind: ServiceKind::Postgres,
                reason: "from DATABASE_URL".to_string(),
            },
            ServiceGuess {
                kind: ServiceKind::Redis,
                reason: "from ioredis in package.json".to_string(),
            },
        ],
        confidence: Confidence::High,
        git_branch: Some("main".to_string()),
    };
    let (favourites, volumes, recent) = dev_places(&home);
    provide_context(DevOpenFolder {
        home,
        browser: Browser {
            columns,
            filter: String::new(),
            show_hidden: false,
        },
        preview: PreviewState::Ready(Preview {
            folder: new_app,
            items: 4,
            detection,
        }),
        favourites,
        volumes,
        recent,
    });
    state
        .overlays
        .write()
        .push(crate::state::Overlay::OpenFolder);
}

/// `~/Documents` that macOS has not let Berth read.
fn no_access_world(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    let home = berth_sys::paths::home();
    let documents = home.join("Documents");
    let (favourites, volumes, recent) = dev_places(&home);
    provide_context(DevOpenFolder {
        home: home.clone(),
        browser: Browser {
            columns: vec![
                column(&home, vec![plain(&home, "Documents", 0)], Some(documents.clone())),
                Column {
                    folder: documents.clone(),
                    state: ColumnState::Failed(FolderError::NoAccess(documents)),
                    selected: None,
                },
            ],
            filter: String::new(),
            show_hidden: false,
        },
        preview: PreviewState::Empty,
        favourites,
        volumes,
        recent,
    });
    state
        .overlays
        .write()
        .push(crate::state::Overlay::OpenFolder);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Open folder / browser / 1440", browser_world),
    Scene::design("Open folder / browser / no access / 1440", no_access_world),
];
