//! ProgressBar (components-spec): determinate work such as a pull, a build
//! or a clean-up, with a label, the value and the bytes. Without a value it
//! is indeterminate. The screen that draws one renders [`PROGRESS_BAR_CSS`].

use dioxus::prelude::*;

/// The component's stylesheet (tokens only).
pub const PROGRESS_BAR_CSS: &str = include_str!("../../assets/css/progress_bar.css");

/// `fraction` is 0 to 1; `None` is indeterminate. `thin` is the per-layer
/// strip of a pull. `value_text` overrides the percent on the right.
#[component]
pub fn ProgressBar(
    fraction: Option<f32>,
    #[props(default)] label: Option<String>,
    #[props(default)] value_text: Option<String>,
    #[props(default)] detail: Option<String>,
    #[props(default)] thin: bool,
    #[props(default)] aria_label: Option<String>,
) -> Element {
    let percent = fraction.map(|f| (f.clamp(0.0, 1.0) * 100.0).round());
    let mut class = String::from("progress");
    if thin {
        class.push_str(" progress-thin");
    }
    if percent.is_none() {
        class.push_str(" progress-indeterminate");
    }
    // A thin bar is the bare strip under a row that says its own state.
    let shown = if thin {
        None
    } else {
        value_text.or_else(|| percent.map(|p| format!("{p:.0}%")))
    };
    let name = aria_label.or_else(|| label.clone());
    // What a screen reader says for the value: the visible text when there
    // is one ("65%", "3 of 8 layers"), else the percent.
    let spoken = shown
        .clone()
        .or_else(|| percent.map(|p| format!("{p:.0}%")));
    let label = if thin { None } else { label };
    rsx! {
        div { class,
            if label.is_some() || shown.is_some() {
                div { class: "progress-head",
                    if let Some(text) = &label {
                        span { class: "progress-label", "{text}" }
                    }
                    if let Some(text) = &shown {
                        span { class: "progress-value", "{text}" }
                    }
                }
            }
            div {
                class: "progress-track",
                role: "progressbar",
                "aria-label": name,
                "aria-valuemin": "0",
                "aria-valuemax": "100",
                "aria-valuenow": percent.map(|p| format!("{p:.0}")),
                "aria-valuetext": spoken,
                div {
                    class: "progress-fill",
                    style: percent.map(|p| format!("width: {p:.0}%;")),
                }
            }
            if let Some(text) = detail {
                div { class: "progress-detail", "{text}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(fraction: Option<f32>) -> String {
        dioxus_ssr::render_element(rsx! {
            ProgressBar {
                fraction,
                label: "Pulling alpine:3".to_string(),
                detail: "68 MB of 105 MB".to_string(),
            }
        })
    }

    #[test]
    fn progress_bar_shows_percent_width_and_bytes() {
        let html = render(Some(0.65));
        assert!(html.contains("Pulling alpine:3"), "{html}");
        assert!(html.contains(">65%<"), "{html}");
        assert!(html.contains("width: 65%;"), "{html}");
        assert!(html.contains("aria-valuenow=\"65\""), "{html}");
        assert!(html.contains("68 MB of 105 MB"), "{html}");
        assert!(!html.contains("progress-indeterminate"), "{html}");
    }

    #[test]
    fn progress_bar_without_a_value_is_indeterminate() {
        let html = render(None);
        assert!(html.contains("progress-indeterminate"), "{html}");
        assert!(!html.contains("aria-valuenow"), "{html}");
        assert!(!html.contains("width:"), "{html}");
    }

    #[test]
    fn progress_bar_thin_is_the_bare_strip_without_a_label_or_percent() {
        let html = dioxus_ssr::render_element(rsx! {
            ProgressBar { fraction: Some(0.4), thin: true, label: "Layer".to_string() }
        });
        assert!(html.contains("progress progress-thin"), "{html}");
        assert!(html.contains("width: 40%;"), "{html}");
        assert!(
            !html.contains("progress-head") && !html.contains("40%<"),
            "{html}"
        );
    }

    #[test]
    fn progress_bar_clamps_to_the_track() {
        assert!(render(Some(1.7)).contains("width: 100%;"));
        assert!(render(Some(-0.2)).contains("width: 0%;"));
    }
}
