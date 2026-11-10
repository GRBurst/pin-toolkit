//! Release-age math: RFC 3339 → unix seconds and the `min-age` cutoff.

/// Days in month `m` (1-12) for year `y` (leap-year aware); 0 if invalid.
fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Days from 1970-01-01 to the civil date (y, m, d) — Hinnant's
/// `civil_days_since_epoch`. Valid inputs only (callers pre-validate).
fn civil_to_days(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 }; // March-based [0, 11]
    let doy = (153 * mp as i64 + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Parse an RFC 3339 timestamp (`YYYY-MM-DDTHH:MM:SS[.frac][Z|±HH:MM]`)
/// to unix seconds. Fractional seconds are dropped; `None` when the
/// string is not a valid, in-range timestamp.
pub fn parse_rfc3339_unix(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let digit = |r: std::ops::Range<usize>| -> Option<i64> {
        std::str::from_utf8(b.get(r)?).ok()?.parse().ok()
    };
    if b.len() < 20 {
        return None;
    }
    let y = digit(0..4)?;
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let m = digit(5..7)?;
    let d = digit(8..10)?;
    if !(1..=12).contains(&m) || !(1..=days_in_month(y, m as u32) as i64).contains(&d) {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let hh = digit(11..13)?;
    let mm = digit(14..16)?;
    let ss = digit(17..19)?;
    if !(0..=23).contains(&hh) || !(0..=59).contains(&mm) || !(0..=60).contains(&ss) {
        return None;
    }
    let mut i = 19usize;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None; // "…T12:00:00." with no fraction digits
        }
    }
    let offset_secs: i64 = if b.get(i) == Some(&b'Z') {
        if i + 1 != b.len() {
            return None;
        }
        0
    } else {
        let sign = match b.get(i) {
            Some(b'+') => 1,
            Some(b'-') => -1,
            _ => return None,
        };
        if i + 6 > b.len() || b[i + 3] != b':' {
            return None;
        }
        let oh = digit(i + 1..i + 3)?;
        let om = digit(i + 4..i + 6)?;
        if !(0..=23).contains(&oh) || !(0..=59).contains(&om) || i + 6 != b.len() {
            return None;
        }
        sign * (oh * 3600 + om * 60)
    };
    Some(civil_to_days(y, m as u32, d as u32) * 86_400 + hh * 3600 + mm * 60 + ss - offset_secs)
}

/// True when `ts_unix` is at least `min_days` days before `now_unix`
/// (boundary-inclusive: age exactly N days is old enough).
/// Consumed by the `--min-age-days` guard in the update path (M4b-1 T2).
pub fn is_old_enough(ts_unix: i64, now_unix: i64, min_days: u64) -> bool {
    ts_unix <= now_unix - min_days as i64 * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_parse_table() {
        // Vectors verified with `date -ud` 2026-11-09 (controller).
        assert_eq!(parse_rfc3339_unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_unix("2026-11-08T14:24:40.263592Z"),
            Some(1794147880)
        );
        assert_eq!(
            parse_rfc3339_unix("2024-02-29T23:59:59+02:00"),
            Some(1709243999)
        );
        assert_eq!(
            parse_rfc3339_unix("2999-01-01T00:00:00Z"),
            Some(32472144000)
        );
        assert_eq!(
            parse_rfc3339_unix("2020-01-01T00:00:00+00:00"),
            parse_rfc3339_unix("2020-01-01T00:00:00Z")
        );
        assert_eq!(parse_rfc3339_unix("2020-01-01T00:00:00Z"), Some(1577836800));
        // Invalid.
        assert_eq!(parse_rfc3339_unix("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_rfc3339_unix("2026-02-30T00:00:00Z"), None);
        assert_eq!(parse_rfc3339_unix("not-a-date"), None);
        assert_eq!(parse_rfc3339_unix("2026-11-08 14:24:40Z"), None);
        assert_eq!(parse_rfc3339_unix("2026-11-08T14:24:40"), None);
        assert_eq!(parse_rfc3339_unix(""), None);
    }

    #[test]
    fn is_old_enough_table() {
        let now = 1_700_000_000i64;
        let day = 86_400i64;
        // Boundary: ts == now - N*86400 → old enough (age >= N days).
        assert!(is_old_enough(now - 14 * day, now, 14));
        // 1s newer than the cutoff → not old enough.
        assert!(!is_old_enough(now - 14 * day + 1, now, 14));
        // N = 0 → any ts <= now is old enough.
        assert!(is_old_enough(now, now, 0));
        assert!(is_old_enough(now - 1, now, 0));
        assert!(!is_old_enough(now + 1, now, 0));
        // Far older.
        assert!(is_old_enough(0, now, 14));
    }
}
