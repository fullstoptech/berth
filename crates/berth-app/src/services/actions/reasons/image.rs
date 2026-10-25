//! Why an action cannot run on a image right now.

use super::{plural, StateView};

pub(super) fn reason(id: &str, action: &str, state: &StateView<'_>) -> Option<String> {
    if action != "image.delete" {
        return None;
    }
    let Some(image) = state.images.iter().find(|i| i.id == id) else {
        return Some("This image no longer exists".to_string());
    };
    if image.used_by.is_empty() {
        None
    } else {
        Some(format!(
            "Used by {}",
            plural(image.used_by.len(), "container", "containers")
        ))
    }
}
