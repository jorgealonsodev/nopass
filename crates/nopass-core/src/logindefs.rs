//! Hand-parsed `/etc/login.defs` `UID_MIN`/`UID_MAX` grammar and uid
//! admission range.
//!
//! Grammar (line-anchored, `strip_prefix`-based, not the `regex` crate,
//! matching the rest of `nopass-core`'s hand-parsed line grammars):
//! `^\s*UID_MIN\s+([0-9]+)` / `^\s*UID_MAX\s+([0-9]+)`.

/// Floor a parsed (or missing/unparseable) `UID_MIN` is always clamped to.
/// uid 0 is rejected unconditionally by [`UidRange::admits`] regardless of
/// this floor (privilege-admission §UID Range Admission).
pub const UID_FLOOR: u32 = 1000;

/// Default `UID_MAX` when `/etc/login.defs` does not declare one.
pub const DEFAULT_MAX: u32 = 60_000;

/// The admissible uid range read from `/etc/login.defs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UidRange {
    pub min: u32,
    pub max: u32,
}

impl UidRange {
    /// True when `uid` falls within `[min, max]` inclusive. uid 0 is
    /// rejected unconditionally, even when `min` was somehow 0 — this is a
    /// hard invariant independent of `/etc/login.defs` content.
    pub fn admits(&self, uid: u32) -> bool {
        if uid == 0 {
            return false;
        }
        uid >= self.min && uid <= self.max
    }
}

/// Parses `UID_MIN`/`UID_MAX` out of `/etc/login.defs` `content`. A missing
/// or unparseable value falls back to its default; a parsed `UID_MIN` below
/// [`UID_FLOOR`] is clamped up to the floor. Passing an empty string (a
/// missing file) yields the same defaults as an unparseable one.
pub fn parse(content: &str) -> UidRange {
    let mut min = None;
    let mut max = None;
    for line in content.lines() {
        if let Some(v) = parse_key_value(line, "UID_MIN") {
            min = Some(v);
        } else if let Some(v) = parse_key_value(line, "UID_MAX") {
            max = Some(v);
        }
    }
    UidRange {
        min: min.unwrap_or(UID_FLOOR).max(UID_FLOOR),
        max: max.unwrap_or(DEFAULT_MAX),
    }
}

/// Matches `^\s*<key>\s+([0-9]+)` and returns the parsed digits, or `None`
/// if the line does not match the grammar.
fn parse_key_value(line: &str, key: &str) -> Option<u32> {
    let rest = line.trim_start().strip_prefix(key)?;
    let rest = rest.strip_prefix(|c: char| c.is_ascii_whitespace())?;
    let digits: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_range_admits_a_default_range_uid() {
        let range = parse("UID_MIN 1000\nUID_MAX 60000\n");
        assert!(range.admits(1000));
        assert!(range.admits(60000));
        assert!(!range.admits(999));
        assert!(!range.admits(60001));
    }

    #[test]
    fn uid_min_0_clamps_to_the_floor() {
        let range = parse("UID_MIN 0\nUID_MAX 60000\n");
        assert_eq!(range.min, UID_FLOOR);
    }

    #[test]
    fn uid_0_is_always_rejected_even_with_uid_min_0() {
        let range = parse("UID_MIN 0\nUID_MAX 60000\n");
        assert!(!range.admits(0));
    }

    #[test]
    fn missing_login_defs_clamps_to_the_default_floor_and_max() {
        let range = parse("");
        assert_eq!(range.min, UID_FLOOR);
        assert_eq!(range.max, DEFAULT_MAX);
    }

    #[test]
    fn unparseable_login_defs_clamps_to_the_default_floor_and_max() {
        let range = parse("not a login.defs file\nUID_MIN abc\nUID_MAX xyz\n# comment\n");
        assert_eq!(range.min, UID_FLOOR);
        assert_eq!(range.max, DEFAULT_MAX);
    }

    #[test]
    fn parsed_uid_min_above_the_floor_is_kept_verbatim() {
        let range = parse("UID_MIN 2000\nUID_MAX 60000\n");
        assert_eq!(range.min, 2000);
        assert!(!range.admits(1500));
        assert!(range.admits(2000));
    }

    #[test]
    fn leading_whitespace_and_extra_tokens_on_the_line_are_tolerated() {
        let range = parse("  UID_MIN\t1500  \nUID_MAX 50000 # trailing comment\n");
        assert_eq!(range.min, 1500);
        assert_eq!(range.max, 50000);
    }
}
