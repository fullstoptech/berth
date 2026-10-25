//! The Files tab of Project detail (P5-T6, frame "Project / files / 1440"):
//! the project folder as a tree on the left, the selected file on the right.
//!
//! The tree opens folders and `.env` files in place. An `.env` file shows
//! its variable names and eight dots for each value; the eye beside a name
//! reads that one value from the file and shows it, and shows nothing once
//! clicked again. Values live only in this component's `revealed` map, so
//! they go when the tab does, and nothing logs them. The text of an env file
//! is never previewed.
//!
//! Reads go through `services::project_files::ProjectFs` on a blocking
//! thread (a memory folder, which a scene seeds, answers on the spot). A
//! read that is under way is a "Reading…" row; a failed one says why in its
//! own row (denied, not on this Mac).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use dioxus::prelude::*;

use super::tree::{first_env, initial_selection, tree_rows, Load, TreeData, TreeRow};
use crate::services::project_files::{
    classify, default_editor_target, file_name, launch, open_in_editor_command, reveal_command,
    search_path_finder, terminal_command, ComposeProblem, Editor, EntryKind, FileRole, FsEntry,
    FsError, Preview, ProjectFs, ProjectFsOverride,
};
use crate::ui::{
    Button, ButtonVariant, ControlSize, EmptyState, Icon, IconButton, IconSize, InlineMessage,
    InlineTone,
};

/// What fills the dots of a hidden value.
pub const MASK: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

/// The folder behind a signal, so a click handler can copy it.
#[derive(Clone)]
struct FsHandle(Arc<dyn ProjectFs>);

impl PartialEq for FsHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What one pass of reading still has to do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Need {
    Dir(String),
    Keys(String),
    Preview(String),
}

/// The reads the tree is waiting for, in the order a person would want them.
fn needs(data: &TreeData) -> Vec<Need> {
    let mut out = Vec::new();
    if !data.listings.contains_key("") {
        out.push(Need::Dir(String::new()));
    }
    if let Some(selected) = &data.selected {
        if classify(file_name(selected)) == FileRole::Env {
            if !data.keys.contains_key(selected) {
                out.push(Need::Keys(selected.clone()));
            }
        } else if !data.previews.contains_key(selected) {
            out.push(Need::Preview(selected.clone()));
        }
    }
    for rel in &data.expanded {
        if classify(file_name(rel)) == FileRole::Env {
            if !data.keys.contains_key(rel) {
                out.push(Need::Keys(rel.clone()));
            }
        } else if !data.listings.contains_key(rel) {
            out.push(Need::Dir(rel.clone()));
        }
    }
    out
}

/// The first time the root listing is in: choose the file to show and open
/// the first env file, as the frame has it.
fn seed(
    data: &mut TreeData,
    folder: &std::path::Path,
    compose_files: &[PathBuf],
    wanted: Option<&str>,
) {
    if data.seeded {
        return;
    }
    let Some(Load::Ready(root)) = data.listings.get("") else {
        return;
    };
    data.seeded = true;
    let chosen = wanted
        .map(str::to_string)
        .or_else(|| initial_selection(Some(folder), compose_files, Some(root)));
    let env = first_env(root).map(|e| e.rel.clone());
    data.selected = chosen;
    if let Some(env) = env {
        data.expanded.insert(env);
    }
}

#[derive(Clone, Copy)]
struct Reader {
    fs: Signal<FsHandle>,
    data: Signal<TreeData>,
}

impl Reader {
    /// Run `job` against the folder: on the spot for a memory folder, else on
    /// a blocking thread, then hand the answer to `done`.
    fn run<T: Send + 'static>(
        self,
        job: impl FnOnce(&dyn ProjectFs) -> Result<T, FsError> + Send + 'static,
        done: impl FnOnce(Load<T>) + 'static,
    ) {
        let fs = self.fs.peek().0.clone();
        if fs.instant() {
            done(Load::from_result(job(&*fs)));
            return;
        }
        spawn(async move {
            let answer = tokio::task::spawn_blocking(move || job(&*fs))
                .await
                .map_err(|err| FsError::Other(err.to_string()))
                .and_then(|result| result);
            done(Load::from_result(answer));
        });
    }

    fn start(self, need: Need) {
        let mut data = self.data;
        match need {
            Need::Dir(rel) => {
                data.write().listings.insert(rel.clone(), Load::Loading);
                let path = rel.clone();
                self.run(
                    move |fs| fs.list(&path),
                    move |answer| {
                        data.write().listings.insert(rel, answer);
                    },
                );
            }
            Need::Keys(rel) => {
                data.write().keys.insert(rel.clone(), Load::Loading);
                let path = rel.clone();
                self.run(
                    move |fs| fs.env_keys(&path),
                    move |answer| {
                        data.write().keys.insert(rel, answer);
                    },
                );
            }
            Need::Preview(rel) => {
                data.write().previews.insert(rel.clone(), Load::Loading);
                let path = rel.clone();
                self.run(
                    move |fs| fs.preview(&path),
                    move |answer| {
                        data.write().previews.insert(rel, answer);
                    },
                );
            }
        }
    }

    /// Start every read the tree is waiting for, and keep going while a
    /// memory folder answers on the spot.
    fn pump(self, folder: &std::path::Path, compose_files: &[PathBuf], wanted: Option<&str>) {
        let mut data = self.data;
        for _ in 0..8 {
            let before = data.peek().clone();
            let mut after = before.clone();
            seed(&mut after, folder, compose_files, wanted);
            if after != before {
                data.set(after);
            }
            let todo = needs(&data.peek());
            if todo.is_empty() {
                return;
            }
            for need in todo {
                self.start(need);
            }
            if !self.fs.peek().0.instant() {
                return;
            }
        }
    }
}

/// Where a folder's files come from: a scene's memory folder, else the disk.
fn source_for(folder: &std::path::Path) -> Arc<dyn ProjectFs> {
    try_consume_context::<ProjectFsOverride>().map_or_else(
        || Arc::new(crate::services::project_files::DiskFs::new(folder)) as Arc<dyn ProjectFs>,
        |o| o.0,
    )
}

fn size_text(bytes: u64) -> String {
    berth_core::fmt::bytes(bytesize::ByteSize::b(bytes))
}

fn thousands(n: usize) -> String {
    crate::ui::file_browser::thousands(n)
}

#[component]
pub fn FilesTab(
    folder: PathBuf,
    /// The folder as `~/…`.
    folder_text: String,
    compose_files: Vec<PathBuf>,
    /// What `compose config` said about the Compose file, when it does not load.
    problem: Option<ComposeProblem>,
    editor: Editor,
    /// Settings' editor and terminal, when set.
    editor_setting: Option<String>,
    terminal_setting: Option<String>,
    /// Read the folder again and check the Compose file again.
    on_refresh: EventHandler<()>,
) -> Element {
    let fs = use_signal({
        let folder = folder.clone();
        move || FsHandle(source_for(&folder))
    });
    let mut data = use_signal(TreeData::default);
    let mut revealed = use_signal(BTreeMap::<(String, String), String>::new);
    let reader = Reader { fs, data };
    // The file the Compose problem names, else the first Compose file.
    let wanted = problem
        .as_ref()
        .and_then(|p| p.file.clone())
        .filter(|f| !f.contains(".."));
    // First pass on mount (a memory folder is read here, so a scene draws
    // the tree on its first frame); later passes follow every change.
    use_hook({
        let folder = folder.clone();
        let compose_files = compose_files.clone();
        let wanted = wanted.clone();
        move || reader.pump(&folder, &compose_files, wanted.as_deref())
    });
    {
        let folder = folder.clone();
        let compose_files = compose_files.clone();
        let wanted = wanted.clone();
        use_effect(move || {
            let _ = data.read();
            reader.pump(&folder, &compose_files, wanted.as_deref());
        });
    }

    let snapshot = data.read().clone();
    let rows = tree_rows(&snapshot);
    let selected = snapshot.selected.clone();
    let selected_role = selected.as_deref().map(|s| classify(file_name(s)));
    let target = selected
        .as_ref()
        .map(|rel| folder.join(rel))
        .or_else(|| default_editor_target(&compose_files, &folder));

    let open_editor = {
        let folder = folder.clone();
        let setting = editor_setting.clone();
        let target = target.clone();
        move |_| {
            let find = search_path_finder();
            if let Some(file) = &target {
                if let Some(command) =
                    open_in_editor_command(&find, setting.as_deref(), &folder, file)
                {
                    launch(command);
                }
            }
        }
    };
    let reveal = {
        let folder = folder.clone();
        move |_| {
            if let Some(command) = reveal_command(&search_path_finder(), &folder) {
                launch(command);
            }
        }
    };
    let terminal = {
        let folder = folder.clone();
        let setting = terminal_setting.clone();
        move |_| {
            if let Some(command) =
                terminal_command(&search_path_finder(), setting.as_deref(), &folder)
            {
                launch(command);
            }
        }
    };
    let copy_text = folder.to_string_lossy().into_owned();
    let hint = format!("Opens in {}", editor.label);

    rsx! {
        div { class: "pd-files",
            div { class: "pd-files-bar",
                span { class: "pd-files-path", title: "{copy_text}", "{folder_text}" }
                span { class: "pd-files-spacer" }
                span { class: "pd-files-hint", "{hint}" }
                Button {
                    label: "Copy path".to_string(),
                    icon: "copy",
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: {
                        let text = copy_text.clone();
                        move |_| crate::ui::key_value_list::copy_to_clipboard(&text)
                    },
                }
                Button {
                    label: "Open terminal here".to_string(),
                    icon: "terminal",
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: terminal,
                }
                Button {
                    label: "Reveal in Finder".to_string(),
                    icon: "folder",
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: reveal,
                }
                Button {
                    label: "Open in editor".to_string(),
                    icon: "open-external",
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Sm,
                    disabled: target.is_none(),
                    disabled_reason: target.is_none().then(|| "There is no file to open".to_string()),
                    onclick: open_editor,
                }
            }
            div { class: "pd-files-body",
                div { class: "pd-tree", role: "tree", "aria-label": "Project files",
                    for row in rows {
                        {
                            let key = row_key(&row);
                            rsx! {
                                div { key: "{key}",
                                    TreeRowView {
                                        row,
                                        revealed: revealed.read().clone(),
                                        on_open: move |entry: FsEntry| {
                                            let mut data = data;
                                            let mut state = data.write();
                                            let opening = opens_here(&entry);
                                            if opening && !state.expanded.remove(&entry.rel) {
                                                state.expanded.insert(entry.rel.clone());
                                            }
                                            if entry.kind != EntryKind::Dir {
                                                state.selected = Some(entry.rel.clone());
                                            }
                                        },
                                        on_reveal: move |(file, name): (String, String)| {
                                            let slot = (file.clone(), name.clone());
                                            if revealed.peek().contains_key(&slot) {
                                                revealed.write().remove(&slot);
                                                return;
                                            }
                                            reader.run(
                                                move |fs| fs.env_value(&file, &name),
                                                move |answer| {
                                                    if let Load::Ready(Some(value)) = answer {
                                                        revealed.write().insert(slot, value);
                                                    }
                                                },
                                            );
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "pd-preview-pane",
                    Pane {
                        selected: selected.clone(),
                        role: selected_role,
                        keys: selected.as_ref().and_then(|s| snapshot.keys.get(s).cloned()),
                        preview: selected.as_ref().and_then(|s| snapshot.previews.get(s).cloned()),
                        problem: problem.clone(),
                        compose_files: compose_files.clone(),
                        folder: folder.clone(),
                    }
                }
            }
            div { class: "pd-files-foot",
                IconButton {
                    icon: "rotate-cw",
                    label: "Read the folder again".to_string(),
                    size: ControlSize::Sm,
                    tooltip: "Read the folder again".to_string(),
                    onclick: move |_| {
                        data.set(TreeData::default());
                        revealed.write().clear();
                        on_refresh.call(());
                    },
                }
            }
        }
    }
}

/// Does a click on this entry open or close something under it?
fn opens_here(entry: &FsEntry) -> bool {
    super::tree::opens(entry)
}

fn row_key(row: &TreeRow) -> String {
    match row {
        TreeRow::Entry { entry, .. } => format!("e:{}", entry.rel),
        TreeRow::Key { file, key, .. } => format!("k:{file}:{key}"),
        TreeRow::Loading { depth } => format!("l:{depth}"),
        TreeRow::Failed { depth, .. } => format!("f:{depth}"),
        TreeRow::More { depth, .. } => format!("m:{depth}"),
        TreeRow::Empty { depth } => format!("z:{depth}"),
    }
}

fn indent_style(depth: usize) -> String {
    format!("padding-left: calc(var(--space-12) + {depth} * var(--space-16));")
}

#[component]
fn TreeRowView(
    row: TreeRow,
    revealed: BTreeMap<(String, String), String>,
    on_open: EventHandler<FsEntry>,
    on_reveal: EventHandler<(String, String)>,
) -> Element {
    match row {
        TreeRow::Entry {
            entry,
            depth,
            expanded,
            selected,
            key_count,
        } => {
            let openable = super::tree::opens(&entry);
            let icon = if entry.kind == EntryKind::Dir {
                "folder"
            } else {
                "file-text"
            };
            let chevron = if expanded {
                "chevron-down"
            } else {
                "chevron-right"
            };
            let class = if selected {
                "pd-row pd-row-selected"
            } else {
                "pd-row"
            };
            let label = if entry.kind == EntryKind::Dir {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            };
            let clicked = entry.clone();
            rsx! {
                button {
                    class,
                    r#type: "button",
                    role: "treeitem",
                    "aria-selected": selected.to_string(),
                    "aria-expanded": openable.then(|| expanded.to_string()),
                    style: "{indent_style(depth)}",
                    title: "{entry.rel}",
                    onclick: move |_| on_open.call(clicked.clone()),
                    span { class: "pd-chevron",
                        if openable {
                            Icon { name: chevron, size: IconSize::Sm }
                        }
                    }
                    Icon { name: icon, size: IconSize::Md }
                    span { class: "pd-row-name", "{label}" }
                    if entry.kind == EntryKind::Link {
                        span { class: "pd-row-tag", "link" }
                    }
                    if let Some(count) = key_count {
                        span { class: "pd-row-count",
                            if count == 1 { "1 key" } else { "{count} keys" }
                        }
                    }
                }
            }
        }
        TreeRow::Key { file, key, depth } => {
            let value = revealed.get(&(file.clone(), key.clone())).cloned();
            let shown = value.is_some();
            let reveal_file = file.clone();
            let reveal_key = key.clone();
            rsx! {
                div { class: "pd-row pd-key", style: "{indent_style(depth)}",
                    span { class: "pd-key-name", title: "{key}", "{key}" }
                    if let Some(value) = value {
                        span { class: "pd-key-value pd-key-shown", title: "{value}", "{value}" }
                    } else {
                        span { class: "pd-key-value", "aria-label": "Value hidden", "{MASK}" }
                    }
                    IconButton {
                        icon: "eye",
                        label: if shown { format!("Hide value of {key}") } else { format!("Reveal value of {key}") },
                        size: ControlSize::Sm,
                        tooltip: if shown { "Hide value".to_string() } else { "Reveal value".to_string() },
                        pressed: Some(shown),
                        onclick: move |_| on_reveal.call((reveal_file.clone(), reveal_key.clone())),
                    }
                }
            }
        }
        TreeRow::Loading { depth } => rsx! {
            div { class: "pd-row pd-row-note", style: "{indent_style(depth)}", role: "status",
                "Reading\u{2026}"
            }
        },
        TreeRow::Failed { depth, message } => rsx! {
            div { class: "pd-row pd-row-note pd-row-failed", style: "{indent_style(depth)}", role: "alert",
                "{message}"
            }
        },
        TreeRow::More { depth, count } => rsx! {
            div { class: "pd-row pd-row-note", style: "{indent_style(depth)}",
                "{thousands(count)} more not listed"
            }
        },
        TreeRow::Empty { depth } => rsx! {
            div { class: "pd-row pd-row-note", style: "{indent_style(depth)}", "Nothing here" }
        },
    }
}

#[component]
fn Pane(
    selected: Option<String>,
    role: Option<FileRole>,
    keys: Option<Load<Vec<String>>>,
    preview: Option<Load<Preview>>,
    problem: Option<ComposeProblem>,
    compose_files: Vec<PathBuf>,
    folder: PathBuf,
) -> Element {
    let Some(rel) = selected else {
        return rsx! {
            EmptyState {
                title: "Choose a file".to_string(),
                body: "Pick one on the left to read it here.".to_string(),
                icon: "file-text",
            }
        };
    };
    let name = file_name(&rel).to_string();
    if role == Some(FileRole::Env) {
        let count = match &keys {
            Some(Load::Ready(keys)) => Some(keys.len()),
            _ => None,
        };
        return rsx! {
            div { class: "pd-card",
                div { class: "pd-card-head",
                    Icon { name: "file-text", size: IconSize::Md }
                    span { class: "pd-card-name", "{name}" }
                    if let Some(count) = count {
                        span { class: "pd-card-meta",
                            if count == 1 { "1 key \u{b7} values hidden" } else { "{count} keys \u{b7} values hidden" }
                        }
                    }
                }
                p { class: "pd-card-note",
                    "The values in this file are secrets, so Berth lists only the names. "
                    "Use the eye beside a name to show one value; it hides again when you leave this tab."
                }
            }
        };
    }
    let error_line = problem.as_ref().and_then(|p| {
        let names_this = match &p.file {
            Some(file) => file_name(file) == name,
            None => compose_files
                .first()
                .and_then(|f| f.strip_prefix(&folder).ok())
                .is_some_and(|first| first.to_string_lossy() == rel),
        };
        names_this.then_some(p.line).flatten()
    });
    match preview {
        None | Some(Load::Loading) => rsx! {
            div { class: "pd-card",
                div { class: "pd-card-head",
                    Icon { name: "file-text", size: IconSize::Md }
                    span { class: "pd-card-name", "{name}" }
                }
                p { class: "pd-card-note", role: "status", "Reading\u{2026}" }
            }
        },
        Some(Load::Failed(err)) => rsx! {
            div { class: "pd-card",
                div { class: "pd-card-head",
                    Icon { name: "file-text", size: IconSize::Md }
                    span { class: "pd-card-name", "{name}" }
                }
                div { class: "pd-card-note",
                    InlineMessage { message: err.describe(&name), tone: InlineTone::Danger }
                }
            }
        },
        Some(Load::Ready(Preview::Binary { bytes })) => rsx! {
            div { class: "pd-card",
                div { class: "pd-card-head",
                    Icon { name: "file-text", size: IconSize::Md }
                    span { class: "pd-card-name", "{name}" }
                    span { class: "pd-card-meta", "{size_text(bytes)}" }
                }
                p { class: "pd-card-note", "This is not a text file, so there is nothing to show here." }
            }
        },
        Some(Load::Ready(Preview::Text {
            lines,
            total_lines,
            bytes,
            truncated,
        })) => {
            let count = if truncated {
                format!(
                    "first {} lines \u{b7} {}",
                    thousands(lines.len()),
                    size_text(bytes)
                )
            } else if total_lines == 1 {
                format!("1 line \u{b7} {}", size_text(bytes))
            } else {
                format!(
                    "{} lines \u{b7} {}",
                    thousands(total_lines),
                    size_text(bytes)
                )
            };
            rsx! {
                div { class: "pd-card",
                    div { class: "pd-card-head",
                        Icon { name: "file-text", size: IconSize::Md }
                        span { class: "pd-card-name", "{name}" }
                        span { class: "pd-card-meta", "{count}" }
                    }
                    div { class: "pd-code", role: "region", "aria-label": "Contents of {name}",
                        for (index, text) in lines.iter().enumerate() {
                            {
                                let number = index + 1;
                                let class = if error_line == Some(number) {
                                    "pd-line pd-line-error"
                                } else {
                                    "pd-line"
                                };
                                rsx! {
                                    div { key: "{number}", class,
                                        span { class: "pd-line-no", "{number}" }
                                        span { class: "pd-line-text", "{text}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
