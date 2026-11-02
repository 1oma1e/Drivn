//! Date conversions for the two Windows date formats we read, without a date crate.

const SECONDS_PER_DAY: i64 = 86_400;
const FILETIME_TICKS_PER_DAY: u64 = 864_000_000_000;
/// Days from 1601-01-01 (FILETIME epoch) to 1970-01-01.
const FILETIME_EPOCH_TO_UNIX_DAYS: i64 = 134_774;
/// Days from 1899-12-30 (OLE automation DATE epoch) to 1970-01-01.
const OLE_EPOCH_TO_UNIX_DAYS: i64 = 25_569;

/// Civil date from days since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_from_unix_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn format_date(unix_days: i64) -> String {
    let (y, m, d) = civil_from_unix_days(unix_days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` from a FILETIME (100 ns ticks since 1601-01-01 UTC), as in `DEVPKEY_Device_DriverDate`.
pub fn date_from_filetime(ticks: u64) -> String {
    let days = (ticks / FILETIME_TICKS_PER_DAY) as i64;
    format_date(days - FILETIME_EPOCH_TO_UNIX_DAYS)
}

/// `YYYY-MM-DD` from an OLE automation DATE (days since 1899-12-30), as in `DriverVerDate`.
/// Returns `None` for values that are not a real date (NaN, infinite, before year 1).
pub fn date_from_ole(date: f64) -> Option<String> {
    if !date.is_finite() || date < -657_434.0 {
        return None;
    }
    Some(format_date(date.floor() as i64 - OLE_EPOCH_TO_UNIX_DAYS))
}

/// `YYYY-MM-DDTHH:MM:SSZ` from an OLE automation DATE holding a UTC time, as in
/// `LastSearchSuccessDate`. `None` for values that are not a real date (NaN, infinite, before 1900,
/// which is what an empty VARIANT turns into).
pub fn iso_utc_from_ole(date: f64) -> Option<String> {
    if !date.is_finite() || date < 1.0 {
        return None;
    }
    let seconds = ((date - OLE_EPOCH_TO_UNIX_DAYS as f64) * SECONDS_PER_DAY as f64).round();
    Some(iso_utc_from_unix_seconds(seconds as i64))
}

/// `YYYY-MM-DDTHH:MM:SSZ` from seconds since 1970-01-01 UTC.
pub fn iso_utc_from_unix_seconds(seconds: i64) -> String {
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let rest = seconds.rem_euclid(SECONDS_PER_DAY);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        format_date(days),
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Seconds since 1970-01-01 UTC, now (0 if the clock is before 1970).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// `YYYY-MM-DDTHH:MM:SSZ`, now.
pub fn now_iso_utc() -> String {
    iso_utc_from_unix_seconds(unix_now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_of_unix_epoch_is_1970_01_01() {
        assert_eq!(date_from_filetime(116_444_736_000_000_000), "1970-01-01");
    }

    #[test]
    fn filetime_of_a_known_driver_date() {
        // 2006-06-21 is the classic inbox DriverVer date; FILETIME computed from the Unix seconds.
        let ticks = (1_150_848_000_u64 + 11_644_473_600) * 10_000_000;
        assert_eq!(date_from_filetime(ticks), "2006-06-21");
    }

    #[test]
    fn ole_date_epoch_and_leap_day() {
        assert_eq!(date_from_ole(0.0).as_deref(), Some("1899-12-30"));
        assert_eq!(date_from_ole(25_569.0).as_deref(), Some("1970-01-01"));
        // 2024-02-29 = 19_782 days after 1970-01-01.
        assert_eq!(
            date_from_ole(25_569.0 + 19_782.75).as_deref(),
            Some("2024-02-29")
        );
    }

    #[test]
    fn ole_datetime_in_utc_and_empty_variant() {
        // 2026-10-31 21:07:56 UTC, the last Windows search seen on the reference PC.
        let date = 25_569.0 + (1_793_480_876.0 / 86_400.0);
        assert_eq!(
            iso_utc_from_ole(date).as_deref(),
            Some("2026-10-31T21:07:56Z")
        );
        assert_eq!(iso_utc_from_ole(0.0), None);
        assert_eq!(iso_utc_from_ole(f64::NAN), None);
    }

    #[test]
    fn ole_date_rejects_non_dates() {
        assert_eq!(date_from_ole(f64::NAN), None);
        assert_eq!(date_from_ole(f64::INFINITY), None);
    }

    #[test]
    fn iso_timestamp() {
        assert_eq!(iso_utc_from_unix_seconds(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            iso_utc_from_unix_seconds(1_793_366_445),
            "2026-10-30T13:20:45Z"
        );
    }
}
