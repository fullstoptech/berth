use dioxus::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeKind {
    #[default]
    Count,
    Tag,
    /// Architecture ("amd64") — warning tone: it runs under emulation on arm64.
    Arch,
}

impl BadgeKind {
    pub fn class(self) -> &'static str {
        match self {
            Self::Count => "badge badge-count",
            Self::Tag => "badge badge-tag",
            Self::Arch => "badge badge-arch",
        }
    }
}

/// Counts and tags. Not status (use `StatusBadge` for that).
#[component]
pub fn Badge(#[props(default)] kind: BadgeKind, text: String) -> Element {
    rsx! {
        span { class: kind.class(), "{text}" }
    }
}
