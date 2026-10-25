//! Text size (owner request 2026-10-25): the steps, the pure stepping
//! function and the row-height maths shared with the virtual tables.
//!
//! The scale rides on `<html>` as the CSS custom property `--text-scale`;
//! `tokens.css` multiplies every font size, line height, row, control and
//! icon size by it. The title bar and spacing do not scale.

use berth_store::Density;

/// The steps, smallest first. `1.0` is the default.
pub const STEPS: [f32; 7] = [0.85, 0.9, 1.0, 1.1, 1.2, 1.3, 1.5];

pub const DEFAULT: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Bigger,
    Smaller,
    Reset,
}

/// The step nearest to `value`; `1.0` for NaN and infinities.
pub fn snap(value: f32) -> f32 {
    if !value.is_finite() {
        return DEFAULT;
    }
    STEPS
        .iter()
        .copied()
        .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
        .unwrap_or(DEFAULT)
}

/// The scale after one keypress. An unknown `current` snaps to its nearest
/// step first; the ends of the list stay put.
pub fn next_scale(current: f32, step: Step) -> f32 {
    let snapped = snap(current);
    let index = STEPS.iter().position(|s| *s == snapped).unwrap_or(2);
    match step {
        Step::Reset => DEFAULT,
        Step::Bigger => STEPS[(index + 1).min(STEPS.len() - 1)],
        Step::Smaller => STEPS[index.saturating_sub(1)],
    }
}

/// `base` px at `scale`, rounded to whole pixels (CSS `round(…, 1px)`).
pub fn scaled_px(base: u32, scale: f32) -> u32 {
    let px = (f64::from(base) * f64::from(snap(scale))).round();
    // `px` is a small non-negative whole number.
    px as u32
}

/// The table row height in px for a density and scale: must equal the
/// `--size-row` the stylesheet computes (the virtual table's maths and the
/// row elements' inline height both come from it).
pub fn row_height_px(density: Density, scale: f32) -> u32 {
    let base = match density {
        Density::Comfortable => 32,
        Density::Compact => 24,
    };
    scaled_px(base, scale)
}

/// The JS that sets (or clears, at the default) `--text-scale` on `<html>`
/// and always sets `data-text-scale` to the step, which the generated table
/// tiers (`screens/table_tiers.rs`) key their exact px thresholds on.
pub fn script(scale: f32) -> String {
    let scale = snap(scale);
    let property = if scale == DEFAULT {
        "document.documentElement.style.removeProperty('--text-scale');".to_string()
    } else {
        format!("document.documentElement.style.setProperty('--text-scale', '{scale}');")
    };
    format!("{property} document.documentElement.dataset.textScale = '{scale}';")
}

/// `BERTH_TEXT_SCALE` for development shots, like `BERTH_THEME`.
pub fn forced_from_env() -> Option<f32> {
    let value: f32 = std::env::var("BERTH_TEXT_SCALE")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    value.is_finite().then(|| snap(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scale_bigger_walks_up_the_steps_and_stops_at_the_top() {
        let mut scale = 1.0;
        let mut seen = vec![scale];
        for _ in 0..6 {
            scale = next_scale(scale, Step::Bigger);
            seen.push(scale);
        }
        assert_eq!(seen, [1.0, 1.1, 1.2, 1.3, 1.5, 1.5, 1.5]);
    }

    #[test]
    fn text_scale_smaller_walks_down_the_steps_and_stops_at_the_bottom() {
        let mut scale = 1.0;
        let mut seen = vec![scale];
        for _ in 0..4 {
            scale = next_scale(scale, Step::Smaller);
            seen.push(scale);
        }
        assert_eq!(seen, [1.0, 0.9, 0.85, 0.85, 0.85]);
    }

    #[test]
    fn text_scale_reset_returns_to_one_from_anywhere() {
        for step in STEPS {
            assert_eq!(next_scale(step, Step::Reset), 1.0);
        }
    }

    #[test]
    fn text_scale_unknown_values_snap_to_the_nearest_step_then_move() {
        // 1.12 is nearest 1.1; one step bigger is 1.2, smaller is 1.0.
        assert_eq!(next_scale(1.12, Step::Bigger), 1.2);
        assert_eq!(next_scale(1.12, Step::Smaller), 1.0);
        // Out-of-range stored values clamp to the ends.
        assert_eq!(snap(9.0), 1.5);
        assert_eq!(snap(0.1), 0.85);
        assert_eq!(snap(-3.0), 0.85);
        // Garbage means the default.
        assert_eq!(snap(f32::NAN), 1.0);
        assert_eq!(snap(f32::INFINITY), 1.0);
    }

    #[test]
    fn text_scale_every_step_is_a_fixed_point_of_snap() {
        for step in STEPS {
            assert_eq!(snap(step), step);
        }
    }

    #[test]
    fn text_scale_row_height_follows_density_and_scale_in_whole_pixels() {
        assert_eq!(row_height_px(Density::Comfortable, 1.0), 32);
        assert_eq!(row_height_px(Density::Compact, 1.0), 24);
        assert_eq!(row_height_px(Density::Comfortable, 1.3), 42);
        assert_eq!(row_height_px(Density::Comfortable, 0.85), 27);
        assert_eq!(row_height_px(Density::Compact, 1.5), 36);
        // An unknown scale snaps first, as the stylesheet's value does.
        assert_eq!(row_height_px(Density::Comfortable, 1.31), 42);
    }

    #[test]
    fn text_scale_script_sets_a_property_on_the_document_element() {
        let big = script(1.3);
        assert!(
            big.contains("documentElement.style.setProperty('--text-scale', '1.3')"),
            "{big}"
        );
        assert!(script(1.0).contains("removeProperty('--text-scale')"));
        // Unknown values never reach the page.
        assert!(script(7.0).contains("'1.5'"));
        // The attribute the table tiers key on carries the step, default included.
        assert!(big.contains("dataset.textScale = '1.3'"), "{big}");
        assert!(script(1.0).contains("dataset.textScale = '1'"));
        assert!(script(0.85).contains("dataset.textScale = '0.85'"));
    }

    const TOKENS_CSS: &str = include_str!("../assets/css/tokens.css");

    fn declaration<'a>(css: &'a str, name: &str) -> &'a str {
        let needle = format!("  {name}: ");
        let start = css
            .find(&needle)
            .unwrap_or_else(|| panic!("{name} missing"));
        let rest = &css[start + needle.len()..];
        &rest[..rest.find(';').expect("declaration ends")]
    }

    #[test]
    fn text_scale_tokens_font_sizes_lines_rows_and_controls_scale() {
        for role in [
            "caption", "label", "body", "body-lg", "h3", "h2", "h1", "display", "code",
        ] {
            for part in ["size", "line"] {
                let value = declaration(TOKENS_CSS, &format!("--text-{role}-{part}"));
                assert!(
                    value.contains("var(--text-scale"),
                    "--text-{role}-{part}: {value}"
                );
            }
        }
        for name in [
            "--size-row",
            "--size-row-lg",
            "--size-control-sm",
            "--size-control-md",
            "--size-control-lg",
            "--size-icon-sm",
            "--size-icon-md",
        ] {
            let value = declaration(TOKENS_CSS, name);
            assert!(value.contains("var(--text-scale"), "{name}: {value}");
        }
        // Compact density scales too.
        let compact = &TOKENS_CSS[TOKENS_CSS
            .find(":root[data-density=\"compact\"]")
            .expect("compact block")..];
        assert!(compact.contains("--size-row: round(calc(24px * var(--text-scale"));
        assert!(compact.contains("--size-control-md: calc(24px * var(--text-scale"));
    }

    #[test]
    fn text_scale_row_height_matches_the_stylesheet_row_at_every_step() {
        // `--size-row: round(calc(<base>px * var(--text-scale, 1)), 1px)`: the
        // base in the stylesheet is the base the virtual table's maths uses.
        for (selector, density) in [
            (":root {", Density::Comfortable),
            (":root[data-density=\"compact\"] {", Density::Compact),
        ] {
            let block = &TOKENS_CSS[TOKENS_CSS.find(selector).expect("block")..];
            let value = declaration(block, "--size-row");
            let base: f64 = value
                .trim_start_matches("round(calc(")
                .split("px")
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("a px base in `{value}`"));
            for step in STEPS {
                assert_eq!(
                    f64::from(row_height_px(density, step)),
                    (base * f64::from(step)).round(),
                    "{density:?} at {step}"
                );
            }
        }
    }

    #[test]
    fn text_scale_gutter_is_16_and_20_from_1200_px_wide() {
        assert_eq!(declaration(TOKENS_CSS, "--gutter"), "16px");
        let wide = &TOKENS_CSS[TOKENS_CSS
            .find("@media (min-width: 1200px) {")
            .expect("a 1200 px block")..];
        assert!(wide.starts_with("@media (min-width: 1200px) {\n  :root { --gutter: 20px; }"));
    }

    #[test]
    fn text_scale_tokens_title_bar_panels_and_spacing_stay_fixed() {
        for name in [
            "--size-titlebar",
            "--size-sidebar",
            "--size-inspector",
            "--space-8",
            "--space-16",
        ] {
            let value = declaration(TOKENS_CSS, name);
            assert!(!value.contains("var("), "{name} must not scale: {value}");
        }
        assert_eq!(declaration(TOKENS_CSS, "--size-titlebar"), "38px");
    }
}
