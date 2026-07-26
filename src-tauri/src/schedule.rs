//! Pure scheduling helpers for the daily auto-update pass (spec §9). Kept free
//! of I/O and `AppHandle` so they unit-test without a running app.

use chrono::{NaiveDate, NaiveTime};

/// Parse an "HH:MM" 24h string into a NaiveTime. None on any malformed input.
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    NaiveTime::from_hms_opt(h, m, 0)
}

/// True when the local clock has reached today's target AND we have not already
/// run today. `target` is "HH:MM"; malformed target → never due (safe).
pub fn due(now_time: NaiveTime, today: NaiveDate, target: &str, last_run: Option<NaiveDate>) -> bool {
    let Some(t) = parse_hhmm(target) else {
        return false;
    };
    if last_run == Some(today) {
        return false;
    }
    now_time >= t
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }
    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn parses_valid_and_rejects_garbage() {
        assert_eq!(parse_hhmm("03:00"), Some(t(3, 0)));
        assert_eq!(parse_hhmm("23:59"), Some(t(23, 59)));
        assert_eq!(parse_hhmm("nope"), None);
        assert_eq!(parse_hhmm("25:00"), None);
        assert_eq!(parse_hhmm("3"), None);
    }

    #[test]
    fn due_only_after_target_and_once_per_day() {
        let today = d(2026, 7, 15);
        // before target → not due
        assert!(!due(t(2, 59), today, "03:00", None));
        // at/after target, not run today → due
        assert!(due(t(3, 0), today, "03:00", None));
        assert!(due(t(9, 0), today, "03:00", None));
        // already ran today → not due even though past target
        assert!(!due(t(9, 0), today, "03:00", Some(today)));
        // ran yesterday → due again today
        assert!(due(t(9, 0), today, "03:00", Some(d(2026, 7, 14))));
        // malformed target → never due
        assert!(!due(t(9, 0), today, "oops", None));
    }
}
