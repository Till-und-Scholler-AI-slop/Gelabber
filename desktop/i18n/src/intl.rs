//! Locale-bound formatting in the interface's language, as
//! web/src/i18n/format.ts does it with `Intl` in `de-DE` and `en-GB`:
//! dates, times, relative times and numbers. Every date, time and number
//! people read goes through here instead of spelling out a language
//! itself. Names sort by `gelabber_client::store::collation_key`.
//!
//! The functions take the calendar fields, not a time type, so this crate
//! stays without dependencies; the caller converts to local time first.

use crate::{Locale, get};

/// A local date and time, as the caller's calendar has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateTime {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    /// 1 to 31.
    pub day: u32,
    /// 0 for Monday to 6 for Sunday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

/// The `Intl.DateTimeFormat` options the web uses for a date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateStyle {
    /// `formatDate`'s default (`day: numeric, month: long, year: numeric`):
    /// "5. November 2026" / "5 November 2026".
    Long,
    /// `dateStyle: "medium"`: "05.11.2026" / "5 Nov 2026".
    Medium,
    /// `dateStyle: "full"`: "Donnerstag, 5. November 2026" /
    /// "Thursday, 5 November 2026".
    Full,
}

const MONTHS_DE: [&str; 12] = [
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
const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_SHORT_DE: [&str; 12] = [
    "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez.",
];
const MONTHS_SHORT_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sept", "Oct", "Nov", "Dec",
];
const WEEKDAYS_DE: [&str; 7] = [
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
    "Sonntag",
];
const WEEKDAYS_EN: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const WEEKDAYS_SHORT_DE: [&str; 7] = ["Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa.", "So."];
const WEEKDAYS_SHORT_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// `month: "long"` (`short`: `month: "short"`), 1 to 12.
pub fn month_name(month: u32, short: bool) -> &'static str {
    let i = (month.clamp(1, 12) - 1) as usize;
    match (get(), short) {
        (Locale::De, false) => MONTHS_DE[i],
        (Locale::De, true) => MONTHS_SHORT_DE[i],
        (Locale::En, false) => MONTHS_EN[i],
        (Locale::En, true) => MONTHS_SHORT_EN[i],
    }
}

/// `weekday: "long"` (`short`: `weekday: "short"`), 0 for Monday.
pub fn weekday_name(weekday: u32, short: bool) -> &'static str {
    let i = weekday.min(6) as usize;
    match (get(), short) {
        (Locale::De, false) => WEEKDAYS_DE[i],
        (Locale::De, true) => WEEKDAYS_SHORT_DE[i],
        (Locale::En, false) => WEEKDAYS_EN[i],
        (Locale::En, true) => WEEKDAYS_SHORT_EN[i],
    }
}

/// `formatDate`: a date without the time.
pub fn format_date(at: &DateTime, style: DateStyle) -> String {
    let (day, month, year) = (at.day, at.month, at.year);
    match (get(), style) {
        (Locale::De, DateStyle::Long) => format!("{day}. {} {year}", month_name(month, false)),
        (Locale::En, DateStyle::Long) => format!("{day} {} {year}", month_name(month, false)),
        (Locale::De, DateStyle::Medium) => format!("{day:02}.{month:02}.{year}"),
        (Locale::En, DateStyle::Medium) => format!("{day} {} {year}", month_name(month, true)),
        (Locale::De, DateStyle::Full) => format!(
            "{}, {day}. {} {year}",
            weekday_name(at.weekday, false),
            month_name(month, false)
        ),
        (Locale::En, DateStyle::Full) => format!(
            "{}, {day} {} {year}",
            weekday_name(at.weekday, false),
            month_name(month, false)
        ),
    }
}

/// `formatTime`'s default (`hour` and `minute` 2-digit): "14:30" in both
/// languages, which count the hours to 24.
pub fn format_time(at: &DateTime) -> String {
    format!("{:02}:{:02}", at.hour, at.minute)
}

/// The units of `Intl.RelativeTimeFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl Unit {
    /// German one/other in the dative ("in 3 Tagen", "vor 1 Tag"), English
    /// one/other.
    fn words(self, locale: Locale) -> (&'static str, &'static str) {
        match (locale, self) {
            (Locale::De, Self::Second) => ("Sekunde", "Sekunden"),
            (Locale::De, Self::Minute) => ("Minute", "Minuten"),
            (Locale::De, Self::Hour) => ("Stunde", "Stunden"),
            (Locale::De, Self::Day) => ("Tag", "Tagen"),
            (Locale::De, Self::Week) => ("Woche", "Wochen"),
            (Locale::De, Self::Month) => ("Monat", "Monaten"),
            (Locale::De, Self::Year) => ("Jahr", "Jahren"),
            (Locale::En, Self::Second) => ("second", "seconds"),
            (Locale::En, Self::Minute) => ("minute", "minutes"),
            (Locale::En, Self::Hour) => ("hour", "hours"),
            (Locale::En, Self::Day) => ("day", "days"),
            (Locale::En, Self::Week) => ("week", "weeks"),
            (Locale::En, Self::Month) => ("month", "months"),
            (Locale::En, Self::Year) => ("year", "years"),
        }
    }
}

/// `formatRelative` with `numeric: "always"`: "in 3 Tagen" / "in 3 days",
/// "vor 1 Stunde" / "1 hour ago".
pub fn format_relative(value: i64, unit: Unit) -> String {
    let locale = get();
    let n = value.unsigned_abs();
    let (one, other) = unit.words(locale);
    let amount = format!(
        "{} {}",
        format_number(n as f64, 0, 0),
        crate::plural(n, one, other)
    );
    match (locale, value < 0) {
        (Locale::De, false) => format!("in {amount}"),
        (Locale::De, true) => format!("vor {amount}"),
        (Locale::En, false) => format!("in {amount}"),
        (Locale::En, true) => format!("{amount} ago"),
    }
}

/// A count in a text, grouped as the web's catalogs format a `{count}`:
/// "3.963" / "3,963".
pub fn count(count: u64) -> String {
    let group = match get() {
        Locale::De => '.',
        Locale::En => ',',
    };
    let digits = count.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(group);
        }
        out.push(digit);
    }
    out
}

/// `formatNumber` with `minimumFractionDigits` and
/// `maximumFractionDigits`: "1.234,5" / "1,234.5". Rounds half away from
/// zero, as `Intl` does.
pub fn format_number(value: f64, min_fraction: usize, max_fraction: usize) -> String {
    let max_fraction = max_fraction.max(min_fraction);
    let (group, decimal) = match get() {
        Locale::De => ('.', ','),
        Locale::En => (',', '.'),
    };
    if !value.is_finite() {
        return value.to_string();
    }
    let (whole, fraction) = round_decimal(&value.abs().to_string(), max_fraction);
    let whole = whole.as_str();
    let mut fraction = fraction.trim_end_matches('0').to_owned();
    while fraction.len() < min_fraction {
        fraction.push('0');
    }
    let mut out = String::new();
    // A negative number that rounds to zero stays "-0", as in `Intl`.
    if value < 0.0 {
        out.push('-');
    }
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            out.push(group);
        }
        out.push(digit);
    }
    if !fraction.is_empty() {
        out.push(decimal);
        out.push_str(&fraction);
    }
    out
}

/// Rounds a plain decimal (`f64`'s shortest `Display`, never an exponent)
/// to `places` fraction digits, half away from zero on the digits as
/// written, as `Intl` does: 1.15 is 1.2, though the float is a hair below.
fn round_decimal(text: &str, places: usize) -> (String, String) {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let up = fraction.as_bytes().get(places).is_some_and(|d| *d >= b'5');
    let mut digits: Vec<u8> = whole
        .bytes()
        .chain(fraction.bytes().chain(std::iter::repeat(b'0')).take(places))
        .collect();
    if up {
        let mut carry = true;
        for d in digits.iter_mut().rev() {
            if *d == b'9' {
                *d = b'0';
            } else {
                *d += 1;
                carry = false;
                break;
            }
        }
        if carry {
            digits.insert(0, b'1');
        }
    }
    let split = digits.len() - places;
    let fraction = String::from_utf8(digits.split_off(split)).unwrap_or_default();
    (String::from_utf8(digits).unwrap_or_default(), fraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::on_this_thread;

    const THURSDAY: DateTime = DateTime {
        year: 2026,
        month: 11,
        day: 5,
        weekday: 3,
        hour: 9,
        minute: 7,
    };

    #[test]
    fn dates_as_de_de_and_en_gb_write_them() {
        let _de = on_this_thread(Locale::De);
        assert_eq!(format_date(&THURSDAY, DateStyle::Long), "5. November 2026");
        assert_eq!(format_date(&THURSDAY, DateStyle::Medium), "05.11.2026");
        assert_eq!(
            format_date(&THURSDAY, DateStyle::Full),
            "Donnerstag, 5. November 2026"
        );
        assert_eq!(format_time(&THURSDAY), "09:07");
        assert_eq!(month_name(3, true), "März");
        assert_eq!(weekday_name(2, true), "Mi.");
        let _en = on_this_thread(Locale::En);
        assert_eq!(format_date(&THURSDAY, DateStyle::Long), "5 November 2026");
        assert_eq!(format_date(&THURSDAY, DateStyle::Medium), "5 Nov 2026");
        assert_eq!(
            format_date(&THURSDAY, DateStyle::Full),
            "Thursday, 5 November 2026"
        );
        assert_eq!(format_time(&THURSDAY), "09:07");
        assert_eq!(month_name(9, true), "Sept");
    }

    #[test]
    fn relative_times_count_both_ways() {
        let _de = on_this_thread(Locale::De);
        assert_eq!(format_relative(3, Unit::Day), "in 3 Tagen");
        assert_eq!(format_relative(1, Unit::Day), "in 1 Tag");
        assert_eq!(format_relative(-1, Unit::Hour), "vor 1 Stunde");
        assert_eq!(format_relative(2, Unit::Month), "in 2 Monaten");
        let _en = on_this_thread(Locale::En);
        assert_eq!(format_relative(3, Unit::Day), "in 3 days");
        assert_eq!(format_relative(1, Unit::Week), "in 1 week");
        assert_eq!(format_relative(-2, Unit::Minute), "2 minutes ago");
    }

    #[test]
    fn numbers_group_and_round_by_language() {
        let _de = on_this_thread(Locale::De);
        assert_eq!(format_number(1234.5, 0, 1), "1.234,5");
        assert_eq!(format_number(1234567.0, 0, 0), "1.234.567");
        assert_eq!(format_number(0.25, 0, 1), "0,3");
        assert_eq!(format_number(2.0, 1, 1), "2,0");
        assert_eq!(format_number(-3.04, 0, 1), "-3");
        assert_eq!(format_number(-0.01, 0, 1), "-0");
        let _en = on_this_thread(Locale::En);
        assert_eq!(format_number(1234.5, 0, 1), "1,234.5");
        assert_eq!(format_number(999.0, 0, 2), "999");
        assert_eq!(format_number(12.346, 2, 2), "12.35");
        // Halves round on the decimal digits, not on the float's.
        assert_eq!(format_number(1.15, 0, 1), "1.2");
        assert_eq!(format_number(0.35, 0, 1), "0.4");
        assert_eq!(format_number(0.145, 0, 2), "0.15");
        assert_eq!(format_number(1.005, 0, 2), "1.01");
        assert_eq!(format_number(2.675, 0, 2), "2.68");
        assert_eq!(format_number(999.96, 0, 1), "1,000");
        assert_eq!(format_number(1_150_000.0, 0, 0), "1,150,000");
    }

    /// Counts are grouped by thousands, as `Intl.NumberFormat` does for
    /// the web's `{count}`.
    #[test]
    fn counts_group_by_thousands() {
        let _de = on_this_thread(Locale::De);
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(3963), "3.963");
        assert_eq!(count(1_234_567), "1.234.567");
        assert_eq!(crate::emoji::picker_found(3963), "3.963 Emojis gefunden");
        assert_eq!(crate::emoji::picker_found(12), "12 Emojis gefunden");
        assert_eq!(
            crate::chat::composer_chars_left(1200),
            "1.200 Zeichen übrig"
        );
        assert_eq!(crate::servers::overview_members(1), "1 Mitglied");
        assert_eq!(
            crate::server_settings::members_title(1500),
            "Mitglieder (1.500)"
        );
        let _en = on_this_thread(Locale::En);
        assert_eq!(count(3963), "3,963");
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(crate::emoji::picker_found(3963), "3,963 emoji found");
        assert_eq!(
            crate::chat::composer_chars_over(1200),
            "1,200 characters too many"
        );
        assert_eq!(crate::auth::invite_members(1), "1 member");
    }
}
