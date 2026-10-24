#![allow(unused_imports)]
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod other;

#[cfg(target_os = "macos")]
pub use macos::{config, use_traffic_lights};
#[cfg(not(target_os = "macos"))]
pub use other::{config, use_traffic_lights};
