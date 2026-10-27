//! Sheets: floating panels over the dimmed shell (PLAN §7.1). Each sheet
//! is the body of one `Overlay` variant; `app.rs` mounts it.

pub mod add_engine;
pub mod add_service;
pub mod clean_up;
pub mod create_network;
pub mod create_volume;
pub mod delete_project;
pub mod detected;
pub mod edit_recreate;
pub mod edit_resources;
pub mod generate;
pub mod new_project;
pub mod open_folder;
pub mod registry_login;
pub mod review_changes;
pub mod run;
