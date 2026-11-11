//! Timestamps of the message list in local time and the interface's
//! language, as web/src/components/messageTime.ts formats them with `Intl`
//! (de-DE / en-GB, through `gelabber_i18n::intl`).

use chrono::{DateTime, Datelike, Local, Timelike, Utc};
use gelabber_i18n::{Locale, chat as texts, intl};

fn local(at: DateTime<Utc>) -> DateTime<Local> {
    at.with_timezone(&Local)
}

fn day_key(at: DateTime<Local>) -> (i32, u32, u32) {
    (at.year(), at.month(), at.day())
}

fn fields(at: DateTime<Local>) -> intl::DateTime {
    intl::DateTime {
        year: at.year(),
        month: at.month(),
        day: at.day(),
        weekday: at.weekday().num_days_from_monday(),
        hour: at.hour(),
        minute: at.minute(),
    }
}

/// A new local calendar day starts between `previous` and `at`.
pub fn day_break(previous: Option<DateTime<Utc>>, at: DateTime<Utc>) -> bool {
    previous.is_none_or(|p| day_key(local(p)) != day_key(local(at)))
}

/// `weekday: long, day: numeric, month: long` (and `year` when asked).
fn long_date(at: DateTime<Local>, year: bool) -> String {
    if year {
        return intl::format_date(&fields(at), intl::DateStyle::Full);
    }
    let weekday = intl::weekday_name(at.weekday().num_days_from_monday(), false);
    let month = intl::month_name(at.month(), false);
    match gelabber_i18n::get() {
        Locale::De => format!("{weekday}, {}. {month}", at.day()),
        Locale::En => format!("{weekday} {} {month}", at.day()),
    }
}

/// "Heute", "Gestern", "Mittwoch, 21. Oktober", "Dienstag, 4. März 2025".
pub fn day_label(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let (at, now) = (local(at), local(now));
    if day_key(at) == day_key(now) {
        return texts::day_today().into();
    }
    let yesterday = now - chrono::Duration::days(1);
    if day_key(at) == day_key(yesterday) {
        return texts::day_yesterday().into();
    }
    long_date(at, at.year() != now.year())
}

/// "14:02" today, "21.10., 14:02" on another day of this year and
/// "21.10.2025, 14:02" before it (in English "21/10, 14:02" and
/// "21/10/2025, 14:02"): the message list and search hits.
pub fn format_when(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let (at, now) = (local(at), local(now));
    let clock = format_clock_local(at);
    if day_key(at) == day_key(now) {
        return clock;
    }
    let (day, month) = (at.day(), at.month());
    match (gelabber_i18n::get(), at.year() == now.year()) {
        (Locale::De, true) => format!("{day:02}.{month:02}., {clock}"),
        (Locale::De, false) => format!("{day:02}.{month:02}.{}, {clock}", at.year()),
        (Locale::En, true) => format!("{day:02}/{month:02}, {clock}"),
        (Locale::En, false) => format!("{day:02}/{month:02}/{}, {clock}", at.year()),
    }
}

fn format_clock_local(at: DateTime<Local>) -> String {
    intl::format_time(&fields(at))
}

/// "14:02", beside a grouped message.
pub fn format_clock(at: DateTime<Utc>) -> String {
    format_clock_local(local(at))
}

/// "Mittwoch, 21. Oktober 2026 um 14:02", for tooltips.
pub fn format_full(at: DateTime<Utc>) -> String {
    let at = local(at);
    texts::time_full(&long_date(at, true), &format_clock_local(at))
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
        assert_eq!(format_when(old, now), "04.03.2025, 08:00");
    }

    #[test]
    fn labels_in_english() {
        let _english = gelabber_i18n::on_this_thread(gelabber_i18n::Locale::En);
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
        assert_eq!(day_label(at(21, 9, 5), now), "Today");
        assert_eq!(day_label(at(20, 9, 5), now), "Yesterday");
        assert_eq!(day_label(at(14, 9, 5), now), "Wednesday 14 October");
        assert_eq!(format_when(at(14, 9, 5), now), "14/10, 09:05");
        assert_eq!(
            format_full(at(21, 14, 2)),
            "Wednesday, 21 October 2026 at 14:02"
        );
        let old = Local
            .with_ymd_and_hms(2025, 3, 4, 8, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(day_label(old, now), "Tuesday, 4 March 2025");
        assert_eq!(format_when(old, now), "04/03/2025, 08:00");
    }
}
