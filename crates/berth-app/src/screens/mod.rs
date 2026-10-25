//! One file per screen (PLAN 6.11). Each screen reads signals and talks to
//! services; none calls an engine (D23).

pub mod activity;
pub mod activity_scene;
pub mod container_detail;
pub mod containers;
#[cfg(test)]
mod css_tracks;
pub mod palette;
pub mod projects;
pub mod recent_activity;
