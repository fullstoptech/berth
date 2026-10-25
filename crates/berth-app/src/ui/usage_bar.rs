//! UsageBar (components-spec): how a disk is used, one segment per kind in a
//! single track with a legend under it ("Images 11.8 GB"). Frame "Engines /
//! dashboard / 1440". Pure markup over [`UsageSegment`]s; [`segments_of`]
//! builds them from `disk_usage`. States: loading, unavailable (with the
//! reason), nearly full.

use berth_core::DiskUsage;
use dioxus::prelude::*;

/// The bar's CSS, rendered with it (tokens only).
pub const USAGE_BAR_CSS: &str = include_str!("../../assets/css/usage_bar.css");

/// At or above this share of the disk the bar says it is nearly full.
pub const NEARLY_FULL: f64 = 0.90;

/// One kind of use and how much of the disk it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSegment {
    pub label: &'static str,
    pub bytes: u64,
}

/// Images, Volumes, Build cache, Containers: the order the frame draws.
#[must_use]
pub fn segments_of(usage: &DiskUsage) -> Vec<UsageSegment> {
    vec![
        UsageSegment {
            label: "Images",
            bytes: usage.images.size.as_u64(),
        },
        UsageSegment {
            label: "Volumes",
            bytes: usage.volumes.size.as_u64(),
        },
        UsageSegment {
            label: "Build cache",
            bytes: usage.build_cache.size.as_u64(),
        },
        UsageSegment {
            label: "Containers",
            bytes: usage.containers.size.as_u64(),
        },
    ]
}

/// What the segments add up to.
#[must_use]
pub fn used_bytes(segments: &[UsageSegment]) -> u64 {
    segments.iter().map(|s| s.bytes).sum()
}

/// Each segment's width as a share of the track, in percent. The track is
/// the disk: when the segments overshoot it (a stale total) they are
/// scaled down together, so the bar never overflows.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn widths(segments: &[UsageSegment], total: u64) -> Vec<f64> {
    let used = used_bytes(segments);
    let basis = total.max(used);
    if basis == 0 {
        return vec![0.0; segments.len()];
    }
    segments
        .iter()
        .map(|s| s.bytes as f64 * 100.0 / basis as f64)
        .collect()
}

/// Is the disk nearly full?
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn nearly_full(used: u64, total: u64) -> bool {
    total > 0 && used as f64 / total as f64 >= NEARLY_FULL
}

/// "22.5 of 60 GB": both figures with one unit when they share it.
#[must_use]
pub fn used_of(used: bytesize::ByteSize, total: bytesize::ByteSize) -> String {
    used_of_text(
        &berth_core::fmt::bytes(used),
        &berth_core::fmt::bytes(total),
    )
}

/// [`used_of`] over figures already written out ("22.5 GB", "60 GB").
#[must_use]
pub fn used_of_text(used: &str, total: &str) -> String {
    match (used.split_once(' '), total.split_once(' ')) {
        (Some((number, unit)), Some((_, total_unit))) if unit == total_unit => {
            format!("{number} of {total}")
        }
        _ => format!("{used} of {total}"),
    }
}

#[component]
pub fn UsageBar(
    segments: Vec<UsageSegment>,
    /// The disk's size; the track is this wide.
    total: u64,
    #[props(default)] loading: bool,
    /// Why there is nothing to draw (engine stopped, usage not readable).
    #[props(default)]
    unavailable: Option<String>,
) -> Element {
    if let Some(reason) = unavailable {
        return rsx! {
            style { "{USAGE_BAR_CSS}" }
            div { class: "usage-bar usage-unavailable",
                div { class: "usage-track" }
                p { class: "usage-reason", "{reason}" }
            }
        };
    }
    if loading {
        return rsx! {
            style { "{USAGE_BAR_CSS}" }
            div { class: "usage-bar usage-loading", "aria-busy": "true",
                div { class: "usage-track" }
                p { class: "usage-reason", "Reading disk usage\u{2026}" }
            }
        };
    }
    let shares = widths(&segments, total);
    let full = nearly_full(used_bytes(&segments), total);
    let summary = segments
        .iter()
        .map(|s| {
            format!(
                "{} {}",
                s.label,
                berth_core::fmt::bytes(bytesize::ByteSize(s.bytes))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let class = if full {
        "usage-bar usage-full"
    } else {
        "usage-bar"
    };
    rsx! {
        style { "{USAGE_BAR_CSS}" }
        div { class, role: "img", "aria-label": "{summary}",
            div { class: "usage-track",
                for (index, share) in shares.iter().enumerate() {
                    span {
                        key: "{index}",
                        class: "usage-seg usage-s{index + 1}",
                        style: "width: {share:.3}%;",
                    }
                }
            }
            ul { class: "usage-legend",
                for (index, segment) in segments.iter().enumerate() {
                    li { key: "{index}", class: "usage-legend-item",
                        span { class: "usage-swatch usage-s{index + 1}", "aria-hidden": "true" }
                        span { class: "usage-legend-label", "{segment.label}" }
                        span { class: "usage-legend-value",
                            {berth_core::fmt::bytes(bytesize::ByteSize(segment.bytes))}
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytesize::ByteSize;

    fn usage() -> DiskUsage {
        let group = |mb: u64| berth_core::UsageGroup {
            count: 1,
            size: ByteSize::mb(mb),
            reclaimable: ByteSize(0),
        };
        DiskUsage {
            images: group(11_800),
            volumes: group(6_200),
            build_cache: group(3_100),
            containers: group(1_400),
            disk_total: Some(ByteSize::gb(60)),
        }
    }

    #[test]
    fn engines_usage_bar_segments_follow_the_frame_order() {
        let labels: Vec<_> = segments_of(&usage()).iter().map(|s| s.label).collect();
        assert_eq!(labels, ["Images", "Volumes", "Build cache", "Containers"]);
    }

    #[test]
    fn engines_usage_bar_widths_are_shares_of_the_disk() {
        let segments = segments_of(&usage());
        let shares = widths(&segments, 60_000_000_000);
        assert!((shares[0] - 19.666).abs() < 0.01, "{shares:?}");
        assert!(
            (shares.iter().sum::<f64>() - 37.5).abs() < 0.01,
            "{shares:?}"
        );
    }

    #[test]
    fn engines_usage_bar_overshooting_segments_scale_to_the_track() {
        let segments = segments_of(&usage());
        let shares = widths(&segments, 10_000_000_000);
        assert!(
            (shares.iter().sum::<f64>() - 100.0).abs() < 0.001,
            "{shares:?}"
        );
        assert_eq!(widths(&segments, 0).len(), 4);
        assert_eq!(widths(&[], 0), Vec::<f64>::new());
    }

    #[test]
    fn engines_usage_bar_nearly_full_starts_at_ninety_percent() {
        assert!(!nearly_full(53_999_999_999, 60_000_000_000));
        assert!(nearly_full(54_000_000_000, 60_000_000_000));
        assert!(!nearly_full(5, 0));
    }

    #[test]
    fn engines_usage_bar_used_of_shares_the_unit() {
        assert_eq!(
            used_of(ByteSize::mb(22_500), ByteSize::gb(60)),
            "22.5 of 60 GB"
        );
        assert_eq!(
            used_of(ByteSize::mb(800), ByteSize::gb(60)),
            "800 MB of 60 GB"
        );
    }

    #[test]
    fn engines_usage_bar_renders_legend_values_and_a_summary_label() {
        let html = dioxus_ssr::render_element(rsx! {
            UsageBar { segments: segments_of(&usage()), total: 60_000_000_000_u64 }
        });
        assert!(html.contains("Images"), "{html}");
        assert!(html.contains("11.8 GB"), "{html}");
        assert!(
            html.contains("aria-label=\"Images 11.8 GB, Volumes 6.2 GB"),
            "{html}"
        );
        assert!(!html.contains("class=\"usage-bar usage-full\""), "{html}");
    }

    #[test]
    fn engines_usage_bar_states_loading_unavailable_and_nearly_full() {
        let loading = dioxus_ssr::render_element(rsx! {
            UsageBar { segments: vec![], total: 0_u64, loading: true }
        });
        assert!(
            loading.contains("class=\"usage-bar usage-loading\""),
            "{loading}"
        );
        let off = dioxus_ssr::render_element(rsx! {
            UsageBar { segments: vec![], total: 0_u64, unavailable: "Engine stopped".to_string() }
        });
        assert!(off.contains("Engine stopped"), "{off}");
        let full = dioxus_ssr::render_element(rsx! {
            UsageBar {
                segments: vec![UsageSegment { label: "Images", bytes: 58_000_000_000 }],
                total: 60_000_000_000_u64,
            }
        });
        assert!(full.contains("class=\"usage-bar usage-full\""), "{full}");
    }
}
