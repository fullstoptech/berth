use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    System,
    Light,
    Dark,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Density {
    Comfortable,
    Compact,
}

/// Where the inspector lives (D21, D22): a right column, or the right pane
/// of the bottom region as in browser dev tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InspectorDock {
    #[default]
    Right,
    Bottom,
}

/// The inspector's width bounds and default in px (D21).
pub const INSPECTOR_WIDTH_MIN: u32 = 320;
pub const INSPECTOR_WIDTH_MAX: u32 = 560;
pub const INSPECTOR_WIDTH_DEFAULT: u32 = 360;
/// The bottom dock's inspector share of the region, in percent (D22).
pub const INSPECTOR_SPLIT_DEFAULT: u8 = 40;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub theme: Theme,
    pub density: Density,
    /// 0.85..=1.5, default 1.0 (owner request 2026-10-25). Settings saved
    /// before this field existed load with the default.
    #[serde(default = "default_text_scale")]
    pub text_scale: f32,
    /// 320..=560, default 360 (owner request, D21). Settings saved before
    /// this field existed load with the default.
    #[serde(default = "default_inspector_width")]
    pub inspector_width_px: u32,
    #[serde(default)]
    pub inspector_dock: InspectorDock,
    /// Bottom dock: the inspector's share of the region, default 40 (D22).
    #[serde(default = "default_inspector_split")]
    pub inspector_split_pct: u8,
    pub default_engine: Option<berth_core::EngineId>,
    pub last_harness: Option<berth_core::HarnessId>,
    pub harness_paths: std::collections::BTreeMap<String, std::path::PathBuf>,
    pub terminal_app: Option<String>,
    pub editor: Option<String>,
    pub favourites: Vec<std::path::PathBuf>,
    pub show_dock_icon: bool,
    pub update_check: bool,
    pub log_buffer_lines: u32,
    pub owner: berth_core::OwnerId,
}

fn default_text_scale() -> f32 {
    1.0
}

fn default_inspector_width() -> u32 {
    INSPECTOR_WIDTH_DEFAULT
}

fn default_inspector_split() -> u8 {
    INSPECTOR_SPLIT_DEFAULT
}

impl Default for Settings {
    fn default() -> Self {
        // PLAN 6.4: favourites defaults to [~/Dev] when it exists.
        let favourites = std::env::home_dir()
            .map(|home| home.join("Dev"))
            .filter(|dev| dev.is_dir())
            .into_iter()
            .collect();
        Self {
            theme: Theme::System,
            density: Density::Comfortable,
            text_scale: default_text_scale(),
            inspector_width_px: default_inspector_width(),
            inspector_dock: InspectorDock::Right,
            inspector_split_pct: default_inspector_split(),
            default_engine: None,
            last_harness: None,
            harness_paths: std::collections::BTreeMap::new(),
            terminal_app: None,
            editor: None,
            favourites,
            show_dock_icon: true,
            update_check: true,
            log_buffer_lines: 100_000,
            owner: berth_core::OwnerId::Local,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_match_plan() {
        let s = Settings::default();
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.density, Density::Comfortable);
        assert!((s.text_scale - 1.0).abs() < f32::EPSILON);
        assert!(s.show_dock_icon);
        assert!(s.update_check);
        assert_eq!(s.log_buffer_lines, 100_000);
        assert_eq!(s.owner, berth_core::OwnerId::Local);
        assert_eq!(s.default_engine, None);
        assert!(s.harness_paths.is_empty());
    }

    #[test]
    fn settings_serde_round_trip() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Settings>(&json).expect("deserialize"),
            s
        );
    }

    #[test]
    fn settings_text_scale_round_trips() {
        let s = Settings {
            text_scale: 1.3,
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let back = serde_json::from_str::<Settings>(&json).expect("deserialize");
        assert!((back.text_scale - 1.3).abs() < f32::EPSILON);
    }

    #[test]
    fn settings_saved_before_text_scale_load_with_one() {
        let mut value = serde_json::to_value(Settings::default()).expect("to_value");
        value
            .as_object_mut()
            .expect("object")
            .remove("text_scale")
            .expect("field was written");
        let old = serde_json::from_value::<Settings>(value).expect("old settings load");
        assert!((old.text_scale - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn settings_inspector_fields_round_trip() {
        let s = Settings {
            inspector_width_px: 520,
            inspector_dock: InspectorDock::Bottom,
            inspector_split_pct: 55,
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let back = serde_json::from_str::<Settings>(&json).expect("deserialize");
        assert_eq!(back.inspector_width_px, 520);
        assert_eq!(back.inspector_dock, InspectorDock::Bottom);
        assert_eq!(back.inspector_split_pct, 55);
    }

    #[test]
    fn settings_saved_before_the_inspector_fields_load_with_their_defaults() {
        let mut value = serde_json::to_value(Settings::default()).expect("to_value");
        let object = value.as_object_mut().expect("object");
        for field in [
            "inspector_width_px",
            "inspector_dock",
            "inspector_split_pct",
        ] {
            object.remove(field).expect("field was written");
        }
        let old = serde_json::from_value::<Settings>(value).expect("old settings load");
        assert_eq!(old.inspector_width_px, 360);
        assert_eq!(old.inspector_dock, InspectorDock::Right);
        assert_eq!(old.inspector_split_pct, 40);
    }
}
