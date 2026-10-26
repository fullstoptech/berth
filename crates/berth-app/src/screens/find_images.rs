//! Images screen, tab "Find images" (P4-T3; PLAN §6.8, §6.17, brief §7.4).
//! Frames "Images / find / 1440", "Images / find / pulling / 1440" and
//! "Images / find / rate limited / 1440".
//!
//! A search field with the registry select and two switches, a results table
//! (44 px rows, two lines) and a 420 px detail panel with the tags per
//! architecture, the platform, the `docker pull` line, Pull, Run… and Add to
//! project…. The view model below is pure; the screen reads signals and the
//! [`FindStore`], and its buttons go through the stores (D23). Pulls run
//! through the Images service (`ImageStore::pull`), whose per-layer state
//! the rows and the panel read.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_core::{Arch, Engine, EngineState};
use berth_registry::{ImageHit, Publisher, TagInfo};

use crate::screens::images::{open_run, pull_target, PullCard};
use crate::services::find_images::{
    self, amd64_only, cached_ago, count_label, default_backend, download_size, name_candidates,
    platform_of, platform_options, publisher_label, pull_command, pulls_label, repo_of_reference,
    reset_text, suggestions, visible_hits, FindStore, HubApi, SearchRequest, SearchStatus,
    Suggestion, TagsEntry, AMD64_ONLY_NOTE, AMD64_ONLY_ROW, DEBOUNCE,
};
use crate::services::images::{with_default_tag, ImageStore, PullState};
use crate::services::Services;
use crate::state::{AddServiceSeed, AppState, Overlay, Toast, ToastTone};
use crate::ui::progress_bar::{ProgressBar, PROGRESS_BAR_CSS};
use crate::ui::{
    Banner, BannerTone, Button, ButtonVariant, CellAlign, ControlSize, EmptyState, Icon, IconSize,
    InlineMessage, InlineTone, SearchField, Select, SelectOption, SkeletonRows, Switch,
    TableHeaderCell,
};

/// The tab's CSS: the hand-written rules and the generated column tiers.
pub const FIND_IMAGES_CSS: &str = concat!(
    include_str!("../../assets/css/find_images.css"),
    include_str!("../../assets/css/find_images_tiers.css"),
);

/// The registries the select offers until Registries (P8-T2) adds more.
pub const REGISTRIES: [(&str, &str); 1] = [("hub", "Docker Hub")];

// ---------------------------------------------------------------------
// View model (pure)
// ---------------------------------------------------------------------

/// What the main area of the tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// Nothing typed: suggestions, or a hint when there are none.
    Suggestions,
    /// A search is on its way and nothing is shown yet.
    Searching,
    /// The table.
    Rows,
    /// The search found nothing (with a "Did you mean" when one is close).
    NoResults,
    /// The switches leave nothing of what was found.
    Filtered,
    /// Docker Hub could not be asked and nothing is cached.
    Unavailable,
}

#[must_use]
pub fn body(status: &SearchStatus, found: usize, shown: usize, has_query: bool) -> Body {
    if !has_query {
        return Body::Suggestions;
    }
    if shown > 0 {
        return Body::Rows;
    }
    if found > 0 {
        return Body::Filtered;
    }
    match status {
        SearchStatus::Idle => Body::Suggestions,
        SearchStatus::Searching => Body::Searching,
        SearchStatus::Done => Body::NoResults,
        SearchStatus::RateLimited { .. } | SearchStatus::Offline | SearchStatus::Failed(_) => {
            Body::Unavailable
        }
    }
}

/// "5 results", "3 of 5 results", "5 results · cached 6 min ago",
/// "Searching…".
#[must_use]
pub fn count_text(
    status: &SearchStatus,
    found: usize,
    shown: usize,
    cached_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> String {
    if *status == SearchStatus::Searching && found == 0 {
        return "Searching\u{2026}".to_string();
    }
    let noun = if found == 1 { "result" } else { "results" };
    let mut text = if shown == found {
        format!("{found} {noun}")
    } else {
        format!("{shown} of {found} {noun}")
    };
    if let Some(at) = cached_at {
        text.push_str(&format!(" \u{b7} cached {}", cached_ago(at, now)));
    }
    text
}

/// The banner over the controls, when Docker Hub could not answer in full:
/// its tone, message, action label and the line under it.
#[derive(Debug, Clone, PartialEq)]
pub struct BannerSpec {
    pub tone: BannerTone,
    pub message: String,
    pub action: &'static str,
    pub note: Option<&'static str>,
    /// The action runs the search again (else it is the Hub login).
    pub retry: bool,
}

#[must_use]
pub fn banner_spec(
    status: &SearchStatus,
    cached_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<BannerSpec> {
    match status {
        SearchStatus::RateLimited { reset_in } => {
            let mut message = "Docker Hub limits anonymous searches and pulls. Log in to Docker Hub to raise the limit."
                .to_string();
            if let Some(reset) = reset_in {
                message.push(' ');
                message.push_str(&reset_text(*reset));
            }
            Some(BannerSpec {
                tone: BannerTone::Warning,
                message,
                action: "Log in to Docker Hub",
                note: Some("Token stored in the macOS Keychain"),
                retry: false,
            })
        }
        SearchStatus::Offline => {
            let mut message = "You are offline. Docker Hub cannot be reached.".to_string();
            if let Some(at) = cached_at {
                message.push_str(&format!(" Showing results cached {}.", cached_ago(at, now)));
            }
            Some(BannerSpec {
                tone: BannerTone::Warning,
                message,
                action: "Try again",
                note: None,
                retry: true,
            })
        }
        SearchStatus::Failed(reason) => Some(BannerSpec {
            tone: BannerTone::Danger,
            message: format!("The search failed: {reason}"),
            action: "Try again",
            note: None,
            retry: true,
        }),
        _ => None,
    }
}

/// The platform the panel shows: the user's choice, else the engine's own,
/// else amd64 for an image with no arm64 build on an arm64 engine.
#[must_use]
pub fn effective_platform(chosen: Option<&str>, engine: &Arch, hit: &ImageHit) -> String {
    if let Some(chosen) = chosen {
        return chosen.to_string();
    }
    if amd64_only(hit) && *engine == Arch::Aarch64 {
        return "linux/amd64".to_string();
    }
    platform_of(engine)
}

/// The `--platform` the pull passes: none when it is the engine's own.
#[must_use]
pub fn pull_platform(platform: &str, engine: &Arch) -> Option<String> {
    (platform != platform_of(engine)).then(|| platform.to_string())
}

/// The tag the panel names: the user's, else the first the Hub listed,
/// else `latest` when the tag list is not known.
#[must_use]
pub fn effective_tag(chosen: Option<&str>, tags: &[TagInfo]) -> String {
    if let Some(chosen) = chosen {
        if tags.is_empty() || tags.iter().any(|t| t.name == chosen) {
            return chosen.to_string();
        }
    }
    tags.first()
        .map_or_else(|| "latest".to_string(), |t| t.name.clone())
}

/// Why Pull is off, or `None`.
#[must_use]
pub fn pull_blocked(
    engine: Option<&Engine>,
    tag: Option<&TagInfo>,
    platform: &str,
    running: bool,
) -> Option<String> {
    if engine.is_none() {
        return Some("No running engine to pull to".to_string());
    }
    if running {
        return Some("Already pulling".to_string());
    }
    if let Some(tag) = tag {
        if !find_images::tag_has_platform(tag, platform) {
            return Some(format!("This tag has no {platform} image"));
        }
    }
    None
}

/// The image id on the engine that carries `reference`, so Run… can open
/// the Run sheet on it.
#[must_use]
pub fn local_image_id(
    by_engine: &std::collections::BTreeMap<berth_core::EngineId, Vec<berth_core::Image>>,
    engine: &berth_core::EngineId,
    reference: &str,
) -> Option<String> {
    let wanted = with_default_tag(reference);
    by_engine
        .get(engine)?
        .iter()
        .find(|image| {
            image.repo_tags.iter().any(|tag| {
                let tag = tag.strip_prefix("docker.io/").unwrap_or(tag);
                let tag = tag.strip_prefix("library/").unwrap_or(tag);
                tag == wanted
            })
        })
        .map(|image| image.id.clone())
}

/// The pull of this Hub repository to show: a running one first, else the
/// newest.
#[must_use]
pub fn pull_of<'a>(pulls: &'a [PullState], repo: &str) -> Option<&'a PullState> {
    let mut of_repo = pulls
        .iter()
        .filter(|p| repo_of_reference(&p.reference) == repo);
    let first = of_repo.next()?;
    std::iter::once(first)
        .chain(of_repo)
        .find(|p| p.is_running())
        .or(Some(first))
}

/// The tag of a reference ("17-alpine" of `postgres:17-alpine`).
#[must_use]
pub fn tag_of_reference(reference: &str) -> String {
    let last = reference.rsplit('/').next().unwrap_or(reference);
    last.split_once(':')
        .map_or_else(|| "latest".to_string(), |(_, tag)| tag.to_string())
}

// ---------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------

/// "Add to project…": the Add service sheet on this image, with the project still to pick (P5-T9).
pub fn add_to_project(state: &mut AppState, reference: &str) {
    state
        .overlays
        .write()
        .push(Overlay::AddService(AddServiceSeed {
            project: None,
            template: None,
            image: Some(reference.to_string()),
        }));
}

fn push_toast(state: &mut AppState, text: &str) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text: text.to_string(),
        tone: ToastTone::Neutral,
        action: None,
    });
}

#[component]
fn PublisherCell(publisher: Publisher) -> Element {
    let icon = match publisher {
        Publisher::Official => Some("badge-check"),
        Publisher::Verified => Some("circle-check"),
        Publisher::SponsoredOss => Some("heart"),
        Publisher::Community => None,
    };
    let class = if publisher == Publisher::Community {
        "fi-cell fi-publisher fi-publisher-community"
    } else {
        "fi-cell fi-publisher"
    };
    rsx! {
        div { class,
            if let Some(name) = icon {
                span { class: "fi-publisher-icon",
                    Icon { name, size: IconSize::Sm }
                }
            }
            span { "{publisher_label(publisher)}" }
        }
    }
}

#[component]
fn PlatformsCell(hit: ImageHit) -> Element {
    rsx! {
        div { class: "fi-cell fi-platforms",
            if amd64_only(&hit) {
                span { class: "fi-emulated", title: "{AMD64_ONLY_NOTE}", "{AMD64_ONLY_ROW}" }
            } else if hit.arm64 == Some(true) {
                span { class: "fi-platform-check",
                    Icon { name: "check", size: IconSize::Sm }
                }
                span { "arm64" }
            } else {
                span { "\u{2014}" }
            }
        }
    }
}

/// A row's hit, whether it is open, its age text and a pull on its way
/// (the tag and the share done).
type RowData = (ImageHit, bool, String, Option<(String, Option<f32>)>);

#[component]
fn FindRow(
    hit: ImageHit,
    selected: bool,
    updated: String,
    pulling: Option<(String, Option<f32>)>,
    onselect: EventHandler<String>,
) -> Element {
    let class = if selected {
        "fi-row fi-selected"
    } else {
        "fi-row"
    };
    let name = hit.name.clone();
    rsx! {
        div {
            class,
            role: "row",
            "aria-selected": selected.to_string(),
            onclick: move |_| onselect.call(name.clone()),
            div { class: "fi-cell fi-name",
                span { class: "fi-name-text", title: "{hit.name}", "{hit.name}" }
                div { class: "fi-desc",
                    if let Some((tag, fraction)) = &pulling {
                        span { class: "fi-desc-text", "Pulling {tag}" }
                        ProgressBar {
                            fraction: *fraction,
                            thin: true,
                            aria_label: Some(format!("Pulling {tag}")),
                        }
                    } else {
                        span { class: "fi-desc-text", title: "{hit.description}", "{hit.description}" }
                    }
                }
            }
            PublisherCell { publisher: hit.publisher }
            div { class: "fi-cell fi-stars", "{count_label(u64::from(hit.stars))}" }
            div { class: "fi-cell fi-pulls", "{pulls_label(hit.pulls)}" }
            div { class: "fi-cell fi-updated", "{updated}" }
            PlatformsCell { hit: hit.clone() }
        }
    }
}

#[component]
fn SuggestionList(items: Vec<Suggestion>, onpick: EventHandler<String>) -> Element {
    if items.is_empty() {
        return rsx! {
            div { class: "fi-state",
                EmptyState {
                    title: "Search Docker Hub".to_string(),
                    body: "Type an image name such as postgres, redis or nginx.".to_string(),
                    icon: "search",
                }
            }
        };
    }
    rsx! {
        div { class: "fi-suggest",
            h2 { class: "fi-suggest-title", "Suggested for you" }
            ul { class: "fi-suggest-list",
                for item in items {
                    li { key: "{item.name}",
                        button {
                            class: "fi-suggest-item",
                            r#type: "button",
                            onclick: {
                                let name = item.name.clone();
                                move |_| onpick.call(name.clone())
                            },
                            span { "{item.name}" }
                            span { class: "fi-suggest-reason", "{item.reason}" }
                        }
                    }
                }
            }
        }
    }
}

/// The 420 px panel of the selected hit.
#[component]
#[allow(clippy::too_many_lines)]
fn Detail(
    hit: ImageHit,
    entry: Option<TagsEntry>,
    cached_at: Option<DateTime<Utc>>,
    rate_limited: bool,
    engine: Option<Engine>,
    local_id: Option<(String, String)>,
    pull: Option<PullState>,
    chosen_tag: Option<String>,
    chosen_platform: Option<String>,
    now: DateTime<Utc>,
    ontag: EventHandler<String>,
    onplatform: EventHandler<String>,
    onpull: EventHandler<(String, Option<String>)>,
    onrun: EventHandler<String>,
) -> Element {
    let mut state = use_context::<AppState>();
    let engine_arch = engine
        .as_ref()
        .and_then(|e| e.resources.as_ref())
        .map_or(Arch::Aarch64, |r| r.arch.clone());
    let tags: Vec<TagInfo> = match &entry {
        Some(TagsEntry::Ready { tags, .. }) => tags.clone(),
        _ => Vec::new(),
    };
    let tag_name = effective_tag(chosen_tag.as_deref(), &tags);
    let tag_info = tags.iter().find(|t| t.name == tag_name).cloned();
    let platform = effective_platform(chosen_platform.as_deref(), &engine_arch, &hit);
    let platform_arg = pull_platform(&platform, &engine_arch);
    let reference = find_images::reference(&hit.name, &tag_name);
    let command = pull_command(&reference, platform_arg.as_deref());
    let running = pull.as_ref().is_some_and(PullState::is_running);
    let blocked = pull_blocked(engine.as_ref(), tag_info.as_ref(), &platform, running);
    let options: Vec<SelectOption> = platform_options(&engine_arch)
        .into_iter()
        .map(|p| SelectOption::new(p.clone(), p))
        .collect();
    let run_id = local_id.as_ref().map(|(id, _)| id.clone());
    let cached_note = cached_at.map(|at| format!("Cached {}", cached_ago(at, now)));
    let tag_rows: Vec<(TagInfo, String, String, String)> = tags
        .iter()
        .map(|tag| {
            (
                tag.clone(),
                download_size(tag, &platform)
                    .map_or_else(|| "\u{2014}".to_string(), berth_core::fmt::bytes),
                find_images::platforms_label(tag),
                tag.pushed
                    .map_or_else(String::new, |at| find_images::ago(at, now)),
            )
        })
        .collect();
    let copy_text = command.clone();
    let pull_reference = reference.clone();
    let add_reference = reference.clone();
    let pull_with = platform_arg.clone();
    rsx! {
        aside { class: "fi-detail", "aria-label": "{hit.name}",
            div { class: "fd",
                div { class: "fd-title",
                    h2 { class: "fd-name", "{hit.name}" }
                    span { class: "fd-publisher",
                        if hit.publisher != Publisher::Community {
                            PublisherCell { publisher: hit.publisher }
                        }
                    }
                }
                p { class: "fd-desc", "{hit.description}" }
                if amd64_only(&hit) {
                    span { class: "fi-emulated fd-note", "{AMD64_ONLY_NOTE}" }
                }
                if let Some(note) = cached_note {
                    p { class: "fd-cached", "{note}" }
                }
                if let Some(pull) = pull.clone() {
                    PullCard { pull, compact: true }
                }
                h3 { class: "fd-heading", "Tags" }
                match entry {
                    None | Some(TagsEntry::Loading) => rsx! {
                        div { class: "fi-tags-loading", SkeletonRows { count: 4_usize } }
                    },
                    Some(TagsEntry::Failed(message)) => rsx! {
                        InlineMessage { message: format!("Could not load the tags: {message}"), tone: InlineTone::Danger }
                    },
                    Some(TagsEntry::Ready { .. }) => rsx! {
                        div { class: "fd-tags", role: "table", "aria-label": "Tags",
                            div { class: "fd-tag-head", role: "row",
                                span { "Tag" }
                                span { "Download" }
                                span { "Platforms" }
                                span { "Pushed" }
                            }
                            for (tag , size , platforms , when) in tag_rows {
                                button {
                                    key: "{tag.name}",
                                    class: if tag.name == tag_name { "fd-tag fd-selected" } else { "fd-tag" },
                                    r#type: "button",
                                    role: "row",
                                    "aria-selected": (tag.name == tag_name).to_string(),
                                    onclick: {
                                        let name = tag.name.clone();
                                        move |_| ontag.call(name.clone())
                                    },
                                    span { class: "fd-tag-name", "{tag.name}" }
                                    span { class: "fd-tag-size", "{size}" }
                                    span { class: "fd-tag-platforms",
                                        span { class: "fi-platform-check",
                                            Icon { name: "check", size: IconSize::Sm }
                                        }
                                        span { "{platforms}" }
                                    }
                                    span { class: "fd-tag-when", "{when}" }
                                }
                            }
                        }
                    },
                }
                if !running {
                    div { class: "fd-field",
                        h3 { class: "fd-heading", "Platform" }
                        Select {
                            options,
                            value: platform.clone(),
                            aria_label: "Platform".to_string(),
                            onchange: move |e: FormEvent| onplatform.call(e.value()),
                        }
                    }
                    div { class: "fd-command",
                        span { class: "fd-command-text", title: "{command}", "{command}" }
                        Button {
                            label: "Copy".to_string(),
                            icon: "copy",
                            size: ControlSize::Sm,
                            onclick: move |_| crate::ui::copy_to_clipboard(&copy_text),
                        }
                    }
                    div { class: "fd-actions",
                        Button {
                            label: format!("Pull {reference}"),
                            variant: ButtonVariant::Primary,
                            disabled: blocked.is_some(),
                            disabled_reason: blocked.clone(),
                            onclick: move |_| onpull.call((pull_reference.clone(), pull_with.clone())),
                        }
                        Button {
                            label: "Run\u{2026}".to_string(),
                            disabled: run_id.is_none(),
                            disabled_reason: run_id.is_none().then(|| "Pull the image first".to_string()),
                            onclick: move |_| {
                                if let Some(id) = &run_id {
                                    onrun.call(id.clone());
                                }
                            },
                        }
                        Button {
                            label: "Add to project\u{2026}".to_string(),
                            onclick: move |_| add_to_project(&mut state, &add_reference),
                        }
                    }
                    if rate_limited {
                        p { class: "fd-footnote", "Pulls may also be limited until you log in" }
                    }
                }
            }
        }
    }
}

/// The "Find images" tab body: controls, results, panel.
#[component]
#[allow(clippy::too_many_lines)]
pub fn FindImages() -> Element {
    let mut state = use_context::<AppState>();
    let find = use_hook(|| try_consume_context::<FindStore>().unwrap_or_default());
    let images = use_hook(|| try_consume_context::<ImageStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let backend: Arc<dyn HubApi> = use_hook(default_backend);
    let root = use_hook(|| {
        services
            .as_ref()
            .map_or_else(CancellationToken::new, |s| s.cancel.clone())
    });

    // The activity feed, once, for the suggestions.
    let recents_services = services.clone();
    use_effect(move || {
        if let Some(services) = recents_services.as_ref() {
            find.ensure_recent_pulls(services);
        }
    });

    let view = find.view.read().clone();
    let official_only = *find.official_only.read();
    let arm64_only = *find.arm64_only.read();
    let shown = visible_hits(&view.hits, official_only, arm64_only);
    let query = find.query.read().clone();
    let has_query = !query.trim().is_empty();
    let now = Utc::now();

    let engines = state.engines.read().clone();
    let scope = state.engine_scope.read().clone();
    let engine = pull_target(&engines, &scope).filter(|e| e.state == EngineState::Running);

    let selected_name = find.selected.read().clone();
    let current: Option<ImageHit> = shown
        .iter()
        .find(|h| selected_name.as_deref() == Some(h.name.as_str()))
        .or_else(|| shown.first())
        .cloned();

    // The panel's tags follow the hit it shows, also when a switch moved
    // the selection to the first hit left.
    let tags_backend = backend.clone();
    let tags_root = root.clone();
    let wanted = current.as_ref().map(|h| h.name.clone());
    use_effect(use_reactive!(|wanted| {
        if let Some(name) = &wanted {
            find.ensure_tags(name, &tags_backend, &tags_root);
        }
    }));

    let suggested = if has_query {
        Vec::new()
    } else {
        suggestions(
            &state.containers.read(),
            &state.projects.read(),
            &find.recent_pulls.read(),
            now,
        )
    };

    // One request builder for typing, a suggestion and Try again.
    let search_backend = backend.clone();
    let search_root = root.clone();
    let local_repos: Vec<String> = images
        .by_engine
        .read()
        .values()
        .flatten()
        .flat_map(|i| i.repo_tags.iter())
        .filter_map(|t| find_images::hub_name(&repo_of_reference(t)))
        .collect();
    let candidates = name_candidates(&local_repos, &suggested);
    let start_search = {
        let candidates = candidates.clone();
        move |text: String, debounce: std::time::Duration| {
            find.search(SearchRequest {
                text,
                debounce,
                parent: search_root.clone(),
                backend: search_backend.clone(),
                candidates: candidates.clone(),
            });
        }
    };

    let kind = body(&view.status, view.hits.len(), shown.len(), has_query);
    let banner = banner_spec(&view.status, view.cached_at, now);
    let count = if has_query {
        count_text(
            &view.status,
            view.hits.len(),
            shown.len(),
            view.cached_at,
            now,
        )
    } else {
        String::new()
    };
    let rev_key = *find.query_rev.read();
    let initial = query.clone();
    let registry_options: Vec<SelectOption> = REGISTRIES
        .iter()
        .map(|(value, label)| SelectOption::new(*value, *label))
        .collect();

    let pulls = images.pulls.read().clone();
    let by_engine = images.by_engine.read().clone();
    let rate_limited = matches!(view.status, SearchStatus::RateLimited { .. });
    let entry = current
        .as_ref()
        .and_then(|h| find.tags.read().get(&h.name).cloned());
    let chosen_tag = find.selected_tag.read().clone();
    let chosen_platform = find.platform.read().clone();

    let rows: Vec<RowData> = shown
        .iter()
        .map(|hit| {
            let pulling = pull_of(&pulls, &hit.name)
                .filter(|p| p.is_running())
                .map(|p| {
                    (
                        tag_of_reference(&p.reference),
                        p.percent().map(|pct| f32::from(pct) / 100.0),
                    )
                });
            (
                hit.clone(),
                current.as_ref().is_some_and(|c| c.name == hit.name),
                hit.updated
                    .map_or_else(String::new, |at| find_images::ago(at, now)),
                pulling,
            )
        })
        .collect();
    let row_count = rows.len();

    let select_backend = backend.clone();
    let select_root = root.clone();
    let on_select = move |name: String| {
        find.select(&name, &select_backend, &select_root);
    };
    let pick_search = start_search.clone();
    let retry_search = start_search.clone();
    let type_search = start_search.clone();
    let did_you_mean = view.did_you_mean.clone();
    let mut query_signal = find.query;
    let mut rev_signal = find.query_rev;
    let mut official_signal = find.official_only;
    let mut arm64_signal = find.arm64_only;
    let mut tag_signal = find.selected_tag;
    let mut platform_signal = find.platform;

    let pull_services = services.clone();
    let pull_engine = engine.clone();
    let on_pull = move |(reference, platform): (String, Option<String>)| {
        let (Some(engine), Some(services)) = (pull_engine.as_ref(), pull_services.as_ref()) else {
            return;
        };
        if let Err(message) = images.pull(services, &engine.id, &reference, platform.as_deref()) {
            tracing::warn!(%message, "find images: pull refused");
        }
    };
    let on_run = move |id: String| open_run(&mut state, &id);

    let panel = current.clone().filter(|_| kind == Body::Rows).map(|hit| {
        let local = engine.as_ref().and_then(|e| {
            let tag = effective_tag(
                chosen_tag.as_deref(),
                match &entry {
                    Some(TagsEntry::Ready { tags, .. }) => tags,
                    _ => &[],
                },
            );
            let reference = find_images::reference(&hit.name, &tag);
            local_image_id(&by_engine, &e.id, &reference).map(|id| (id, reference))
        });
        let cached = match &entry {
            Some(TagsEntry::Ready { cached_at, .. }) => cached_at.or(view.cached_at),
            _ => view.cached_at,
        };
        let pull = pull_of(&pulls, &hit.name).cloned();
        (hit, local, cached, pull)
    });

    rsx! {
        style { "{FIND_IMAGES_CSS}" }
        style { "{PROGRESS_BAR_CSS}" }
        if let Some(spec) = banner {
            div { class: "fi-banner",
                Banner {
                    message: spec.message.clone(),
                    tone: spec.tone,
                    action_label: Some(spec.action.to_string()),
                    onaction: {
                        let retry_text = query.clone();
                        let retry = spec.retry;
                        move |()| {
                            if retry {
                                retry_search(retry_text.clone(), std::time::Duration::ZERO);
                            } else {
                                push_toast(&mut state, "Registry login arrives with Registries");
                            }
                        }
                    },
                }
                if let Some(note) = spec.note {
                    p { class: "fi-banner-note", "{note}" }
                }
            }
        }
        div { class: "im-controls fi-controls",
            div { class: "im-filter",
                for key in [rev_key] {
                    SearchField {
                        key: "{key}",
                        initial_value: initial.clone(),
                        placeholder: "Search Docker Hub".to_string(),
                        aria_label: "Search images".to_string(),
                        oninput: {
                            let typed = type_search.clone();
                            move |e: FormEvent| typed(e.value(), DEBOUNCE)
                        },
                        onclear: move |_| {
                            rev_signal.set(key + 1);
                            find.clear();
                        },
                    }
                }
            }
            div { class: "fi-registry",
                Select {
                    options: registry_options,
                    value: "hub".to_string(),
                    aria_label: "Registry".to_string(),
                }
            }
            Switch {
                on: official_only,
                label: Some("Official only".to_string()),
                on_toggle: move |on: bool| official_signal.set(on),
            }
            Switch {
                on: arm64_only,
                label: Some("arm64 only".to_string()),
                on_toggle: move |on: bool| arm64_signal.set(on),
            }
            span { class: "fi-count", "{count}" }
        }
        div { class: "fi-body",
            div { class: "fi-main",
                match kind {
                    Body::Suggestions => rsx! {
                        SuggestionList {
                            items: suggested,
                            onpick: move |name: String| {
                                query_signal.set(name.clone());
                                rev_signal.set(rev_key + 1);
                                pick_search(name, std::time::Duration::ZERO);
                            },
                        }
                    },
                    Body::Searching => rsx! {
                        div { class: "fi-state", SkeletonRows { count: 6_usize } }
                    },
                    Body::NoResults => rsx! {
                        div { class: "fi-state",
                            EmptyState {
                                title: format!("No images found for \u{201c}{}\u{201d}", view.query),
                                body: did_you_mean
                                    .as_ref()
                                    .map_or_else(
                                        || "Check the spelling, or try a shorter name.".to_string(),
                                        |name| format!("Did you mean {name}?"),
                                    ),
                                icon: "search",
                                action_label: did_you_mean.as_ref().map(|name| format!("Search {name}")),
                                onaction: {
                                    let name = did_you_mean.clone().unwrap_or_default();
                                    move |()| {
                                        query_signal.set(name.clone());
                                        rev_signal.set(rev_key + 1);
                                        start_search(name.clone(), std::time::Duration::ZERO);
                                    }
                                },
                            }
                        }
                    },
                    Body::Filtered => rsx! {
                        div { class: "fi-state",
                            EmptyState {
                                title: "No images match the switches".to_string(),
                                body: "Turn off Official only or arm64 only to see the rest.".to_string(),
                                icon: "filter",
                            }
                        }
                    },
                    Body::Unavailable => rsx! {
                        div { class: "fi-state",
                            EmptyState {
                                title: "Nothing to show for this search".to_string(),
                                body: "Docker Hub could not be asked, and this search is not cached yet."
                                    .to_string(),
                                icon: "cloud",
                            }
                        }
                    },
                    Body::Rows => rsx! {
                        div { class: "fi-table", role: "table", "aria-label": "Search results", "aria-rowcount": "{row_count}",
                            div { class: "fi-head", role: "row",
                                TableHeaderCell { label: "Name".to_string() }
                                TableHeaderCell { label: "Publisher".to_string() }
                                TableHeaderCell { label: "Stars".to_string(), align: CellAlign::Right }
                                TableHeaderCell { label: "Pulls".to_string(), align: CellAlign::Right }
                                TableHeaderCell { label: "Updated".to_string(), align: CellAlign::Right }
                                TableHeaderCell { label: "Platforms".to_string() }
                            }
                            for (hit , is_selected , updated , pulling) in rows {
                                FindRow {
                                    key: "{hit.name}",
                                    hit,
                                    selected: is_selected,
                                    updated,
                                    pulling,
                                    onselect: on_select.clone(),
                                }
                            }
                        }
                    },
                }
            }
            if let Some((hit , local , cached , pull)) = panel {
                Detail {
                    key: "{hit.name}",
                    hit,
                    entry,
                    cached_at: cached,
                    rate_limited,
                    engine: engine.clone(),
                    local_id: local,
                    pull,
                    chosen_tag,
                    chosen_platform,
                    now,
                    ontag: move |name: String| tag_signal.set(Some(name)),
                    onplatform: move |value: String| platform_signal.set(Some(value)),
                    onpull: on_pull,
                    onrun: on_run,
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "find_images_tests.rs"]
mod tests;
