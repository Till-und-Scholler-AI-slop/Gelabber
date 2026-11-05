//! Timestamps of the message list in de-DE and local time, as
//! web/src/components/messageTime.ts formats them with `Intl`.

use chrono::{DateTime, Datelike, Local, Timelike, Utc};

const WEEKDAYS: [&str; 7] = [
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
    "Sonntag",
];
const MONTHS: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];

fn local(at: DateTime<Utc>) -> DateTime<Local> {
    at.with_timezone(&Local)
}

fn day_key(at: DateTime<Local>) -> (i32, u32, u32) {
    (at.year(), at.month(), at.day())
}

/// A new local calendar day starts between `previous` and `at`.
pub fn day_break(previous: Option<DateTime<Utc>>, at: DateTime<Utc>) -> bool {
    previous.is_none_or(|p| day_key(local(p)) != day_key(local(at)))
}

fn long_date(at: DateTime<Local>, year: bool) -> String {
    let weekday = WEEKDAYS[at.weekday().num_days_from_monday() as usize];
    let month = MONTHS[at.month0() as usize];
    if year {
        format!("{weekday}, {}. {month} {}", at.day(), at.year())
    } else {
        format!("{weekday}, {}. {month}", at.day())
    }
}

/// "Heute", "Gestern", "Mittwoch, 21. Oktober", "Dienstag, 4. März 2025".
pub fn day_label(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let (at, now) = (local(at), local(now));
    if day_key(at) == day_key(now) {
        return "Heute".into();
    }
    let yesterday = now - chrono::Duration::days(1);
    if day_key(at) == day_key(yesterday) {
        return "Gestern".into();
    }
    long_date(at, at.year() != now.year())
}

/// "14:02" today, "21.10., 14:02" on another day.
pub fn format_when(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let (at, now) = (local(at), local(now));
    if day_key(at) == day_key(now) {
        format_clock_local(at)
    } else {
        format!(
            "{:02}.{:02}., {}",
            at.day(),
            at.month(),
            format_clock_local(at)
        )
    }
}

fn format_clock_local(at: DateTime<Local>) -> String {
    format!("{:02}:{:02}", at.hour(), at.minute())
}

/// "14:02", beside a grouped message.
pub fn format_clock(at: DateTime<Utc>) -> String {
    format_clock_local(local(at))
}

/// "Mittwoch, 21. Oktober 2026 um 14:02", for tooltips.
pub fn format_full(at: DateTime<Utc>) -> String {
    let at = local(at);
    format!("{} um {}", long_date(at, true), format_clock_local(at))
}

/// `toLocaleString("de-DE")`: "21.10.2026, 14:02:00".
pub fn format_search(at: DateTime<Utc>) -> String {
    let at = local(at);
    format!(
        "{}.{}.{}, {:02}:{:02}:{:02}",
        at.day(),
        at.month(),
        at.year(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn labels() {
        let now = Local
            .with_ymd_and_hms(2026, 10, 21, 18, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let at = |d, h, m| {
            Local
                .with_ymd_and_hms(2026, 10, d, h, m, 0)
                .unwrap()
                .with_timezone(&Utc)
        };
        assert_eq!(day_label(at(21, 9, 5), now), "Heute");
        assert_eq!(day_label(at(20, 9, 5), now), "Gestern");
        assert_eq!(day_label(at(14, 9, 5), now), "Mittwoch, 14. Oktober");
        assert_eq!(format_when(at(21, 9, 5), now), "09:05");
        assert_eq!(format_when(at(14, 9, 5), now), "14.10., 09:05");
        assert_eq!(
            format_full(at(21, 14, 2)),
            "Mittwoch, 21. Oktober 2026 um 14:02"
        );
        let old = Local
            .with_ymd_and_hms(2025, 3, 4, 8, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(day_label(old, now), "Dienstag, 4. März 2025");
    }
}
