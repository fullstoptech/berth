//! Free host port suggestions. P5-T2 implements.

/// Next free port above `wanted`.
// P5-T2 implements
#[must_use]
pub fn suggest(wanted: u16, _taken: &[u16]) -> u16 {
    wanted
}
