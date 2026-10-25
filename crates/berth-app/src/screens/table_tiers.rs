//! The column-drop tiers of the Containers and Projects tables, generated.
//!
//! A table drops columns as its container narrows: Containers loses Image,
//! then Uptime, then Engine, then gives Name a tighter minimum; Projects loses
//! Engine and Branch, then Ports, then Path. Each tier is a track list that
//! fits down to the width its tracks add up to. Em thresholds could not say
//! that: the gutters, the checkbox and the buttons' padding do not grow with
//! the text, so at 85 % the thresholds fell in the wrong place.
//!
//! This module reads the hand-written track lists, sums their minimums per
//! text size (`css_tracks`) and writes `@container (max-width: Npx)` rules
//! for every text size, scoped by `:root[data-text-scale="…"]` (set next to
//! `--text-scale`). The output is `assets/css/containers_tiers.css` and
//! `assets/css/projects_tiers.css`, checked in; the tests below fail when
//! they are stale. Regenerate with
//!
//! ```text
//! BERTH_REGEN_TIERS=1 cargo nextest run -p berth-app table_tiers
//! ```
//!
//! The same chains answer "which list is in force at this width" for the
//! table tests, so a test and the stylesheet cannot disagree.

use std::fmt::Write as _;

use super::css_tracks as css;
use crate::text_scale::STEPS;

const CONTAINERS_HAND: &str = include_str!("../../assets/css/containers.css");
const CONTAINERS_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/css/containers_tiers.css"
);
const PROJECTS_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/css/projects_tiers.css");

/// The viewport the page gutter and the button gaps depend on: 16 px and 4 px
/// under 1200 px wide, 20 px and 8 px from there up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Narrow,
    Wide,
}

impl View {
    pub(super) fn gutter(self) -> f32 {
        match self {
            Self::Narrow => 16.0,
            Self::Wide => 20.0,
        }
    }

    pub(super) fn media(self) -> &'static str {
        match self {
            Self::Narrow => "(max-width: 1199px)",
            Self::Wide => "(min-width: 1200px)",
        }
    }
}

/// A column a tier hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    Image,
    Uptime,
    Engine,
}

impl Col {
    fn selectors(self) -> &'static [&'static str] {
        match self {
            Self::Image => &[".ct-image", ".ct-image-head"],
            Self::Uptime => &[".ct-uptime"],
            Self::Engine => &[".ct-engine"],
        }
    }
}

/// `max-width` for the container query that leaves a list: the widest width
/// (whole px) under which the list's `need` no longer fits.
pub(super) fn threshold(need: f32) -> u32 {
    // The 0.001 keeps a need that is a whole number of px from rounding up.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let up = (need - 0.001).ceil().max(1.0) as u32;
    up - 1
}

/// Selectors scoped to a text size. The default size also matches before the
/// attribute is set.
pub(super) fn scoped(scale: f32, selectors: &str) -> String {
    let roots: Vec<String> = if (scale - 1.0).abs() < f32::EPSILON {
        vec![
            ":root:not([data-text-scale])".to_string(),
            ":root[data-text-scale=\"1\"]".to_string(),
        ]
    } else {
        vec![format!(":root[data-text-scale=\"{scale}\"]")]
    };
    selectors
        .split(", ")
        .flat_map(|selector| {
            roots
                .iter()
                .map(move |root| format!("{root} {selector}"))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join(",\n    ")
}

// ---------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------

/// One track list of a family and the columns it has hidden.
#[derive(Debug, Clone)]
pub struct Step {
    pub cols: String,
    pub hide: Vec<Col>,
}

/// The lists one family of the table steps through as it narrows, widest first.
#[derive(Debug, Clone)]
pub struct Chain {
    /// The table classes the family has (Uptime and Engine columns).
    pub uptime: bool,
    pub engine: bool,
    pub selector: &'static str,
    pub steps: Vec<Step>,
}

fn hand(header: &str, name: &str) -> String {
    css::declaration(css::block(CONTAINERS_HAND, header), name)
}

fn step(cols: String, hide: &[Col]) -> Step {
    Step {
        cols,
        hide: hide.to_vec(),
    }
}

/// The room a Containers list asks for: its tracks' minimums, the 12 px
/// gaps and the page gutter on both sides.
fn containers_need(step: &Step, scale: f32, view: View) -> f32 {
    let tracks = css::tracks(&step.cols);
    css::min_sum(&tracks, scale) + 12.0 * (tracks.len() as f32 - 1.0) + 2.0 * view.gutter()
}

fn containers_chains(view: View) -> Vec<Chain> {
    use Col::{Engine, Image, Uptime};
    let plain = ("\n.ct-table {", "--ct-cols", "--ct-cols-noimage");
    let both = "\n.ct-table.ct-has-uptime.ct-has-engine {";
    let compact = hand("\n.ct-table {", "--ct-cols-compact");
    let list = |header: &str, name: &str| hand(header, name);
    match view {
        View::Narrow => {
            let header = "@media (max-width: 1199px) {\n  .ct-table,";
            let narrow = css::block(CONTAINERS_HAND, header);
            vec![Chain {
                uptime: false,
                engine: false,
                selector: ".ct-table",
                steps: vec![
                    step(css::declaration(narrow, "--ct-cols"), &[]),
                    step(css::declaration(narrow, "--ct-cols-noimage"), &[Image]),
                    step(compact, &[Image]),
                ],
            }]
        }
        View::Wide => vec![
            Chain {
                uptime: false,
                engine: false,
                selector: ".ct-table:not(.ct-has-uptime):not(.ct-has-engine)",
                steps: vec![
                    step(list(plain.0, plain.1), &[]),
                    step(list(plain.0, plain.2), &[Image]),
                    step(compact.clone(), &[Image]),
                ],
            },
            Chain {
                uptime: false,
                engine: true,
                selector: ".ct-table.ct-has-engine:not(.ct-has-uptime)",
                steps: vec![
                    step(list("\n.ct-table.ct-has-engine {", "--ct-cols"), &[]),
                    step(
                        list("\n.ct-table.ct-has-engine {", "--ct-cols-noimage"),
                        &[Image],
                    ),
                    step(list(plain.0, plain.2), &[Image, Engine]),
                    step(compact.clone(), &[Image, Engine]),
                ],
            },
            Chain {
                uptime: true,
                engine: false,
                selector: ".ct-table.ct-has-uptime:not(.ct-has-engine)",
                steps: vec![
                    step(list("\n.ct-table.ct-has-uptime {", "--ct-cols"), &[]),
                    step(
                        list("\n.ct-table.ct-has-uptime {", "--ct-cols-noimage"),
                        &[Image],
                    ),
                    step(list(plain.0, plain.2), &[Image, Uptime]),
                    step(compact.clone(), &[Image, Uptime]),
                ],
            },
            Chain {
                uptime: true,
                engine: true,
                selector: ".ct-table.ct-has-uptime.ct-has-engine",
                steps: vec![
                    step(list(both, "--ct-cols"), &[]),
                    step(list(both, "--ct-cols-noimage"), &[Image]),
                    step(
                        list("\n.ct-table.ct-has-engine {", "--ct-cols-noimage"),
                        &[Image, Uptime],
                    ),
                    step(list(plain.0, plain.2), &[Image, Uptime, Engine]),
                    step(compact, &[Image, Uptime, Engine]),
                ],
            },
        ],
    }
}

/// The lists in force for a Containers table `width` px wide.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainersState {
    pub tracks: Vec<String>,
    pub hidden: Vec<Col>,
    /// Which step of its chain (0 is the full list).
    pub step: usize,
}

pub fn containers_state_at(
    view: View,
    uptime: bool,
    engine: bool,
    width: f32,
    scale: f32,
) -> ContainersState {
    let chains = containers_chains(view);
    let chain = match view {
        View::Narrow => &chains[0],
        View::Wide => chains
            .iter()
            .find(|c| c.uptime == uptime && c.engine == engine)
            .unwrap_or(&chains[0]),
    };
    let mut index = 0;
    while index + 1 < chain.steps.len()
        && width <= threshold(containers_need(&chain.steps[index], scale, view)) as f32
    {
        index += 1;
    }
    ContainersState {
        tracks: css::tracks(&chain.steps[index].cols),
        hidden: chain.steps[index].hide.clone(),
        step: index,
    }
}

/// The room a Containers table needs in its narrowest state: what the
/// inspector's main-area floor has to cover.
pub fn containers_least_need(view: View, scale: f32) -> f32 {
    containers_chains(view)
        .iter()
        .map(|chain| containers_need(chain.steps.last().expect("a chain has steps"), scale, view))
        .fold(0.0, f32::max)
}

fn containers_css() -> String {
    let mut out = String::from(
        "/* Generated by screens/table_tiers.rs from the track lists in containers.css.\n   \
         Never edit by hand: BERTH_REGEN_TIERS=1 cargo nextest run -p berth-app table_tiers.\n\n   \
         Each rule leaves a list at the exact width its tracks stop fitting (the sum of\n   \
         their minimums, the 12 px gaps and both page gutters) for one text size. */\n",
    );
    for view in [View::Narrow, View::Wide] {
        let _ = writeln!(out, "\n@media {} {{", view.media());
        for scale in STEPS {
            let _ = writeln!(out, "  /* text size {scale} */");
            for chain in containers_chains(view) {
                for (k, window) in chain.steps.windows(2).enumerate() {
                    let at = threshold(containers_need(&chain.steps[k], scale, view));
                    let next = &window[1];
                    let _ = writeln!(out, "  @container containers (max-width: {at}px) {{");
                    let _ = writeln!(
                        out,
                        "    {} {{\n      --ct-cols: {};\n    }}",
                        scoped(scale, chain.selector),
                        next.cols
                    );
                    let hidden: Vec<String> = next
                        .hide
                        .iter()
                        .flat_map(|col| col.selectors())
                        .map(|sel| format!("{} {sel}", chain.selector))
                        .collect();
                    if !hidden.is_empty() {
                        let _ = writeln!(
                            out,
                            "    {} {{\n      display: none;\n    }}",
                            scoped(scale, &hidden.join(", "))
                        );
                    }
                    out.push_str("  }\n");
                }
            }
        }
        out.push_str("}\n");
    }
    out
}

// ---------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------

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
    let hand = include_str!("../../assets/css/projects.css");
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

// ---------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------

const IMAGES_HAND: &str = include_str!("../../assets/css/images.css");
const IMAGES_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/css/images_tiers.css");

/// A column the Images table drops as it narrows: Created first, then ID,
/// then Arch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageCol {
    Created,
    Id,
    Arch,
}

impl ImageCol {
    /// The cell class and the header cell (the header's children are in
    /// column order: Image, ID, Size, Arch, Created, Used by, buttons).
    fn selectors(self) -> &'static [&'static str] {
        match self {
            Self::Created => &[".im-created", ".im-head > :nth-child(5)"],
            Self::Id => &[".im-id", ".im-head > :nth-child(2)"],
            Self::Arch => &[".im-arch", ".im-head > :nth-child(4)"],
        }
    }
}

/// The lists the Images table steps through as it narrows, widest first.
pub fn images_steps() -> Vec<(String, Vec<ImageCol>)> {
    let body = css::block(IMAGES_HAND, "\n.im-table {");
    let list = |name: &str| css::declaration(body, name);
    vec![
        (list("--im-cols"), vec![]),
        (list("--im-cols-no-created"), vec![ImageCol::Created]),
        (
            list("--im-cols-no-id"),
            vec![ImageCol::Created, ImageCol::Id],
        ),
        (
            list("--im-cols-compact"),
            vec![ImageCol::Created, ImageCol::Id, ImageCol::Arch],
        ),
    ]
}

/// The room an Images list asks for: its tracks' minimums, the 12 px gaps
/// and the page gutter on both sides.
fn images_need(cols: &str, scale: f32, view: View) -> f32 {
    let tracks = css::tracks(cols);
    css::min_sum(&tracks, scale) + 12.0 * (tracks.len() as f32 - 1.0) + 2.0 * view.gutter()
}

/// The list in force for an Images table `width` px wide: its tracks, the
/// hidden columns and the step (0 is the full list).
#[derive(Debug, Clone, PartialEq)]
pub struct ImagesState {
    pub tracks: Vec<String>,
    pub hidden: Vec<ImageCol>,
    pub step: usize,
}

pub fn images_state_at(view: View, width: f32, scale: f32) -> ImagesState {
    let steps = images_steps();
    let mut index = 0;
    while index + 1 < steps.len()
        && width <= threshold(images_need(&steps[index].0, scale, view)) as f32
    {
        index += 1;
    }
    ImagesState {
        tracks: css::tracks(&steps[index].0),
        hidden: steps[index].1.clone(),
        step: index,
    }
}

/// The room the narrowest Images list needs: what the inspector's main-area
/// floor has to cover.
pub fn images_least_need(view: View, scale: f32) -> f32 {
    let steps = images_steps();
    images_need(&steps.last().expect("steps").0, scale, view)
}

fn images_css() -> String {
    let steps = images_steps();
    let mut out = String::from(
        "/* Generated by screens/table_tiers.rs from the track lists in images.css.\n   \
         Never edit by hand: BERTH_REGEN_TIERS=1 cargo nextest run -p berth-app table_tiers.\n\n   \
         Each rule leaves a list at the exact width its tracks stop fitting (the sum of\n   \
         their minimums, the 12 px gaps and both page gutters) for one text size. */\n",
    );
    for view in [View::Narrow, View::Wide] {
        let _ = writeln!(out, "\n@media {} {{", view.media());
        for scale in STEPS {
            let _ = writeln!(out, "  /* text size {scale} */");
            for window in steps.windows(2) {
                let at = threshold(images_need(&window[0].0, scale, view));
                let (cols, hide) = &window[1];
                let _ = writeln!(out, "  @container images (max-width: {at}px) {{");
                let _ = writeln!(
                    out,
                    "    {} {{\n      --im-cols: {cols};\n    }}",
                    scoped(scale, ".im-table")
                );
                let hidden: Vec<String> = hide
                    .iter()
                    .flat_map(|col| col.selectors())
                    .map(|sel| format!(".im-table {sel}"))
                    .collect();
                if !hidden.is_empty() {
                    let _ = writeln!(
                        out,
                        "    {} {{\n      display: none;\n    }}",
                        scoped(scale, &hidden.join(", "))
                    );
                }
                out.push_str("  }\n");
            }
        }
        out.push_str("}\n");
    }
    out
}

// ---------------------------------------------------------------------
// The files
// ---------------------------------------------------------------------

pub(super) fn write_or_check(path: &str, generated: &str) {
    if std::env::var_os("BERTH_REGEN_TIERS").is_some() {
        std::fs::write(path, generated).expect("write the generated stylesheet");
        return;
    }
    let on_disk = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        on_disk == generated,
        "{path} is stale: regenerate with `BERTH_REGEN_TIERS=1 cargo nextest run -p berth-app table_tiers`"
    );
}

#[test]
fn table_tiers_containers_stylesheet_is_the_generator_output() {
    write_or_check(CONTAINERS_FILE, &containers_css());
}

#[test]
fn table_tiers_projects_stylesheet_is_the_generator_output() {
    write_or_check(PROJECTS_FILE, &projects_css());
}

#[test]
fn table_tiers_images_stylesheet_is_the_generator_output() {
    write_or_check(IMAGES_FILE, &images_css());
}

#[test]
fn table_tiers_images_thresholds_fall_and_name_every_text_size() {
    for view in [View::Narrow, View::Wide] {
        for scale in STEPS {
            let needs: Vec<f32> = images_steps()
                .iter()
                .map(|(cols, _)| images_need(cols, scale, view))
                .collect();
            assert!(
                needs.windows(2).all(|w| w[0] > w[1]),
                "images {view:?} {scale}: {needs:?}"
            );
        }
    }
    let css = images_css();
    assert!(css.contains(":root:not([data-text-scale])"));
    for scale in STEPS {
        if scale != 1.0 {
            assert!(
                css.contains(&format!("[data-text-scale=\"{scale}\"]")),
                "{scale}"
            );
        }
    }
}

#[test]
fn table_tiers_thresholds_fall_as_the_lists_narrow() {
    // Each rule must override the one before it, so a chain's thresholds
    // only ever go down.
    for view in [View::Narrow, View::Wide] {
        for scale in STEPS {
            for chain in containers_chains(view) {
                let needs: Vec<f32> = chain
                    .steps
                    .iter()
                    .map(|s| containers_need(s, scale, view))
                    .collect();
                assert!(
                    needs.windows(2).all(|w| w[0] >= w[1]),
                    "{view:?} {scale} {}: {needs:?}",
                    chain.selector
                );
            }
            let needs: Vec<f32> = (0..4).map(|t| projects_need(t, scale, view)).collect();
            assert!(
                needs.windows(2).all(|w| w[0] >= w[1]),
                "projects {view:?} {scale}: {needs:?}"
            );
        }
    }
}

#[test]
fn table_tiers_move_with_the_text_size_in_exact_px_not_em() {
    // The bug the generator fixes: at 85 % the gutters (40 px) are a bigger
    // share of the width than em thresholds assumed, so the plain table's
    // Image has to go sooner than 57 em (630 px at 85 %) says.
    let plain_full_85 =
        containers_need(&containers_chains(View::Wide)[0].steps[0], 0.85, View::Wide);
    assert!(
        plain_full_85 > 57.0 * 13.0 * 0.85,
        "{plain_full_85} px is where Image must go at 85 %"
    );
    // And the stylesheet carries that number for that size only.
    let css = containers_css();
    let at = threshold(plain_full_85);
    assert!(
        css.contains(&format!("(max-width: {at}px)")),
        "{at} px missing"
    );
}

#[test]
fn table_tiers_attribute_values_are_the_ones_the_page_sets() {
    // The stylesheet keys on `data-text-scale="<step>"` and the script sets
    // the same string for each step (the default also works before it runs).
    let containers = containers_css();
    let projects = projects_css();
    for scale in STEPS {
        let script = crate::text_scale::script(scale);
        assert!(script.contains(&format!("dataset.textScale = '{scale}'")));
        if scale == 1.0 {
            assert!(containers.contains(":root:not([data-text-scale])"));
        }
        let key = format!("[data-text-scale=\"{scale}\"]");
        assert!(
            containers.contains(&key) && projects.contains(&key),
            "{scale}"
        );
    }
}

#[test]
fn table_tiers_every_width_gets_a_list_that_fits_it() {
    // For every width from the narrowest a tier can serve up to a wide
    // window, at every text size, the list the chain picks fits.
    for view in [View::Narrow, View::Wide] {
        for scale in STEPS {
            let floor = containers_least_need(view, scale).ceil() as u32;
            for flags in [(false, false), (true, false), (false, true), (true, true)] {
                for width in (floor..1500).step_by(3) {
                    let width = width as f32;
                    let state = containers_state_at(view, flags.0, flags.1, width, scale);
                    let used = css::min_sum(&state.tracks, scale)
                        + 12.0 * (state.tracks.len() as f32 - 1.0)
                        + 2.0 * view.gutter();
                    assert!(
                        used <= width + 0.5,
                        "{view:?} {scale} {flags:?} {width} px: step {} needs {used}",
                        state.step
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// Networks (P4-T5)
// ---------------------------------------------------------------------

const NETWORKS_HAND: &str = include_str!("../../assets/css/networks.css");
const NETWORKS_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/css/networks_tiers.css");

/// A column the Networks table drops as it narrows: Project first, then Driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetCol {
    Project,
    Driver,
}

impl NetCol {
    fn selectors(self) -> &'static [&'static str] {
        match self {
            Self::Project => &[".nw-project", ".nw-project-head"],
            Self::Driver => &[".nw-driver", ".nw-driver-head"],
        }
    }
}

/// One track list of the Networks table and the columns it has hidden.
#[derive(Debug, Clone)]
pub struct NetStep {
    pub cols: String,
    pub hide: Vec<NetCol>,
}

/// The lists the Networks table steps through as it narrows, widest first,
/// read from the hand-written block in `networks.css`.
pub fn networks_steps() -> Vec<NetStep> {
    let block = css::block(NETWORKS_HAND, "\n.nw-table {");
    let list = |name: &str, hide: &[NetCol]| NetStep {
        cols: css::declaration(block, name),
        hide: hide.to_vec(),
    };
    vec![
        list("--nw-cols", &[]),
        list("--nw-cols-noproject", &[NetCol::Project]),
        list("--nw-cols-compact", &[NetCol::Project, NetCol::Driver]),
    ]
}

/// The room a Networks list asks for: its tracks' minimums, the 12 px gaps
/// and the page gutter on both sides (the rows' own padding and margin add
/// up to the gutter).
fn networks_need(step: &NetStep, scale: f32, view: View) -> f32 {
    let tracks = css::tracks(&step.cols);
    css::min_sum(&tracks, scale) + 12.0 * (tracks.len() as f32 - 1.0) + 2.0 * view.gutter()
}

/// The lists in force for a Networks table `width` px wide.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworksState {
    pub tracks: Vec<String>,
    pub hidden: Vec<NetCol>,
    /// Which step (0 is the full list).
    pub step: usize,
}

pub fn networks_state_at(view: View, width: f32, scale: f32) -> NetworksState {
    let steps = networks_steps();
    let mut index = 0;
    while index + 1 < steps.len()
        && width <= threshold(networks_need(&steps[index], scale, view)) as f32
    {
        index += 1;
    }
    NetworksState {
        tracks: css::tracks(&steps[index].cols),
        hidden: steps[index].hide.clone(),
        step: index,
    }
}

/// The room the narrowest Networks list needs: what the inspector's
/// main-area floor has to cover.
pub fn networks_least_need(view: View, scale: f32) -> f32 {
    let steps = networks_steps();
    networks_need(steps.last().expect("a chain has steps"), scale, view)
}

fn networks_css() -> String {
    let mut out = String::from(
        "/* Generated by screens/table_tiers.rs from the track lists in networks.css.\n   \
         Never edit by hand: BERTH_REGEN_TIERS=1 cargo nextest run -p berth-app table_tiers.\n\n   \
         Each rule leaves a list at the exact width its tracks stop fitting (the sum of\n   \
         their minimums, the 12 px gaps and both page gutters) for one text size. */\n",
    );
    let steps = networks_steps();
    for view in [View::Narrow, View::Wide] {
        let _ = writeln!(out, "\n@media {} {{", view.media());
        for scale in STEPS {
            let _ = writeln!(out, "  /* text size {scale} */");
            for (k, window) in steps.windows(2).enumerate() {
                let at = threshold(networks_need(&steps[k], scale, view));
                let next = &window[1];
                let _ = writeln!(out, "  @container networks (max-width: {at}px) {{");
                let _ = writeln!(
                    out,
                    "    {} {{\n      --nw-cols: {};\n    }}",
                    scoped(scale, ".nw-table"),
                    next.cols
                );
                let hidden: Vec<String> = next
                    .hide
                    .iter()
                    .flat_map(|col| col.selectors())
                    .map(|sel| format!(".nw-table {sel}"))
                    .collect();
                if !hidden.is_empty() {
                    let _ = writeln!(
                        out,
                        "    {} {{\n      display: none;\n    }}",
                        scoped(scale, &hidden.join(", "))
                    );
                }
                out.push_str("  }\n");
            }
        }
        out.push_str("}\n");
    }
    out
}

#[test]
fn table_tiers_networks_stylesheet_is_the_generator_output() {
    write_or_check(NETWORKS_FILE, &networks_css());
}

#[test]
fn table_tiers_networks_thresholds_fall_as_the_lists_narrow() {
    for view in [View::Narrow, View::Wide] {
        for scale in STEPS {
            let steps = networks_steps();
            let needs: Vec<f32> = steps
                .iter()
                .map(|s| networks_need(s, scale, view))
                .collect();
            assert!(
                needs.windows(2).all(|w| w[0] >= w[1]),
                "networks {view:?} {scale}: {needs:?}"
            );
        }
    }
}

#[test]
fn table_tiers_networks_every_width_gets_a_list_that_fits_it() {
    for view in [View::Narrow, View::Wide] {
        for scale in STEPS {
            let floor = networks_least_need(view, scale).ceil() as u32;
            for width in (floor..1500).step_by(3) {
                let state = networks_state_at(view, width as f32, scale);
                let used = css::min_sum(&state.tracks, scale)
                    + 12.0 * (state.tracks.len() as f32 - 1.0)
                    + 2.0 * view.gutter();
                assert!(
                    used <= width as f32 + 0.5,
                    "{view:?} {scale} {width} px: step {} needs {used}",
                    state.step
                );
            }
        }
    }
}

#[test]
fn table_tiers_networks_table_fits_every_window_inspector_width_and_text_size() {
    // The D24 sweep for the Networks table: windows 1024 / 1220 / 1440,
    // inspector widths 320 / 360 / 480 / 560, every text size. A docked
    // inspector takes the table's room; an overlay leaves the whole area.
    let mut docked_cases = 0;
    for window in [1024.0_f64, 1220.0, 1440.0] {
        for stored in [320, 360, 480, 560] {
            for scale in STEPS {
                let width = match crate::ui::dock::docked_width(window, stored, scale) {
                    Some(inspector) => {
                        docked_cases += 1;
                        window as f32 - 220.0 - inspector as f32
                    }
                    None => window as f32 - 220.0,
                };
                let view = if window < 1200.0 {
                    View::Narrow
                } else {
                    View::Wide
                };
                let state = networks_state_at(view, width, scale);
                let used = css::min_sum(&state.tracks, scale)
                    + 12.0 * (state.tracks.len() as f32 - 1.0)
                    + 2.0 * view.gutter();
                assert!(
                    used <= width + 0.5,
                    "{window} px, inspector {stored}, {scale}: table {width} px needs {used}"
                );
            }
        }
    }
    assert!(docked_cases > 30, "{docked_cases} docked cases");
}

#[test]
fn table_tiers_networks_keep_every_column_at_100_percent_and_drop_project_then_driver() {
    // The 1440 window with a docked 360 px inspector leaves 860 px and the
    // 1024 window 804 px: at 100 % every column shows in both.
    let wide = networks_state_at(View::Wide, 860.0, 1.0);
    assert_eq!(wide.step, 0, "860 px at 100 % keeps all six columns");
    assert!(wide.hidden.is_empty());
    let narrow = networks_state_at(View::Narrow, 804.0, 1.0);
    assert_eq!(narrow.step, 0, "804 px at 100 % keeps all six columns");
    // At 150 % the 1024 window (804 px) cannot hold Project or Driver.
    let big = networks_state_at(View::Narrow, 804.0, 1.5);
    assert_eq!(big.hidden, vec![NetCol::Project, NetCol::Driver]);
    // The cells and the header cells go with the track.
    let css = networks_css();
    assert!(css.contains(".nw-table .nw-project,"), "{css}");
    assert!(css.contains(".nw-table .nw-driver-head"), "{css}");
}
