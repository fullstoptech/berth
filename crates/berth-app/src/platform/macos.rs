use dioxus::desktop::muda::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use dioxus::desktop::{Config, LogicalPosition, LogicalSize, WindowBuilder, WindowCloseBehaviour};
use dioxus::prelude::{use_effect, use_hook, ReadSignal, ReadableExt};

use crate::actions::ActionId;
use crate::assets;
use crate::state::AppState;

const TRAFFIC_LIGHT_INSET: (f64, f64) = (12.0, 19.0);

pub fn config() -> Config {
    use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
    let window = WindowBuilder::new()
        .with_title("Berth")
        .with_inner_size(LogicalSize::new(
            super::window_size().0,
            super::window_size().1,
        ))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .with_titlebar_transparent(true)
        .with_fullsize_content_view(true)
        .with_title_hidden(true);
    Config::new()
        .with_window(window)
        .with_menu(build_menu())
        .with_custom_head(assets::head_html())
        // D38: closing the window leaves the menu bar item running; `platform::quit` flips this.
        .with_close_behaviour(WindowCloseBehaviour::WindowHides)
        // The tray's left click opens its menu; it must not also raise the window.
        .with_tray_icon_show_window_on_click(false)
        .with_custom_event_handler(|event, target| {
            apply_dock_request(target);
            // A click on the Dock icon while the window is hidden brings it back.
            if let dioxus::desktop::tao::event::Event::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                super::show_window();
            }
            // AD36: Quit from the Dock ends the loop through `applicationWillTerminate:`, not
            // through `platform::quit`, so the final snapshot is saved here, synchronously.
            if matches!(event, dioxus::desktop::tao::event::Event::LoopDestroyed) {
                let saved = super::final_save();
                tracing::debug!(saved, "loop destroyed: final snapshot saved");
            }
        })
}

/// Dock icon changes the app asked for, applied on the event loop (only it holds the window
/// target that can change the activation policy). 0 = none, 1 = show, 2 = hide.
static DOCK_REQUEST: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// What the app last asked for (the dock icon starts shown).
static DOCK_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Apply a pending dock change (D67): shown is a regular app, hidden is an accessory one that
/// keeps its menu bar item.
fn apply_dock_request<T>(target: &dioxus::desktop::tao::event_loop::EventLoopWindowTarget<T>) {
    use dioxus::desktop::tao::platform::macos::{ActivationPolicy, EventLoopWindowTargetExtMacOS};
    use std::sync::atomic::Ordering;
    let policy = match DOCK_REQUEST.swap(0, Ordering::SeqCst) {
        1 => ActivationPolicy::Regular,
        2 => ActivationPolicy::Accessory,
        _ => return,
    };
    target.set_activation_policy_at_runtime(policy);
}

/// Show or hide the Dock icon. The menu bar item stays either way. Takes effect on the next event
/// the loop handles (a redraw is requested to cause one).
pub fn set_dock_icon_visible(visible: bool) {
    use std::sync::atomic::Ordering;
    if DOCK_SHOWN.swap(visible, Ordering::SeqCst) == visible {
        return;
    }
    DOCK_REQUEST.store(if visible { 1 } else { 2 }, Ordering::SeqCst);
    super::WINDOW.with(|slot| {
        if let Some(window) = slot.borrow().as_ref() {
            window.window.request_redraw();
        }
    });
}

/// Place the traffic lights inside Berth's own 38 px title bar (D8).
/// Goes through the webview because wry replaces the content view, so
/// tao's own with_traffic_light_inset never applies (Ferry precedent).
pub fn use_traffic_lights() {
    use dioxus::desktop::wry::WebViewExtMacOS;
    let window = dioxus::desktop::use_window();
    use_hook(move || {
        let inset = LogicalPosition::new(TRAFFIC_LIGHT_INSET.0, TRAFFIC_LIGHT_INSET.1);
        if let Err(e) = window.webview.set_traffic_light_inset(inset) {
            tracing::warn!(error = %e, "could not place the traffic lights");
        }
    });
}

// ---------------------------------------------------------------------
// Menu bar item (D38, ADR 0010). The model is pure (`services::menubar`); this turns it into
// native muda items and keeps one tray icon in step with it.
// ---------------------------------------------------------------------

use crate::services::menubar::{command_id, MenuCommand, MenuIcon, MenuItem as Entry2, MenuModel};

const TRAY_ICON_PX: u32 = 36;

fn tray_icon_bytes(icon: MenuIcon) -> &'static [u8] {
    match icon {
        MenuIcon::AllGood => include_bytes!("../../assets/tray/all-good.rgba"),
        MenuIcon::Failing => include_bytes!("../../assets/tray/failing.rgba"),
        MenuIcon::EngineStopped => include_bytes!("../../assets/tray/engine-stopped.rgba"),
    }
}

fn tray_icon(icon: MenuIcon) -> Option<dioxus::desktop::trayicon::Icon> {
    dioxus::desktop::trayicon::Icon::from_rgba(
        tray_icon_bytes(icon).to_vec(),
        TRAY_ICON_PX,
        TRAY_ICON_PX,
    )
    .map_err(|err| tracing::warn!(%err, "tray icon failed"))
    .ok()
}

/// The native menu for a model. A row is a submenu of its commands (muda rows hold no buttons);
/// a row with none, and the summary and headers, are disabled plain items.
fn tray_menu(model: &MenuModel) -> Menu {
    let menu = Menu::new();
    for item in &model.items {
        let added = match item {
            Entry2::Summary { text, .. } => menu.append(&MenuItem::new(
                format!("Berth \u{2014} {text}"),
                false,
                None,
            )),
            Entry2::Header(title) => menu.append(&MenuItem::new(*title, false, None)),
            Entry2::Row {
                label,
                detail,
                nested,
                commands,
                ..
            } => {
                let indent = if *nested { "\u{2003}\u{2003}" } else { "" };
                let text = format!("{indent}{label} \u{2014} {detail}");
                if commands.is_empty() {
                    menu.append(&MenuItem::new(text, false, None))
                } else {
                    let sub = Submenu::new(text, true);
                    for (name, command) in commands {
                        let entry = MenuItem::with_id(command_id(command), name, true, None);
                        if let Err(err) = sub.append(&entry) {
                            tracing::warn!(%err, "tray command failed");
                        }
                    }
                    menu.append(&sub)
                }
            }
            Entry2::Separator => menu.append(&PredefinedMenuItem::separator()),
            Entry2::Command { label, command } => {
                let accelerator = matches!(command, MenuCommand::Quit)
                    .then(|| "Cmd+Q".parse().ok())
                    .flatten();
                menu.append(&MenuItem::with_id(
                    command_id(command),
                    *label,
                    true,
                    accelerator,
                ))
            }
        };
        if let Err(err) = added {
            tracing::warn!(%err, "tray item failed");
        }
    }
    menu
}

thread_local! {
    /// The one tray icon (main thread only); dropping it would remove the item.
    static TRAY: std::cell::RefCell<Option<dioxus::desktop::trayicon::TrayIcon>> =
        const { std::cell::RefCell::new(None) };
}

/// Keep the menu bar item in step with `model`. The effect re-runs only when the model changes
/// (`MenuModel: PartialEq`, read through a memo), and each run replaces the menu and the icon.
pub fn use_tray(model: ReadSignal<MenuModel>) {
    use dioxus::desktop::trayicon::TrayIconBuilder;
    let window = dioxus::desktop::use_window();
    use_hook(move || super::register_window(window));
    use_effect(move || {
        let model = model.read().clone();
        let icon = tray_icon(model.icon);
        TRAY.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(tray) = slot.as_ref() {
                tray.set_menu(Some(Box::new(tray_menu(&model))));
                if let Some(icon) = icon {
                    if let Err(err) = tray.set_icon_with_as_template(Some(icon), true) {
                        tracing::warn!(%err, "tray icon update failed");
                    }
                }
                return;
            }
            let mut builder = TrayIconBuilder::new()
                .with_menu(Box::new(tray_menu(&model)))
                .with_menu_on_left_click(true)
                .with_icon_as_template(true)
                .with_tooltip("Berth");
            if let Some(icon) = icon {
                builder = builder.with_icon(icon);
            }
            match builder.build() {
                Ok(tray) => *slot = Some(tray),
                Err(err) => tracing::warn!(%err, "tray item failed"),
            }
        });
    });
}

// ---------------------------------------------------------------------
// Open in Terminal app (D42): a 0700 `.command` file that runs
// `docker exec -it` with DOCKER_HOST set, opened with the terminal app.
// ---------------------------------------------------------------------

/// A terminal app Berth can hand a session to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalApp {
    pub name: &'static str,
    pub bundle: std::path::PathBuf,
}

const TERMINAL_APPS: [(&str, &str); 4] = [
    ("Terminal", "Terminal.app"),
    ("iTerm", "iTerm.app"),
    ("Ghostty", "Ghostty.app"),
    ("WezTerm", "WezTerm.app"),
];

/// The terminal apps found under `roots` (D42: `/Applications` and
/// `/System/Applications/Utilities`), in preference order per root.
pub fn terminal_apps_in(roots: &[&std::path::Path]) -> Vec<TerminalApp> {
    let mut found = Vec::new();
    for root in roots {
        for (name, bundle) in TERMINAL_APPS {
            let path = root.join(bundle);
            if path.exists() {
                found.push(TerminalApp { name, bundle: path });
            }
        }
    }
    found
}

pub fn terminal_apps() -> Vec<TerminalApp> {
    terminal_apps_in(&[
        std::path::Path::new("/Applications"),
        std::path::Path::new("/System/Applications/Utilities"),
    ])
}

/// What to run in the terminal app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecTarget {
    pub docker_host: String,
    pub container: String,
    pub shell: String,
    pub user: Option<String>,
    pub workdir: Option<String>,
}

/// Single-quote one shell word.
pub fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The `.command` file's text. It removes itself first, so nothing is left
/// behind in the run directory.
pub fn command_script(docker: &std::path::Path, target: &ExecTarget) -> String {
    let mut line = format!("exec {} exec -it", shell_quote(&docker.to_string_lossy()));
    if let Some(user) = target.user.as_deref().filter(|u| !u.is_empty()) {
        line.push_str(&format!(" -u {}", shell_quote(user)));
    }
    if let Some(dir) = target.workdir.as_deref().filter(|d| !d.is_empty()) {
        line.push_str(&format!(" -w {}", shell_quote(dir)));
    }
    line.push_str(&format!(
        " {} {}",
        shell_quote(&target.container),
        shell_quote(&target.shell)
    ));
    format!(
        "#!/bin/bash\nrm -f -- \"$0\"\nexport DOCKER_HOST={}\n{line}\n",
        shell_quote(&target.docker_host)
    )
}

/// Write the script (mode 0700, in Berth's run directory) and open it with
/// the first terminal app found. Returns the app's name.
pub async fn open_in_terminal_app(target: ExecTarget) -> Result<&'static str, String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let apps = terminal_apps();
    let app = apps
        .first()
        .ok_or_else(|| "No terminal app was found in /Applications".to_string())?;
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let docker = search
        .find("docker")
        .ok_or_else(|| "The docker command was not found on this Mac".to_string())?;
    let open = search
        .find("open")
        .ok_or_else(|| "The open command was not found".to_string())?;
    let dir = berth_sys::paths::runtime_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    let file = dir.join(format!("exec-{}.command", uuid::Uuid::now_v7()));
    {
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&file)
            .map_err(|e| e.to_string())?;
        out.write_all(command_script(&docker, &target).as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let status = tokio::process::Command::new(open)
        .arg("-a")
        .arg(&app.bundle)
        .arg(&file)
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(app.name)
    } else {
        let _ = std::fs::remove_file(&file);
        Err(format!("{} could not be opened", app.name))
    }
}

// ---------------------------------------------------------------------
// App menu (D39). `menu_spec` is the data; `build_menu` turns it into
// muda objects on the main thread; `menu_action` maps a clicked item id
// back to what Berth does. Edit carries the predefined items so Cmd+C,
// V, X, A and Z reach text inputs in WKWebView.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Predefined {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    CloseWindow,
    Fullscreen,
    Minimize,
    Zoom,
    BringAllToFront,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// A Berth action: `id` is its menu id; `accelerator` in muda syntax.
    Item {
        id: &'static str,
        label: &'static str,
        accelerator: Option<&'static str>,
    },
    Predefined(Predefined),
    Separator,
}

struct SubmenuSpec {
    title: &'static str,
    entries: Vec<Entry>,
}

const QUIT_ID: &str = "app.quit";
const DOCK_ID: &str = "view.dock_inspector";
const HELP_SHORTCUTS_ID: &str = "help.shortcuts";

const fn item(id: &'static str, label: &'static str, accelerator: Option<&'static str>) -> Entry {
    Entry::Item {
        id,
        label,
        accelerator,
    }
}

fn menu_spec() -> Vec<SubmenuSpec> {
    use Entry::{Predefined as P, Separator as Sep};
    use Predefined as K;
    vec![
        SubmenuSpec {
            title: "Berth",
            entries: vec![
                P(K::About),
                Sep,
                item("global.settings", "Settings\u{2026}", Some("Cmd+,")),
                Sep,
                P(K::Services),
                Sep,
                P(K::Hide),
                P(K::HideOthers),
                P(K::ShowAll),
                Sep,
                // Not the predefined Quit: `NSApp terminate` exits the
                // process without dropping the app, so the final
                // snapshot save (D23) would never run.
                item(QUIT_ID, "Quit Berth", Some("Cmd+Q")),
            ],
        },
        SubmenuSpec {
            title: "File",
            entries: vec![
                item("global.open_folder", "Open Folder\u{2026}", Some("Cmd+O")),
                item("global.new_project", "New Project\u{2026}", Some("Cmd+N")),
                Sep,
                P(K::CloseWindow),
            ],
        },
        SubmenuSpec {
            title: "Edit",
            entries: vec![
                P(K::Undo),
                P(K::Redo),
                Sep,
                P(K::Cut),
                P(K::Copy),
                P(K::Paste),
                P(K::SelectAll),
            ],
        },
        SubmenuSpec {
            title: "View",
            entries: vec![
                item("global.palette", "Command Palette", Some("Cmd+K")),
                item(
                    "global.toggle_bottom_panel",
                    "Toggle Bottom Panel",
                    Some("Ctrl+`"),
                ),
                item("global.toggle_inspector", "Toggle Inspector", None),
                // The label names the side a click moves to; `set_dock_menu_label` flips it.
                item(DOCK_ID, "Dock Inspector at Bottom", Some("Cmd+Shift+D")),
                Sep,
                item("view.text_bigger", "Bigger Text", Some("Cmd+=")),
                item("view.text_smaller", "Smaller Text", Some("Cmd+-")),
                item("view.text_reset", "Actual Text Size", Some("Cmd+0")),
                Sep,
                P(K::Fullscreen),
            ],
        },
        SubmenuSpec {
            title: "Window",
            entries: vec![P(K::Minimize), P(K::Zoom), Sep, P(K::BringAllToFront)],
        },
        SubmenuSpec {
            title: "Help",
            entries: vec![item(HELP_SHORTCUTS_ID, "Keyboard Shortcuts", None)],
        },
    ]
}

/// What a clicked menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Run an `ActionDef` through `keys::dispatch`.
    Dispatch(ActionId),
    /// Close the window so the app drops cleanly (final snapshot save).
    Quit,
}

pub fn menu_action(id: &str) -> Option<MenuAction> {
    match id {
        QUIT_ID => Some(MenuAction::Quit),
        HELP_SHORTCUTS_ID => Some(MenuAction::Dispatch("global.palette")),
        other => menu_spec()
            .into_iter()
            .flat_map(|s| s.entries)
            .find_map(|e| match e {
                Entry::Item { id, .. } if id == other => {
                    crate::actions::action_by_id(id).map(|def| MenuAction::Dispatch(def.id))
                }
                _ => None,
            }),
    }
}

fn predefined(kind: Predefined) -> PredefinedMenuItem {
    match kind {
        Predefined::About => PredefinedMenuItem::about(
            Some("About Berth"),
            Some(AboutMetadata {
                name: Some("Berth".to_string()),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                ..AboutMetadata::default()
            }),
        ),
        Predefined::Services => PredefinedMenuItem::services(None),
        Predefined::Hide => PredefinedMenuItem::hide(None),
        Predefined::HideOthers => PredefinedMenuItem::hide_others(None),
        Predefined::ShowAll => PredefinedMenuItem::show_all(None),
        Predefined::Undo => PredefinedMenuItem::undo(None),
        Predefined::Redo => PredefinedMenuItem::redo(None),
        Predefined::Cut => PredefinedMenuItem::cut(None),
        Predefined::Copy => PredefinedMenuItem::copy(None),
        Predefined::Paste => PredefinedMenuItem::paste(None),
        Predefined::SelectAll => PredefinedMenuItem::select_all(None),
        Predefined::CloseWindow => PredefinedMenuItem::close_window(None),
        Predefined::Fullscreen => PredefinedMenuItem::fullscreen(None),
        Predefined::Minimize => PredefinedMenuItem::minimize(None),
        Predefined::Zoom => PredefinedMenuItem::maximize(Some("Zoom")),
        Predefined::BringAllToFront => PredefinedMenuItem::bring_all_to_front(None),
    }
}

/// Build the menu bar. muda requires the main thread: call from `main`
/// through `config()`.
fn build_menu() -> Menu {
    let menu = Menu::new();
    for spec in menu_spec() {
        let submenu = Submenu::new(spec.title, true);
        for entry in spec.entries {
            let added = match entry {
                Entry::Item {
                    id,
                    label,
                    accelerator,
                } => {
                    let accelerator = accelerator.and_then(|a| a.parse().ok());
                    let menu_item = MenuItem::with_id(id, label, true, accelerator);
                    if id == DOCK_ID {
                        DOCK_ITEM.with(|slot| *slot.borrow_mut() = Some(menu_item.clone()));
                    }
                    submenu.append(&menu_item)
                }
                Entry::Predefined(kind) => submenu.append(&predefined(kind)),
                Entry::Separator => submenu.append(&PredefinedMenuItem::separator()),
            };
            if let Err(err) = added {
                tracing::warn!(%err, menu = spec.title, "menu item failed");
            }
        }
        match spec.title {
            "Window" => submenu.set_as_windows_menu_for_nsapp(),
            "Help" => submenu.set_as_help_menu_for_nsapp(),
            _ => {}
        }
        if let Err(err) = menu.append(&submenu) {
            tracing::warn!(%err, menu = spec.title, "menu failed");
        }
    }
    menu
}

thread_local! {
    /// The View menu's dock item, kept to change its label (main thread only).
    static DOCK_ITEM: std::cell::RefCell<Option<MenuItem>> = const { std::cell::RefCell::new(None) };
}

/// Name the side the dock item moves the inspector to ("Dock Inspector at
/// Bottom" while it is at the right).
pub fn set_dock_menu_label(dock: berth_store::InspectorDock) {
    DOCK_ITEM.with(|slot| {
        if let Some(item) = slot.borrow().as_ref() {
            item.set_text(crate::ui::dock::dock_menu_label(dock));
        }
    });
}

/// Route clicks on Berth's own menu items (everything but the predefined
/// ones) to the same dispatcher the keyboard uses.
pub fn use_menu_events(state: AppState) {
    dioxus::desktop::use_muda_event_handler(move |event| {
        let mut state = state;
        tracing::debug!(id = %event.id().0, "menu event");
        // Tray items arrive here too: muda keeps one event handler, the app menu's.
        if event
            .id()
            .0
            .starts_with(crate::services::menubar::ID_PREFIX)
        {
            let actions =
                dioxus::prelude::try_consume_context::<crate::services::actions::ActionsService>();
            let link = dioxus::prelude::try_consume_context::<
                Option<crate::services::engines::EngineLink>,
            >()
            .flatten();
            crate::services::menubar::handle_item(
                &mut state,
                actions.as_ref(),
                link.as_ref(),
                &event.id().0,
            );
            return;
        }
        match menu_action(&event.id().0) {
            Some(MenuAction::Dispatch(action)) => crate::keys::dispatch(&mut state, action),
            Some(MenuAction::Quit) => super::quit(),
            None => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_app_script_sets_the_host_and_quotes_every_word() {
        let target = ExecTarget {
            docker_host: "unix:///Users/me/.colima/default/docker.sock".to_string(),
            container: "abc123".to_string(),
            shell: "bash".to_string(),
            user: Some("root".to_string()),
            workdir: Some("/my app/it's".to_string()),
        };
        let script = command_script(std::path::Path::new("/opt/x y/docker"), &target);
        assert!(
            script.starts_with("#!/bin/bash\nrm -f -- \"$0\"\n"),
            "{script}"
        );
        assert!(
            script.contains("export DOCKER_HOST='unix:///Users/me/.colima/default/docker.sock'\n")
        );
        assert!(
            script.contains(
                "exec '/opt/x y/docker' exec -it -u 'root' -w '/my app/it'\\''s' 'abc123' 'bash'\n"
            ),
            "{script}"
        );
        let plain = ExecTarget {
            user: None,
            workdir: None,
            ..target
        };
        let script = command_script(std::path::Path::new("/d"), &plain);
        assert!(
            script.contains("exec '/d' exec -it 'abc123' 'bash'\n"),
            "{script}"
        );
    }

    #[test]
    fn terminal_app_list_offers_only_installed_apps() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(dir.path().join("Terminal.app")).expect("app");
        std::fs::create_dir(dir.path().join("Ghostty.app")).expect("app");
        let apps = terminal_apps_in(&[dir.path()]);
        let names: Vec<_> = apps.iter().map(|a| a.name).collect();
        assert_eq!(names, ["Terminal", "Ghostty"]);
        assert!(terminal_apps_in(&[std::path::Path::new("/no/such/root")]).is_empty());
    }

    #[test]
    fn shell_menu_bar_has_the_six_menus_of_d39() {
        let titles: Vec<_> = menu_spec().iter().map(|s| s.title).collect();
        assert_eq!(titles, ["Berth", "File", "Edit", "View", "Window", "Help"]);
    }

    #[test]
    fn shell_menu_edit_has_every_predefined_clipboard_item() {
        let spec = menu_spec();
        let edit = spec.iter().find(|s| s.title == "Edit").expect("Edit menu");
        for kind in [
            Predefined::Undo,
            Predefined::Redo,
            Predefined::Cut,
            Predefined::Copy,
            Predefined::Paste,
            Predefined::SelectAll,
        ] {
            assert!(
                edit.entries.contains(&Entry::Predefined(kind)),
                "{kind:?} missing from Edit"
            );
        }
    }

    /// "Cmd+K" -> "⌘K": the glyph form the ActionDef table uses.
    fn glyphs(accelerator: &str) -> String {
        accelerator
            .replace("Cmd+", "\u{2318}")
            .replace("Ctrl+", "\u{2303}")
            .replace("Shift+", "\u{21e7}")
    }

    #[test]
    fn shell_menu_items_are_real_actions_with_the_actions_shortcuts() {
        for spec in menu_spec() {
            for entry in spec.entries {
                let Entry::Item {
                    id, accelerator, ..
                } = entry
                else {
                    continue;
                };
                if id == QUIT_ID || id == HELP_SHORTCUTS_ID {
                    continue;
                }
                let def = crate::actions::action_by_id(id)
                    .unwrap_or_else(|| panic!("menu item {id} is not an ActionDef"));
                if let Some(accelerator) = accelerator {
                    assert!(
                        accelerator
                            .parse::<dioxus::desktop::muda::accelerator::Accelerator>()
                            .is_ok(),
                        "{accelerator} must parse"
                    );
                    assert_eq!(def.shortcut, Some(glyphs(accelerator).as_str()), "{id}");
                }
            }
        }
    }

    #[test]
    fn shell_menu_dock_item_flips_the_inspector_and_starts_with_the_right_dock_label() {
        let spec = menu_spec();
        let view = spec.iter().find(|s| s.title == "View").expect("View menu");
        let entry = view
            .entries
            .iter()
            .find_map(|e| match e {
                Entry::Item {
                    id: DOCK_ID, label, ..
                } => Some(*label),
                _ => None,
            })
            .expect("a dock item in View");
        // The inspector starts at the right, so the item offers the bottom.
        assert_eq!(
            entry,
            crate::ui::dock::dock_menu_label(berth_store::InspectorDock::Right)
        );
        assert_eq!(
            menu_action(DOCK_ID),
            Some(MenuAction::Dispatch("view.dock_inspector"))
        );
    }

    #[test]
    fn shell_menu_quit_closes_the_window_and_items_dispatch() {
        assert_eq!(menu_action(QUIT_ID), Some(MenuAction::Quit));
        assert_eq!(
            menu_action("global.palette"),
            Some(MenuAction::Dispatch("global.palette"))
        );
        assert_eq!(
            menu_action(HELP_SHORTCUTS_ID),
            Some(MenuAction::Dispatch("global.palette"))
        );
        assert_eq!(menu_action("not.an.item"), None);
    }
}
