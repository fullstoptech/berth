//! Free host port suggestions.

/// The port to offer for `wanted`: `wanted` itself when nothing holds it, otherwise the next free port above it.
/// When everything above is taken it falls back to the nearest free port below (never under 1024).
/// When every port is taken it returns `wanted`.
#[must_use]
pub fn suggest(wanted: u16, taken: &[u16]) -> u16 {
    let taken: std::collections::HashSet<u16> = taken.iter().copied().collect();
    let free = |p: u16| !taken.contains(&p);
    if free(wanted) {
        return wanted;
    }
    if let Some(p) = (wanted.saturating_add(1)..=u16::MAX).find(|p| free(*p)) {
        return p;
    }
    (1024..wanted).rev().find(|p| free(*p)).unwrap_or(wanted)
}

#[cfg(test)]
mod tests {
    use super::suggest;

    #[test]
    fn a_free_port_is_kept() {
        assert_eq!(suggest(3000, &[5432, 8080]), 3000);
    }

    #[test]
    fn a_taken_port_moves_to_the_next_free_one() {
        assert_eq!(suggest(5432, &[5432]), 5433);
    }

    #[test]
    fn skips_a_run_of_taken_ports_in_any_order() {
        assert_eq!(suggest(3000, &[3002, 3000, 3001, 3004]), 3003);
    }

    #[test]
    fn at_the_top_of_the_range_it_looks_below() {
        assert_eq!(suggest(65535, &[65535]), 65534);
        assert_eq!(suggest(65534, &[65534, 65535]), 65533);
    }

    #[test]
    fn when_every_port_is_taken_it_gives_the_wanted_one_back() {
        let all: Vec<u16> = (1024..=u16::MAX).collect();
        assert_eq!(suggest(8080, &all), 8080);
    }
}
