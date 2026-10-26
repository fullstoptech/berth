//! Rendering the context chips into prompt text. Filled in by P7-T3 or later;
//! until then this is a stub with the final signature (PLAN §6.9).

use berth_core::ContextItem;

#[must_use]
pub fn render(_items: &[ContextItem], _resolve: &dyn Fn(&ContextItem) -> String) -> String {
    String::new()
}
