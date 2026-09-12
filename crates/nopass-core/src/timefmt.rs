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
    let days = (epoch / 86_400) as i64;
    let secs_of_day = epoch % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
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
