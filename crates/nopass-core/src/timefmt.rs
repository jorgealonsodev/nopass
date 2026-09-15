//! Hand-rolled UTC RFC 3339 formatting for `systemd-run --on-calendar`.
//!
//! One direction of one conversion (`epoch: u64` → `YYYY-MM-DDTHH:MM:SSZ`),
//! with no timezone database, no parsing, and no locale — so `chrono`/`time`
//! would add a transitive dependency tree to a privileged binary for a
//! function fully pinned by a table of epoch → string vectors (see
//! design.md Architecture Decisions). Uses Howard Hinnant's
//! `civil_from_days` algorithm: proleptic-Gregorian, no leap seconds.

/// Formats `epoch` (seconds since the Unix epoch, UTC) as
/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_utc_rfc3339(epoch: u64) -> String {
    let (year, month, day, hour, minute, second) = split_utc(epoch);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The same instant in the form `systemd.time(7)` actually parses:
/// `YYYY-MM-DD HH:MM:SS UTC`, space-separated, with an explicit timezone
/// suffix and no `T` and no `Z`.
///
/// This is NOT cosmetic and it is NOT interchangeable with
/// [`format_utc_rfc3339`]. systemd's calendar grammar resembles RFC-3339
/// without being it: verified against systemd 255 with
/// `systemd-analyze calendar`, the `T` separator and the `Z` designator are
/// each rejected on their own, and only the space-separated form with a
/// named zone is accepted.
///
/// Emitting the RFC-3339 form to `--on-calendar` is why no timed grant ever
/// worked from M1 until this was found: `systemd-run` refused every schedule
/// with "Failed to parse calendar event specification", the helper correctly
/// rolled the rule back, and the user saw an operation that did nothing.
///
/// The argv tests could not catch it because they assert that we produce a
/// string, never that systemd accepts one. `calendar_strings_are_accepted_by_systemd_analyze`
/// closes that gap by asking the real tool.
pub fn format_systemd_calendar(epoch: u64) -> String {
    let (year, month, day, hour, minute, second) = split_utc(epoch);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

/// Shared civil-time decomposition, so the two renderings can never disagree
/// about which instant they describe.
fn split_utc(epoch: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = (epoch / 86_400) as i64;
    let secs_of_day = epoch % 86_400;
    let (year, month, day) = civil_from_days(days);
    (year, month, day, secs_of_day / 3600, (secs_of_day % 3600) / 60, secs_of_day % 60)
}

/// Howard Hinnant's `civil_from_days`: converts a day count since
/// 1970-01-01 into a proleptic-Gregorian `(year, month, day)` triple.
/// `month` and `day` are 1-based.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// The test that would have caught the defect this module now documents: it
/// asks the real tool whether the real string parses, instead of asserting
/// that we produced the string we meant to produce.
///
/// `systemd-analyze calendar` needs no root and no running systemd, only the
/// binary. Where it is absent — a container image without systemd, a
/// non-Linux developer machine — these tests FAIL rather than skip.
///
/// H3 (verify-report.md): an earlier version of these tests "skipped
/// loudly" by `eprintln!`-ing and returning early. That does not work:
/// `cargo test` captures the stderr of a passing test, so on a machine
/// without `systemd-analyze` both tests reported `ok` in 0.00 s with exit
/// 0 — the exact silent-pass signature that let the RFC-3339-vs-calendar
/// defect this module exists to fix survive its own container test in the
/// first place. A skip that is invisible in the gate's own exit status and
/// default output is not a skip a reviewer can trust; it is the same
/// mechanism that hid the original bug, reproduced inside its own remedy.
///
/// So this crate takes the harder, more honest position: `systemd-analyze`
/// is a required tool for `cargo test -p nopass-core`, not an optional one.
/// A machine that cannot provide it must not be able to pass this gate
/// silently — it has to either install `systemd-analyze` or make a visible
/// decision (deleting or explicitly `#[ignore]`-ing these tests, which
/// shows up as a nonzero "ignored" count in `cargo test`'s own summary,
/// unlike a captured `eprintln!`) to run without it.
#[cfg(test)]
mod systemd_contract {
    use super::format_systemd_calendar;

    fn require_systemd_analyze() {
        // Delegates to the single shared "fail loudly, never skip" gate
        // (design.md §6, `toolgate::require`) instead of keeping a
        // standalone copy of the same `.expect(...)` here — this module's
        // own doc comment above explains why a second copy of this rule
        // is exactly how it gets softened.
        crate::toolgate::require(
            "systemd-analyze",
            "it is the only thing that validates the --on-calendar value systemd actually \
             accepts, and this crate would rather fail this test loudly than let that contract \
             degrade back to being pinned by our own assertion alone (verify-report.md H3)",
        );
    }

    #[test]
    fn calendar_strings_are_accepted_by_systemd_analyze() {
        require_systemd_analyze();

        // One ordinary instant, one epoch-adjacent, one leap day, one far
        // future: the same spread the formatter's own vectors use.
        for epoch in [1_789_000_000_u64, 100, 1_582_979_696, 2_147_483_647] {
            let rendered = format_systemd_calendar(epoch);
            let out = std::process::Command::new("systemd-analyze")
                .arg("calendar")
                .arg(&rendered)
                .output()
                .expect("systemd-analyze ran once already");
            assert!(
                out.status.success(),
                "systemd rejected our own --on-calendar value {rendered:?}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }

    #[test]
    fn the_rfc3339_rendering_is_the_one_systemd_refuses() {
        // Pins WHY the two renderings exist. If a future change makes systemd
        // accept the RFC-3339 form, this test fails and someone re-reads the
        // decision instead of discovering it by accident years later.
        require_systemd_analyze();

        let rfc = super::format_utc_rfc3339(1_789_000_000);
        let out = std::process::Command::new("systemd-analyze").arg("calendar").arg(&rfc).output().unwrap();
        assert!(
            !out.status.success(),
            "systemd now accepts {rfc:?}; the two renderings may no longer need to differ"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_unix_epoch() {
        assert_eq!(format_utc_rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn formats_a_leap_day_with_a_non_midnight_time() {
        // 2020-02-29T12:34:56Z, verified via `date -u -d @1582979696`.
        assert_eq!(format_utc_rfc3339(1_582_979_696), "2020-02-29T12:34:56Z");
    }

    #[test]
    fn formats_the_2038_boundary() {
        // i32::MAX seconds — the classic "Year 2038 problem" instant.
        assert_eq!(format_utc_rfc3339(2_147_483_647), "2038-01-19T03:14:07Z");
    }

    #[test]
    fn formats_the_design_document_example_epoch() {
        assert_eq!(format_utc_rfc3339(1_789_000_000), "2026-09-10T00:26:40Z");
    }
}
