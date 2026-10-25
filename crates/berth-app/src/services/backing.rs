//! What the engines behind the app can do. Under `BERTH_MOCK=1` every
//! engine is a `MockEngine`, which answers `EngineError::Unsupported` to the
//! calls that need a real daemon (exec, create, recreate, files). A button
//! that would only end in that raw error is disabled instead, with the
//! reason below, and every check goes through [`Backing::unsupported`] so
//! no screen compares strings or reads the environment itself.
//!
//! `EngineKind` cannot carry this: a mock Colima profile has the same kind
//! as a real one, so the app records how it was started.

/// What backs the engines this run talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backing {
    /// Real Docker engines.
    #[default]
    Live,
    /// The bundled mock world (`BERTH_MOCK=1`).
    Mock,
}

/// Something a `MockEngine` cannot do, grouped by what the user sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// A shell in a container (`exec`).
    Terminals,
    /// Creating or recreating a container (`create`, `recreate`): Edit and
    /// recreate, Duplicate, Recreate with latest image, Run an image.
    CreateContainers,
    /// Listing, downloading and uploading files (`list_files`, `download`,
    /// `upload`): the Files tab, the volume browser, volume backups.
    Files,
}

impl Capability {
    fn what(self) -> &'static str {
        match self {
            Self::Terminals => "Terminals",
            Self::CreateContainers => "Creating containers",
            Self::Files => "Files",
        }
    }

    /// The capability an action id needs, if any.
    #[must_use]
    pub fn of_action(action: &str) -> Option<Self> {
        match action {
            "container.terminal" => Some(Self::Terminals),
            "container.edit"
            | "container.duplicate"
            | "container.recreate_latest"
            | "image.run" => Some(Self::CreateContainers),
            "volume.browse" | "volume.backup" | "volume.restore" => Some(Self::Files),
            _ => None,
        }
    }
}

impl Backing {
    /// Why `capability` is unavailable, or `None` when the engines can do it.
    #[must_use]
    pub fn unsupported(self, capability: Capability) -> Option<String> {
        match self {
            Self::Live => None,
            Self::Mock => Some(format!(
                "{} need a real Docker engine. You're viewing mock data.",
                capability.what()
            )),
        }
    }

    /// [`Self::unsupported`] for an action id.
    #[must_use]
    pub fn unsupported_action(self, action: &str) -> Option<String> {
        self.unsupported(Capability::of_action(action)?)
    }

    /// The backing a run starts with: the mock only when `BERTH_MOCK=1` is
    /// the whole story. A `BERTH_SCENE` frame shows live buttons as drawn.
    #[must_use]
    pub fn at_boot(mock: bool, scene: bool) -> Self {
        if mock && !scene {
            Self::Mock
        } else {
            Self::Live
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backing_mock_says_why_and_live_says_nothing() {
        for capability in [
            Capability::Terminals,
            Capability::CreateContainers,
            Capability::Files,
        ] {
            assert_eq!(Backing::Live.unsupported(capability), None);
            let reason = Backing::Mock.unsupported(capability).expect("a reason");
            assert!(
                reason.ends_with("need a real Docker engine. You're viewing mock data."),
                "{reason}"
            );
        }
        assert_eq!(
            Backing::Mock.unsupported(Capability::Terminals).as_deref(),
            Some("Terminals need a real Docker engine. You're viewing mock data.")
        );
    }

    #[test]
    fn backing_every_action_the_mock_engine_refuses_maps_to_a_capability() {
        for action in [
            "container.terminal",
            "container.edit",
            "container.duplicate",
            "container.recreate_latest",
            "image.run",
            "volume.browse",
            "volume.backup",
        ] {
            assert!(
                Backing::Mock.unsupported_action(action).is_some(),
                "{action}"
            );
            assert_eq!(Backing::Live.unsupported_action(action), None, "{action}");
        }
        // What the mock engine can do stays enabled.
        for action in ["container.start", "container.stop", "container.delete"] {
            assert_eq!(Backing::Mock.unsupported_action(action), None, "{action}");
        }
    }

    #[test]
    fn backing_a_scene_keeps_the_frame_buttons_live() {
        assert_eq!(Backing::at_boot(true, false), Backing::Mock);
        assert_eq!(Backing::at_boot(true, true), Backing::Live);
        assert_eq!(Backing::at_boot(false, false), Backing::Live);
    }
}
