//! `GrantDuration`: the six grant lengths the tray offers, and the one
//! place their `--until`/`--until-reboot` XOR is spelled (design.md §2).
//!
//! `Permanent` producing `vec![]` — "neither flag" — is expressed by
//! construction here rather than remembered by a caller: every branch of
//! [`GrantDuration::args`] reads [`GrantDuration::expiry`], so there is
//! exactly one decision point for what argv a duration renders, matching
//! `crates/nopass-helper/src/cli.rs:19`'s
//! `ArgGroup::new("when").multiple(false)` enforcing the same XOR from the
//! helper side of the boundary.
//!
//! [`GrantDuration::ALL`] is the **single** canonical ordering of the six
//! variants in this crate. Phase 6's menu build renders items from it in
//! order, and Phase 7's `RadioGroup::select` looks up the user's click by
//! index into it (design.md Open Questions). A second ordering anywhere
//! else would be a silent mis-selection — the user picks "4 hours" and
//! the tray enables "until reboot" instead, with no error — so this
//! module is the only place `ALL` may be spelled, and
//! `all_is_the_single_canonical_ordering_...` below pins the exact
//! sequence so reordering it fails a test instead of shipping unreviewed.

use nopass_core::expiry::Expiry;

/// One of the six grant lengths the tray's duration submenu (or a
/// `default_duration` config value) can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantDuration {
    Minutes15,
    Hour1,
    Hours4,
    Hours8,
    UntilReboot,
    Permanent,
}

impl GrantDuration {
    /// The single canonical ordering — see this module's doc comment.
    pub const ALL: [GrantDuration; 6] = [
        GrantDuration::Minutes15,
        GrantDuration::Hour1,
        GrantDuration::Hours4,
        GrantDuration::Hours8,
        GrantDuration::UntilReboot,
        GrantDuration::Permanent,
    ];

    /// The [`Expiry`] this duration resolves to, given `now` (seconds
    /// since the Unix epoch). This is the one place the fixed durations'
    /// second counts are spelled; [`GrantDuration::args`] reads this
    /// rather than re-deriving the same numbers, so the two can never
    /// disagree about what a variant means.
    pub fn expiry(self, now: u64) -> Expiry {
        match self {
            GrantDuration::Minutes15 => Expiry::At { epoch: now + 900 },
            GrantDuration::Hour1 => Expiry::At { epoch: now + 3_600 },
            GrantDuration::Hours4 => Expiry::At { epoch: now + 14_400 },
            GrantDuration::Hours8 => Expiry::At { epoch: now + 28_800 },
            GrantDuration::UntilReboot => Expiry::Reboot,
            GrantDuration::Permanent => Expiry::Never,
        }
    }

    /// The argv this duration appends after `enable` — the ONLY place the
    /// `--until`/`--until-reboot` XOR is spelled on the tray side.
    /// `Permanent` yields `vec![]`, i.e. neither flag: "permanent" is
    /// unrepresentable as anything other than "no expiry flag at all",
    /// by construction, because this match has no arm that could emit
    /// both.
    pub fn args(self, now: u64) -> Vec<String> {
        match self.expiry(now) {
            Expiry::At { epoch } => vec!["--until".to_string(), epoch.to_string()],
            Expiry::Reboot => vec!["--until-reboot".to_string()],
            Expiry::Never => vec![],
        }
    }

    /// The stable string this variant is written to and read from
    /// `config.toml` as (Phase 2's `default_duration` key).
    pub fn config_key(self) -> &'static str {
        match self {
            GrantDuration::Minutes15 => "15m",
            GrantDuration::Hour1 => "1h",
            GrantDuration::Hours4 => "4h",
            GrantDuration::Hours8 => "8h",
            GrantDuration::UntilReboot => "reboot",
            GrantDuration::Permanent => "permanent",
        }
    }

    /// The inverse of [`GrantDuration::config_key`]. `None` for anything
    /// that is not one of the six documented keys — including an old or
    /// hand-edited `config.toml` value — so a caller can fall back to a
    /// default rather than trusting an unrecognized string (tolerant-read
    /// contract, design.md §4).
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|duration| duration.config_key() == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_789_000_000;

    #[test]
    fn args_never_contain_both_until_and_until_reboot() {
        for duration in GrantDuration::ALL {
            let args = duration.args(NOW);
            let has_until = args.iter().any(|a| a == "--until");
            let has_until_reboot = args.iter().any(|a| a == "--until-reboot");
            assert!(
                !(has_until && has_until_reboot),
                "{duration:?} produced both --until and --until-reboot: {args:?}"
            );
        }
    }

    #[test]
    fn exactly_one_variant_over_all_produces_neither_flag() {
        let neither_count =
            GrantDuration::ALL.iter().filter(|d| d.args(NOW).is_empty()).count();
        assert_eq!(neither_count, 1, "exactly one variant (Permanent) must produce neither flag");
        assert_eq!(GrantDuration::Permanent.args(NOW), Vec::<String>::new());
    }

    #[test]
    fn the_four_fixed_durations_render_until_with_the_documented_offset() {
        assert_eq!(GrantDuration::Minutes15.args(NOW), vec!["--until", &(NOW + 900).to_string()]);
        assert_eq!(GrantDuration::Hour1.args(NOW), vec!["--until", &(NOW + 3_600).to_string()]);
        assert_eq!(GrantDuration::Hours4.args(NOW), vec!["--until", &(NOW + 14_400).to_string()]);
        assert_eq!(GrantDuration::Hours8.args(NOW), vec!["--until", &(NOW + 28_800).to_string()]);
    }

    #[test]
    fn until_reboot_renders_exactly_the_bare_flag() {
        assert_eq!(GrantDuration::UntilReboot.args(NOW), vec!["--until-reboot"]);
    }

    #[test]
    fn config_key_and_parse_round_trip_for_every_variant() {
        for duration in GrantDuration::ALL {
            let key = duration.config_key();
            assert_eq!(GrantDuration::parse(key), Some(duration), "round trip failed for {key:?}");
        }
    }

    #[test]
    fn parse_rejects_an_unknown_string() {
        assert_eq!(GrantDuration::parse("60m"), None);
        assert_eq!(GrantDuration::parse(""), None);
        assert_eq!(GrantDuration::parse("Permanent"), None, "config_key is lowercase; parse must not be lenient");
    }

    #[test]
    fn expiry_matches_the_offsets_args_is_derived_from() {
        assert_eq!(GrantDuration::Minutes15.expiry(NOW), Expiry::At { epoch: NOW + 900 });
        assert_eq!(GrantDuration::Hour1.expiry(NOW), Expiry::At { epoch: NOW + 3_600 });
        assert_eq!(GrantDuration::Hours4.expiry(NOW), Expiry::At { epoch: NOW + 14_400 });
        assert_eq!(GrantDuration::Hours8.expiry(NOW), Expiry::At { epoch: NOW + 28_800 });
        assert_eq!(GrantDuration::UntilReboot.expiry(NOW), Expiry::Reboot);
        assert_eq!(GrantDuration::Permanent.expiry(NOW), Expiry::Never);
    }

    /// Pins `GrantDuration::ALL` as the single canonical ordering this
    /// crate may ever define — see this module's doc comment. Phase 6's
    /// submenu build and Phase 7's `RadioGroup::select` index lookup both
    /// read this exact array; a second ordering, or a reordering of this
    /// one without reviewing both call sites, is a silent mis-selection
    /// (design Open Questions), not a compile error, which is exactly
    /// why this needs an explicit pin rather than relying on the enum
    /// declaration order "obviously" matching.
    #[test]
    fn all_is_the_single_canonical_ordering_the_menu_and_radio_group_both_index_into() {
        assert_eq!(
            GrantDuration::ALL,
            [
                GrantDuration::Minutes15,
                GrantDuration::Hour1,
                GrantDuration::Hours4,
                GrantDuration::Hours8,
                GrantDuration::UntilReboot,
                GrantDuration::Permanent,
            ]
        );
    }
}
