//! The inspector body for one image (P4-T1; no frame, so it follows the
//! container panel): a header with Run, Pull latest, Tag and Delete, and the
//! tabs Overview (tags, platform, size, ports), Layers (the history) and
//! Containers (what uses it).
//!
//! Self-contained: the architect mounts [`ImageDetailPanel`] in the
//! inspector for `Selection::Image`. The view model is pure; the panel reads
//! signals and the [`ImageStore`] and sends clicks to it (D23). Tag is an
//! inline bar under the header, like Rename on a container.

use dioxus::prelude::*;

use berth_core::{ContainerId, Engine, EngineId, Image, ImageDetail};

use crate::screens::images::{delete_reason, open_delete, open_run, short_id, ImageRow};
use crate::services::images::{check_reference, check_tag, ImageEntry, ImageStore};
use crate::services::Services;
use crate::state::{AppState, Section, Selection, Toast, ToastTone};
use crate::ui::{
    copy_to_clipboard, Badge, BadgeKind, Button, ButtonVariant, ControlSize, IconButton,
    InlineMessage, InlineTone, KeyValue, KeyValueList, TabItem, Tabs, TextField,
};

/// The panel's CSS: the container panel's header, tabs and rows, then the
/// parts only an image has (in `images.css`).
pub const IMAGE_DETAIL_CSS: &str = concat!(
    include_str!("../../assets/css/container_detail.css"),
    include_str!("../../assets/css/images.css"),
);

// ---------------------------------------------------------------------
// View model (pure)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageTab {
    #[default]
    Overview,
    Layers,
    Containers,
}

impl ImageTab {
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Layers => "layers",
            Self::Containers => "containers",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Overview, Self::Layers, Self::Containers]
            .into_iter()
            .find(|t| t.id() == id)
    }
}

/// "linux/arm64", or "—" when the engine did not say.
#[must_use]
pub fn platform_label(image: &Image) -> String {
    let arch = image.arch.docker_name();
    if arch.is_empty() || arch == "unknown" {
        return String::new();
    }
    if image.os.is_empty() {
        arch.to_string()
    } else {
        format!("{}/{arch}", image.os)
    }
}

/// The Overview rows: ID, size, created, platform, engine, digest, ports,
/// environment keys (never values: an image's env is its defaults, but the
/// keys are all the panel needs) and what uses it.
#[must_use]
pub fn overview_rows(
    detail: &ImageDetail,
    engine_name: &str,
    containers: &[berth_core::Container],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<KeyValue> {
    let image = &detail.image;
    let ports = detail
        .exposed_ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let env = if detail.env_keys.is_empty() {
        String::new()
    } else {
        detail.env_keys.join(", ")
    };
    let used = crate::screens::images::used_by_text(&image.used_by, containers);
    vec![
        KeyValue::mono("ID", short_id(&image.id)),
        KeyValue::text("Size", berth_core::fmt::bytes(image.size)),
        KeyValue::text(
            "Created",
            format!(
                "{} ({})",
                image.created.format("%Y-%m-%d %H:%M"),
                berth_core::fmt::age(image.created, now)
            ),
        ),
        KeyValue::mono("Platform", platform_label(image)).empty_text("\u{2014}"),
        KeyValue::text("Engine", engine_name),
        KeyValue::mono("Digest", image.digests.first().cloned().unwrap_or_default()),
        KeyValue::mono("Exposed ports", ports),
        KeyValue::mono("Environment", env).empty_text("No variables"),
        KeyValue::text(
            "Used by",
            if image.used_by.is_empty() {
                String::new()
            } else {
                used
            },
        )
        .empty_text("Nothing"),
    ]
}

/// One line of an image's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerRow {
    pub size: String,
    pub command: String,
    pub age: String,
}

/// The history line Docker wrote, read as the Dockerfile line it came from:
/// `/bin/sh -c #(nop) CMD ["x"]` is `CMD ["x"]`, `/bin/sh -c apk add x` is
/// `RUN apk add x`, BuildKit's trailing `# buildkit` goes.
#[must_use]
pub fn layer_command(created_by: &str) -> String {
    let text = created_by.trim();
    let text = text.strip_suffix("# buildkit").unwrap_or(text).trim();
    if let Some(rest) = text.strip_prefix("/bin/sh -c #(nop)") {
        return rest.trim().to_string();
    }
    if let Some(rest) = text.strip_prefix("/bin/sh -c ") {
        return format!("RUN {}", rest.trim());
    }
    if text.is_empty() {
        return "(no command)".to_string();
    }
    text.to_string()
}

/// The history, newest first as Docker lists it.
#[must_use]
pub fn layer_rows(detail: &ImageDetail, now: chrono::DateTime<chrono::Utc>) -> Vec<LayerRow> {
    detail
        .layers
        .iter()
        .map(|layer| LayerRow {
            size: if layer.size.as_u64() == 0 {
                "0 B".to_string()
            } else {
                berth_core::fmt::bytes(layer.size)
            },
            command: layer_command(&layer.created_by),
            age: layer
                .created
                .map(|at| berth_core::fmt::age(at, now))
                .unwrap_or_default(),
        })
        .collect()
}

/// Split `repo:tag` for the Tag bar; no tag reads as `latest`. A registry
/// port (`localhost:5000/app`) is not a tag.
#[must_use]
pub fn split_tag(reference: &str) -> (String, String) {
    let last = reference.rsplit('/').next().unwrap_or(reference);
    match last.rsplit_once(':') {
        Some((_, tag)) if !reference.contains('@') => (
            reference[..reference.len() - tag.len() - 1].to_string(),
            tag.to_string(),
        ),
        _ => (reference.to_string(), "latest".to_string()),
    }
}

/// Why the Tag button is off, or `None` when `repo:tag` is a name Docker
/// takes. A tag is part of the name, so one that has a digest is refused.
#[must_use]
pub fn tag_problem(repo: &str, tag: &str) -> Option<String> {
    let repo = repo.trim();
    let tag = tag.trim();
    if repo.is_empty() {
        return Some("Type a repository such as myapp".to_string());
    }
    if repo.contains('@') || repo.rsplit('/').next().is_some_and(|l| l.contains(':')) {
        return Some("Put the tag in the Tag field".to_string());
    }
    if tag.is_empty() {
        return Some("Type a tag such as latest".to_string());
    }
    check_tag(tag)
        .and_then(|()| check_reference(&format!("{repo}:{tag}")).map(drop))
        .err()
}

// ---------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------

fn toast(state: &mut AppState, text: String, tone: ToastTone) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone,
        action: None,
    });
}

/// The inline Tag bar: repository and tag, Tag and Cancel.
#[component]
fn TagBar(engine: EngineId, image: Image, onclose: EventHandler<()>) -> Element {
    let mut state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let (repo0, tag0) = image
        .repo_tags
        .first()
        .map_or_else(|| (String::new(), "latest".to_string()), |t| split_tag(t));
    let mut repo = use_signal(|| repo0.clone());
    let mut tag = use_signal(|| tag0.clone());
    let mut failure = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let problem = tag_problem(&repo.read(), &tag.read());
    let target = image.clone();
    rsx! {
        div { class: "imd-tag-bar", role: "group", "aria-label": "Tag image",
            div { class: "imd-tag-fields",
                TextField {
                    label: "Repository".to_string(),
                    initial_value: repo0,
                    placeholder: "myapp".to_string(),
                    aria_label: "Repository".to_string(),
                    oninput: move |e: FormEvent| {
                        repo.set(e.value());
                        failure.set(None);
                    },
                }
                TextField {
                    label: "Tag".to_string(),
                    initial_value: tag0,
                    placeholder: "latest".to_string(),
                    aria_label: "Tag".to_string(),
                    oninput: move |e: FormEvent| {
                        tag.set(e.value());
                        failure.set(None);
                    },
                }
            }
            if let Some(message) = failure.read().clone() {
                InlineMessage { message, tone: InlineTone::Danger }
            }
            div { class: "imd-tag-actions",
                Button {
                    label: "Tag".to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Sm,
                    disabled: problem.is_some(),
                    disabled_reason: problem.clone(),
                    loading: *busy.read(),
                    onclick: move |_| {
                        let Some(services) = services.as_ref() else { return };
                        let repo_text = repo.peek().trim().to_string();
                        let tag_text = tag.peek().trim().to_string();
                        busy.set(true);
                        let name = format!("{repo_text}:{tag_text}");
                        store.tag(services, &engine, &target, &repo_text, &tag_text, move |result| {
                            busy.set(false);
                            match result {
                                Ok(()) => {
                                    toast(&mut state, format!("Tagged as {name}"), ToastTone::Success);
                                    onclose.call(());
                                }
                                Err(message) => failure.set(Some(message)),
                            }
                        });
                    },
                }
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: move |_| onclose.call(()),
                }
            }
        }
    }
}

/// The image the panel shows and the engine that has it.
fn find_image(
    store: &ImageStore,
    scope: &crate::state::EngineScope,
    id: &str,
) -> Option<(EngineId, Image)> {
    store
        .find(id, scope)
        .map(|image| (image.engine.clone(), image))
}

/// The inspector body for one selected image. Keyed by the caller per image
/// so the tab and the Tag bar reset on a new selection.
#[component]
pub fn ImageDetailPanel(id: String, #[props(default)] initial_tab: ImageTab) -> Element {
    let mut state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let mut tab = use_signal(move || initial_tab);
    let mut tagging = use_signal(|| false);

    let scope = state.engine_scope.read().clone();
    let found = find_image(&store, &scope, &id);
    let first_seen = use_hook(|| found.clone());

    let effect_id = id.clone();
    let effect_services = services.clone();
    let owner = found.as_ref().map(|(engine, _)| engine.clone());
    use_effect(move || {
        let _ = store.by_engine.read();
        if let (Some(engine), Some(services)) = (owner.clone(), effect_services.as_ref()) {
            store.ensure_detail(services, &engine, &effect_id);
        }
    });

    let Some((engine_id, image)) = found else {
        let name = first_seen
            .as_ref()
            .and_then(|(_, i)| i.repo_tags.first().cloned())
            .unwrap_or_else(|| short_id(&id));
        let mut selection = state.selection;
        return rsx! {
            style { "{IMAGE_DETAIL_CSS}" }
            div { class: "cd cd-removed",
                h2 { class: "cd-removed-name", "{name}" }
                p { class: "cd-removed-text", "This image was removed." }
                Button {
                    label: "Close".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: move |_| selection.set(Selection::None),
                }
            }
        };
    };

    let engines: Vec<Engine> = state.engines.read().clone();
    let engine_name = engines
        .iter()
        .find(|e| e.id == engine_id)
        .map_or_else(|| engine_id.0.clone(), |e| e.name.clone());
    let engine_arch = engines
        .iter()
        .find(|e| e.id == engine_id)
        .and_then(|e| e.resources.as_ref())
        .map_or(berth_core::Arch::Aarch64, |r| r.arch.clone());
    let row = ImageRow::new(&engine_id, &image, &engine_arch);
    let title = if row.name.is_empty() {
        "<none>".to_string()
    } else {
        row.name.clone()
    };
    let entry = store
        .details
        .read()
        .get(&(engine_id.clone(), id.clone()))
        .cloned();
    let containers = state.containers.read().clone();
    let now = chrono::Utc::now();
    let selected = *tab.read();
    let using: Vec<(ContainerId, String)> = image
        .used_by
        .iter()
        .map(|cid| {
            (
                cid.clone(),
                containers
                    .iter()
                    .find(|c| c.id == *cid)
                    .map_or_else(|| cid.0.chars().take(12).collect(), |c| c.name.clone()),
            )
        })
        .collect();
    let tab_items = vec![
        TabItem::new("overview", "Overview"),
        TabItem::new("layers", "Layers"),
        TabItem::with_count("containers", "Containers", using.len()),
    ];
    let first_tag = image.repo_tags.first().cloned();
    let reason = delete_reason(&row);
    let copy_full = image.id.clone();
    let run_id = image.id.clone();
    let delete_id = image.id.clone();
    let delete_engine = engine_id.clone();
    let pull_engine = engine_id.clone();
    let pull_services = services.clone();
    let pull_ref = first_tag.clone();
    let pull_reason = first_tag
        .is_none()
        .then(|| "An image with no tag has nothing to pull again".to_string());

    rsx! {
        style { "{IMAGE_DETAIL_CSS}" }
        div { class: "cd",
            header { class: "ih",
                div { class: "ih-title",
                    h2 { class: "ih-name", title: "{title}", "{title}" }
                }
                div { class: "ih-status",
                    if row.foreign_arch {
                        Badge { kind: BadgeKind::Arch, text: row.arch.clone() }
                    } else {
                        Badge { kind: BadgeKind::Tag, text: row.arch.clone() }
                    }
                    span { class: "ih-note", "{berth_core::fmt::bytes(image.size)} \u{b7} {engine_name}" }
                }
                div { class: "ih-id",
                    span { class: "ih-id-label", "ID" }
                    span { class: "ih-id-value", "{row.short_id}" }
                    button {
                        class: "ih-id-copy",
                        r#type: "button",
                        "aria-label": "Copy image ID",
                        onclick: move |_| copy_to_clipboard(&copy_full),
                        crate::ui::Icon { name: "copy", size: crate::ui::IconSize::Sm }
                    }
                }
                div { class: "ih-buttons",
                    span { class: "ih-btn",
                        Button {
                            label: "Run".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            icon: "play",
                            onclick: move |_| open_run(&mut state, &run_id),
                        }
                    }
                    span { class: "ih-btn",
                        Button {
                            label: "Pull latest".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            icon: "arrow-down-to-line",
                            disabled: pull_reason.is_some(),
                            disabled_reason: pull_reason,
                            onclick: move |_| {
                                let (Some(reference), Some(services)) = (pull_ref.as_ref(), pull_services.as_ref()) else {
                                    return;
                                };
                                match store.pull(services, &pull_engine, reference, None) {
                                    Ok(()) => toast(&mut state, format!("Pulling {reference}"), ToastTone::Neutral),
                                    Err(message) => toast(&mut state, message, ToastTone::Danger),
                                }
                            },
                        }
                    }
                    span { class: "ih-btn",
                        Button {
                            label: "Tag".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            onclick: move |_| {
                                let open = *tagging.peek();
                                tagging.set(!open);
                            },
                        }
                    }
                    span { class: "ih-btn ih-btn-danger",
                        Button {
                            label: "Delete".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            icon: "trash-2",
                            disabled: reason.is_some(),
                            disabled_reason: reason,
                            onclick: move |_| open_delete(&mut state, &store, &delete_engine, &delete_id),
                        }
                    }
                }
            }
            if *tagging.read() {
                TagBar {
                    key: "{id}",
                    engine: engine_id.clone(),
                    image: image.clone(),
                    onclose: move |()| tagging.set(false),
                }
            }
            div { class: "cd-tabs",
                Tabs {
                    items: tab_items,
                    selected: selected.id().to_string(),
                    aria_label: "Image detail".to_string(),
                    on_select: move |tab_id: String| {
                        if let Some(next) = ImageTab::from_id(&tab_id) {
                            tab.set(next);
                        }
                    },
                }
            }
            div { class: "cd-body", role: "tabpanel",
                {
                    match selected {
                        ImageTab::Overview => overview_body(&image, entry, &engine_name, &containers, now),
                        ImageTab::Layers => layers_body(entry, now),
                        ImageTab::Containers => containers_body(&using, &mut state),
                    }
                }
            }
        }
    }
}

fn entry_problem(entry: Option<&ImageEntry>) -> Option<Element> {
    match entry {
        None => Some(rsx! { p { class: "cd-note", "Loading details\u{2026}" } }),
        Some(ImageEntry::Failed { message, gone }) => {
            let text = if *gone {
                "This image no longer exists.".to_string()
            } else {
                format!("Could not read the details: {message}")
            };
            Some(rsx! { InlineMessage { message: text, tone: InlineTone::Danger } })
        }
        Some(ImageEntry::Ready(_)) => None,
    }
}

fn overview_body(
    image: &Image,
    entry: Option<ImageEntry>,
    engine_name: &str,
    containers: &[berth_core::Container],
    now: chrono::DateTime<chrono::Utc>,
) -> Element {
    let tags = image.repo_tags.clone();
    let detail = match entry {
        Some(ImageEntry::Ready(detail)) => Some(*detail),
        other => {
            if let Some(problem) = entry_problem(other.as_ref()) {
                return rsx! {
                    TagList { tags }
                    {problem}
                };
            }
            None
        }
    };
    let rows = detail
        .as_ref()
        .map(|d| overview_rows(d, engine_name, containers, now))
        .unwrap_or_default();
    rsx! {
        TagList { tags }
        KeyValueList { rows }
    }
}

#[component]
fn TagList(tags: Vec<String>) -> Element {
    if tags.is_empty() {
        return rsx! {
            section { class: "cd-section",
                h3 { class: "cd-section-title", "Tags" }
                p { class: "cd-note", "No tags: nothing names this image (dangling)." }
            }
        };
    }
    rsx! {
        section { class: "cd-section",
            h3 { class: "cd-section-title", "Tags" }
        }
        ul { class: "imd-tags",
            for tag in tags {
                {
                    let copy = tag.clone();
                    rsx! {
                        li { key: "{tag}", class: "imd-tag",
                            span { class: "imd-tag-text", title: "{tag}", "{tag}" }
                            IconButton {
                                icon: "copy",
                                label: format!("Copy {tag}"),
                                size: ControlSize::Sm,
                                onclick: move |_| copy_to_clipboard(&copy),
                            }
                        }
                    }
                }
            }
        }
    }
}

fn layers_body(entry: Option<ImageEntry>, now: chrono::DateTime<chrono::Utc>) -> Element {
    let detail = match entry {
        Some(ImageEntry::Ready(detail)) => *detail,
        other => {
            return entry_problem(other.as_ref()).unwrap_or_else(|| rsx! {});
        }
    };
    let rows = layer_rows(&detail, now);
    if rows.is_empty() {
        return rsx! { p { class: "cd-note", "The engine kept no history for this image." } };
    }
    rsx! {
        for (index , row) in rows.into_iter().enumerate() {
            div { key: "{index}", class: "imd-layer",
                span { class: "imd-layer-size", "{row.size}" }
                span { class: "imd-layer-cmd", "{row.command}" }
                if !row.age.is_empty() {
                    span { class: "imd-layer-when", "{row.age}" }
                }
            }
        }
    }
}

fn containers_body(using: &[(ContainerId, String)], state: &mut AppState) -> Element {
    if using.is_empty() {
        return rsx! { p { class: "cd-note", "No container uses this image." } };
    }
    let state = *state;
    rsx! {
        ul { class: "imd-containers",
            for (id , name) in using.iter().cloned() {
                li { key: "{id.0}", class: "imd-container",
                    button {
                        class: "imd-container-name",
                        r#type: "button",
                        title: "Show {name}",
                        onclick: move |_| {
                            let mut state = state;
                            state.selection.set(Selection::Containers(vec![id.clone()]));
                            state.section.set(Section::Containers);
                        },
                        "{name}"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "image_detail_tests.rs"]
mod tests;
