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
//! One file per table (`containers.rs`, `projects.rs`…) holds its track
//! lists, its generator and its own tests, and exports a [`Table`]; the
//! `tier_tables!` list below is the registry the shared sweeps walk. A new
//! table adds its file and one name to that list.
//!
//! The same chains answer "which list is in force at this width" for the
//! table tests, so a test and the stylesheet cannot disagree.

use crate::text_scale::STEPS;

/// What the shared sweeps need of one table.
pub(super) struct Table {
    pub name: &'static str,
    /// The checked-in stylesheet the generator writes.
    pub file: &'static str,
    pub css: fn() -> String,
    /// The room each list asks for at a view and text size, widest first;
    /// one vector per chain of lists the table steps through.
    pub needs: fn(View, f32) -> Vec<Vec<f32>>,
}

/// Declares the table modules and the registry of their [`Table`]s.
/// One line per table.
macro_rules! tier_tables {
    ($($table:ident),* $(,)?) => {
        $(pub mod $table;)*

        const TABLES: &[&Table] = &[$(&$table::TABLE),*];
    };
}

tier_tables! {
    containers,
    find_images,
    images,
    networks,
    projects,
    volumes,
}

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
fn table_tiers_every_registered_table_is_the_generator_output() {
    for table in TABLES {
        write_or_check(table.file, &(table.css)());
    }
}

#[test]
fn table_tiers_every_stylesheet_on_disk_belongs_to_a_registered_table() {
    // A generated `*_tiers.css` nobody registered would go stale unseen.
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/css");
    let mut found = 0;
    for entry in std::fs::read_dir(dir).expect("the css directory") {
        let path = entry.expect("a directory entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.ends_with("_tiers.css") {
            found += 1;
            assert!(
                TABLES.iter().any(|t| t.file.ends_with(name)),
                "{name} is not in the tier_tables! list"
            );
        }
    }
    assert_eq!(found, TABLES.len(), "one stylesheet per registered table");
}

#[test]
fn table_tiers_every_registered_table_asks_for_less_room_as_it_narrows() {
    for table in TABLES {
        for view in [View::Narrow, View::Wide] {
            for scale in STEPS {
                for needs in (table.needs)(view, scale) {
                    assert!(
                        needs.windows(2).all(|w| w[0] >= w[1]),
                        "{} {view:?} {scale}: {needs:?}",
                        table.name
                    );
                }
            }
        }
    }
}

#[test]
fn table_tiers_attribute_values_are_the_ones_the_page_sets() {
    // The stylesheets key on `data-text-scale="<step>"` and the script sets
    // the same string for each step (the default also works before it runs).
    for scale in STEPS {
        let script = crate::text_scale::script(scale);
        assert!(script.contains(&format!("dataset.textScale = '{scale}'")));
        let key = format!("[data-text-scale=\"{scale}\"]");
        for table in TABLES {
            let css = (table.css)();
            assert!(css.contains(&key), "{} {scale}", table.name);
            if scale == 1.0 {
                assert!(
                    css.contains(":root:not([data-text-scale])"),
                    "{}",
                    table.name
                );
            }
        }
    }
}
