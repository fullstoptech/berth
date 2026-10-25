//! UI primitives (P2-T2), one file per component per PLAN 7.3 and
//! `design/components-spec.md`. All styling lives in `assets/css/ui.css`
//! and reads only `tokens.css` variables. Text inputs are uncontrolled
//! (D59); every other control is controlled through props + callbacks.

pub mod badge;
pub mod banner;
pub mod bottom_panel;
pub mod bulk_bar;
pub mod button;
pub mod checkbox;
pub mod confirm_dialog;
pub mod container_row;
pub mod empty_state;
pub mod engine_switcher;
pub mod file_browser;
pub mod files_panel;
pub mod group_header_row;
pub mod icon;
pub mod icon_button;
pub mod inline_message;
pub mod inspector;
pub mod inspector_header;
pub mod json_tree;
pub mod kbd;
pub mod key_value_list;
pub mod menu;
pub mod metric_tile;
pub mod palette_item;
pub mod palette_trigger;
pub mod problem_item;
pub mod project_row;
pub mod search_field;
pub mod segmented_control;
pub mod select;
pub mod sidebar;
pub mod sidebar_item;
pub mod skeleton_row;
pub mod status_badge;
pub mod status_bar;
pub mod status_dot;
pub mod switch;
pub mod table_header_cell;
pub mod tabs;
pub mod terminal;
pub mod text_field;
pub mod title_bar;
pub mod toast;
pub mod tooltip;
pub mod virtual_table;

pub use badge::{Badge, BadgeKind};
pub use banner::{Banner, BannerTone};
pub use bottom_panel::{clamp_panel_height, BottomPanel};
pub use bulk_bar::{BulkAction, BulkBar, BulkCounts};
pub use button::{Button, ButtonVariant};
pub use checkbox::{CheckState, Checkbox};
pub use confirm_dialog::{ConfirmDialog, ConfirmOverlay};
pub use empty_state::EmptyState;
pub use engine_switcher::{engine_dot, scope_includes, EngineSwitcher};
pub use file_browser::{FileBrowser, Listing};
pub use files_panel::FilesPanel;
pub use group_header_row::{count_label, GroupHeaderRow};
pub use icon::{Icon, IconSize};
pub use icon_button::{IconButton, IconButtonTone};
pub use inline_message::{InlineMessage, InlineTone};
pub use inspector::{inspector_visible, Inspector};
pub use inspector_header::{HeaderButton, HeaderTone, InspectorHeader};
pub use json_tree::JsonTree;
pub use kbd::{Kbd, KbdShortcut, KbdSymbol};
pub use key_value_list::{copy_to_clipboard, KeyValue, KeyValueList};
pub use menu::{Menu, MenuEntry};
pub use metric_tile::{MetricTile, MetricTileData, StatsPanel};
pub use palette_item::PaletteItem;
pub use palette_trigger::PaletteTrigger;
pub use problem_item::{FixButton, ProblemItem, ProblemView};
pub use search_field::SearchField;
pub use segmented_control::{SegmentedControl, SegmentedItem};
pub use select::{Select, SelectOption};
pub use sidebar::{sidebar_model, Sidebar, SidebarModel};
pub use sidebar_item::{project_dot, Dot, DotShape, DotSpec, SidebarItem};
pub use skeleton_row::{SkeletonRow, SkeletonRows};
pub use status_badge::StatusBadge;
pub use status_bar::{
    connection, engine_summary, problems_label, Connection, ProblemsChip, StatusBar,
};
pub use status_dot::StatusDot;
pub use switch::Switch;
pub use table_header_cell::{CellAlign, SortDirection, TableHeaderCell};
pub use tabs::{TabItem, Tabs, TabsVariant};
pub use text_field::TextField;
pub use title_bar::TitleBar;
pub use toast::{dismiss_toast, toast_timeout, ToastStack};
pub use tooltip::Tooltip;
pub use virtual_table::{visible_range, VirtualTable};

use dioxus::prelude::*;

/// Control heights come from the tokens: sm `--size-control-sm` (24),
/// md `--size-control-md` (28), lg `--size-control-lg` (32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ControlSize {
    Sm,
    #[default]
    Md,
    Lg,
}

impl ControlSize {
    pub fn class(self) -> &'static str {
        match self {
            Self::Sm => "size-sm",
            Self::Md => "size-md",
            Self::Lg => "size-lg",
        }
    }
}

/// The CSS tone class for a `berth_core::Tone` (components-spec tone map,
/// via `StatusKind::tone`).
pub fn tone_class(tone: berth_core::Tone) -> &'static str {
    match tone {
        berth_core::Tone::Success => "tone-success",
        berth_core::Tone::Warning => "tone-warning",
        berth_core::Tone::Danger => "tone-danger",
        berth_core::Tone::Stopped => "tone-stopped",
        berth_core::Tone::Info => "tone-info",
    }
}

/// The statuses the gallery and tests walk, in spec order.
pub fn all_statuses() -> [berth_core::StatusKind; 11] {
    use berth_core::StatusKind::*;
    [
        Running,
        Healthy,
        Starting,
        Restarting,
        Unhealthy,
        Paused,
        Exited(0),
        Exited(137),
        Created,
        Removing,
        Dead,
    ]
}

#[component]
fn GallerySection(title: &'static str, children: Element) -> Element {
    rsx! {
        section { class: "gallery-section",
            h2 { class: "gallery-title", "{title}" }
            div { class: "gallery-row", {children} }
        }
    }
}

/// Dev gallery for the "Components / primitives" scene: every variant and
/// state of every P2-T2 primitive, laid out like the Components page sets.
#[component]
pub fn PrimitivesGallery() -> Element {
    let statuses = all_statuses();
    rsx! {
        div { class: "gallery",
            h1 { class: "gallery-heading", "Components / primitives" }
            GallerySection { title: "Button",
                Button { label: "Stop container", variant: ButtonVariant::Primary }
                Button { label: "Stop", variant: ButtonVariant::Primary, size: ControlSize::Sm }
                Button { label: "Stop container", variant: ButtonVariant::Secondary }
                Button { label: "Stop", variant: ButtonVariant::Secondary, size: ControlSize::Sm }
                Button { label: "Stop", variant: ButtonVariant::Ghost }
                Button { label: "Delete container", variant: ButtonVariant::Danger }
                Button { label: "Stop container", variant: ButtonVariant::Primary, icon: "square" }
                Button { label: "Restart", variant: ButtonVariant::Secondary, kbd_letter: "R" }
                Button {
                    label: "Stop container",
                    variant: ButtonVariant::Primary,
                    disabled: true,
                    disabled_reason: "Container is not running",
                }
                Button { label: "Stopping\u{2026}", variant: ButtonVariant::Primary, loading: true }
                Button { label: "Stopping\u{2026}", variant: ButtonVariant::Danger, loading: true }
            }
            GallerySection { title: "IconButton",
                IconButton { icon: "rotate-cw", label: "Restart api-db-1", tooltip: "Restart \u{00b7} \u{2318}R" }
                IconButton { icon: "rotate-cw", label: "Restart api-db-1 (small)", size: ControlSize::Sm }
                IconButton { icon: "trash-2", label: "Delete api-db-1", tone: IconButtonTone::Danger }
                IconButton { icon: "rotate-cw", label: "Restart disabled", disabled: true }
                IconButton { icon: "rotate-cw", label: "Restart loading", loading: true }
                IconButton { icon: "wrap-text", label: "Wrap lines", pressed: true }
                Tooltip { text: "Restart", kbd_letter: "R",
                    IconButton { icon: "rotate-cw", label: "Restart with tooltip component" }
                }
            }
            GallerySection { title: "Kbd",
                Kbd { symbol: KbdSymbol::Cmd }
                Kbd { symbol: KbdSymbol::Shift }
                Kbd { symbol: KbdSymbol::Ctrl }
                Kbd { symbol: KbdSymbol::Option }
                Kbd { symbol: KbdSymbol::Backspace }
                Kbd { symbol: KbdSymbol::Return }
                Kbd { symbol: KbdSymbol::Letter("K".to_string()) }
                KbdShortcut { letter: "K" }
            }
            GallerySection { title: "Icon",
                Icon { name: "play", size: IconSize::Sm }
                Icon { name: "play" }
                Icon { name: "terminal" }
                Icon { name: "folder" }
                Icon { name: "alert-triangle" }
            }
            GallerySection { title: "StatusDot",
                for status in statuses {
                    span { key: "{status:?}", class: "gallery-status",
                        StatusDot { status }
                        span { {status.label()} }
                    }
                }
            }
            GallerySection { title: "StatusBadge",
                for status in statuses {
                    StatusBadge { key: "{status:?}", status }
                }
            }
            GallerySection { title: "Badge",
                Badge { kind: BadgeKind::Count, text: "14" }
                Badge { kind: BadgeKind::Tag, text: "latest" }
                Badge { kind: BadgeKind::Arch, text: "amd64" }
            }
            GallerySection { title: "TextField",
                TextField {
                    label: "Container name",
                    id: "gallery-name",
                    placeholder: "e.g. shop-api-cache-1",
                    help: "Letters, digits, dashes and underscores.",
                }
                TextField {
                    label: "Container name",
                    id: "gallery-name-filled",
                    initial_value: "shop-api-cache-1",
                    help: "Letters, digits, dashes and underscores.",
                }
                TextField {
                    label: "Container name",
                    id: "gallery-name-error",
                    initial_value: "shop-api-api-1",
                    error: "Name is already used by shop-api-api-1",
                }
                TextField {
                    label: "Container name",
                    id: "gallery-name-disabled",
                    placeholder: "e.g. shop-api-cache-1",
                    disabled: true,
                    help: "Letters, digits, dashes and underscores.",
                }
                TextField {
                    label: "Searchable name",
                    id: "gallery-name-icon",
                    placeholder: "e.g. shop-api-cache-1",
                    prefix_icon: "search",
                }
                TextField {
                    label: "Name with shortcut",
                    id: "gallery-name-kbd",
                    placeholder: "e.g. shop-api-cache-1",
                    kbd: KbdSymbol::Return,
                }
            }
            GallerySection { title: "SearchField",
                SearchField { placeholder: "Filter containers" }
                SearchField { initial_value: "api", matched: 12, total: 48, aria_label: "Filter containers" }
                SearchField { initial_value: "zzz", matched: 0, total: 48, aria_label: "Filter containers" }
            }
            GallerySection { title: "Select",
                Select {
                    label: "Engine",
                    id: "gallery-engine",
                    value: "default",
                    options: vec![
                        SelectOption::new("default", "default (Colima)"),
                        SelectOption::new("work", "work (Colima)"),
                        SelectOption::new("hetzner-1", "hetzner-1 (SSH)"),
                    ],
                }
                Select {
                    label: "Engine",
                    id: "gallery-engine-disabled",
                    value: "default",
                    disabled: true,
                    options: vec![SelectOption::new("default", "default (Colima)")],
                }
            }
            GallerySection { title: "SegmentedControl",
                SegmentedControl {
                    selected: "running",
                    aria_label: "Project filter",
                    items: vec![
                        SegmentedItem::with_count("all", "All", 19),
                        SegmentedItem::with_count("running", "Running", 14),
                        SegmentedItem::with_count("failing", "Failing", 2),
                        SegmentedItem::with_count("stopped", "Stopped", 3),
                    ],
                }
            }
            GallerySection { title: "Checkbox / Switch",
                Checkbox { state: CheckState::Off, aria_label: "Select api-db-1" }
                Checkbox { state: CheckState::On, aria_label: "Select api-db-1" }
                Checkbox { state: CheckState::Mixed, aria_label: "Select all" }
                Checkbox { state: CheckState::On, disabled: true, aria_label: "Select disabled" }
                Checkbox { state: CheckState::Off, label: "Also delete its anonymous volumes" }
                Switch { on: false, label: "Follow" }
                Switch { on: true, label: "Follow" }
                Switch { on: true, disabled: true, label: "Wrap" }
            }
            GallerySection { title: "Tabs",
                Tabs {
                    selected: "logs",
                    aria_label: "Container detail",
                    items: vec![
                        TabItem::new("overview", "Overview"),
                        TabItem::new("logs", "Logs"),
                        TabItem::new("terminal", "Terminal"),
                        TabItem::new("stats", "Stats"),
                        TabItem::new("inspect", "Inspect"),
                        TabItem::new("env", "Env"),
                        TabItem::new("files", "Files"),
                        TabItem::new("limits", "Limits"),
                    ],
                }
                Tabs {
                    selected: "assistant",
                    variant: TabsVariant::Panel,
                    aria_label: "Bottom panel",
                    items: vec![
                        TabItem::closable("logs", "Logs"),
                        TabItem::closable("terminal", "Terminal"),
                        TabItem::with_count("problems", "Problems", 3),
                        TabItem::new("assistant", "Assistant"),
                    ],
                }
            }
            GallerySection { title: "Menu",
                Menu {
                    aria_label: "Container actions",
                    active: 1,
                    items: vec![
                        MenuEntry::new("terminal", "Open terminal").icon("terminal").kbd("T"),
                        MenuEntry::new("logs", "Show logs").icon("logs").kbd("L"),
                        MenuEntry::new("copy", "Copy container ID").icon("copy"),
                        MenuEntry::new("pause", "Pause").icon("pause").separator_before(),
                        MenuEntry::new("kill", "Kill").icon("zap").danger(),
                        MenuEntry::new("delete", "Delete container\u{2026}")
                            .icon("trash-2")
                            .danger()
                            .separator_before(),
                    ],
                }
                Menu {
                    aria_label: "Container actions with a disabled item",
                    items: vec![
                        MenuEntry::new("terminal", "Open terminal")
                            .icon("terminal")
                            .disabled("Container is not running"),
                        MenuEntry::new("logs", "Show logs").icon("logs").kbd("L"),
                    ],
                }
            }
            GallerySection { title: "Tooltip",
                Tooltip { text: "Copy container ID",
                    IconButton { icon: "copy", label: "Copy container ID" }
                }
                Tooltip { text: "Terminal unavailable: the container is not running.",
                    IconButton { icon: "terminal", label: "Open terminal", disabled: true }
                }
            }
        }
    }
}

#[cfg(test)]
mod shell_tests;
#[cfg(test)]
mod tests;
