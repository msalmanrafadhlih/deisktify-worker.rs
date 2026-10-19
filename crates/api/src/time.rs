//! Waktu UTC tanpa dependensi tanggal (wasm kecil). Algoritma sipil Howard
//! Hinnant (`civil_from_days` / `days_from_civil`).

/// Hari Unix -> (tahun, bulan 1..=12, hari 1..=31).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// (tahun, bulan 1..=12, hari 1..=31) -> hari Unix.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if month > 2 { month - 3 } else { month + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Hari dalam seminggu untuk hari Unix: 0 = Minggu ... 6 = Sabtu.
pub fn weekday_from_days(days: i64) -> u8 {
    // 1970-01-01 adalah hari Kamis (4).
    u8::try_from((days + 4).rem_euclid(7)).unwrap_or(0)
}

/// Detik Unix -> `YYYY-MM-DDTHH:MM:SSZ` (UTC).
pub fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// `YYYY-MM-DD` untuk hari Unix.
pub fn format_date(days: i64) -> String {
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Kebalikan [`format_utc`]: hanya bentuk persis `YYYY-MM-DDTHH:MM:SSZ`.
pub fn parse_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let part = &s[r];
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, m, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || h > 23 || m > 59 || sec > 59 {
        return None;
    }
    let days = days_from_civil(i64::from(year), month, day);
    // Tolak tanggal yang tidak ada (mis. 31 Februari), bukan menggulirkannya.
    if civil_from_days(days) != (i64::from(year), month, day) {
        return None;
    }
    Some(days * 86_400 + i64::from(h) * 3600 + i64::from(m) * 60 + i64::from(sec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_utc(-1), "1969-12-31T23:59:59Z");
        assert_eq!(format_date(20_381), "2025-10-20");
    }

    #[test]
    fn civil_conversion_round_trips_over_two_centuries() {
        for days in (-25_000..60_000).step_by(7) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
        for days in 19_000..19_800 {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn weekdays_start_on_sunday() {
        // 1970-01-01 Kamis; 2026-10-20 Selasa; 2000-02-29 Selasa; 2026-10-18 Minggu.
        assert_eq!(weekday_from_days(0), 4);
        assert_eq!(weekday_from_days(days_from_civil(2026, 10, 20)), 2);
        assert_eq!(weekday_from_days(days_from_civil(2000, 2, 29)), 2);
        assert_eq!(weekday_from_days(days_from_civil(2026, 10, 18)), 0);
        assert_eq!(weekday_from_days(-1), 3);
    }

    #[test]
    fn parse_is_the_inverse_of_format_and_strict() {
        for secs in [0, 1_700_000_000, 951_782_400, 4_102_444_799, 1_792_000_000] {
            assert_eq!(parse_utc(&format_utc(secs)), Some(secs));
        }
        for bad in [
            "",
            "2026-10-20",
            "2026-10-20 12:00:00Z",
            "2026-10-20T12:00:00",
            "2026-10-20T12:00:00z",
            "2026-13-20T12:00:00Z",
            "2026-02-31T12:00:00Z",
            "2026-10-20T24:00:00Z",
            "2026-10-20T12:60:00Z",
            "2026-10-20T12:00:60Z",
            "2026-1x-20T12:00:00Z",
            "2026-10-20T12:00:00ZZ",
            "+026-10-20T12:00:00Z",
        ] {
            assert_eq!(parse_utc(bad), None, "{bad:?}");
        }
    }
}
