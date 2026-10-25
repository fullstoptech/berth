//! Built-in service templates (D84) and Add service plans. P5-T2 implements.

use crate::RulesError;

// P5-T2 implements
#[must_use]
pub fn built_in() -> Vec<berth_core::Template> {
    Vec::new()
}

// P5-T2 implements
#[must_use]
pub fn render(
    _t: &berth_core::Template,
    _inputs: &std::collections::BTreeMap<String, String>,
) -> (String, Vec<(String, String)>) {
    (String::new(), Vec::new())
}

/// 24 chars, [A-Za-z0-9].
// P5-T2 implements
#[must_use]
pub fn generate_password() -> String {
    String::new()
}

/// `compose_rel`: the file may be docker-compose.yml.
// P5-T2 implements
pub fn add_service_plan(
    root: &std::path::Path,
    _compose_rel: &std::path::Path,
    _compose_text: &str,
    _env_text: Option<&str>,
    _t: &berth_core::Template,
    _inputs: &std::collections::BTreeMap<String, String>,
) -> Result<berth_core::GenerationPlan, RulesError> {
    Err(RulesError::Read(
        root.to_path_buf(),
        "P5-T2 implements".to_string(),
    ))
}
