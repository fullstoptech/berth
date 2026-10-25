//! The Projects table's column-drop tiers.

use std::fmt::Write as _;

use super::{scoped, threshold, write_or_check, Table, View};
use crate::screens::css_tracks as css;
use crate::text_scale::STEPS;

const PROJECTS_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/css/projects_tiers.css");

/// The tiers of the Projects table, widest first: the full list (read from
/// projects.css), then without Engine and Branch, without Ports, without Path.
const PROJECTS_NARROW: &str = "calc(116px * var(--text-scale, 1)) minmax(0, 104fr) minmax(0, 112fr) calc(104px * var(--text-scale, 1)) calc(48px * var(--text-scale, 1)) calc(56px * var(--text-scale, 1)) calc(160px * var(--text-scale, 1) + var(--projects-actions-w))";

const PROJECTS_NO_PORTS: &str = "minmax(calc(96px * var(--text-scale, 1)), 116fr) minmax(0, 104fr) minmax(0, 112fr) 0px calc(48px * var(--text-scale, 1)) calc(56px * var(--text-scale, 1)) calc(160px * var(--text-scale, 1) + var(--projects-actions-w))";

const PROJECTS_NO_PATH: &str = "minmax(calc(14ch + var(--size-icon-md) + var(--space-16)), 116fr) minmax(0, 216fr) 0px calc(48px * var(--text-scale, 1)) calc(56px * var(--text-scale, 1)) calc(160px * var(--text-scale, 1) + var(--projects-actions-w))";

/// What the lists leave to the flexible tracks, in px: Path and Services at
/// the 1024 frame (104 and 112), then Path at least 120 and Services 130 (so
/// Path goes between 120 and 130 % text at 1024), then Services alone 96.
const PROJECTS_RESERVE: [f32; 3] = [216.0, 250.0, 96.0];

pub fn projects_lists() -> [String; 4] {
    let hand = include_str!("../../../assets/css/projects.css");
    [
        css::declaration(css::block(hand, ".projects-screen {"), "--projects-cols"),
        PROJECTS_NARROW.to_string(),
        PROJECTS_NO_PORTS.to_string(),
        PROJECTS_NO_PATH.to_string(),
    ]
}

/// The buttons' track is 44 px (text apart) plus three gaps of 4 px under
/// 1200 px wide, 8 px from there up.
fn actions_w(view: View) -> f32 {
    match view {
        View::Narrow => 56.0,
        View::Wide => 68.0,
    }
}

/// The room a Projects list asks for: the minimums of its tracks, what the
/// flexible tracks need (`reserve`), the rows' 8 px padding and the page
/// gutter on both sides.
fn projects_need(tier: usize, scale: f32, view: View) -> f32 {
    let lists = projects_lists();
    let tracks = css::tracks(&lists[tier]);
    let vars = [("--projects-actions-w", actions_w(view))];
    css::min_sum_with(&tracks, scale, &vars)
        + projects_reserve(tier, scale, view)
        + 16.0
        + 2.0 * view.gutter()
}

/// The Projects tier (0 is the full list) for a table `width` px wide.
pub fn projects_tier_at(view: View, width: f32, scale: f32) -> usize {
    let mut tier = 0;
    while tier < 3 && width <= threshold(projects_need(tier, scale, view)) as f32 {
        tier += 1;
    }
    tier
}

/// The Projects track list in force.
pub fn projects_tracks_at(view: View, width: f32, scale: f32) -> Vec<String> {
    css::tracks(&projects_lists()[projects_tier_at(view, width, scale)])
}

/// What the flexible tracks of a tier keep: the buttons for the full list
/// (its `1fr` track), the `PROJECTS_RESERVE` of the others.
fn projects_reserve(tier: usize, scale: f32, view: View) -> f32 {
    if tier == 0 {
        160.0 * scale + actions_w(view)
    } else {
        PROJECTS_RESERVE[tier - 1]
    }
}

fn projects_tier_body(tier: usize, scale: f32, lists: &[String; 4]) -> String {
    let rule = |selectors: &str, declarations: &[&str]| {
        let body: String = declarations
            .iter()
            .map(|d| format!("      {d}\n"))
            .collect();
        format!("    {} {{\n{body}    }}\n", scoped(scale, selectors))
    };
    let cols = format!("--projects-cols: {};", lists[tier]);
    match tier {
        1 => [
            rule(".projects-table", &[&cols]),
            rule(
                ".projects-head > :nth-child(3), .projects-head > :nth-child(4), .project-cell-engine, .project-cell-branch, .project-problem-tag",
                &["display: none;"],
            ),
            rule(
                ".ports-unreachable",
                &["min-width: 0;", "overflow: hidden;", "text-overflow: ellipsis;"],
            ),
        ]
        .concat(),
        2 => [
            rule(".projects-table", &[&cols]),
            rule(
                ".projects-head > :nth-child(6), .project-cell-ports",
                &["padding: 0;", "overflow: hidden;"],
            ),
            rule(".project-cell-ports > *", &["visibility: hidden;"]),
            rule(
                ".project-row:has(.ports-unreachable) .project-cell-ports, .project-cell-ports.project-cell-span",
                &["grid-column: span 3;", "padding-right: var(--space-8);"],
            ),
            rule(
                ".project-row:has(.ports-unreachable) .project-cell-ports > *, .project-cell-ports.project-cell-span > *",
                &["visibility: visible;"],
            ),
            rule(
                ".project-row:has(.ports-unreachable) .project-cell-num",
                &["display: none;"],
            ),
            rule(".projects-header", &["flex-wrap: wrap;"]),
            rule(
                ".projects-controls",
                &["flex: 1 1 auto;", "justify-content: flex-end;"],
            ),
        ]
        .concat(),
        _ => [
            rule(".projects-table", &[&cols]),
            rule(
                ".projects-head > :nth-child(2), .project-cell-path",
                &["display: none;"],
            ),
        ]
        .concat(),
    }
}

fn projects_css() -> String {
    let lists = projects_lists();
    let mut out = String::from(
        "/* Generated by screens/table_tiers.rs from the track lists in projects.css.\n   \
         Never edit by hand: BERTH_REGEN_TIERS=1 cargo nextest run -p berth-app table_tiers.\n\n   \
         Tier 1 drops Engine and Branch; tier 2 closes Ports (the cells stay in the\n   \
         grid so every later cell keeps its track; an unreachable engine's message\n   \
         and the not-set-up hint use the Ports, CPU and Memory tracks together; the\n   \
         header controls wrap); tier 3 drops Path (its cells leave the grid, Name keeps\n   \
         14 characters and Services takes the rest). Each rule leaves a list at the\n   \
         exact width its tracks stop fitting for one text size: the minimums of the\n   \
         tracks, the room the flexible tracks keep, the rows' 8 px padding and both\n   \
         page gutters. */\n",
    );
    for view in [View::Narrow, View::Wide] {
        let _ = writeln!(out, "\n@media {} {{", view.media());
        for scale in STEPS {
            let _ = writeln!(out, "  /* text size {scale} */");
            for tier in 0..3 {
                let at = threshold(projects_need(tier, scale, view));
                let _ = writeln!(out, "  @container projects (max-width: {at}px) {{");
                out.push_str(&projects_tier_body(tier + 1, scale, &lists));
                out.push_str("  }\n");
            }
        }
        out.push_str("}\n");
    }
    out
}

pub(super) const TABLE: Table = Table {
    name: "projects",
    file: PROJECTS_FILE,
    css: projects_css,
    needs: projects_needs,
};

fn projects_needs(view: View, scale: f32) -> Vec<Vec<f32>> {
    vec![(0..4).map(|t| projects_need(t, scale, view)).collect()]
}

#[test]
fn table_tiers_projects_stylesheet_is_the_generator_output() {
    write_or_check(PROJECTS_FILE, &projects_css());
}

#[test]
fn table_tiers_projects_thresholds_fall_as_the_lists_narrow() {
    for view in [View::Narrow, View::Wide] {
        for scale in STEPS {
            let needs: Vec<f32> = (0..4).map(|t| projects_need(t, scale, view)).collect();
            assert!(
                needs.windows(2).all(|w| w[0] >= w[1]),
                "projects {view:?} {scale}: {needs:?}"
            );
        }
    }
}
