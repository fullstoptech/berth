//! One file per screen (PLAN 6.11). Each screen reads signals and talks to
//! services; none calls an engine (D23).

pub mod activity;
pub mod activity_scene;
pub mod container_detail;
pub mod container_limits;
pub mod containers;
#[cfg(test)]
pub(crate) mod css_tracks;
pub mod logs;
pub mod logs_scene;
pub mod palette;
pub mod projects;
pub mod recent_activity;
