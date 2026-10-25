//! The Containers table's column-drop tiers.

use std::fmt::Write as _;

use super::{scoped, threshold, write_or_check, Table, View};
use crate::screens::css_tracks as css;
use crate::text_scale::STEPS;

const CONTAINERS_HAND: &str = include_str!("../../../assets/css/containers.css");

const CONTAINERS_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/css/containers_tiers.css"
);

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

pub(super) const TABLE: Table = Table {
    name: "containers",
    file: CONTAINERS_FILE,
    css: containers_css,
    needs: containers_needs,
};

fn containers_needs(view: View, scale: f32) -> Vec<Vec<f32>> {
    containers_chains(view)
        .iter()
        .map(|chain| {
            chain
                .steps
                .iter()
                .map(|step| containers_need(step, scale, view))
                .collect()
        })
        .collect()
}

#[test]
fn table_tiers_containers_stylesheet_is_the_generator_output() {
    write_or_check(CONTAINERS_FILE, &containers_css());
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
