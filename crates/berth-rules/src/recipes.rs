//! Generation recipes per stack. P5-T2 implements.

use crate::RulesError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeOptions {
    pub ports: std::collections::BTreeMap<String, u16>,
    pub node_version: Option<String>,
    pub services: Vec<berth_core::ServiceKind>,
}

// P5-T2 implements
#[must_use]
pub fn options_for(_d: &berth_core::Detection, _taken_ports: &[u16]) -> RecipeOptions {
    RecipeOptions {
        ports: std::collections::BTreeMap::new(),
        node_version: None,
        services: Vec::new(),
    }
}

/// Dockerfile (when missing), .dockerignore (when missing), compose.yaml, .env additions. Never overwrites a file the folder has.
/// Reads and hashes .env for Modify diffs.
// P5-T2 implements
pub fn plan(
    d: &berth_core::Detection,
    _o: &RecipeOptions,
) -> Result<berth_core::GenerationPlan, RulesError> {
    Err(RulesError::Read(
        d.folder.clone(),
        "P5-T2 implements".to_string(),
    ))
}

// P5-T2 implements
pub fn compose_for_dockerfile(
    d: &berth_core::Detection,
    _o: &RecipeOptions,
) -> Result<berth_core::GenerationPlan, RulesError> {
    Err(RulesError::Read(
        d.folder.clone(),
        "P5-T2 implements".to_string(),
    ))
}
