//! One column header (PLAN 7.3): label, sort arrow, right alignment for
//! numbers. Plain markup; sorting state is the caller's signal.

use dioxus::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    None,
    Asc,
    Desc,
}

impl SortDirection {
    /// Icon sprite name for the arrow, `None` when unsorted.
    pub fn arrow_icon(self) -> Option<&'static str> {
        match self {
            SortDirection::None => None,
            SortDirection::Asc => Some("arrow-up"),
            SortDirection::Desc => Some("arrow-down"),
        }
    }

    /// Value for the `aria-sort` attribute.
    pub fn aria_sort(self) -> &'static str {
        match self {
            SortDirection::None => "none",
            SortDirection::Asc => "ascending",
            SortDirection::Desc => "descending",
        }
    }

    /// The direction after one more click: None → Asc → Desc → Asc.
    pub fn next(self) -> Self {
        match self {
            SortDirection::None | SortDirection::Desc => SortDirection::Asc,
            SortDirection::Asc => SortDirection::Desc,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellAlign {
    #[default]
    Left,
    /// Numbers (CPU, memory, sizes) per components-spec.
    Right,
}

impl CellAlign {
    pub fn class(self) -> &'static str {
        match self {
            CellAlign::Left => "align-left",
            CellAlign::Right => "align-right",
        }
    }
}

#[component]
pub fn TableHeaderCell(
    label: String,
    #[props(default)] sort: SortDirection,
    #[props(default)] align: CellAlign,
    #[props(default)] sortable: bool,
    onsort: Option<EventHandler<()>>,
) -> Element {
    let align_class = align.class();
    let sorted_class = if sort == SortDirection::None {
        ""
    } else {
        " sorted"
    };
    rsx! {
        div {
            class: "table-header-cell {align_class}{sorted_class}",
            role: "columnheader",
            "aria-sort": "{sort.aria_sort()}",
            if sortable {
                button {
                    class: "table-header-button {align_class}",
                    r#type: "button",
                    "aria-label": "Sort by {label}",
                    onclick: move |_| {
                        if let Some(handler) = onsort {
                            handler.call(());
                        }
                    },
                    span { class: "table-header-label", "{label}" }
                    if let Some(icon) = sort.arrow_icon() {
                        svg { class: "table-header-arrow", "aria-hidden": "true",
                            r#use { href: "/icons.svg#{icon}" }
                        }
                    }
                }
            } else {
                span { class: "table-header-label", "{label}" }
                if let Some(icon) = sort.arrow_icon() {
                    svg { class: "table-header-arrow", "aria-hidden": "true",
                        r#use { href: "/icons.svg#{icon}" }
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
    fn virtual_table_header_cell_sort_cycle_and_arrows() {
        assert_eq!(SortDirection::None.next(), SortDirection::Asc);
        assert_eq!(SortDirection::Asc.next(), SortDirection::Desc);
        assert_eq!(SortDirection::Desc.next(), SortDirection::Asc);
        assert_eq!(SortDirection::Asc.arrow_icon(), Some("arrow-up"));
        assert_eq!(SortDirection::Desc.arrow_icon(), Some("arrow-down"));
        assert_eq!(SortDirection::None.arrow_icon(), None);
    }

    #[test]
    fn virtual_table_header_cell_ssr_sorted_desc_shows_arrow() {
        let html = dioxus_ssr::render_element(rsx! {
            TableHeaderCell {
                label: "Name".to_string(),
                sort: SortDirection::Desc,
                sortable: true,
            }
        });
        assert!(html.contains("Name"), "{html}");
        assert!(html.contains("/icons.svg#arrow-down"), "{html}");
        assert!(html.contains("descending"), "{html}");
        assert!(html.contains("Sort by Name"), "{html}");
    }

    #[test]
    fn virtual_table_header_cell_ssr_numbers_right_aligned_unsorted() {
        let html = dioxus_ssr::render_element(rsx! {
            TableHeaderCell {
                label: "Memory".to_string(),
                align: CellAlign::Right,
            }
        });
        assert!(html.contains("align-right"), "{html}");
        assert!(!html.contains("arrow-"), "{html}");
        assert!(html.contains("Memory"), "{html}");
    }
}
