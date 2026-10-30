use aarambh_core::Id;

/// A stable, time-sortable id derived from its parts (FNV-1a, 128-bit), so
/// re-running the judge on the same input gives the same edge and risk ids.
pub fn derived_id(ts_ms: u64, parts: &[&str]) -> Id {
    let mut h: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for p in parts {
        for b in p.bytes().chain(std::iter::once(0)) {
            h ^= u128::from(b);
            h = h.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
        }
    }
    Id::new(ts_ms, h)
}

/// Collapses whitespace (including newlines) to single spaces.
pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_ids_are_stable_and_distinct() {
        let a = derived_id(1000, &["edge", "x", "y"]);
        assert_eq!(a, derived_id(1000, &["edge", "x", "y"]));
        assert_ne!(
            a,
            derived_id(1000, &["edge", "xy", ""]),
            "parts are separated"
        );
        assert_eq!(a.timestamp_ms(), 1000);
    }

    #[test]
    fn one_line_collapses_whitespace() {
        assert_eq!(one_line("  a\n\n b\tc "), "a b c");
    }
}
