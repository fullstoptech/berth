use serde::{Deserialize, Serialize};

use crate::container::{ContainerState, Health};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StatusKind {
    Running,
    Healthy,
    Starting,
    Restarting,
    Unhealthy,
    Paused,
    Exited(i32),
    Created,
    Removing,
    Dead,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tone {
    Success,
    Warning,
    Danger,
    Stopped,
    Info,
}

#[must_use]
pub fn status_of(
    state: ContainerState,
    health: Option<Health>,
    exit_code: Option<i32>,
) -> StatusKind {
    match state {
        ContainerState::Running => match health {
            Some(Health::Healthy) => StatusKind::Healthy,
            Some(Health::Unhealthy) => StatusKind::Unhealthy,
            Some(Health::Starting) => StatusKind::Starting,
            None => StatusKind::Running,
        },
        ContainerState::Created => StatusKind::Created,
        ContainerState::Paused => StatusKind::Paused,
        ContainerState::Restarting => StatusKind::Restarting,
        ContainerState::Removing => StatusKind::Removing,
        ContainerState::Exited => StatusKind::Exited(exit_code.unwrap_or(0)),
        ContainerState::Dead => StatusKind::Dead,
    }
}

impl StatusKind {
    #[must_use]
    pub fn tone(self) -> Tone {
        match self {
            Self::Running | Self::Healthy => Tone::Success,
            Self::Starting | Self::Restarting | Self::Unhealthy | Self::Paused => Tone::Warning,
            Self::Exited(0) | Self::Removing => Tone::Stopped,
            Self::Exited(_) | Self::Dead => Tone::Danger,
            Self::Created => Tone::Info,
        }
    }

    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Running => "Running".to_string(),
            Self::Healthy => "Healthy".to_string(),
            Self::Starting => "Starting".to_string(),
            Self::Restarting => "Restarting".to_string(),
            Self::Unhealthy => "Unhealthy".to_string(),
            Self::Paused => "Paused".to_string(),
            Self::Exited(code) => format!("Exited ({code})"),
            Self::Created => "Created".to_string(),
            Self::Removing => "Removing".to_string(),
            Self::Dead => "Dead".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_health_variants() {
        assert_eq!(
            status_of(ContainerState::Running, None, None),
            StatusKind::Running
        );
        assert_eq!(
            status_of(ContainerState::Running, Some(Health::Healthy), None),
            StatusKind::Healthy
        );
        assert_eq!(
            status_of(ContainerState::Running, Some(Health::Unhealthy), None),
            StatusKind::Unhealthy
        );
        assert_eq!(
            status_of(ContainerState::Running, Some(Health::Starting), None),
            StatusKind::Starting
        );
    }

    #[test]
    fn non_running_states_map_directly() {
        assert_eq!(
            status_of(ContainerState::Created, None, None),
            StatusKind::Created
        );
        assert_eq!(
            status_of(ContainerState::Paused, None, None),
            StatusKind::Paused
        );
        assert_eq!(
            status_of(ContainerState::Restarting, None, None),
            StatusKind::Restarting
        );
        assert_eq!(
            status_of(ContainerState::Removing, None, None),
            StatusKind::Removing
        );
        assert_eq!(
            status_of(ContainerState::Dead, None, None),
            StatusKind::Dead
        );
        assert_eq!(
            status_of(ContainerState::Exited, None, Some(137)),
            StatusKind::Exited(137)
        );
        assert_eq!(
            status_of(ContainerState::Exited, None, None),
            StatusKind::Exited(0)
        );
    }

    #[test]
    fn tone_map_matches_components_spec() {
        assert_eq!(StatusKind::Running.tone(), Tone::Success);
        assert_eq!(StatusKind::Healthy.tone(), Tone::Success);
        assert_eq!(StatusKind::Starting.tone(), Tone::Warning);
        assert_eq!(StatusKind::Restarting.tone(), Tone::Warning);
        assert_eq!(StatusKind::Unhealthy.tone(), Tone::Warning);
        assert_eq!(StatusKind::Paused.tone(), Tone::Warning);
        assert_eq!(StatusKind::Exited(137).tone(), Tone::Danger);
        assert_eq!(StatusKind::Dead.tone(), Tone::Danger);
        assert_eq!(StatusKind::Exited(0).tone(), Tone::Stopped);
        assert_eq!(StatusKind::Created.tone(), Tone::Info);
        assert_eq!(StatusKind::Removing.tone(), Tone::Stopped);
    }

    #[test]
    fn labels_use_status_vocabulary() {
        assert_eq!(StatusKind::Running.label(), "Running");
        assert_eq!(StatusKind::Exited(137).label(), "Exited (137)");
        assert_eq!(StatusKind::Exited(0).label(), "Exited (0)");
        assert_eq!(StatusKind::Dead.label(), "Dead");
    }
}
