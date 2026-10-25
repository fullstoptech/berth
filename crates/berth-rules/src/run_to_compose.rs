//! `docker run` to Compose. P5-T2 implements.

use crate::RulesError;

/// shell-words; -p -e -v --name --restart --network.
// P5-T2 implements
pub fn convert(_docker_run: &str) -> Result<String, RulesError> {
    Err(RulesError::Read(
        std::path::PathBuf::new(),
        "P5-T2 implements".to_string(),
    ))
}
