//! Reading an instant, and writing it the way an author asked.
//!
//! A clock reading travels as an ISO 8601 instant with its offset. Which time it
//! *is* is a machine fact, so the components render as written — a display never
//! re-converts, and every screen shows the host's time.
//!
//! ponytail: names and number separators are English. When a theme's language
//! must read in its own words, swap the tables here for an ICU-backed crate.

use jiff::Timestamp;
use jiff::civil::Weekday;
use jiff::tz::TimeZone;

/// The instant's own components, as its offset holder reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Sunday is 0.
    pub weekday: u32,
}

/// Reads an ISO 8601 instant into the components its own offset describes. A
/// value that is not one returns `None`, so the caller shows the raw text rather
/// than inventing a date.
pub fn parse_instant(value: &str) -> Option<Parts> {
    let (date, time) = value.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() {
        return None;
    }

    // Strip the offset, whether it is `Z`, `+HH:MM` or `+HHMM`; it has already
    // done its work by being present, and the components below are wall clock.
    let without_zone = match time.strip_suffix(['Z', 'z']) {
        Some(rest) => rest,
        // An instant without an offset is not one: the components below need
        // the offset that settled them.
        None => &time[..time.rfind(['+', '-']).filter(|at| *at > 0)?],
    };
    let clock = without_zone
        .split_once('.')
        .map_or(without_zone, |(whole, _)| whole);
    let mut clock_parts = clock.split(':');
    let hour: u32 = clock_parts.next()?.parse().ok()?;
    let minute: u32 = clock_parts.next()?.parse().ok()?;
    let second: u32 = match clock_parts.next() {
        Some(seconds) => seconds.parse().ok()?,
        None => 0,
    };
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let weekday = weekday_of(year, month, day)?;
    Some(Parts {
        year,
        month,
        day,
        hour,
        minute,
        second,
        weekday,
    })
}

/// The instant's components as a named zone reads them, so a daylight-saving
/// boundary reads correctly rather than by a fixed offset.
pub fn parts_in_zone(value: &str, time_zone: &str) -> Option<Parts> {
    let instant: Timestamp = value.parse().ok()?;
    let zone = TimeZone::get(time_zone).ok()?;
    let zoned = instant.to_zoned(zone);
    Some(Parts {
        year: i32::from(zoned.year()),
        month: zoned.month() as u32,
        day: zoned.day() as u32,
        hour: zoned.hour() as u32,
        minute: zoned.minute() as u32,
        second: zoned.second() as u32,
        weekday: sunday_zero(zoned.weekday()),
    })
}

fn weekday_of(year: i32, month: u32, day: u32) -> Option<u32> {
    let date = jiff::civil::Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()?;
    Some(sunday_zero(date.weekday()))
}

fn sunday_zero(weekday: Weekday) -> u32 {
    u32::from(weekday.to_monday_one_offset() as u8) % 7
}

/// Renders an instant with the author's tokens; literals pass through.
///
/// Longest token first so `HH` is read before `H` and `MMMM` before `MMM`.
/// Bracketed text is the author's literal: `[Today is ]dddd`. A token this
/// format does not know renders literally rather than blanking the value, so a
/// typo is visible.
pub fn format_instant(value: &str, format: &str, time_zone: Option<&str>) -> Option<String> {
    let parts = match time_zone {
        Some(zone) => parts_in_zone(value, zone)?,
        None => parse_instant(value)?,
    };
    let tokens = tokens();
    let mut out = String::with_capacity(format.len() + 8);
    let mut rest = format;
    while !rest.is_empty() {
        if let Some(literal) = rest.strip_prefix('[') {
            match literal.split_once(']') {
                Some((text, tail)) => {
                    out.push_str(text);
                    rest = tail;
                }
                None => {
                    out.push_str(literal);
                    return Some(out);
                }
            }
            continue;
        }
        match tokens.iter().find(|(name, _)| rest.starts_with(name)) {
            Some((name, render)) => {
                out.push_str(&render(parts));
                rest = &rest[name.len()..];
            }
            None => {
                let character = rest.chars().next().expect("a non-empty remainder");
                out.push(character);
                rest = &rest[character.len_utf8()..];
            }
        }
    }
    Some(out)
}

type Token = (&'static str, fn(Parts) -> String);

fn tokens() -> [Token; 18] {
    [
        ("YYYY", |p| format!("{:04}", p.year)),
        ("YY", |p| format!("{:02}", p.year.rem_euclid(100))),
        ("MMMM", |p| month_name(p.month, true).to_owned()),
        ("MMM", |p| month_name(p.month, false).to_owned()),
        ("MM", |p| format!("{:02}", p.month)),
        ("M", |p| p.month.to_string()),
        ("dddd", |p| weekday_name(p.weekday, true).to_owned()),
        ("ddd", |p| weekday_name(p.weekday, false).to_owned()),
        ("DD", |p| format!("{:02}", p.day)),
        ("D", |p| p.day.to_string()),
        ("HH", |p| format!("{:02}", p.hour)),
        ("H", |p| p.hour.to_string()),
        ("hh", |p| format!("{:02}", hour12(p.hour))),
        ("h", |p| hour12(p.hour).to_string()),
        ("mm", |p| format!("{:02}", p.minute)),
        ("ss", |p| format!("{:02}", p.second)),
        ("A", |p| day_period(p.hour).to_owned()),
        ("a", |p| day_period(p.hour).to_lowercase()),
    ]
}

fn hour12(hour: u32) -> u32 {
    match hour % 12 {
        0 => 12,
        hour => hour,
    }
}

fn day_period(hour: u32) -> &'static str {
    if hour < 12 { "AM" } else { "PM" }
}

const MONTHS_LONG: [&str; 12] = [
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

const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const WEEKDAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

const WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn month_name(month: u32, long: bool) -> &'static str {
    let index = month.saturating_sub(1).min(11) as usize;
    if long {
        MONTHS_LONG[index]
    } else {
        MONTHS_SHORT[index]
    }
}

fn weekday_name(weekday: u32, long: bool) -> &'static str {
    let index = (weekday as usize).min(6);
    if long {
        WEEKDAYS_LONG[index]
    } else {
        WEEKDAYS_SHORT[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instant_reads_as_its_components() {
        let parts = parse_instant("2026-09-24T23:07:09+09:00").expect("an instant");
        assert_eq!(parts.year, 2026);
        assert_eq!(parts.month, 9);
        assert_eq!(parts.day, 24);
        assert_eq!(parts.hour, 23);
        assert_eq!(parts.minute, 7);
        assert_eq!(parts.second, 9);
        assert_eq!(parts.weekday, 4, "Thursday");
    }

    #[test]
    fn a_value_that_is_not_an_instant_is_refused() {
        assert!(parse_instant("2026-09-24T23:07:09").is_none());
        assert!(parse_instant("2026-13-01T00:00:00Z").is_none());
        assert!(parse_instant("2026-02-30T00:00:00Z").is_none());
        assert!(parse_instant("now").is_none());
    }

    #[test]
    fn the_author_tokens_spell_what_they_say() {
        let value = "2026-09-24T14:07:09+07:00";
        assert_eq!(format_instant(value, "HH:mm", None).as_deref(), Some("14:07"));
        assert_eq!(
            format_instant(value, "DD MMM YYYY", None).as_deref(),
            Some("24 Sep 2026")
        );
        assert_eq!(format_instant(value, "dddd", None).as_deref(), Some("Thursday"));
        assert_eq!(
            format_instant(value, "[Today is ]dddd", None).as_deref(),
            Some("Today is Thursday")
        );
        assert_eq!(format_instant(value, "h:mm A", None).as_deref(), Some("2:07 PM"));
    }

    #[test]
    fn a_named_zone_reads_that_zones_wall_clock() {
        let value = "2026-09-24T14:07:09Z";
        assert_eq!(
            format_instant(value, "HH:mm ddd", Some("Asia/Tokyo")).as_deref(),
            Some("23:07 Thu")
        );
    }

    #[test]
    fn an_unknown_zone_has_no_answer_rather_than_a_guess() {
        assert_eq!(format_instant("2026-09-24T14:07:09Z", "HH:mm", Some("Mars/Base")), None);
    }
}
