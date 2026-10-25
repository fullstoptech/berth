//! The toolbar over a log view (PLAN §7.6): Follow, Wrap, Time and Raw toggles, search with
//! "N of M", level chips with counts, stream and time-range selects, Next error, the
//! "142 new lines" pill and the more menu. It holds no state: the view passes a model in and
//! gets events out. The search text is uncontrolled (the field is remounted through `epoch`).

use std::collections::BTreeSet;

use berth_logs::{Level, LevelCounts, LogFilter, SearchSummary};
use chrono::{DateTime, Duration, Utc};
use dioxus::prelude::*;

use super::button::{Button, ButtonVariant};
use super::icon::{Icon, IconSize};
use super::icon_button::IconButton;
use super::kbd::{Kbd, KbdSymbol};
use super::menu::{Menu, MenuEntry};
use super::select::{Select, SelectOption};
use super::ControlSize;

/// Which streams the view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamChoice {
    #[default]
    All,
    Stdout,
    Stderr,
}

impl StreamChoice {
    pub const ALL: [Self; 3] = [Self::All, Self::Stdout, Self::Stderr];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All streams",
            Self::Stdout => "stdout only",
            Self::Stderr => "stderr only",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|c| c.id() == id)
            .unwrap_or_default()
    }

    /// `(stdout, stderr)` as the filter wants them.
    #[must_use]
    pub fn flags(self) -> (bool, bool) {
        match self {
            Self::All => (true, true),
            Self::Stdout => (true, false),
            Self::Stderr => (false, true),
        }
    }
}

/// How far back the view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeRange {
    #[default]
    All,
    Min5,
    Min15,
    Hour1,
    Hour24,
}

impl TimeRange {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Min5,
        Self::Min15,
        Self::Hour1,
        Self::Hour24,
    ];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Min5 => "5m",
            Self::Min15 => "15m",
            Self::Hour1 => "1h",
            Self::Hour24 => "24h",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All time",
            Self::Min5 => "Last 5 min",
            Self::Min15 => "Last 15 min",
            Self::Hour1 => "Last hour",
            Self::Hour24 => "Last 24 h",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|r| r.id() == id)
            .unwrap_or_default()
    }

    /// The earliest line shown when this range is chosen at `now` (a fixed cut-off: lines age
    /// out of the range only when it is chosen again).
    #[must_use]
    pub fn since(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let span = match self {
            Self::All => return None,
            Self::Min5 => Duration::minutes(5),
            Self::Min15 => Duration::minutes(15),
            Self::Hour1 => Duration::hours(1),
            Self::Hour24 => Duration::hours(24),
        };
        Some(now - span)
    }
}

/// The filter the toolbar's level chips, stream select and time range ask for.
#[must_use]
pub fn build_filter(
    levels: &BTreeSet<Level>,
    stream: StreamChoice,
    range: TimeRange,
    now: DateTime<Utc>,
) -> LogFilter {
    let (stdout, stderr) = stream.flags();
    LogFilter {
        stdout,
        stderr,
        levels: levels.clone(),
        containers: None,
        since: range.since(now),
    }
}

/// "4 of 4" once a match is current, "4 matches" before one is, `None` when nothing matches (the
/// caller shows "No matches").
#[must_use]
pub fn search_count_label(summary: &SearchSummary) -> Option<String> {
    match (summary.matches, summary.current) {
        (0, _) => None,
        (total, Some(current)) => Some(format!("{current} of {total}")),
        (1, None) => Some("1 match".to_string()),
        (total, None) => Some(format!("{total} matches")),
    }
}

/// The level chips: `(level, label, count shown)`. Error and Warn carry their counts when there
/// are any; Info and Debug are plain, as in the frames.
#[must_use]
pub fn level_chips(counts: &LevelCounts) -> [(Level, &'static str, Option<u32>); 4] {
    let some = |n: u32| (n > 0).then_some(n);
    [
        (Level::Error, "Error", some(counts.error)),
        (Level::Warn, "Warn", some(counts.warn)),
        (Level::Info, "Info", None),
        (Level::Debug, "Debug", None),
    ]
}

/// The pill text: "142 new lines".
#[must_use]
pub fn new_lines_label(count: u64) -> String {
    if count == 1 {
        "1 new line".to_string()
    } else {
        format!("{count} new lines")
    }
}

/// What the toolbar shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolbarModel {
    pub follow: bool,
    pub wrap: bool,
    pub time: bool,
    pub raw: bool,
    pub regex: bool,
    pub case_sensitive: bool,
    pub levels: BTreeSet<Level>,
    pub counts: LevelCounts,
    pub stream: StreamChoice,
    pub range: TimeRange,
    /// Seeds the search field; changing `epoch` remounts it with this text.
    pub query: String,
    pub epoch: u32,
    pub has_query: bool,
    pub search_label: Option<String>,
    pub new_lines: u64,
    pub menu_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolbarEvent {
    Follow,
    Wrap,
    Time,
    Raw,
    Search(String),
    ClearSearch,
    NextMatch,
    PrevMatch,
    Level(Level),
    Stream(StreamChoice),
    Range(TimeRange),
    NextError,
    PrevError,
    /// The "N new lines" pill.
    JumpToLatest,
    ToggleMenu,
    CloseMenu,
    /// A more-menu entry: `save`, `copy-all`, `clear`, `regex`, `case`.
    Menu(String),
}

/// The entries of the more menu.
#[must_use]
pub fn menu_entries(model: &ToolbarModel) -> Vec<MenuEntry> {
    let mut regex = MenuEntry::new("regex", "Regular expression");
    if model.regex {
        regex = regex.icon("check");
    }
    let mut case = MenuEntry::new("case", "Match case");
    if model.case_sensitive {
        case = case.icon("check");
    }
    vec![
        MenuEntry::new("save", "Save to file\u{2026}").icon("arrow-down-to-line"),
        MenuEntry::new("copy-all", "Copy all lines").icon("copy"),
        MenuEntry::new("clear", "Clear view").separator_before(),
        regex.separator_before(),
        case,
    ]
}

#[component]
fn ToggleButton(
    label: &'static str,
    on: bool,
    tooltip: &'static str,
    onclick: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            class: if on { "log-toggle log-toggle-on" } else { "log-toggle" },
            r#type: "button",
            "aria-pressed": on.to_string(),
            "data-tooltip": tooltip,
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

/// The toolbar.
#[component]
pub fn LogToolbar(model: ToolbarModel, onevent: EventHandler<ToolbarEvent>) -> Element {
    let chips = level_chips(&model.counts);
    let stream_options: Vec<SelectOption> = StreamChoice::ALL
        .iter()
        .map(|c| SelectOption::new(c.id(), c.label()))
        .collect();
    let range_options: Vec<SelectOption> = TimeRange::ALL
        .iter()
        .map(|r| SelectOption::new(r.id(), r.label()))
        .collect();
    let no_matches = model.has_query && model.search_label.is_none();
    let count_class = if no_matches {
        "search-count search-count-none"
    } else {
        "search-count"
    };
    let count_text = match (&model.search_label, no_matches) {
        (Some(label), _) => Some(label.clone()),
        (None, true) => Some("No matches".to_string()),
        (None, false) => None,
    };
    rsx! {
        div { class: "log-toolbar", role: "toolbar", "aria-label": "Log view",
            div { class: "log-toggles",
                ToggleButton {
                    label: "Follow",
                    on: model.follow,
                    tooltip: "Keep the newest line in view",
                    onclick: move |()| onevent.call(ToolbarEvent::Follow),
                }
                ToggleButton {
                    label: "Wrap",
                    on: model.wrap,
                    tooltip: "Wrap long lines",
                    onclick: move |()| onevent.call(ToolbarEvent::Wrap),
                }
                ToggleButton {
                    label: "Time",
                    on: model.time,
                    tooltip: "Show timestamps",
                    onclick: move |()| onevent.call(ToolbarEvent::Time),
                }
                ToggleButton {
                    label: "Raw",
                    on: model.raw,
                    tooltip: "Show JSON lines as written",
                    onclick: move |()| onevent.call(ToolbarEvent::Raw),
                }
            }
            span { class: "log-divider", "aria-hidden": "true" }
            div { class: "search-field log-search",
                span { class: "field-prefix",
                    Icon { name: "search", size: IconSize::Sm }
                }
                for epoch in [model.epoch] {
                    input {
                        key: "{epoch}",
                        class: "search-input",
                        r#type: "search",
                        initial_value: "{model.query}",
                        placeholder: "Search",
                        "aria-label": "Search lines",
                        onkeydown: move |e| {
                            use dioxus::html::ModifiersInteraction;
                            let key = e.key();
                            if key == Key::F8 {
                                // F8 is the shortcut for Next error, also while typing.
                                return;
                            }
                            if key == Key::Enter {
                                e.stop_propagation();
                                onevent.call(if e.data.modifiers().contains(dioxus::html::Modifiers::SHIFT) {
                                    ToolbarEvent::PrevMatch
                                } else {
                                    ToolbarEvent::NextMatch
                                });
                                return;
                            }
                            let mods = e.data.modifiers();
                            if !mods.contains(dioxus::html::Modifiers::META)
                                && !mods.contains(dioxus::html::Modifiers::CONTROL)
                            {
                                e.stop_propagation();
                            }
                        },
                        oninput: move |e| onevent.call(ToolbarEvent::Search(e.value())),
                    }
                }
                if let Some(text) = &count_text {
                    span { class: count_class, "{text}" }
                }
                if model.has_query {
                    button {
                        class: "search-clear",
                        r#type: "button",
                        "aria-label": "Clear search",
                        onclick: move |_| onevent.call(ToolbarEvent::ClearSearch),
                        Icon { name: "x", size: IconSize::Sm }
                    }
                }
            }
            div { class: "segmented log-levels", role: "group", "aria-label": "Levels",
                for (level, label, count) in chips {
                    {
                        let active = model.levels.contains(&level);
                        let mut class = String::from("segment");
                        if active {
                            class.push_str(" segment-selected");
                        } else if count.is_some() {
                            class.push_str(" log-level-has");
                        }
                        rsx! {
                            button {
                                key: "{label}",
                                class,
                                r#type: "button",
                                "aria-pressed": active.to_string(),
                                onclick: move |_| onevent.call(ToolbarEvent::Level(level)),
                                span { "{label}" }
                                if let Some(n) = count {
                                    span { class: "segment-count", "{n}" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "log-select",
                Select {
                    options: stream_options,
                    value: model.stream.id().to_string(),
                    aria_label: "Streams".to_string(),
                    onchange: move |e: FormEvent| onevent.call(ToolbarEvent::Stream(StreamChoice::from_id(&e.value()))),
                }
            }
            div { class: "log-select",
                Select {
                    options: range_options,
                    value: model.range.id().to_string(),
                    aria_label: "Time range".to_string(),
                    onchange: move |e: FormEvent| onevent.call(ToolbarEvent::Range(TimeRange::from_id(&e.value()))),
                }
            }
            button {
                class: "btn btn-ghost size-md log-next-error",
                r#type: "button",
                "data-tooltip": "Previous error \u{b7} \u{21e7}F8",
                onclick: move |_| onevent.call(ToolbarEvent::NextError),
                span { class: "btn-label", "Next error" }
                Kbd { symbol: KbdSymbol::Letter("F8".to_string()) }
            }
            span { class: "log-spacer" }
            if model.new_lines > 0 && !model.follow {
                Button {
                    label: new_lines_label(model.new_lines),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    icon: "arrow-down",
                    onclick: move |_| onevent.call(ToolbarEvent::JumpToLatest),
                }
            }
            div { class: "log-more",
                IconButton {
                    icon: "more-horizontal",
                    label: "More".to_string(),
                    tooltip: "More".to_string(),
                    pressed: Some(model.menu_open),
                    onclick: move |_| onevent.call(ToolbarEvent::ToggleMenu),
                }
                if model.menu_open {
                    div {
                        class: "log-menu-scrim",
                        onclick: move |_| onevent.call(ToolbarEvent::CloseMenu),
                    }
                    div { class: "log-menu",
                        Menu {
                            items: menu_entries(&model),
                            aria_label: "More".to_string(),
                            on_select: move |id: String| onevent.call(ToolbarEvent::Menu(id)),
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_toolbar_search_label_reads_current_of_matches() {
        assert_eq!(
            search_count_label(&SearchSummary {
                matches: 41,
                current: Some(3)
            }),
            Some("3 of 41".to_string())
        );
        assert_eq!(
            search_count_label(&SearchSummary {
                matches: 4,
                current: None
            }),
            Some("4 matches".to_string())
        );
        assert_eq!(search_count_label(&SearchSummary::default()), None);
    }

    #[test]
    fn logs_toolbar_level_chips_carry_counts_for_error_and_warn_only() {
        let counts = LevelCounts {
            error: 5,
            warn: 1,
            info: 30,
            debug: 2,
        };
        let chips = level_chips(&counts);
        assert_eq!(chips[0], (Level::Error, "Error", Some(5)));
        assert_eq!(chips[1], (Level::Warn, "Warn", Some(1)));
        assert_eq!(chips[2], (Level::Info, "Info", None));
        assert_eq!(chips[3], (Level::Debug, "Debug", None));
        let none = level_chips(&LevelCounts::default());
        assert_eq!(none[0].2, None, "no errors, no count");
    }

    #[test]
    fn logs_toolbar_filter_follows_chips_stream_and_range() {
        let now = DateTime::from_timestamp(1_792_850_550, 0).unwrap_or_default();
        let levels: BTreeSet<Level> = [Level::Error].into_iter().collect();
        let filter = build_filter(&levels, StreamChoice::Stderr, TimeRange::Min15, now);
        assert!(!filter.stdout && filter.stderr);
        assert_eq!(filter.levels, levels);
        assert_eq!(filter.since, Some(now - Duration::minutes(15)));
        let all = build_filter(&BTreeSet::new(), StreamChoice::All, TimeRange::All, now);
        assert!(all.stdout && all.stderr && all.since.is_none());
    }

    #[test]
    fn logs_toolbar_choices_round_trip_through_their_ids() {
        for choice in StreamChoice::ALL {
            assert_eq!(StreamChoice::from_id(choice.id()), choice);
        }
        for range in TimeRange::ALL {
            assert_eq!(TimeRange::from_id(range.id()), range);
        }
        assert_eq!(TimeRange::from_id("bogus"), TimeRange::All);
    }

    #[test]
    fn logs_toolbar_pill_text_counts_lines() {
        assert_eq!(new_lines_label(142), "142 new lines");
        assert_eq!(new_lines_label(1), "1 new line");
    }

    #[test]
    fn logs_toolbar_menu_marks_the_search_options_that_are_on() {
        let mut model = ToolbarModel::default();
        let entries = menu_entries(&model);
        assert!(entries.iter().all(|e| e.id != "regex" || e.icon.is_none()));
        model.regex = true;
        let entries = menu_entries(&model);
        let regex = entries.iter().find(|e| e.id == "regex").expect("regex");
        assert_eq!(regex.icon, Some("check"));
        assert!(entries.iter().any(|e| e.id == "clear"));
    }
}
