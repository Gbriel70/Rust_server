//! IMF-fixdate only; obsolete HTTP date formats are deliberately unsupported.
use std::time::{Duration, SystemTime, UNIX_EPOCH};
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + i64::from(m <= 2), m, d)
}
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = m + if m > 2 { -3 } else { 9 };
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + d - 1 - 719468
}
pub fn format_http_date(time: SystemTime) -> String {
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX / 2),
        Err(e) => -(e.duration().as_secs() as i64) - i64::from(e.duration().subsec_nanos() != 0),
    };
    let day = seconds.div_euclid(86400);
    let tod = seconds.rem_euclid(86400);
    let (y, m, d) = civil(day);
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        DAYS[(day + 4).rem_euclid(7) as usize],
        d,
        MONTHS[(m - 1) as usize],
        y,
        tod / 3600,
        tod / 60 % 60,
        tod % 60
    )
}
pub fn parse_http_date(input: &str) -> Option<SystemTime> {
    let b = input.as_bytes();
    if b.len() != 29
        || !input.is_ascii()
        || &b[3..5] != b", "
        || b[7] != b' '
        || b[11] != b' '
        || b[16] != b' '
        || b[19] != b':'
        || b[22] != b':'
        || &b[25..] != b" GMT"
    {
        return None;
    }
    let number = |a: usize, z: usize| -> Option<i64> {
        let s = &input[a..z];
        if !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    let d = number(5, 7)?;
    let y = number(12, 16)?;
    let m = MONTHS.iter().position(|m| *m == &input[8..11])? as i64 + 1;
    let h = number(17, 19)?;
    let min = number(20, 22)?;
    let sec = number(23, 25)?;
    if !(1..=31).contains(&d) || h > 23 || min > 59 || sec > 59 {
        return None;
    }
    let day = days(y, m, d);
    if civil(day) != (y, m, d) || DAYS[(day + 4).rem_euclid(7) as usize] != &input[..3] {
        return None;
    }
    let seconds = day * 86400 + h * 3600 + min * 60 + sec;
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vectors_and_roundtrips() {
        assert_eq!(
            format_http_date(UNIX_EPOCH),
            "Thu, 01 Jan 1970 00:00:00 GMT"
        );
        assert_eq!(
            format_http_date(UNIX_EPOCH + Duration::from_secs(784111777)),
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
        for s in [
            0, 1, 86399, 86400, 784111777, 951782400, 1709164800, 1735689599, 1735689600,
            4107542400,
        ] {
            let time = UNIX_EPOCH + Duration::from_secs(s);
            assert_eq!(parse_http_date(&format_http_date(time)), Some(time));
        }
        assert_eq!(
            format_http_date(UNIX_EPOCH + Duration::from_secs(951782400)),
            "Tue, 29 Feb 2000 00:00:00 GMT"
        );
        assert_eq!(
            format_http_date(UNIX_EPOCH + Duration::from_secs(1709164800)),
            "Thu, 29 Feb 2024 00:00:00 GMT"
        );
    }
    #[test]
    fn invalid() {
        for s in [
            "",
            "garbage",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
            "Sun, 31 Feb 2024 00:00:00 GMT",
            "Thu, 01 Jan 1970 24:00:00 GMT",
            "Thu, 01 Jan 1970 00:00:60 GMT",
            "Fri, 01 Jan 1970 00:00:00 GMT",
        ] {
            assert_eq!(parse_http_date(s), None);
        }
    }
}
