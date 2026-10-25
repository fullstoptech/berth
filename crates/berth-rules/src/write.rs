//! Guarded plan application (D31, ADR 0012). P5-T2 implements.

use crate::RulesError;

#[derive(Debug)]
pub struct WriteReport {
    pub written: Vec<std::path::PathBuf>,
}

/// D31: same-folder temp file + rename; refuses outside root and on hash mismatch; writes nothing if any check fails.
// P5-T2 implements
pub fn apply(plan: &berth_core::GenerationPlan) -> Result<WriteReport, RulesError> {
    Err(RulesError::Read(
        plan.root.clone(),
        "P5-T2 implements".to_string(),
    ))
}
