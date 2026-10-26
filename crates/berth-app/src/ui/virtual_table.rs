//! Fixed-row-height virtual table (PLAN D24). Only the visible window plus
//! overscan is rendered; spacer divs hold the scroll height. Rows are plain
//! elements from the caller's render callback — no per-row effects.

use dioxus::prelude::*;

/// Rows rendered above and below the viewport (D24).
pub const DEFAULT_OVERSCAN: usize = 10;
/// Fallback row height in px, matching `--size-row` (32, 24 compact).
/// The rendered height comes from CSS; this value only drives the range math.
pub const DEFAULT_ROW_HEIGHT_PX: u32 = 32;
/// Fallback viewport height when the caller does not size the table.
pub const DEFAULT_VIEWPORT_HEIGHT_PX: u32 = 560;

/// The range of row indexes to render for a scroll position (D24).
/// `scroll_top` and `viewport_h` are CSS pixels; fractional scroll offsets
/// from the browser are floored for the first row and ceiled for the last,
/// so a partially visible row at either edge is always included.
pub fn visible_range(
    scroll_top: f64,
    viewport_h: f64,
    row_h: u32,
    total: usize,
    overscan: usize,
) -> std::ops::Range<usize> {
    if total == 0 || row_h == 0 {
        return 0..0;
    }
    let row_h = f64::from(row_h);
    let top = scroll_top.max(0.0);
    let first_visible = (top / row_h).floor() as usize;
    let start = first_visible.saturating_sub(overscan).min(total);
    let last_visible = ((top + viewport_h.max(0.0)) / row_h).ceil() as usize;
    let end = last_visible.saturating_add(overscan).min(total).max(start);
    start..end
}

/// Height of the spacer standing in for rows before `range_start`.
pub fn top_spacer_px(range_start: usize, row_h: u32) -> u64 {
    range_start as u64 * u64::from(row_h)
}

/// Height of the spacer standing in for rows from `range_end` to `total`.
pub fn bottom_spacer_px(total: usize, range_end: usize, row_h: u32) -> u64 {
    total.saturating_sub(range_end) as u64 * u64::from(row_h)
}

/// A scrollable table body that renders only `visible_range` rows.
///
/// `row_height_px` must match the CSS row height (`--size-row`); it is a
/// prop so tests and SSR scenes render deterministically without measuring.
/// `viewport_height_px` is the scroll container's height in px; callers size
/// it to their layout. `initial_scroll_top_px` exists for scenes/tests.
/// With a `label` the body is the table itself; without one it sits inside a
/// labelled table (a header above it) and is that table's row group.
#[component]
pub fn VirtualTable(
    total: usize,
    render_row: Callback<usize, Element>,
    #[props(default = DEFAULT_ROW_HEIGHT_PX)] row_height_px: u32,
    #[props(default = DEFAULT_VIEWPORT_HEIGHT_PX)] viewport_height_px: u32,
    #[props(default = DEFAULT_OVERSCAN)] overscan: usize,
    #[props(default = 0)] initial_scroll_top_px: u32,
    #[props(default)] label: Option<String>,
) -> Element {
    let mut scroll_top = use_signal(|| f64::from(initial_scroll_top_px));
    let range = visible_range(
        *scroll_top.read(),
        f64::from(viewport_height_px),
        row_height_px,
        total,
        overscan,
    );
    let top_px = top_spacer_px(range.start, row_height_px);
    let bottom_px = bottom_spacer_px(total, range.end, row_height_px);
    rsx! {
        div {
            class: "virtual-table",
            role: if label.is_some() { "table" } else { "rowgroup" },
            "aria-label": label,
            "aria-rowcount": "{total}",
            style: "max-height: {viewport_height_px}px;",
            onscroll: move |e| {
                scroll_top.set(e.data.scroll_top());
            },
            div {
                class: "vt-spacer vt-spacer-top",
                style: "height: {top_px}px;",
            }
            for i in range.clone() {
                div {
                    key: "{i}",
                    class: "vt-row",
                    role: "row",
                    "aria-rowindex": "{i + 1}",
                    style: "height: {row_height_px}px;",
                    {render_row.call(i)}
                }
            }
            div {
                class: "vt-spacer vt-spacer-bottom",
                style: "height: {bottom_px}px;",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_scale::row_height_px as css_row_height_px;
    use berth_store::Density;

    const TABLES_CSS: &str = include_str!("../../assets/css/tables.css");

    /// The text of the `{ ... }` block of `selector` in `css`.
    fn rule<'a>(css: &'a str, selector: &str) -> &'a str {
        let start = css
            .find(&format!("\n{selector} {{"))
            .unwrap_or_else(|| panic!("no `{selector}` rule"))
            + selector.len()
            + 3;
        &css[start..start + css[start..].find('}').expect("rule closes")]
    }

    #[test]
    fn virtual_table_row_height_constant_is_the_stylesheet_row_height() {
        // The range maths assumes every row is `row_height_px` tall.
        assert_eq!(
            DEFAULT_ROW_HEIGHT_PX,
            css_row_height_px(Density::Comfortable, 1.0)
        );
        // The row's inline `height` is the box the table scrolls by, so the 1 px
        // border must sit inside it: with content-box sizing a bordered row is
        // 1 px taller than assumed and the scroll offsets drift 1 px per row.
        for selector in [".vt-row", ".table-header-cell"] {
            let body = rule(TABLES_CSS, selector);
            let borders = body.contains("border-bottom") || body.contains("border-top");
            assert!(
                !borders || body.contains("box-sizing: border-box;"),
                "`{selector}` has a border but no border-box sizing: {body}"
            );
            assert!(body.contains("height: var(--size-row);"), "{body}");
        }
    }

    #[test]
    fn virtual_table_visible_range_at_top_includes_overscan_below() {
        // 560px viewport / 28px rows = 20 visible, +10 overscan below only.
        assert_eq!(visible_range(0.0, 560.0, 28, 1000, 10), 0..30);
    }

    #[test]
    fn virtual_table_visible_range_at_bottom_clamps_to_total() {
        let range = visible_range(1000.0 * 28.0 - 560.0, 560.0, 28, 1000, 10);
        assert_eq!(range, 970..1000);
    }

    #[test]
    fn virtual_table_visible_range_mid_scroll_overscans_both_sides() {
        // Row 100 at the top, 20 visible: 100-10 .. 120+10.
        assert_eq!(visible_range(2800.0, 560.0, 28, 1000, 10), 90..130);
    }

    #[test]
    fn virtual_table_visible_range_zero_rows_is_empty() {
        assert_eq!(visible_range(0.0, 560.0, 28, 0, 10), 0..0);
        assert_eq!(visible_range(500.0, 560.0, 28, 0, 10), 0..0);
    }

    #[test]
    fn virtual_table_visible_range_partial_rows_at_edges_are_included() {
        // Scrolled 14px into row 0; viewport ends 14px into row 20.
        assert_eq!(visible_range(14.0, 560.0, 28, 1000, 0), 0..21);
        // Mid-row at both edges without overscan.
        assert_eq!(visible_range(2793.0, 100.0, 28, 1000, 0), 99..104);
    }

    #[test]
    fn virtual_table_visible_range_zero_row_height_is_empty() {
        assert_eq!(visible_range(0.0, 560.0, 0, 1000, 10), 0..0);
    }

    #[test]
    fn virtual_table_visible_range_scroll_past_end_is_empty_at_total() {
        assert_eq!(visible_range(999_999.0, 560.0, 28, 50, 10), 50..50);
    }

    #[test]
    fn virtual_table_visible_range_viewport_smaller_than_one_row() {
        assert_eq!(visible_range(0.0, 10.0, 28, 100, 0), 0..1);
    }

    #[test]
    fn virtual_table_visible_range_negative_scroll_clamps_to_top() {
        assert_eq!(visible_range(-120.0, 560.0, 28, 1000, 10), 0..30);
    }

    #[test]
    fn virtual_table_spacers_sum_with_visible_rows_to_total_height() {
        let range = visible_range(2800.0, 560.0, 28, 1000, 10);
        let sum = top_spacer_px(range.start, 28)
            + (range.end - range.start) as u64 * 28
            + bottom_spacer_px(1000, range.end, 28);
        assert_eq!(sum, 1000 * 28);
    }

    #[component]
    fn BigTable(initial_scroll_top_px: u32) -> Element {
        let render_row = use_callback(|i: usize| {
            rsx! { span { "Row {i}" } }
        });
        rsx! {
            VirtualTable {
                total: 1000_usize,
                render_row,
                row_height_px: 28_u32,
                viewport_height_px: 560_u32,
                overscan: 10_usize,
                initial_scroll_top_px,
            }
        }
    }

    fn render_big_table(initial_scroll_top_px: u32) -> String {
        let mut dom = VirtualDom::new_with_props(
            BigTable,
            BigTableProps {
                initial_scroll_top_px,
            },
        );
        let _ = dom.rebuild_to_vec();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn virtual_table_ssr_1000_rows_renders_at_most_60_row_elements() {
        let html = render_big_table(0);
        let rows = html.matches("vt-row").count();
        assert!(rows <= 60, "rendered {rows} rows: {html}");
        assert_eq!(rows, 30, "20 visible + 10 overscan");
        assert!(html.contains("Row 0"));
        assert!(html.contains("Row 29"));
        assert!(!html.contains("Row 30"));
        assert!(!html.contains("Row 999"));
    }

    #[test]
    fn virtual_table_ssr_spacers_hold_the_unrendered_height() {
        let html = render_big_table(0);
        assert!(html.contains("vt-spacer-top"), "{html}");
        // (1000 - 30) * 28 = 27160 below the rendered window.
        assert!(html.contains("height: 27160px"), "{html}");
    }

    #[test]
    fn virtual_table_ssr_initial_scroll_renders_the_scrolled_window() {
        let html = render_big_table(2800);
        assert!(html.contains("Row 100"), "{html}");
        assert!(!html.contains(">Row 0<"), "{html}");
        // 90 rows above the window: 90 * 28 = 2520.
        assert!(html.contains("height: 2520px"), "{html}");
        let rows = html.matches("vt-row").count();
        assert_eq!(rows, 40, "10 overscan + 20 visible + 10 overscan");
    }
}
