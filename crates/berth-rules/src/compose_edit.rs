//! D32: text edits that keep comments. P5-T2 implements.

use crate::RulesError;

fn pending() -> RulesError {
    RulesError::Read(std::path::PathBuf::new(), "P5-T2 implements".to_string())
}

// P5-T2 implements
pub fn add_service(_text: &str, _name: &str, _block_yaml: &str) -> Result<String, RulesError> {
    Err(pending())
}

// P5-T2 implements
pub fn set_host_port(
    _text: &str,
    _service: &str,
    _container_port: u16,
    _new_host_port: u16,
) -> Result<String, RulesError> {
    Err(pending())
}

// P5-T2 implements
#[must_use]
pub fn add_env_lines(_env_text: Option<&str>, _lines: &[(String, String)]) -> String {
    String::new()
}

// P5-T2 implements
pub fn service_names(_text: &str) -> Result<Vec<String>, RulesError> {
    Err(pending())
}

/// `${VAR}` and `$VAR`.
// P5-T2 implements
#[must_use]
pub fn referenced_vars(_text: &str) -> Vec<String> {
    Vec::new()
}

/// Top-level volumes: entry for a VolumeName input.
// P5-T2 implements
pub fn add_named_volume(_text: &str, _name: &str) -> Result<String, RulesError> {
    Err(pending())
}

/// OOM `mem_limit`, `platform: linux/arm64` fixes.
// P5-T2 implements
pub fn set_service_key(
    _text: &str,
    _service: &str,
    _key: &str,
    _value: &str,
) -> Result<String, RulesError> {
    Err(pending())
}
