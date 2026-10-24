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

// PLAN 6.4 types. The SQLite Store itself lands in P1-T2; Settings exists now
// because berth-app's AppState (P0-T4) already holds a Settings signal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub theme: Theme,
    pub density: Density,
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
    fn compiles() {}

    #[test]
    fn settings_defaults_match_plan() {
        let s = Settings::default();
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.density, Density::Comfortable);
        assert!(s.show_dock_icon);
        assert!(s.update_check);
        assert_eq!(s.log_buffer_lines, 100_000);
        assert_eq!(s.owner, berth_core::OwnerId::Local);
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
}
